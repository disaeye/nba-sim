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

/// 一次封盖的裁定结果。
pub(crate) struct BlockOutcome {
    /// 封盖后球的新状态（松球，由双方争夺）。
    pub next_state: BallTrajectoryKind,
    /// 发布的比赛事实。
    pub event: GameEvent,
}

impl MatchEngine {
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
        if !matches!(phase, ActionPhase::Execution) {
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
        let speed =
            (self.config.rules.ball_max_speed_ftps * policy.deflection_speed_ratio).max(1.0);
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
