//! 弹道与球态：按球态逐 tick 推进球的位置，产出待处理的结果事实，
//! 并保持球态与物理层派生标志同步。
//!
//! 依据 `docs/architecture.md` §3.1/§3.3：位置是派生量，`BallState` 不保存逐 tick
//! 的位置与速度；飞行与松球的轨迹由参数闭式采样得出，本模块只处理**离散的
//! 归属与路由事件**。所有球态变更都经唯一写入口 [`MatchEngine::transition_ball_state`]。
//!
//! [`MatchEngine::resolve_ball_flight`] 只做裁决与采样，不消费结果：产出
//! [`BallFlightOutcome`] 交给调度器，由调度器决定后续的球权转移、犯规与罚球程序。
//! 这样做的原因是该阶段夹在「执行」与「战术导航」之间，其结果可能短路本 tick
//! （松球出界即结束），而短路必须由调度器统一处理以跳过账本提交与不变量检查之外的路径。
//!
//! [`MatchEngine::sync_ball_holder`] 与 [`MatchEngine::mark_receiver`] 维护的是
//! 物理层的派生态（`has_ball` / `is_receiving_pass` / 界外豁免），它们必须与
//! 权威球态同一步更新，否则物理会按上一 tick 的旧标志执行。

use glam::Vec2;
use nba_domain::{GameEvent, GameFlowState, SubPhase};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};

use super::MatchEngine;

mod arms;
mod outcome;
mod write_entry;

use arms::FlightContext;

/// 一次弹道裁决产出的待处理事实。
///
/// 这些值在裁决过程中逐步累积（一个 tick 最多确定一项），因此以整体形式
/// 返回，由调度器按优先级消费：出界 → 突分 → 急停 → 抢断 → 松球掌控 → 新球态。
#[derive(Default)]
pub(crate) struct BallFlightOutcome {
    /// 裁决后的新球态（无变更时为 `None`）。
    pub new_ball_state: Option<BallTrajectoryKind>,
    /// 传球被抢断时触发的防守者 id。
    pub steal_triggered_defender: Option<String>,
    /// 松球被掌控时触发的（球员 id, 球位置, 高度）。
    pub loose_ball_secured_player: Option<(String, Vec2, f32)>,
    /// 死球发球失败时把比赛拉回活球的标志。
    pub live_ball_triggered: bool,
    /// 松球出界时的（边界点, 球的三维位置）。
    pub loose_ball_out_of_bounds: Option<(Vec2, (Vec2, f32))>,
    /// 突破分球（Kickout）待执行的动作参数。
    pub drive_kickout_action: Option<(String, String, Vec2, Vec2, f32)>,
    /// 突破急停跳投（Pull-up）待执行的动作参数。
    pub drive_pullup_action: Option<(String, Vec2, f32)>,
    /// 封盖发生时发布的比赛事实。
    pub blocked_shot_event: Option<GameEvent>,
}

impl MatchEngine {
    pub(crate) fn resolve_ball_flight(
        &mut self,
        current_t: f32,
        is_home: bool,
        dt: f32,
    ) -> BallFlightOutcome {
        let ctx = FlightContext {
            current_t,
            is_home,
            dt,
        };
        let mut outcome = BallFlightOutcome::default();
        // ## 封盖判定必须在 match 之前（D25）
        //
        // `match &self.ball.ball_state` 在整个匹配期间持有不可变借用，
        // 而封盖裁定需要 `&mut self`（读物理世界、掷骰、写事件）。
        // 因此把所需字段先取出来（靠 `Copy` 与克隆），在借用开始前裁定。
        //
        // 时机上这也更正确：封盖是出手者在**可干扰区间**内发生的触及，
        // 与“球是否已到达篮筐”无关——先判封盖，再让后续分支看到新球态。
        let block_candidate = if let Some(pending) = self.observations.pending_shot_release.as_ref()
        {
            // 挂起的投篮（球在手、窗口执行段）：封盖在 Execution 段的第一个
            // tick 掷一次，与旧时序（Shot 球态在入口建立）的「刚进入
            // Execution」触发点一致。出手点用当前球位（球在手，出手人所在
            // 即球所在），与封盖入射速度的「出手点到筐」口径一致。
            let window = self.observations.active_windows.get(&pending.shooter_id);
            let just_entered_execution = window
                .map(|w| {
                    let elapsed = current_t - w.start_time;
                    let prev = elapsed - self.config.rules.tick_seconds;
                    elapsed >= w.prep_duration && prev < w.prep_duration
                })
                .unwrap_or(false);
            just_entered_execution.then(|| {
                (
                    pending.shooter_id.clone(),
                    self.ball.ball_pos_3d.0,
                    pending.release_time,
                    pending.flight_time,
                    pending.is_made,
                    pending.is_three,
                )
            })
        } else {
            match &self.ball.ball_state {
            BallTrajectoryKind::Shot {
                shooter_id,
                from_pos,
                start_time,
                duration,
                is_made,
                is_three,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let _ = tau;
                // 只在**刚进入 `Execution` 阶段**的那一个 tick 上掷一次。
                //
                // 掷骰是“这一发会不会被封”的一次判定，不是逐 tick 重复的抽样：
                // 若每个飞行 tick 都掷，0.95s 的飞行有 24 次机会，单次概率会被
                // 放大成几乎必然（实测逐 tick 掷得到 36.6% 的出手被封）。
                //
                // 触发点是**阶段刚变为 `Execution`**，而不是窗口起点：
                // 后者的 `interference_start` 位于合球阶段（`Preparation`），
                // 那个窗口属于切球（Strip），与封盖是两个不同的判定口径。
                let window = self.observations.active_windows.get(shooter_id);
                let just_entered_execution = window
                    .map(|w| {
                        let elapsed = current_t - w.start_time;
                        let prev = elapsed - self.config.rules.tick_seconds;
                        elapsed >= w.prep_duration && prev < w.prep_duration
                    })
                    .unwrap_or(false);
                just_entered_execution.then(|| {
                    (
                        shooter_id.clone(),
                        *from_pos,
                        *start_time,
                        *duration,
                        *is_made,
                        *is_three,
                    )
                })
            }
            _ => None,
        }
        };
        if let Some((shooter_id, from_pos, _start_time, _duration, is_made, is_three)) =
            block_candidate
        {
            if let Some(block) =
                self.try_resolve_shot_block(&shooter_id, &from_pos, current_t, is_made, is_three)
            {
                outcome.new_ball_state = Some(block.next_state);
                outcome.blocked_shot_event = Some(block.event);
                // 被封盖的出手永远不再 Release：球已转松球，窗口照旧推进
                // 到 FollowThrough（封盖是身体接触，出手者动作不中断），
                // 但冻结的裁定载荷必须同步作废，否则 Exec→Follow 边界会
                // 把一个已被封盖的出手重新发布为 Shot。
                if let Some(pending) = self.observations.pending_shot_release.as_ref() {
                    assert_eq!(
                        pending.shooter_id, shooter_id,
                        "blocked shot release belongs to a different shooter"
                    );
                }
                self.observations.pending_shot_release = None;
            }
        }
        // 自由球-人接触（ADR-017 第三步）：与封盖判定同一借用纪律——
        // 接触裁定需要 `&mut self`（读物理世界、写结算账目），先在
        // 主 match 之前完成，RimRebound/LooseBall 分支内消费结果。
        // RimRebound 只在飞行中（tau<1）结算；LooseBall 在本 tick 末
        // 位置积分**之后**判定，因此把采样点与当前球态传给检测函数，
        // 由它自行按球态种类区分这两条路径。
        let free_ball_contact = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound {
                start_time,
                duration,
                ..
            } => {
                let tau = if *duration <= f32::EPSILON {
                    1.0
                } else {
                    ((current_t - start_time) / duration).clamp(0.0, 1.0)
                };
                // 飞行中球-人接触：球在飞向落点的途中撞到在场球员身体时，
                // 做几何弹开并重解落点；归属裁定（try_resolve_rebounder）
                // 仍只发生在 tau>=1 的落点。
                (tau < 1.0).then_some(self.ball.ball_pos_3d)
            }
            BallTrajectoryKind::LooseBall { pos, vel, z, .. } => {
                // 地板球：先按本 tick 末位置积分（与下方分支同式），
                // 接触判定用积分后的位置——球到达哪里就在哪里撞人。
                let next = *pos + *vel * ctx.dt;
                let margin = self.config.rules.player_radius_ft;
                let oob = next.x < margin
                    || next.x > self.config.rules.court.width_ft - margin
                    || next.y < margin
                    || next.y > self.config.rules.court.height_ft - margin;
                // 出界球不参与身体接触（下一分支将按出界事实转移）。
                if oob {
                    None
                } else {
                    Some((next, *z))
                }
            }
            _ => None,
        }
        .and_then(|spot| self.resolve_free_ball_player_contact(spot, current_t));
        if let Some((contact_state, contact_id)) = free_ball_contact {
            self.ball.loose_contact_resolved.push(contact_id);
            outcome.new_ball_state = Some(contact_state);
        }
        match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id } => {
                let cid = carrier_id.clone();
                self.ball.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball.ball_state,
                    current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );
                if self.systems.physics.get_player(&cid).is_none() {
                    self.ball.ball_pos_3d = (
                        Vec2::new(
                            self.config.rules.court.width_ft / 2.0,
                            self.config.rules.court.height_ft / 2.0,
                        ),
                        self.config.rules.ball_holder_height_ft,
                    );
                }
            }
            BallTrajectoryKind::ControlTransfer {
                carrier_id,
                target_pos,
                start_time,
                duration,
                ..
            } => {
                let cid = carrier_id.clone();
                self.ball.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball.ball_state,
                    current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );
                let receiver_ready = self
                    .systems
                    .physics
                    .get_player(&cid)
                    .map(|player| {
                        (player.pos_ft - *target_pos).length()
                            <= self.config.rules.invariant_holder_leash_ft
                    })
                    .unwrap_or(false);
                // D5.1（本轮实测）：飞行早已结束、但接球人被动作窗口锁定
                // （`lock_kinematics`）而永远走不到冻结点时，交接会永久
                // 悬置——seed 6 实测 69,466 帧（约 2,780 秒）活锁，全场仅
                // 11 个回合。交接是「球到人」的事实：飞行时长已满即视为
                // 到达，球收敛到接球人的实际位置，不再要求人体额外位移
                // （球员运动学由 physics 独占，引擎不得瞬移球员）。
                let flight_done = current_t - *start_time >= *duration;
                if !receiver_ready && flight_done {
                    if let Some(player) = self.systems.physics.get_player(&cid) {
                        let offset = player.pos_ft - *target_pos;
                        let leash = self.config.rules.invariant_holder_leash_ft;
                        // 球位于冻结点与接球人之间，距接球人不超过 leash，
                        // 保证 `Held` 状态下 BALL_WITH_HOLDER 成立。
                        let landing = if offset.length() > leash {
                            player.pos_ft
                                - offset.normalize_or_zero()
                                    * leash
                                    * self.config.rules.transfer_landing_leash_ratio
                        } else {
                            *target_pos
                        };
                        self.ball.ball_pos_3d = (landing, self.config.rules.ball_holder_height_ft);
                    }
                }
                if flight_done
                    && (receiver_ready
                        || self.ball.ball_pos_3d.0.distance(
                            self.systems
                                .physics
                                .get_player(&cid)
                                .map(|p| p.pos_ft)
                                .unwrap_or(*target_pos),
                        ) <= self.config.rules.invariant_holder_leash_ft)
                {
                    if self.ball.pending_pass_receiver.as_deref() == Some(cid.as_str()) {
                        // round-6：与 Pass 分支同口径——接球事实的位置必须是球
                        // 当前所处的位置（此处已经过 ControlTransfer 收敛，保证在
                        // 接球人 leash 内），而**不是**冻结点。
                        //
                        // 两个分支的语义分工：
                        // - `Pass` 分支：球已到冻结点，接球人恰好处于 leash 内
                        //   → `position = to_pos`（终点事实，见上）；
                        // - `ControlTransfer` 分支：接球人未到位，球已收敛到他身上
                        //   → `position = 球的实际位置`（接球事实）。
                        // 评判器因此可以区分「终点」与「接球」，不再需要第三个解释
                        // （gap.md §9.5）。
                        self.journal.pending_events.push(GameEvent::PassReceived {
                            receiver_id: cid.clone(),
                            position: (self.ball.ball_pos_3d.0.x, self.ball.ball_pos_3d.0.y),
                        });
                        self.ball.pending_pass_receiver = None;
                        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
                        self.ball.receiver_estimate = None;
                        self.ball.last_passer_id = None;
                        if self.ball.pending_pass_inbound {
                            self.transition_phase(SubPhase::Initiation);
                            self.set_game_flow(GameFlowState::LiveBall);
                            self.clock.sub_phase_timer = 0.0;
                            self.clock.last_decision_time =
                                -self.config.rules.decision_interval_seconds;
                            self.ball.pending_pass_inbound = false;
                        }
                    }
                    outcome.new_ball_state = Some(BallTrajectoryKind::Held { carrier_id: cid });
                }
            }
            BallTrajectoryKind::InboundTransfer {
                baseline_pos,
                inbounder_id,
                start_time,
                duration,
                ..
            } => {
                // 防御性恢复：发球员必须是在场球员。若因换人/犯满/旧状态
                // 使其离场，必须重新指派一名在场球员继续发球程序，否则
                // `inbounder_arrived` 永不成立，比赛停滞在 DeadBall
                // （本轮 seed 3/15/26 实测：inbounder action=Bench）。
                let inbounder_valid = self
                    .systems
                    .physics
                    .get_player(inbounder_id)
                    .map(|p| p.on_court)
                    .unwrap_or(false);
                if !inbounder_valid {
                    let replacement = self.new_possession_pg();
                    if replacement != *inbounder_id
                        && self
                            .systems
                            .physics
                            .get_player(&replacement)
                            .map(|p| p.on_court)
                            .unwrap_or(false)
                    {
                        if let Some(old) = self.systems.physics.get_player_mut(inbounder_id) {
                            old.out_of_bounds_placement = false;
                        }
                        if let Some(player) = self.systems.physics.get_player_mut(&replacement) {
                            player.target_pos_ft = *baseline_pos;
                            player.action = "InboundPositioning".to_string();
                            player.out_of_bounds_placement = true;
                        }
                        self.journal
                            .pending_events
                            .push(GameEvent::PlacementApplied {
                                player_id: replacement.clone(),
                                from: (baseline_pos.x, baseline_pos.y),
                                to: (baseline_pos.x, baseline_pos.y),
                                reason: "INBOUNDER_REASSIGNED".to_string(),
                                phase: self.phase_type().as_str().to_string(),
                            });
                        outcome.new_ball_state = Some(BallTrajectoryKind::InboundTransfer {
                            from_pos: self.ball.ball_pos_3d.0,
                            from_z: self.ball.ball_pos_3d.1,
                            baseline_pos: *baseline_pos,
                            inbounder_id: replacement,
                            start_time: current_t,
                            duration: *duration,
                        });
                    }
                } else {
                    let ball_arrived = current_t - start_time >= *duration;
                    let inbounder_dist = self
                        .systems
                        .physics
                        .get_player(inbounder_id)
                        .map(|p| (p.pos_ft - *baseline_pos).length())
                        .unwrap_or(0.0);
                    let inbounder_arrived =
                        inbounder_dist <= self.config.rules.inbound_boundary_tolerance_ft;
                    if ball_arrived && inbounder_arrived {
                        let inb_pos = self
                            .systems
                            .physics
                            .get_player(inbounder_id)
                            .map(|p| p.pos_ft)
                            .unwrap_or(*baseline_pos);
                        self.ball.ball_pos_3d = (inb_pos, self.config.rules.chest_height_ft);
                        outcome.new_ball_state = Some(BallTrajectoryKind::InboundReady {
                            baseline_pos: inb_pos,
                            inbounder_id: inbounder_id.clone(),
                        });
                    }
                }
            }
            BallTrajectoryKind::InboundReady {
                baseline_pos,
                inbounder_id,
            } => {
                // 球随发球员移动保持在胸高位置
                if let Some(inbounder) = self.systems.physics.get_player(inbounder_id) {
                    self.ball.ball_pos_3d = (inbounder.pos_ft, self.config.rules.chest_height_ft);
                } else {
                    self.ball.ball_pos_3d = (*baseline_pos, self.config.rules.chest_height_ft);
                }
            }

            BallTrajectoryKind::Pass {
                target_id,
                start_time,
                duration,
                from_pos,
                to_pos,
                inbound,
                receive_success,
                ..
            } => {
                // 臂体见 arms.rs（D29 划分）：接触检测、接球/点掉/坠地裁决。
                self.resolve_pass_arm(
                    target_id.clone(),
                    *start_time,
                    *duration,
                    *from_pos,
                    *to_pos,
                    *inbound,
                    *receive_success,
                    ctx,
                    &mut outcome,
                );
            }
            BallTrajectoryKind::Drive {
                driver_id,
                target_pos,
                start_time,
                duration,
                successful,
                finish_made,
                fouler_id,
                ..
            } => {
                // 臂体见 arms.rs（D29 划分）：推进、分流与终结裁定。
                self.resolve_drive_arm(
                    driver_id.clone(),
                    *target_pos,
                    *start_time,
                    *duration,
                    *successful,
                    *finish_made,
                    fouler_id.clone(),
                    ctx,
                    &mut outcome,
                );
            }
            BallTrajectoryKind::Shot {
                shooter_id,
                hoop_pos,
                start_time,
                duration,
                is_made,
                is_three,
                from_pos,
                fouled,
                fouler_id,
                ..
            } => {
                // 臂体见 arms.rs（D29 划分）：载荷从总 match 借用中取出，
                // 执行体在独立模块，行为由黄金哈希守卫。
                self.resolve_shot_arm(
                    shooter_id.clone(),
                    *hoop_pos,
                    *start_time,
                    *duration,
                    *is_made,
                    *is_three,
                    *from_pos,
                    *fouled,
                    fouler_id.clone(),
                    ctx,
                    &mut outcome,
                );
            }
            BallTrajectoryKind::RimRebound {
                target_landing,
                start_time,
                duration,
                last_touch_player,
                ..
            } => {
                // 臂体见 arms.rs（D29 划分）。
                self.resolve_rim_rebound_arm(
                    *target_landing,
                    *start_time,
                    *duration,
                    last_touch_player.clone(),
                    ctx,
                    &mut outcome,
                );
            }
            BallTrajectoryKind::LooseBall {
                pos,
                vel,
                z,
                vel_z,
                last_touch_player,
                ..
            } => {
                // 臂体见 arms.rs（D29 划分）。
                self.resolve_loose_ball_arm(
                    *pos,
                    *vel,
                    *z,
                    *vel_z,
                    last_touch_player.clone(),
                    ctx,
                    &mut outcome,
                );
            }
            BallTrajectoryKind::Dead { .. } => {}
        }
        outcome
    }
}
