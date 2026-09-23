//! 封盖裁定：防守方在出手飞行中触及球的事实（D25）。
//!
//! ## 为什么封盖是独立模块
//!
//! 它与出手裁决（`execution.rs` 的命中率掷骰）和弹道推进（`ball_flight`）都是
//! 不同的关注点：出手裁决决定「若不封盖会怎样」，弹道推进决定「球现在在哪」，
//! 而本模块决定「防守方是否在正确的时机触及了球」。把它塞进任一现有模块都会让
//! 那个模块同时承担两种判定口径。
//!
//! ## 判定窗口绑定动作阶段（`architecture.md` §6、`action_window.rs`）
//!
//! 合法封盖只在出手者的 `ActionPhase::Execution`（起跳上升与出手瞬间）内成立。
//! 更早的 `Preparation` 是**切球/Strip** 的窗口（`action_window.rs` 明言
//! 「防守人可尝试切球抢断，不计投篮犯规」），它已有自己的处理链
//! （`on_ball_poke_phase` 的 `Held → LooseBall`），不应在此重复计为封盖；
//! 更晚的 `FollowThrough` 球已离手，不存在封盖。
//!
//! ## 能力消费（`attributes.md` §2.4）
//!
//! 封盖概率消费 `block`（手法与时机）与 `vertical` + 身高（触及高度），
//! 对照出手者的 `release_height`（出手点越高越难封）。这使 `block` 与
//! `vertical` 两维都有生产消费点。

use glam::Vec2;
use nba_domain::action_window::{ActionPhase, ActionTimeWindow};
use nba_domain::{GameEvent, Possession};
use nba_physics::ballistics::BallTrajectoryKind;
use rand::Rng;

use super::MatchEngine;

fn is_block_phase(phase: ActionPhase) -> bool {
    matches!(phase, ActionPhase::Execution)
}

/// 一次封盖的裁定结果。
pub(crate) struct BlockOutcome {
    /// 封盖后球的新状态（松球，由双方争夺）。
    pub next_state: BallTrajectoryKind,
    /// 发布的比赛事实。
    pub event: GameEvent,
}

impl MatchEngine {
    /// 封盖入射速度：当前 Shot 球态的投篮飞行水平速度（出手点到筐距离
    /// / 飞行时长）。与弹道采样同一对冻结参数（`from_pos`/`hoop_pos`/
    /// `duration`），非当前球位差分（那是采样点，随时长变化）。无 Shot
    /// 上下文时退回零速度（调用方 clamp 后取保底 1.0 ft/s）。
    fn shot_flight_horizontal_speed(&self, shooter_id: &str) -> f32 {
        match &self.ball.ball_state {
            BallTrajectoryKind::Shot {
                from_pos,
                hoop_pos,
                duration,
                shooter_id: state_shooter,
                ..
            } if state_shooter == shooter_id => {
                (*hoop_pos - *from_pos).length() / duration.max(f32::EPSILON)
            }
            _ => 0.0,
        }
    }

    /// 尝试裁定一次封盖。返回 `None` 表示本次飞行中没有防守方触及球。
    ///
    /// 调用方（`resolve_ball_flight` 的 `Shot` 分支）已确认本次出手尚未被封盖，
    /// 且处在释放后的可干扰区间内（每发只调一次）。
    pub(crate) fn try_resolve_shot_block(
        &mut self,
        shooter_id: &str,
        from_pos: &Vec2,
        current_t: f32,
        would_have_made: bool,
        is_three: bool,
    ) -> Option<BlockOutcome> {
        // 1. 出手者的动作阶段必须是合法的封盖窗口。
        //
        // 窗口由出手者自己的 `ActionTimeWindow` 决定，而不是另立一套计时：
        // 同一个动作的两处判定必须共享同一份阶段定义。
        //
        // ## 为何只接受 `Execution` 而不接受 `Preparation`
        //
        // `Preparation`（合球阶段）在 `action_window.rs` 里是**切球（Strip）**的
        // 窗口，并已由 `on_ball_poke_phase` 的实现链负责（`Held → LooseBall`）。
        // 把它也算作封盖会产生两个口径重叠的事实。
        let phase = self.observations.active_windows.get(shooter_id)?.phase;
        if !is_block_phase(phase) {
            return None;
        }
        // `Preparation` 末段（合球完成、起跳前）与 `Execution` 全过程都属于
        // `action_window.rs` 声明的可干扰区间。
        //
        // `interference_start` / `interference_end` 是**绝对时间**
        // （构造时由 `start_time + offset` 得出），因此与 `current_t` 直接比较，
        // 不先减 `start_time`——把绝对的窗口边界当成相对耗时比较会让判定
        // 永远不成立（0.16 >= 9.62 恒假，实测封盖数恒为 0）。
        let interference_ok = self
            .observations
            .active_windows
            .get(shooter_id)
            .map(|window: &ActionTimeWindow| {
                current_t >= window.interference_start && current_t <= window.interference_end
            })
            .unwrap_or(false);
        if !interference_ok {
            return None;
        }

        let policy = self.config.rules.resolve.block.clone();
        // 2. 找出手者附近最近的防守人。
        let shooter_pos = self
            .systems
            .physics
            .get_player(shooter_id)
            .map(|p| p.pos_ft)
            .unwrap_or(*from_pos);
        let shooter_team = self
            .systems
            .physics
            .get_player(shooter_id)
            .map(|p| p.team.clone())?;
        let mut candidate: Option<(String, f32)> = None;
        for (id, player) in self.systems.physics.get_players() {
            if !player.on_court || id == shooter_id || player.team == shooter_team {
                continue;
            }
            let dist = (player.pos_ft - shooter_pos).length();
            if dist > policy.contest_radius_ft {
                continue;
            }
            match &candidate {
                Some((_, best)) if dist >= *best => {}
                _ => candidate = Some((id.clone(), dist)),
            }
        }
        let (blocker_id, _block_distance) = candidate?;
        let blocker = self.systems.physics.get_player(&blocker_id)?.clone();
        // 身高是量纲事实（cm），只在档案里；从两队名册按 id 查。
        let blocker_height_cm = self
            .config
            .home_team
            .players
            .iter()
            .chain(self.config.away_team.players.iter())
            .find(|p| p.id == blocker_id)
            .map(|p| p.height_cm)
            .unwrap_or(200);

        // 3. 概率：手法（`block`）× 触及高度（`vertical` + 身高）− 出手点高度。
        //
        // 身高是量纲事实（cm），先换成英尺再归一到“臂展高度因子”（以 7 ft
        // 为参考）：不能直接拿英尺数值乘权重，否则 6.5 ft × 0.08 就把概率顶到
        // 上限，属性差异被压平。
        const CM_PER_FOOT: f64 = 30.48;
        const REACH_REFERENCE_FT: f32 = 7.0;
        let height_ft = (f64::from(blocker_height_cm) / CM_PER_FOOT) as f32;
        let height_factor = (height_ft / REACH_REFERENCE_FT).clamp(0.0, 1.0);
        let height_weight = policy.reach_height_weight;
        let reach =
            height_factor * height_weight + blocker.attributes.vertical * (1.0 - height_weight);
        let skill = policy.base_probability
            + blocker.attributes.block * policy.blocker_skill_weight
            + reach * policy.blocker_reach_weight;
        // 出手点越高越难封：用出手者在飞行中的球高度作为参考。
        let release_height = self.ball.ball_pos_3d.1;
        let probability = (skill - release_height * policy.release_height_penalty_per_ft)
            .clamp(f32::EPSILON, policy.probability_ceiling);
        let blocked = self.systems.rng.gen_bool(f64::from(probability));
        if !blocked {
            return None;
        }

        // 4. 封盖事实：球被打向远离篮筐的方向，成为双方争夺的松球。
        let contact_height_ft = self.ball.ball_pos_3d.1;
        let ball_pos = self.ball.ball_pos_3d;
        let away = (ball_pos.0 - shooter_pos).normalize_or_zero();
        let deflection = if away.length_squared() > f32::EPSILON {
            away
        } else {
            Vec2::new(-1.0, 0.0)
        };
        // 封盖初速从接触推导（ADR-017 第三步）：入射 = 投篮飞行的水平
        // 速度（出手点到筐的距离 / 飞行时长，与 Shot 采样同一对参数），
        // 弹出 = 入射 × block_restitution。能量从入射中来：远投被封后
        // 扇得远，近投被封后弹得近。速度 clamp 球速包络（保 BALL_SPEED）。
        let shot_horizontal_speed = self.shot_flight_horizontal_speed(shooter_id);
        let cap = (self.config.rules.ball_max_speed_ftps
            - self.config.rules.invariant_speed_tolerance_ftps)
            .max(self.config.rules.invariant_speed_tolerance_ftps);
        let speed = (shot_horizontal_speed * policy.block_restitution).clamp(1.0, cap);
        let last_touch_team = match self.flow.possession {
            Possession::Home => Possession::Away,
            Possession::Away => Possession::Home,
        };
        Some(BlockOutcome {
            next_state: BallTrajectoryKind::LooseBall {
                pos: ball_pos.0,
                vel: deflection * speed,
                z: contact_height_ft,
                vel_z: 0.0,
                last_touch_team,
                // 物理最后触球人是封盖者。
                last_touch_player: Some(blocker_id.clone()),
            },
            event: GameEvent::BlockedShot {
                shooter_id: shooter_id.to_string(),
                blocker_id,
                ball_pos: (ball_pos.0.x, ball_pos.0.y, contact_height_ft),
                contact_height_ft,
                phase,
                would_have_made,
                is_three,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::is_block_phase;
    use nba_domain::action_window::ActionPhase;

    #[test]
    fn block_gate_rejects_preparation_and_follow_through() {
        assert!(!is_block_phase(ActionPhase::Preparation));
        assert!(!is_block_phase(ActionPhase::FollowThrough));
    }

    #[test]
    fn block_gate_accepts_execution() {
        assert!(is_block_phase(ActionPhase::Execution));
    }
}
