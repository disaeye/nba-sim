//! 弹道裁决的单臂实现：按球态种类各占一节。
//!
//! 划分依据（D29 行数门 + §10.2 搬移纪律）：`resolve_ball_flight` 的总
//! match 保留在 `mod.rs` 作薄调度；每个臂的执行体按球态搬到这里。臂体
//! 只通过「载荷参数 + 上下文 + 累加器」与调度器交互：
//!
//! - 载荷参数：臂内需要的 `BallTrajectoryKind` 字段（从总 match 的借用中
//!   clone/复制出来，与臂内原有纪律一致——match 持不可变借用，臂内要
//!   `&mut self` 就必须先取出所需值）；
//! - 上下文：[`FlightContext`]（tick 时间、进攻方向、步长）；
//! - 累加器：[`BallFlightOutcome`]（调度器按优先级消费）。
//!
//! 搬移保真：臂体逐行搬移，任何一行行为改动都会被黄金哈希抓住。

use glam::Vec2;
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};

use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_semantics::SemanticEvaluator;
use rand::Rng;

use super::BallFlightOutcome;
use super::MatchEngine;

/// 弹道裁决的逐 tick 上下文（调度器传入，臂内只读）。
#[derive(Debug, Clone, Copy)]
pub(crate) struct FlightContext {
    /// 当前仿真墙钟时间（秒）。
    pub current_t: f32,
    /// 进攻方向是否主队（篮筐几何与得分归属依赖）。
    pub is_home: bool,
    /// 本 tick 步长（秒）。
    pub dt: f32,
}

fn clamp_unit(value: f32) -> f32 {
    value.clamp(f32::from(0u8), f32::from(1u8))
}

impl MatchEngine {
    /// `Pass` 臂：接触检测、接球/点掉/坠地裁决。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_pass_arm(
        &mut self,
        target_id: String,
        start_time: f32,
        duration: f32,
        from_pos: Vec2,
        to_pos: Vec2,
        inbound: bool,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let is_inbound_pass = inbound;
        let tau = ((ctx.current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
        let segment = to_pos - from_pos;
        let horizontal_speed_ftps = self
            .ball
            .prev_observed_ball_pos
            .map(|previous| {
                (self.ball.ball_pos_3d.0 - previous).length()
                    / self.config.rules.tick_seconds.max(f32::EPSILON)
            })
            .unwrap_or_else(|| segment.length() / duration.max(f32::EPSILON));
        // match 持有 `&self.ball.ball_state` 的不可变借用，而接触检测
        // 需要 `&mut self`（连续接触状态与掷骰）：先取出所需字段再调用。
        let receiver_id = target_id;
        let frozen_to_pos = to_pos;
        let contact =
            self.resolve_pass_contact(&receiver_id, ctx.is_home, ctx.dt, horizontal_speed_ftps);
        let passer_id = self.ball.last_passer_id.clone().unwrap_or_default();
        // Fix（round-15）：接球是「首次触球」事件。旧实现只在 tau≥1
        // 裁决——接球人估计点随自身移动后退（追赶曲线），「朝来球
        // 移动」的正确行为反而必然导致终点 miss（22 例层 A 失败）。
        // 触发条件放宽为「飞行结束或已进入接球半径」，裁决逻辑复用。
        //
        // 接触优先于接球：防守者先碰到球，球就到不了接球人手里。
        let receiver_touch = self
            .systems
            .physics
            .get_player(&receiver_id)
            .map(|r| {
                let radius = nba_domain::effective_catch_radius(&self.config.rules, &r.attributes);
                (r.pos_ft - self.ball.ball_pos_3d.0).length() <= radius
            })
            .unwrap_or(false);
        if let Some((defender_id, secured)) = contact {
            let position = self.ball.ball_pos_3d.0;
            if secured {
                self.journal
                    .pending_events
                    .push(GameEvent::PassIntercepted {
                        passer_id,
                        receiver_id: receiver_id.clone(),
                        defender_id: defender_id.clone(),
                        position: (position.x, position.y),
                    });
                // 拦截成立时把球锚定到抢断者身上：`Held{stealer}`
                // 要求球与持球人一致（否则 BALL_WITH_HOLDER）。
                // 拦截点与抢断者的距离可能达 reach（1.8+ 可达 ft），
                // 直接沿用球坐标会造成球人分离（实测 3.86 ft）。
                self.ball.ball_pos_3d = (position, self.config.rules.ball_holder_height_ft);

                out.steal_triggered_defender = Some(defender_id);
            } else {
                let tipped_by = defender_id.clone();
                self.journal.pending_events.push(GameEvent::PassTipped {
                    passer_id,
                    receiver_id,
                    defender_id,
                    position: (position.x, position.y),
                });
                if is_inbound_pass {
                    out.live_ball_triggered = true;
                }
                self.ball.pending_loose_ball_terminal =
                    Some(nba_domain::PossessionEndCause::TurnoverPassTipped);
                out.new_ball_state = Some(BallTrajectoryKind::LooseBall {
                    pos: position,
                    // 同上：点掉的松球初速也须收敛到球速上限。
                    vel: {
                        let dir = segment.normalize_or_zero();
                        let cap = (self.config.rules.ball_max_speed_ftps
                            - self.config.rules.invariant_speed_tolerance_ftps)
                            .max(self.config.rules.invariant_speed_tolerance_ftps);
                        dir * cap
                    },
                    z: self.ball.ball_pos_3d.1,
                    vel_z: 0.0,
                    last_touch_team: self.flow.possession,
                    // 物理最后触球人是点球的防守人（进攻方责任人
                    // 由 pending_loose_ball_terminal 链路归因）。
                    last_touch_player: Some(tipped_by),
                });
            }
        } else if tau >= 1.0 || receiver_touch {
            // 层 A（P-1）：球到达时判定**实际空间接近度**。
            //
            // 旧口径是「接球人距**冻结点** ≤ leash」—— 那等于用
            // 传球人的意图当判据，接球人只要站在他该在的地方就算成功，
            // 与他是否真在球旁边无关。实测因此产生 9 条
            // `PASS_CORRIDOR_REACHABLE` Hard。
            //
            // 新口径（层 A）：用**球与接球人的实际距离**对比
            // `effective_catch_radius`。
            //
            // ## 层序修正（round-10）
            //
            // 旧实现把层 B（`will_receive`，release 时的**位置无关**
            // 概率掷骰）放在**外层**，层 A 放内层：掷到 false 就直接
            // 判掉球，层 A 连执行机会都没有。后果（实测）：接球人
            // 站在球旁边（甚至 0 ft）也会"接不到"；233 次 drop 中
            // 球**全都精确到达冻结接球点**（d=0.00），而接球人距球
            // 中位 6.37 ft —— 但这是层 B 先否决后才产生的位移，不是原因。
            //
            // 正确的因果顺序（真实篮球）：
            //   位置决定**能否到达球**（层 A，确定性）
            //   → 技术/干扰决定**接得稳不稳**（层 B，概率）
            // 因此层 A 必须在**外层**：不可达则直接 loose ball，
            // 层 B 只在可达时生效。
            let catch_radius = self
                .systems
                .physics
                .get_player(&receiver_id)
                .map(|r| nba_domain::effective_catch_radius(&self.config.rules, &r.attributes))
                .unwrap_or(self.config.rules.player_radius_ft);
            let ball_to_receiver = self
                .systems
                .physics
                .get_player(&receiver_id)
                .map(|r| (r.pos_ft - self.ball.ball_pos_3d.0).length())
                .unwrap_or(f32::MAX);
            // 层 A：物理可达（确定性）；层 B：接稳（概率，在到达时判定）。
            let estimated_landing = self.estimate_receiver_landing(&receiver_id, frozen_to_pos);
            let passer_id_for_catch = self.ball.last_passer_id.clone().unwrap_or_default();
            let catch_secure = ball_to_receiver <= catch_radius
                && self.resolve_pass_success(
                    &passer_id_for_catch,
                    &receiver_id,
                    from_pos,
                    estimated_landing,
                );
            let receiver_ready = catch_secure;
            if receiver_ready {
                // 接球事实的位置是球的实际到达位置（round-6/round-10）：
                // 层 A 按实际距离判定后球可能距冻结点数英尺，瞬移到
                // to_pos 会伪造飞行终点并触发 BALL_WITH_HOLDER 越 leash
                //（seed 5 tick 14338）。收到接球人身上（持球锚点）。
                let catch_spot = self.ball.ball_pos_3d.0;
                let catch_height = self.ball.ball_pos_3d.1;
                assert!(
                    (self
                        .systems
                        .physics
                        .get_player(&receiver_id)
                        .map(|receiver| (receiver.pos_ft - catch_spot).length())
                        .unwrap_or(f32::MAX))
                        <= catch_radius,
                    "successful pass catch must occur within the catch radius"
                );
                // 发布接球点修正事实（层 A，P-1）：当实际到达位置与
                // 传球人冻结的意图不同时，把差异登记为事实，使
                // 事实账本自洽（不允许下游各自解释同一传球）。
                let divergence = (catch_spot - frozen_to_pos).length();
                if divergence > f32::EPSILON {
                    self.journal
                        .pending_events
                        .push(GameEvent::PassLandingCorrected {
                            receiver_id: receiver_id.clone(),
                            intended: (frozen_to_pos.x, frozen_to_pos.y),
                            actual: (catch_spot.x, catch_spot.y),
                            divergence_ft: divergence,
                        });
                }
                let is_cut_reception =
                    self.is_cut_reception(&receiver_id, catch_spot, ctx.current_t);
                self.journal.pending_events.push(GameEvent::PassReceived {
                    receiver_id: receiver_id.clone(),
                    position: (catch_spot.x, catch_spot.y),
                    is_cut_reception,
                });
                if is_cut_reception {
                    self.possession_ctx.last_cut_reception_time = Some(ctx.current_t);
                    self.possession_ctx.last_cut_reception_player = Some(receiver_id.clone());
                }
                self.ball.pending_pass_inbound = false;
                if is_inbound_pass {
                    self.transition_phase(SubPhase::Initiation);
                    self.set_game_flow(GameFlowState::LiveBall);
                    self.clock.sub_phase_timer = 0.0;
                    self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
                }
                let carrier_id = receiver_id.clone();
                let held_target = BallisticsEngine::sample_ball_position(
                    &BallTrajectoryKind::Held {
                        carrier_id: carrier_id.clone(),
                    },
                    ctx.current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );
                let vertical_distance = (held_target.1 - catch_height).abs();
                let lateral_distance = (held_target.0 - catch_spot).length();
                let speed_budget = (self.config.rules.ball_max_speed_ftps
                    - self.config.rules.invariant_speed_tolerance_ftps)
                    .max(self.config.rules.invariant_speed_tolerance_ftps);
                let horizontal_distance = lateral_distance
                    + self.config.rules.ball_max_speed_ftps
                        * (self.config.rules.tick_seconds - f32::EPSILON);
                let transfer_duration = ((horizontal_distance.hypot(vertical_distance)
                    / speed_budget)
                    .max(self.config.rules.min_pass_duration_seconds))
                .max(self.config.rules.tick_seconds);
                self.ball.last_passer_id = None;
                out.new_ball_state = Some(BallTrajectoryKind::ControlTransfer {
                    from_pos: catch_spot,
                    from_z: catch_height,
                    target_pos: held_target.0,
                    target_z: held_target.1,
                    carrier_id,
                    start_time: ctx.current_t,
                    duration: transfer_duration,
                });
            } else {
                // 层 A/层 B 不通过：球到达它**实际到达的位置**。
                // 层 A 不可达 → 球在人之外；层 B 未接稳 → 球在人身旁。
                // 两者都是 loose ball（用户批准的设计点 1）：
                // 「接不到就是接不到」，不是全知全能地送到手里。
                //
                // 发球传球失败时必须回到活球阶段，否则游戏卡在
                // DeadBall/ActionExecution（实测 120k tick 无进展）。
                let arrival = self.ball.ball_pos_3d.0;
                if is_inbound_pass {
                    self.transition_phase(SubPhase::Initiation);
                    self.set_game_flow(GameFlowState::LiveBall);
                    self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
                }
                self.ball.pending_pass_receiver = None;
                // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
                self.ball.receiver_estimate = None;
                self.journal.pending_events.push(GameEvent::PassDropped {
                    passer_id,
                    receiver_id: receiver_id.clone(),
                    position: (arrival.x, arrival.y),
                });
                // 归因：传球失误（层 A 不可达 或 层 B 未接稳）。
                self.ball.pending_loose_ball_terminal =
                    Some(nba_domain::PossessionEndCause::TurnoverPassDropped);
                out.new_ball_state = Some(self.loose_ball_from(arrival, segment));
            }
        }
    }

    /// 篮板来源地板球的双人争抢裁定：取攻、守两侧距球最近者，
    /// 以与空中篮板同一的卡位/补篮有效距离与 resolve_rebound 概率
    /// 决定归属。单侧在场时直接收球，双方都不在时交由上层继续松球。
    fn resolve_contested_floor_rebound(
        &mut self,
        candidates: &[String],
        ball_pos: Vec2,
    ) -> Option<String> {
        let offense_team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let mut off_best: Option<(String, f32)> = None;
        let mut def_best: Option<(String, f32)> = None;
        for id in candidates {
            let Some(player) = self.systems.physics.get_player(id) else {
                continue;
            };
            let distance = (player.pos_ft - ball_pos).length();
            let slot = if player.team == offense_team {
                &mut off_best
            } else {
                &mut def_best
            };
            if slot.as_ref().is_none_or(|(_, d)| distance < *d) {
                *slot = Some((id.clone(), distance));
            }
        }
        let (offense_id, offensive_distance) = off_best?;
        let Some((defensive_id, defensive_distance)) = def_best else {
            return Some(offense_id);
        };
        let Some(offensive_player) = self.systems.physics.get_player(&offense_id).cloned() else {
            return Some(defensive_id);
        };
        let Some(defensive_player) = self.systems.physics.get_player(&defensive_id).cloned() else {
            return Some(offense_id);
        };
        let def_boxout_bonus = nba_domain::capability::effective_defensive_boxout_bonus(
            &self.config.rules,
            &defensive_player.attributes,
        );
        let off_putback_bias = nba_domain::capability::effective_putback_bias(
            &self.config.rules,
            &offensive_player.attributes,
        );
        let rebound_policy = &self.config.rules.resolve.rebound;
        let effective_def_dist = (defensive_distance
            * (f32::from(1u8) - def_boxout_bonus * rebound_policy.boxout_distance_discount))
            .max(f32::from(0u8));
        let effective_off_dist = (offensive_distance
            * (f32::from(1u8) - off_putback_bias * rebound_policy.putback_distance_discount))
            .max(f32::from(0u8));
        match ResolutionLayer::resolve_rebound(
            &offensive_player,
            &defensive_player,
            effective_off_dist,
            effective_def_dist,
            rebound_policy,
            &mut self.systems.rng,
        ) {
            ResolutionOutcome::ReboundSecured { rebounder_id, .. } => Some(rebounder_id),
            _ => Some(defensive_id),
        }
    }

    /// `RimRebound` 臂：落点归属裁定与无人控制时弹地转化。
    pub(crate) fn resolve_rim_rebound_arm(
        &mut self,
        target_landing: Vec2,
        start_time: f32,
        duration: f32,
        last_touch_player: Option<String>,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let tau = if duration <= f32::EPSILON {
            1.0
        } else {
            ((ctx.current_t - start_time) / duration).clamp(0.0, 1.0)
        };
        if tau < 1.0 {
            return;
        }
        let reb_pos = target_landing;
        let rebound_last_touch = last_touch_player;
        let max_reach =
            self.config.rules.player_radius_ft + self.config.rules.defender_reach_ft + 1.5;
        let maybe_reb_id = self.try_resolve_rebounder(reb_pos, max_reach);

        if let Some(reb_id) = maybe_reb_id {
            let reb_name = self
                .systems
                .physics
                .get_player(&reb_id)
                .map(|p| p.jersey.clone())
                .unwrap_or(reb_id.clone());
            let original_offense = if ctx.is_home { "home" } else { "away" };
            let is_offensive = self
                .systems
                .physics
                .get_player(&reb_id)
                .map(|p| p.team == original_offense)
                .unwrap_or(false);
            self.journal.pending_events.push(GameEvent::ReboundContest {
                rebounder_id: reb_id.clone(),
                landing_pos: (reb_pos.x, reb_pos.y),
                is_offensive,
            });
            self.journal.current_event = Some("REBOUND".to_string());
            self.journal.current_callout = Some(format!(
                "{} 抢到{}篮板，重新组织进攻！",
                reb_name,
                if is_offensive { "前场" } else { "防守" }
            ));
            if !is_offensive {
                let p_pos = self
                    .systems
                    .physics
                    .get_player(&reb_id)
                    .map(|p| p.pos_ft)
                    .unwrap_or(reb_pos);
                let dist = (p_pos - reb_pos).length();
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::DefensiveRebound,
                    Some(reb_id.clone()),
                    None,
                    Some(dist),
                );
            }
            self.start_rebound_outlet(reb_id, reb_pos, is_offensive);
        } else {
            // 无人在有效范围内保护篮板，球弹地变地板球。
            //
            // ## 篮板源归因（round-17 修复）
            //
            // 投/罚不中弹出的地板球**不是失误**：防守方收下是
            // 防守篮板，进攻方收下是前场篮板。原实现默认
            // `TurnoverLooseBall`，把「防守方拿到罚球不中的球」
            // 记成失误——既虚增失误数，又因不存在「失误球员」
            // 触发 TURNOVER_ACTOR_CONSISTENCY Hard
            // （实测 seed 2 possession 83：FT 不中→地板球→
            // 防守收下→turnover_player_id=null）。
            self.ball.pending_loose_ball_terminal =
                Some(nba_domain::PossessionEndCause::DefensiveRebound);
            out.new_ball_state = Some(BallTrajectoryKind::LooseBall {
                pos: reb_pos,
                vel: Vec2::ZERO,
                z: self.ball.ball_pos_3d.1,
                vel_z: 0.0,
                last_touch_team: self.flow.possession,
                // 篮板弹地的最后触球人沿篮板飞行载荷延续
                //（出手人，round-17：该球不算失误，但物理
                // 触球人需可追溯）。
                last_touch_player: rebound_last_touch,
            });
            self.journal.current_event = Some("LOOSE_BALL".to_string());
            self.journal.current_callout = Some("篮板球弹出无人抢到，双方争夺地板球！".to_string());
        }
    }

    /// `LooseBall` 臂：地板球积分、出界事实、掌控/弹开判定。
    ///
    /// 载荷直接取自松球态（pos/vel/z/vel_z/last_touch_player），
    /// 与调度器的 match 解构一一对应；参数数量是球态载荷的投影，
    /// 不另立中间结构。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_loose_ball_arm(
        &mut self,
        pos: Vec2,
        vel: Vec2,
        z: f32,
        vel_z: f32,
        last_touch_player: Option<String>,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        // 第一性原理：球飞出边界就是**出界事实**，应触发裁定，
        // 而不是被 `clamp_playable` 硬夹回场内。
        //
        // 此前把界外松球直接夹回边界，单 tick 产生数英尺位移，
        // 被 L1 判为 `BALL_SPEED`（实测 122 ft/s > 85 上限，
        // seed 1/12/16 各 1–2 条 Hard）。球没有"贴边弹回"这种
        // 物理；出界必须是一个显式状态转移。
        let raw_next = pos + vel * ctx.dt;
        let margin = self.config.rules.player_radius_ft;
        let out_of_bounds = raw_next.x < margin
            || raw_next.x > self.config.rules.court.width_ft - margin
            || raw_next.y < margin
            || raw_next.y > self.config.rules.court.height_ft - margin;
        if out_of_bounds {
            // 出界：球权交给最后触球方的对手，进入发球程序。
            // 放在主循环之后统一执行（此处不能提前 return，
            // 否则会跳过账本提交与不变量检查）。
            out.loose_ball_out_of_bounds = Some((
                Court::nearest_boundary_with_geometry(pos, self.config.rules.court),
                (pos, z),
            ));
        }
        let next_pos = if out_of_bounds {
            // 本 tick 不再推进球的位置：出界点就是事实位置。
            pos
        } else {
            raw_next
        };
        let mut next_vel_z = vel_z - self.config.rules.ball_gravity_ftps2 * ctx.dt;
        let mut next_z = z + next_vel_z * ctx.dt;
        // 滚动摩擦：恒定减速度沿当前速度方向衰减（物理上木地板对
        // 滚动球近似恒定摩擦力；逐 tick 指数衰减是空气阻力模型，
        // 会让球 0.3s 内减速到爬行，争抢窗口消失）。
        let speed = vel.length();
        let mut next_vel = if speed > f32::EPSILON {
            let decel = self.config.rules.loose_ball_rolling_decel_ftps2 * ctx.dt;
            (vel / speed) * (speed - decel).max(0.0)
        } else {
            vel
        };
        if next_z <= 0.0 {
            // 地面碰撞反弹：反弹恢复系数 e = 0.70，地面摩擦衰减
            next_z = 0.0;
            next_vel_z = (-next_vel_z * 0.70).max(0.0);
            next_vel *= 0.85;
        }
        let reach = self.config.rules.player_radius_ft + self.config.rules.defender_reach_ft;
        let home_jumper = self.select_jumper_id(Possession::Home);
        let away_jumper = self.select_jumper_id(Possession::Away);
        let mut candidates: Vec<(String, f32)> = self
            .systems
            .physics
            .query_nearby(pos, reach, &nba_physics::EntityFilter::Any)
            .into_iter()
            .map(|id| {
                let dist = self
                    .systems
                    .physics
                    .get_player(&id)
                    .map(|p| (p.pos_ft - pos).length())
                    .unwrap_or(f32::INFINITY);
                (id, dist)
            })
            .collect();
        // 真实规则：跳球员在球触地或被其他人触及前，严禁直接控球
        if z > 0.5 {
            candidates.retain(|(id, _)| *id != home_jumper && *id != away_jumper);
        }
        // 收球竞争按真实距离排序（id 仅作同距的确定性 tie-break，
        // charter C4）。旧行为按 id 字符串排序取第一个：抢断落球后
        // 场上 id 最小的球员直接收走球——观感「自动送球到对方手上」，
        // 且原持球人因 id 序靠前能在球弹回脚边时立即重收，抹掉
        // 争抢窗口。
        candidates.sort_by(|left, right| {
            left.1
                .partial_cmp(&right.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });
        // 球-人身体接触（ADR-017 第三步）：本 tick 预检已命中时，
        // 弹开优先于捡起——球被弹开而不是被收下，弹开后的
        // 松球态已在函数顶部写入 `new_ball_state`，此处跳过
        // 本 tick 的掌控判定与位置积分（弹开球态就是本 tick 的
        // 权威下一态）。
        //
        // 速度门（同一步）：手臂可及半径（5.8 ft）是「慢球可以
        // 伸手拿到」的口径；超过 `loose_ball_control_speed_ftps`
        // 的快球拿不住，继续按物理飞行，直到撞到身体弹开或被
        // 摩擦减速后重新可收。否则快球会在几英尺外被凭空收下
        // （审计标记过的「瞬移收球」）。
        let bounce_active = matches!(
            out.new_ball_state,
            Some(BallTrajectoryKind::LooseBall { .. })
        );
        let next_speed_3d = (next_vel.length_squared() + next_vel_z * next_vel_z).sqrt();
        let controllable = next_speed_3d <= self.config.rules.loose_ball_control_speed_ftps;
        // 篮板来源的地板球（投/罚不中弹出）：双方都在可及范围时，
        // 归属是能力对决而非先到先得——与空中篮板同一裁决
        // （resolve_rebound + 卡位/补篮有效距离，D27）。否则卡位
        // 能力只在罕见的空中双人同窗场景可见，主流篮板路径完全不
        // 消费它。其他来源（拨断/传球掉落）保持既有的确定性顺序收球。
        let rebound_sourced = matches!(
            self.ball.pending_loose_ball_terminal,
            Some(nba_domain::PossessionEndCause::DefensiveRebound)
        );
        let secure_candidate = if bounce_active || !controllable {
            None
        } else if rebound_sourced {
            self.resolve_contested_floor_rebound(
                &candidates
                    .iter()
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>(),
                next_pos,
            )
        } else {
            candidates.into_iter().next().map(|(id, _)| id)
        };
        if bounce_active {
            // 弹开路径：不再覆盖 new_ball_state。
        } else if let Some(player_id) = secure_candidate {
            self.journal
                .pending_events
                .push(GameEvent::LooseBallSecured {
                    player_id: player_id.clone(),
                    position: (next_pos.x, next_pos.y),
                });
            out.loose_ball_secured_player = Some((player_id, next_pos, next_z));
        } else {
            out.new_ball_state = Some(BallTrajectoryKind::LooseBall {
                pos: next_pos,
                vel: next_vel,
                z: next_z,
                vel_z: next_vel_z,
                last_touch_team: self.flow.possession,
                // 松球逐 tick 采样：最后触球人沿当前载荷延续。
                last_touch_player,
            });
        }
    }

    /// `Drive` 臂：突破推进、中途分流与终结裁定。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_drive_arm(
        &mut self,
        driver_id: String,
        target_pos: Vec2,
        start_time: f32,
        duration: f32,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let tau = ((ctx.current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
        let prev_ball = self.ball.ball_pos_3d;
        let (sampled_pos, sampled_z) = BallisticsEngine::sample_ball_position(
            &self.ball.ball_state,
            ctx.current_t,
            self.systems.physics.get_players(),
            &self.config.rules,
        );
        let driver_carrier_pos = self
            .systems
            .physics
            .get_player(&driver_id)
            .map(|player| player.pos_ft);
        self.ball.ball_pos_3d = self.clamp_held_family_ball_pos(
            prev_ball,
            (sampled_pos, sampled_z),
            driver_carrier_pos,
            ctx.dt,
        );

        let driver_pos = self
            .systems
            .physics
            .get_player(&driver_id)
            .map(|player| player.pos_ft)
            .unwrap_or(self.ball.ball_pos_3d.0);
        let hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
        let dist_to_hoop = (driver_pos - hoop_pos).length();

        // ## 过人后重定向（round-18 两段式几何的第二段）
        //
        // 起手目标是被过防守人身旁的空档（第一步过人）；越过他
        // （沿突破方向投影领先）后，把目标切回**篮筐**，完成
        // 「过人 → 攻框」的完整几何。
        // 时间基重定向：被过防守人以极速让位，约一个加速窗口
        // （0.35s）后通道即开，届时切回攻框目标。空间基判定
        // （「越过防守人」）在碰撞边界处永不可达，实测停滞。
        if self.ball.beaten_defender_id.is_some() && ctx.current_t - start_time >= 0.35 {
            if let Some(dp) = self.systems.physics.get_player(&driver_id) {
                let drv_spd = dp.target_speed_ftps;
                let morale = dp.morale.clone();
                self.systems.physics.set_player_target(
                    &driver_id,
                    target_pos,
                    drv_spd,
                    "DriveToBasket",
                    "BallHandler",
                    &morale,
                );
                self.ball.beaten_defender_id = None;
            }
        }
        // 1. 中途分流决策（Drive Branching: 突分 Kickout 或急停中投 Pull-up）
        let elapsed = ctx.current_t - start_time;
        let mut branched = false;
        // 出手冻结期间不得分支：该突破者已有一条挂起的投篮释放
        // （execute_shot 冻结的裁定）， Drive 球态保持到 release 才转
        // Shot。分支若在此期间再触发 execute_shot，会撞上 pending
        // 非空的 fast-fail（实测 seed100 全量赛 tick ~35994：
        // pending 挂起 + Drive 臂 pullup 分支二次出手）。
        let frozen_by_pending = self
            .observations
            .pending_shot_release
            .as_ref()
            .is_some_and(|pending| pending.shooter_id == driver_id);
        if elapsed
            >= self
                .config
                .rules
                .tactics
                .drive_decision_check_interval_seconds
            && tau < 0.85
            && !frozen_by_pending
        {
            let spacing = SemanticEvaluator::spacing(
                self.flow.possession,
                driver_pos,
                &self.systems.physics,
                &self.config.rules,
            );
            // 触发度量：突破路线是否被防守封堵（lane_openness 低）。
            // 旧条件 paint_crowding（以篮筐为圆心的**进攻方**队友密度）
            // 语义错位——五外战术全员在外线，禁区内没有队友，crowding
            // 恒低，kickout 在最需要它的战术里永不触发；kickout 的决策
            // 理由是「突破路线被协防封死、外线有空位」，度量对象是
            // 持球人周围的防守密度。
            let lane_blocked = 1.0 - spacing.lane_openness;

            // 突破路线被封且持球人在中远距离时，评估突分（Kickout）给外线空位队友
            if lane_blocked > self.config.rules.tactics.drive_kickout_max_crowding {
                let mut best_kickout: Option<(String, Vec2)> = None;
                let mut min_opp_dist = self.config.rules.tactics.drive_kickout_min_defender_dist_ft;

                let all_players: Vec<_> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .cloned()
                    .collect();
                let off_team_str = match self.flow.possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                for teammate in &all_players {
                    // ## on_court 过滤（round-18 修复）
                    //
                    // 此前漏掉：突破分球把球传给了**替补席上的队友**
                    // （实测 seed 31337：分球给站在板凳区 y=-4 的
                    // H_08，球被「已下场的人接住」，BALL_HOLDER_ON_COURT
                    // Hard 190 次）。
                    if teammate.on_court
                        && teammate.team == off_team_str
                        && teammate.id != driver_id
                        // 回场条款（charter §6.2）：前场已建立时不得把球分回
                        // 后场的队友——这类传球是白送球权的违例。
                        && !(self.clock.frontcourt_established
                            && self
                                .config
                                .rules
                                .court
                                .is_backcourt(teammate.pos_ft, self.flow.possession == Possession::Home))
                    {
                        let dist_to_team_hoop = (teammate.pos_ft - hoop_pos).length();
                        if dist_to_team_hoop >= self.config.rules.tactics.drive_kickout_pass_dist_ft
                        {
                            // 检查该空位队友最近的防守人距离
                            let mut nearest_def_dist = f32::MAX;
                            for opp in &all_players {
                                if opp.team != off_team_str {
                                    let d = (opp.pos_ft - teammate.pos_ft).length();
                                    if d < nearest_def_dist {
                                        nearest_def_dist = d;
                                    }
                                }
                            }
                            if nearest_def_dist > min_opp_dist {
                                min_opp_dist = nearest_def_dist;
                                best_kickout = Some((teammate.id.clone(), teammate.pos_ft));
                            }
                        }
                    }
                }

                if let Some((target_id, target_spot)) = best_kickout {
                    out.drive_kickout_action = Some((
                        driver_id.clone(),
                        target_id,
                        driver_pos,
                        target_spot,
                        ctx.current_t,
                    ));
                    branched = true;
                } else if lane_blocked > self.config.rules.tactics.drive_pullup_min_crowding
                    && dist_to_hoop > self.config.rules.tactics.drive_early_finish_dist_ft
                    && dist_to_hoop <= self.config.rules.tactics.drive_mid_range_pullup_dist_ft
                {
                    // 人堆受阻，急停中距离跳投 (Pull-up)
                    out.drive_pullup_action = Some((driver_id.clone(), driver_pos, ctx.current_t));
                    branched = true;
                }
            }
        }

        // 2. 提前进入冲框终结判定：进入终结区（Early Finish Gate）或时间耗尽
        let early_finish = dist_to_hoop <= self.config.rules.tactics.drive_early_finish_dist_ft
            && elapsed >= self.config.rules.tactics.drive_min_duration_seconds;
        if !branched && (tau >= 1.0 || early_finish) {
            let driver_id = driver_id.clone();
            self.possession_ctx.current_possession_turnover_player = Some(driver_id.clone());
            let driver_pos = self
                .systems
                .physics
                .get_player(&driver_id)
                .map(|player| player.pos_ft)
                .unwrap_or(self.ball.ball_pos_3d.0);
            let holder_height = self.config.rules.ball_holder_height_ft;
            // ## 事实修正（round-16）：先判定空间门，再申报结果
            //
            // 原实现无条件把预掷的 `finish_made` 写进事件，但空间门
            // （距筐 >16ft）会把该「进球」静默丢弃——实测 6 场 397 次
            // DRIVE_SCORE 中 80%（场均 52.7 次）未变成出手，事件流与
            // 记分簿自相矛盾（事实账目违规）。
            let stall_hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
            let driver_now = self.systems.physics.get_player(&driver_id).cloned();
            let openness = self.systems.physics.openness(&driver_id);
            let reached = driver_now.as_ref().is_some_and(|player| {
                let stamina = clamp_unit(player.stamina / player.max_stamina.max(f32::EPSILON));
                let lane_density = SemanticEvaluator::spacing(
                    self.flow.possession,
                    driver_pos,
                    &self.systems.physics,
                    &self.config.rules,
                )
                .paint_crowding;
                let policy = &self.config.rules.resolve.drive;
                let skill_delta = (player.attributes.finishing - f32::from(1u8) / f32::from(2u8))
                    * self.config.rules.resolve.player_skill.finishing_weight
                    * policy.skill_delta_scale;
                let fatigue_delta = (stamina - f32::from(1u8))
                    * self.config.rules.resolve.player_skill.finishing_weight;
                let reach_probability = clamp_unit(
                    self.config.rules.resolve.base_rates.drive_success
                        + skill_delta
                        + fatigue_delta
                        - lane_density * policy.lane_density_penalty
                        - openness.contest_intensity * policy.contest_penalty,
                );
                self.systems.rng.gen_bool(reach_probability as f64)
            });
            let dist_now = (driver_pos - stall_hoop_pos).length();
            let stall_no_finish =
                reached && dist_now > self.config.rules.tactics.drive_finish_range_ft;
            let successful = reached && !stall_no_finish;
            self.journal.pending_events.push(GameEvent::DriveOutcome {
                driver_id: driver_id.clone(),
                successful,
            });

            if successful {
                let hoop_pos = self.config.rules.court.hoop_pos(ctx.is_home);
                let finish_pos = driver_pos;
                let drive_distance = (driver_pos - target_pos).length();
                let early_finish = drive_distance
                    <= self.config.rules.tactics.drive_early_finish_dist_ft
                    && elapsed <= self.config.rules.tactics.drive_min_duration_seconds;
                let dist_to_hoop = (hoop_pos - finish_pos).length();
                // Spatial gate: if driver is still outside the paint / perimeter,
                // this drive was stalled before reaching finishing position.
                let finish_range = self.config.rules.tactics.drive_finish_range_ft;
                if dist_to_hoop > finish_range
                    && drive_distance > self.config.rules.tactics.drive_early_finish_dist_ft
                    && !early_finish
                {
                    self.ball.ball_pos_3d = (driver_pos, holder_height);
                    out.new_ball_state = Some(BallTrajectoryKind::Held {
                        carrier_id: driver_id.clone(),
                    });
                    // ## 停滞不重新发起战术（round-16）
                    //
                    // 原实现转回 Initiation，而 Initiation 在活球下要等
                    // `tactical_initiation_seconds = 6.5s` 才回到可决策
                    // 状态——停滞一次就白燃 6.5s 进攻时钟。突破受阻是
                    // **进攻的延续**，不是新回合，直接回 ActionExecution
                    // 下一决策间隔（2.4s）即可行动。
                    // 实测该机制是 24s 违例（场均 12.8 次，真实 ~0.5）
                    // 的主要时间吞噬器。
                    self.transition_phase(SubPhase::ActionExecution);
                    self.journal.current_event = Some("DRIVE_STOPPED".to_string());
                    self.journal.current_callout =
                        Some(format!("{} 突破被防守延误于外线，重新组织", driver_id));
                } else {
                    let driver_p = self.systems.physics.get_player(&driver_id).cloned();
                    let finishing_skill = driver_p
                        .as_ref()
                        .map(|p| p.attributes.finishing)
                        .unwrap_or(0.5);
                    let lane_density = SemanticEvaluator::spacing(
                        self.flow.possession,
                        driver_pos,
                        &self.systems.physics,
                        &self.config.rules,
                    )
                    .paint_crowding;
                    let takeoff = (hoop_pos - finish_pos).normalize_or_zero()
                        * drive_distance.min(self.config.rules.tactics.drive_finish_extend_ft);
                    let finish_pos = self
                        .config
                        .rules
                        .court
                        .clamp_playable(finish_pos + takeoff, self.config.rules.player_radius_ft);
                    let dist_to_hoop = (hoop_pos - finish_pos).length();
                    let (finish_kind, action_name, callout_action) = if dist_to_hoop
                        < self.config.rules.tactics.drive_dunk_max_dist_ft
                        && finishing_skill > self.config.rules.tactics.drive_dunk_min_finishing
                        && lane_density < self.config.rules.tactics.drive_dunk_max_lane_density
                    {
                        (
                            nba_domain::action_window::RimFinishKind::Dunk,
                            "Dunk",
                            "腾空暴扣！单臂炸筐！",
                        )
                    } else if dist_to_hoop > self.config.rules.tactics.drive_floater_min_dist_ft {
                        (
                            nba_domain::action_window::RimFinishKind::Floater,
                            "Floater",
                            "行进间柔和抛投！",
                        )
                    } else {
                        (
                            nba_domain::action_window::RimFinishKind::Layup,
                            "Layup",
                            "三步并两步，低手上篮！",
                        )
                    };
                    self.transition_phase(SubPhase::ShotAttempt);
                    let contest_val = openness.contest_intensity;
                    let finish_zone = self.config.rules.court.shot_zone(
                        finish_pos,
                        ctx.is_home,
                        self.config.rules.league.three_point_distance_ft,
                        self.config.rules.league.corner_three_distance_ft,
                    );
                    let finish_skill = clamp_unit(
                        driver_p
                            .as_ref()
                            .map(|player| match finish_zone {
                                nba_domain::ShotZone::Rim => {
                                    let contest = clamp_unit(contest_val);
                                    player.attributes.shooting_close * (f32::from(1u8) - contest)
                                        + player.attributes.finishing * contest
                                }
                                nba_domain::ShotZone::Near => player.attributes.shooting_near,
                                nba_domain::ShotZone::Mid => player.attributes.shooting_mid,
                                nba_domain::ShotZone::Three => player.attributes.shooting_three,
                            })
                            .unwrap_or(f32::from(1u8) / f32::from(2u8)),
                    );
                    let base_fg = match finish_zone {
                        nba_domain::ShotZone::Rim => {
                            self.config.rules.resolve.base_rates.shot_make_rim
                        }
                        nba_domain::ShotZone::Near => {
                            self.config.rules.resolve.base_rates.shot_make_near
                        }
                        nba_domain::ShotZone::Mid => {
                            self.config.rules.resolve.base_rates.shot_make_mid
                        }
                        nba_domain::ShotZone::Three => {
                            self.config.rules.resolve.base_rates.shot_make_3pt
                        }
                    };
                    let skill_adjustment = (finish_skill - f32::from(1u8) / f32::from(2u8))
                        * self.config.rules.resolve.player_skill.shooting_weight
                        * f32::from(2u8);
                    let make_probability = (base_fg + skill_adjustment
                        - contest_val * self.config.rules.shot_contest_sensitivity)
                        .clamp(
                            self.config.rules.shot_pct_floor,
                            self.config.rules.shot_pct_ceiling,
                        );
                    let transition_event_id = self.possession_ctx.transition_context_event(
                        ctx.current_t,
                        self.config.rules.tactics.transition_finish_window_seconds,
                    );
                    let transition_context =
                        transition_event_id.is_some() && dist_to_hoop <= finish_range;
                    // 转换语境下的攻框按转换终结归类（描述回合创建语境，
                    // 优先于动作路径）：突破只是到达篮下的手段。
                    let creation_source = if transition_context {
                        nba_domain::ShotCreationSource::TransitionFinish
                    } else {
                        nba_domain::ShotCreationSource::DriveFinish
                    };
                    let source_event_id = if transition_context {
                        transition_event_id
                    } else {
                        None
                    };
                    self.journal.pending_events.push(GameEvent::ShotRelease {
                        shooter_id: driver_id.clone(),
                        pos: (finish_pos.x, finish_pos.y),
                        creation_source,
                        transition_context,
                        transition_event_id: transition_context
                            .then_some(transition_event_id)
                            .flatten(),
                        source_event_id,
                        is_three: finish_zone == nba_domain::ShotZone::Three,
                        contest_level: contest_val,
                        make_probability,
                    });
                    self.possession_ctx.current_possession_shooter = Some(driver_id.clone());
                    self.possession_ctx.current_possession_contest = Some(contest_val);
                    self.journal.current_event = Some("SHOT_RELEASE".to_string());
                    self.clear_recent_shot_creation_context();
                    let window = if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                        ActionTimeWindow::new_dunk(&driver_id, ctx.current_t, &self.config.rules)
                    } else {
                        ActionTimeWindow::new_layup(&driver_id, ctx.current_t, &self.config.rules)
                    };
                    self.start_action_window(
                        &driver_id,
                        window,
                        None,
                        Some((hoop_pos.x, hoop_pos.y)),
                    );
                    if let Some(p) = self.systems.physics.get_player_mut(&driver_id) {
                        p.action = action_name.to_string();
                        let hoop_dir = (hoop_pos - p.pos_ft).normalize_or_zero();
                        if hoop_dir.length_squared() > 0.1 {
                            p.facing_dir = hoop_dir;
                        }
                    }
                    let driver_display = self
                        .systems
                        .physics
                        .get_player(&driver_id)
                        .map(|p| format!("{}号", p.jersey))
                        .unwrap_or_else(|| driver_id.clone());
                    self.journal.current_callout =
                        Some(format!("{} {}", driver_display, callout_action));
                    let aim_pos = BallisticsEngine::sample_shot_aim(
                        finish_pos,
                        hoop_pos,
                        make_probability,
                        &mut self.systems.rng,
                        &self.config.rules,
                    );
                    // 释放起点 = 球的实际位置（末次运球采样点）：出手事件与
                    // 终结分区仍按向篮筐延伸的 finish_pos 统计，但球的飞行
                    // 必须从球所在处连续出发，否则单帧向篮筐瞬移
                    // drive_finish_extend_ft，越出球速包络（实测 90–156 ft/s）。
                    let release_origin = self.ball.ball_pos_3d.0;
                    let shot_dur = BallisticsEngine::shot_duration(
                        (aim_pos - release_origin).length(),
                        self.config.rules.rim_height_ft + 1.5,
                        &self.config.rules,
                    );
                    out.new_ball_state = Some(BallTrajectoryKind::Shot {
                        shooter_id: driver_id.clone(),
                        from_pos: release_origin,
                        hoop_pos,
                        aim_pos,
                        start_time: ctx.current_t,
                        duration: shot_dur,
                        collision_checked_until: ctx.current_t,
                        is_three: finish_zone == nba_domain::ShotZone::Three,
                        peak_z: if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                            self.config.rules.rim_height_ft + 0.5
                        } else {
                            self.config.rules.rim_height_ft + 1.5
                        },
                        make_probability,
                        contest_intensity: contest_val,
                    });
                }
            } else {
                self.ball.ball_pos_3d = (driver_pos, holder_height);
                out.new_ball_state = Some(BallTrajectoryKind::Held {
                    carrier_id: driver_id.clone(),
                });
                // 同上：突破未成是进攻延续，不重置为战术发起等待。
                self.transition_phase(SubPhase::ActionExecution);
                self.journal.current_event = Some("DRIVE_STOPPED".to_string());
                self.journal.current_callout =
                    Some(format!("{} 突破被防守延误，重新组织", driver_id));
            }
            self.clock.last_decision_time = ctx.current_t;
        }
    }

    /// `Shot` 臂：到筐结算（得分/打铁路由）与事实发布。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn resolve_shot_arm(
        &mut self,
        shooter_id: String,
        hoop_pos: Vec2,
        _start_time: f32,
        duration: f32,
        is_three: bool,
        from_pos: Vec2,
        aim_pos: Vec2,
        _make_probability: f32,
        collision_checked_until: f32,
        contest_intensity: f32,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let sid = shooter_id.clone();
        let result = BallisticsEngine::shot_flight_step(
            &self.ball.ball_state,
            collision_checked_until,
            ctx.current_t,
            &self.config.rules,
        );
        match result {
            nba_physics::ballistics::ShotFlightStep::InFlight => {
                let BallTrajectoryKind::Shot {
                    collision_checked_until: checked_until,
                    ..
                } = &mut self.ball.ball_state
                else {
                    unreachable!("shot state changed during shot flight detection");
                };
                *checked_until = ctx.current_t;
                self.ball.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball.ball_state,
                    ctx.current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );
            }
            nba_physics::ballistics::ShotFlightStep::Made {
                time,
                ball_position,
            } => {
                let arrival_event = GameEvent::ShotTrajectoryArrival {
                    shooter_id: shooter_id.clone(),
                    ball_position: (ball_position.x, ball_position.y, ball_position.z),
                };
                self.journal.pending_events.push(arrival_event);
                self.ball.ball_pos_3d = (ball_position.truncate(), ball_position.z);
                let arrival_t = FlightContext {
                    current_t: time,
                    ..ctx
                };
                self.shot_arrival_settlement(
                    shooter_id,
                    hoop_pos,
                    from_pos,
                    aim_pos,
                    None,
                    None,
                    true,
                    is_three,
                    contest_intensity,
                    duration,
                    arrival_t,
                    out,
                );
            }
            nba_physics::ballistics::ShotFlightStep::Contact(contact) => {
                self.journal
                    .pending_events
                    .push(GameEvent::ShotTrajectoryArrival {
                        shooter_id: shooter_id.clone(),
                        ball_position: (
                            contact.ball_position.x,
                            contact.ball_position.y,
                            contact.ball_position.z,
                        ),
                    });
                self.journal.pending_events.push(GameEvent::ShotContact {
                    shooter_id: shooter_id.clone(),
                    surface: match contact.surface {
                        nba_physics::ballistics::ShotContactSurface::Rim => "Rim",
                        nba_physics::ballistics::ShotContactSurface::Backboard => "Backboard",
                    }
                    .to_string(),
                    position: (
                        contact.contact_position.x,
                        contact.contact_position.y,
                        contact.contact_position.z,
                    ),
                });
                self.ball.ball_pos_3d = (contact.ball_position.truncate(), contact.ball_position.z);
                if !self.ledger.free_throw_queue.is_empty() {
                    let arrival_t = FlightContext {
                        current_t: contact.time,
                        ..ctx
                    };
                    self.shot_arrival_settlement(
                        shooter_id,
                        hoop_pos,
                        from_pos,
                        aim_pos,
                        Some(contact.contact_position.truncate()),
                        Some(contact.incoming_velocity),
                        false,
                        is_three,
                        contest_intensity,
                        duration,
                        arrival_t,
                        out,
                    );
                    return;
                }
                let landing = match contact.surface {
                    nba_physics::ballistics::ShotContactSurface::Rim => {
                        BallisticsEngine::compute_rebound_landing_from_contact_velocity(
                            from_pos,
                            hoop_pos,
                            contact.incoming_velocity,
                            contact.contact_position.truncate(),
                            &mut self.systems.rng,
                            &self.config.rules,
                        )
                    }
                    nba_physics::ballistics::ShotContactSurface::Backboard => {
                        BallisticsEngine::compute_rebound_landing_bank_from_contact_velocity(
                            contact.contact_position.truncate(),
                            contact.contact_position.z,
                            contact.incoming_velocity,
                            &self.config.rules,
                        )
                    }
                };
                self.journal.pending_events.push(GameEvent::HoopArrival {
                    shooter_id: shooter_id.clone(),
                    shot_origin: (from_pos.x, from_pos.y),
                    is_made: false,
                    is_three,
                    contest_intensity,
                });
                if let Some(state) = self.observations.modulation.get_mut(&sid) {
                    state.record_shot(false);
                }
                self.transition_phase(SubPhase::FlightAndRebound);
                self.ball.ball_pos_3d = (
                    contact.contact_position.truncate(),
                    contact.contact_position.z,
                );
                out.new_ball_state = Some(BallTrajectoryKind::RimRebound {
                    from_pos: landing.contact_pos,
                    from_z: landing.contact_z,
                    hoop_pos,
                    target_landing: landing.landing_pos,
                    start_time: contact.time,
                    duration: landing.flight_duration,
                    peak_z: landing.peak_z,
                    last_touch_team: self.flow.possession,
                    last_touch_player: Some(shooter_id),
                });
                self.assign_rebound_pursuit(landing.landing_pos, contact.time);
            }
            nba_physics::ballistics::ShotFlightStep::Miss {
                time: _,
                ball_position,
                incoming_velocity,
            } => {
                self.journal
                    .pending_events
                    .push(GameEvent::ShotTrajectoryArrival {
                        shooter_id: shooter_id.clone(),
                        ball_position: (ball_position.x, ball_position.y, ball_position.z),
                    });
                self.journal.pending_events.push(GameEvent::HoopArrival {
                    shooter_id: shooter_id.clone(),
                    shot_origin: (from_pos.x, from_pos.y),
                    is_made: false,
                    is_three,
                    contest_intensity,
                });
                if let Some(state) = self.observations.modulation.get_mut(&sid) {
                    state.record_shot(false);
                }
                self.ball.ball_pos_3d = (ball_position.truncate(), ball_position.z);
                if !self.ledger.free_throw_queue.is_empty() {
                    self.try_start_next_free_throw_program(out);
                    return;
                }
                self.transition_phase(SubPhase::FlightAndRebound);
                out.new_ball_state = Some(BallTrajectoryKind::LooseBall {
                    pos: ball_position.truncate(),
                    vel: incoming_velocity.truncate(),
                    z: ball_position.z,
                    vel_z: incoming_velocity.z,
                    last_touch_team: self.flow.possession,
                    last_touch_player: Some(shooter_id),
                });
            }
        }
    }
}
