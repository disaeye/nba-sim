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
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_semantics::SemanticEvaluator;

use super::MatchEngine;

mod outcome;
mod write_entry;

/// 一次弹道裁决产出的待处理事实。
///
/// 这些值在裁决过程中逐步累积（一个 tick 最多确定一项），因此以整体形式
/// 返回，由调度器按优先级消费：出界 → 突分 → 急停 → 抢断 → 松球掌控 → 新球态。
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
        let mut new_ball_state = None;
        let mut steal_triggered_defender = None;
        let mut loose_ball_secured_player: Option<(String, Vec2, f32)> = None;
        let mut live_ball_triggered = false;
        let mut loose_ball_out_of_bounds: Option<(Vec2, (Vec2, f32))> = None;
        let mut drive_kickout_action = None;
        let mut drive_pullup_action = None;
        let mut blocked_shot_event: Option<GameEvent> = None;
        // ## 封盖判定必须在 match 之前（D25）
        //
        // `match &self.ball.ball_state` 在整个匹配期间持有不可变借用，
        // 而封盖裁定需要 `&mut self`（读物理世界、掷骰、写事件）。
        // 因此把所需字段先取出来（靠 `Copy` 与克隆），在借用开始前裁定。
        //
        // 时机上这也更正确：封盖是出手者在**可干扰区间**内发生的触及，
        // 与“球是否已到达篮筐”无关——先判封盖，再让后续分支看到新球态。
        let block_candidate = match &self.ball.ball_state {
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
        };
        if let Some((shooter_id, from_pos, _start_time, _duration, is_made, is_three)) =
            block_candidate
        {
            if let Some(outcome) =
                self.try_resolve_shot_block(&shooter_id, &from_pos, current_t, is_made, is_three)
            {
                new_ball_state = Some(outcome.next_state);
                blocked_shot_event = Some(outcome.event);
            }
        }
        // 自由球-人接触（ADR-017 第三步）：与封盖判定同一借用纪律——
        // 接触裁定需要 `&mut self`（读物理世界、写结算账目），先在
        // 主 match 之前完成，RimRebound/LooseBall 分支内消费结果。
        // RimRebound 只在飞行中（tau<1）结算；LooseBall 在本 tick 末
        // 位置积分**之后**判定，因此把采样点与当前球态传给检测函数，
        // 由它自行按球态种类区分这两条路径。
        let free_ball_contact = match &self.ball.ball_state {
            BallTrajectoryKind::RimRebound { start_time, duration, .. } => {
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
                let next = *pos + *vel * dt;
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
            new_ball_state = Some(contact_state);
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
                    new_ball_state = Some(BallTrajectoryKind::Held { carrier_id: cid });
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
                        new_ball_state = Some(BallTrajectoryKind::InboundTransfer {
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
                        new_ball_state = Some(BallTrajectoryKind::InboundReady {
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
                intercept: intercept_fact,
                ..
            } => {
                let _duration_val = *duration;
                let is_inbound_pass = *inbound;
                let will_receive = *receive_success;
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let def_team = if is_home { "away" } else { "home" };
                let segment = *to_pos - *from_pos;
                let _ = def_team;
                // 第一性原理修复（本轮）：概率语义与发生时机分离。
                //
                // - **概率**在释放时刻裁定一次（`resolve_pass_interception`），
                //   回答「这次传球是否被拦截、由谁」；
                // - **时机**仍由逐 tick 几何决定：只有当球在空间上真正飞到
                //   该防守者的可及范围时才结算。
                //
                // 这样既消除了「逐 tick 独立掷骰导致概率随时长累积」的错误
                // （实测每次传球失败 35.8%，真实约 8–10%），又保证拦截发生
                // 在物理上合理的位置（否则抢断会在传球起点触发，球权瞬间
                // 转给远处防守者，造成 BALL_WITH_HOLDER 分离 23.83 ft）。
                let mut intercept: Option<(String, bool)> = None;
                if let Some((defender_id, is_steal)) = intercept_fact.clone() {
                    if let Some(defender) = self.systems.physics.get_player(&defender_id) {
                        let reach = self.config.rules.player_radius_ft
                            + self.config.rules.defender_reach_ft;
                        let dist_to_ball = (defender.pos_ft - self.ball.ball_pos_3d.0).length();
                        // 球在可及范围内，或已飞过该防守者所在位置（投影已过），
                        // 则视为拦截成立；否则继续飞行。
                        // 拦截只在「球已飞到防守者的拦截点」时结算：
                        // 否则抢断会在释放当 tick 触发，而球仍在传球人手中，
                        // 球权却已转给防守者（实测 BALL_WITH_HOLDER 3.74 ft）。
                        let projection_t = ((defender.pos_ft - *from_pos).dot(segment)
                            / segment.length_squared().max(f32::EPSILON))
                        .clamp(0.0, 1.0);
                        let _intercept_point = *from_pos + segment * projection_t;
                        let travelled = (self.ball.ball_pos_3d.0 - *from_pos).dot(segment)
                            / segment.length_squared().max(f32::EPSILON);
                        // 球必须已到达（或越过）该防守者的拦截点。
                        let reached = travelled >= projection_t - f32::EPSILON;
                        if reached && dist_to_ball <= reach {
                            intercept = Some((defender_id, is_steal));
                        }
                    } else {
                        // 防守者已离场：不结算拦截，让传球正常完成。
                    }
                }

                let passer_id = self.ball.last_passer_id.clone().unwrap_or_default();
                // Fix（round-15 第一性原理）：接球是「首次触球」事件，不是
                // 「飞行终点」事件。
                //
                // 旧实现只在 tau≥1 时裁决。实测（22 例层 A 失败）：接球人的
                // 估计模型以自身为参照系（`initial = ball + dir×|ball−我|`），
                // 他朝来球走 → 距离收缩 → 估计点随之后退（追赶曲线）→
                // 接球人不断走向传球人一侧，球却在冻结终点触地。最终
                // 估计误差 p50=3.37 ft、接球人正确到达自己的估计（距估计
                // 1.34 ft）、球距 3.64 ft —— 「朝来球移动」这一正确篮球行为
                // 反而必然导致终点 miss。
                //
                // 真实篮球里接球发生在第一次触球。这里把到达裁决的触发条件
                // 从「飞行结束」放宽为「飞行结束 **或** 接球人已进入接球半径」；
                // 裁决逻辑本身（层 A 距离 + 层 B 概率 + 状态转移）完全复用，
                // 不新增路径。
                //
                // 高度安全性：传球全程为胸高平飞（`ballistics.rs` Pass 分支，
                // `pass_peak_ft == chest_height_ft == 4.0`，弧项为 0），
                // 不存在“空中高处被接住”的物理问题。
                // 拦截优先级不变：本 tick 若有拦截事实，仍先走拦截分支。
                let receiver_touch = self
                    .systems
                    .physics
                    .get_player(target_id)
                    .map(|r| {
                        let radius =
                            nba_domain::effective_catch_radius(&self.config.rules, &r.attributes);
                        (r.pos_ft - self.ball.ball_pos_3d.0).length() <= radius
                    })
                    .unwrap_or(false);
                if let Some((defender_id, secured)) = intercept {
                    let position = self.ball.ball_pos_3d.0;
                    if secured {
                        self.journal
                            .pending_events
                            .push(GameEvent::PassIntercepted {
                                passer_id,
                                receiver_id: target_id.clone(),
                                defender_id: defender_id.clone(),
                                position: (position.x, position.y),
                            });
                        // 拦截成立时把球锚定到抢断者身上：`Held{stealer}`
                        // 要求球与持球人一致（否则 BALL_WITH_HOLDER）。
                        // 拦截点与抢断者的距离可能达 reach（1.8+ 可达 ft），
                        // 直接沿用球坐标会造成球人分离（实测 3.86 ft）。
                        if let Some(defender) = self.systems.physics.get_player(&defender_id) {
                            self.ball.ball_pos_3d =
                                (defender.pos_ft, self.config.rules.ball_holder_height_ft);
                        }
                        steal_triggered_defender = Some(defender_id);
                    } else {
                        self.journal.pending_events.push(GameEvent::PassTipped {
                            passer_id,
                            receiver_id: target_id.clone(),
                            defender_id,
                            position: (position.x, position.y),
                        });
                        if is_inbound_pass {
                            live_ball_triggered = true;
                        }
                        self.ball.pending_loose_ball_terminal =
                            Some(nba_domain::PossessionEndCause::TurnoverPassTipped);
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
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
                        });
                    }
                } else if tau >= 1.0 || receiver_touch {
                    let receiver_id = target_id.clone();
                    {
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
                            .map(|r| {
                                nba_domain::effective_catch_radius(
                                    &self.config.rules,
                                    &r.attributes,
                                )
                            })
                            .unwrap_or(self.config.rules.player_radius_ft);
                        let ball_to_receiver = self
                            .systems
                            .physics
                            .get_player(&receiver_id)
                            .map(|r| (r.pos_ft - self.ball.ball_pos_3d.0).length())
                            .unwrap_or(f32::MAX);
                        // 层 A：物理可达（确定性）；层 B：接稳（概率）。
                        let receiver_ready = ball_to_receiver <= catch_radius && will_receive;
                        if receiver_ready {
                            // 接球事实的位置必须是**冻结的传球终点**，而不是接球人
                            // 当时的身体位置。
                            // round-6：`PassReceived.position` 应携带**球的实际到达位置**，
                            // 而不是冻结点 `to_pos`。
                            //
                            // round-10（层 A）更正：层 A 判定改用「球与接球人的**实际**
                            // 距离 ≤ catch_radius」后，接球成功时球可能距冻结点数英尺
                            // （因为接球人按自己的估计跑位）。若仍把球瞬移到 `to_pos`，
                            // 会产生两个错误：
                            //   (a) 球的飞行终点被暴改为一个它从未到达的位置（伪造事实）；
                            //   (b) `Held` 要求 `BALL_WITH_HOLDER` ≤ leash，而接球人可能
                            //       距 `to_pos` 超过 leash —— 实测 seed 5 tick 14338
                            //       报 `BALL_WITH_HOLDER: ball 3.07 ft from holder`。
                            //
                            // 正确做法：接球成功时把球**收到接球人身上**（持球锚点），
                            // 位置即接球人当前位置——这才是物理事实，也天然满足 leash。
                            let catch_spot = self
                                .systems
                                .physics
                                .get_player(&receiver_id)
                                .map(|r| r.pos_ft)
                                .unwrap_or(*to_pos);
                            self.ball.ball_pos_3d =
                                (catch_spot, self.config.rules.ball_holder_height_ft);
                            // 发布接球点修正事实（层 A，P-1）：当实际到达位置与
                            // 传球人冻结的意图不同时，把差异登记为事实，使
                            // 事实账本自洽（不允许下游各自解释同一传球）。
                            let divergence = (catch_spot - *to_pos).length();
                            if divergence > f32::EPSILON {
                                self.journal
                                    .pending_events
                                    .push(GameEvent::PassLandingCorrected {
                                        receiver_id: receiver_id.clone(),
                                        intended: (to_pos.x, to_pos.y),
                                        actual: (catch_spot.x, catch_spot.y),
                                        divergence_ft: divergence,
                                    });
                            }
                            self.journal.pending_events.push(GameEvent::PassReceived {
                                receiver_id: receiver_id.clone(),
                                position: (catch_spot.x, catch_spot.y),
                            });
                            self.ball.pending_pass_inbound = false;
                            if is_inbound_pass {
                                self.transition_phase(SubPhase::Initiation);
                                self.set_game_flow(GameFlowState::LiveBall);
                                self.clock.sub_phase_timer = 0.0;
                                self.clock.last_decision_time =
                                    -self.config.rules.decision_interval_seconds;
                            }
                            // The pass trajectory already ends at the frozen
                            // target. The receiver's body must not teleport the
                            // ball at catch.
                            new_ball_state = Some(BallTrajectoryKind::Held {
                                carrier_id: receiver_id,
                            });
                            self.ball.last_passer_id = None;
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
                                self.clock.last_decision_time =
                                    -self.config.rules.decision_interval_seconds;
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
                            new_ball_state = Some(self.loose_ball_from(arrival, segment));
                        }
                    }
                }
            }
            BallTrajectoryKind::Drive {
                driver_id,
                target_pos: ref target_pos_ref,
                start_time,
                duration,
                successful,
                finish_made,
                fouler_id,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                self.ball.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball.ball_state,
                    current_t,
                    self.systems.physics.get_players(),
                    &self.config.rules,
                );

                let driver_pos = self
                    .systems
                    .physics
                    .get_player(driver_id)
                    .map(|player| player.pos_ft)
                    .unwrap_or(self.ball.ball_pos_3d.0);
                let hoop_pos = self.config.rules.court.hoop_pos(is_home);
                let dist_to_hoop = (driver_pos - hoop_pos).length();

                // ## 过人后重定向（round-18 两段式几何的第二段）
                //
                // 起手目标是被过防守人身旁的空档（第一步过人）；越过他
                // （沿突破方向投影领先）后，把目标切回**篮筐**，完成
                // 「过人 → 攻框」的完整几何。
                // 时间基重定向：被过防守人以极速让位，约一个加速窗口
                // （0.35s）后通道即开，届时切回攻框目标。空间基判定
                // （「越过防守人」）在碰撞边界处永不可达，实测停滞。
                if self.ball.beaten_defender_id.is_some() && current_t - start_time >= 0.35 {
                    if let Some(dp) = self.systems.physics.get_player(driver_id) {
                        let drv_spd = dp.target_speed_ftps;
                        let morale = dp.morale.clone();
                        self.systems.physics.set_player_target(
                            driver_id,
                            *target_pos_ref,
                            drv_spd,
                            "DriveToBasket",
                            "BallHandler",
                            &morale,
                        );
                        self.ball.beaten_defender_id = None;
                    }
                }
                // 1. 中途分流决策（Drive Branching: 突分 Kickout 或急停中投 Pull-up）
                let elapsed = current_t - start_time;
                let mut branched = false;
                if elapsed
                    >= self
                        .config
                        .rules
                        .tactics
                        .drive_decision_check_interval_seconds
                    && tau < 0.85
                {
                    let paint_crowding = SemanticEvaluator::spacing(
                        self.flow.possession,
                        driver_pos,
                        &self.systems.physics,
                        &self.config.rules,
                    )
                    .paint_crowding;

                    // 当内线极度拥挤且持球人在中远距离时，评估突分（Kickout）给外线空位队友
                    if paint_crowding > self.config.rules.tactics.drive_kickout_max_crowding {
                        let mut best_kickout: Option<(String, Vec2)> = None;
                        let mut min_opp_dist =
                            self.config.rules.tactics.drive_kickout_min_defender_dist_ft;

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
                                && teammate.id != *driver_id
                            {
                                let dist_to_team_hoop = (teammate.pos_ft - hoop_pos).length();
                                if dist_to_team_hoop
                                    >= self.config.rules.tactics.drive_kickout_pass_dist_ft
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
                            drive_kickout_action = Some((
                                driver_id.clone(),
                                target_id,
                                driver_pos,
                                target_spot,
                                current_t,
                            ));
                            branched = true;
                        } else if paint_crowding
                            > self.config.rules.tactics.drive_pullup_min_crowding
                            && dist_to_hoop > self.config.rules.tactics.drive_early_finish_dist_ft
                            && dist_to_hoop
                                <= self.config.rules.tactics.drive_mid_range_pullup_dist_ft
                        {
                            // 人堆受阻，急停中距离跳投 (Pull-up)
                            drive_pullup_action = Some((driver_id.clone(), driver_pos, current_t));
                            branched = true;
                        }
                    }
                }

                // 2. 提前进入冲框终结判定：进入终结区（Early Finish Gate）或时间耗尽
                let early_finish = dist_to_hoop
                    <= self.config.rules.tactics.drive_early_finish_dist_ft
                    && elapsed >= self.config.rules.tactics.drive_min_duration_seconds;
                if !branched && (tau >= 1.0 || early_finish) {
                    let driver_id = driver_id.clone();
                    self.possession_ctx.current_possession_turnover_player =
                        Some(driver_id.clone());
                    let successful = *successful;
                    let finish_made = *finish_made;
                    let fouler_id = fouler_id.clone();
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
                    let stall_hoop_pos = self.config.rules.court.hoop_pos(is_home);
                    let stall_no_finish =
                        successful && ((driver_pos - stall_hoop_pos).length() > 16.0_f32);
                    self.journal.pending_events.push(GameEvent::DriveOutcome {
                        driver_id: driver_id.clone(),
                        successful: successful && !stall_no_finish,
                        finish_made: finish_made && !stall_no_finish,
                    });

                    if let Some(fouler_id) = fouler_id {
                        self.journal.pending_events.push(GameEvent::Foul {
                            fouled_player_id: driver_id.clone(),
                            fouler_id,
                            is_shooting: true,
                        });
                        self.ball.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(self.dead_state(driver_pos, holder_height));
                        self.transition_phase(SubPhase::DeadBallReset);
                        self.journal.current_event = Some("DRIVE_FOUL".to_string());
                        self.journal.current_callout =
                            Some(format!("{} 突破造成投篮犯规，获得罚球机会", driver_id));
                    } else if successful {
                        let hoop_pos = self.config.rules.court.hoop_pos(is_home);
                        let dist_to_hoop = (driver_pos - hoop_pos).length();
                        // Spatial gate: if driver is still outside the paint / perimeter,
                        // this drive was stalled before reaching finishing position.
                        let finish_range = self.config.rules.tactics.drive_finish_range_ft;
                        if dist_to_hoop > finish_range {
                            self.ball.ball_pos_3d = (driver_pos, holder_height);
                            new_ball_state = Some(BallTrajectoryKind::Held {
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
                            let (finish_kind, action_name, callout_action) = if dist_to_hoop < 4.0
                                && finishing_skill > 0.7
                                && lane_density < 0.35
                            {
                                (
                                    nba_domain::action_window::RimFinishKind::Dunk,
                                    "Dunk",
                                    "腾空暴扣！单臂炸筐！",
                                )
                            } else if dist_to_hoop > 7.0 {
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
                            let shot_dur = BallisticsEngine::shot_duration(
                                dist_to_hoop,
                                self.config.rules.rim_height_ft,
                                &self.config.rules,
                            )
                            .max(0.4);
                            self.transition_phase(SubPhase::ShotAttempt);
                            self.journal.pending_events.push(GameEvent::ShotRelease {
                                shooter_id: driver_id.clone(),
                                pos: (driver_pos.x, driver_pos.y),
                                is_three: false,
                                contest_level: 0.2,
                                make_probability: if finish_made { 1.0 } else { 0.0 },
                            });
                            self.possession_ctx.current_possession_shooter =
                                Some(driver_id.clone());
                            let contest_val = if finish_made { 0.35 } else { 0.65 };
                            self.possession_ctx.current_possession_contest = Some(contest_val);
                            self.journal.current_event = Some("SHOT_RELEASE".to_string());
                            if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                                self.observations.active_windows.insert(
                                    driver_id.clone(),
                                    ActionTimeWindow::new_dunk(
                                        &driver_id,
                                        current_t,
                                        &self.config.rules,
                                    ),
                                );
                            } else {
                                self.observations.active_windows.insert(
                                    driver_id.clone(),
                                    ActionTimeWindow::new_layup(
                                        &driver_id,
                                        current_t,
                                        &self.config.rules,
                                    ),
                                );
                            }
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
                            new_ball_state = Some(BallTrajectoryKind::Shot {
                                shooter_id: driver_id.clone(),
                                from_pos: driver_pos,
                                hoop_pos,
                                start_time: current_t,
                                duration: shot_dur,
                                is_made: finish_made,
                                is_three: false,
                                peak_z: if finish_kind
                                    == nba_domain::action_window::RimFinishKind::Dunk
                                {
                                    self.config.rules.rim_height_ft + 0.5
                                } else {
                                    self.config.rules.rim_height_ft + 1.5
                                },
                                // 突破犯规已在上面单独分支处理（直接进罚球、
                                // 不创建 Shot 状态），因此本路径的出手必定无犯规。
                                fouled: false,
                                fouler_id: None,
                            });
                        }
                    } else {
                        self.ball.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(BallTrajectoryKind::Held {
                            carrier_id: driver_id.clone(),
                        });
                        // 同上：突破未成是进攻延续，不重置为战术发起等待。
                        self.transition_phase(SubPhase::ActionExecution);
                        self.journal.current_event = Some("DRIVE_STOPPED".to_string());
                        self.journal.current_callout =
                            Some(format!("{} 突破被防守延误，重新组织", driver_id));
                    }
                    self.clock.last_decision_time = current_t;
                }
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
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                if tau >= 1.0 {
                    let sid = shooter_id.clone();
                    let h_pos = *hoop_pos;
                    let s_pos = *from_pos;
                    let made = *is_made;
                    let three = *is_three;
                    let was_fouled = *fouled;
                    let fouler = fouler_id.clone();
                    self.journal.pending_events.push(GameEvent::HoopArrival {
                        shooter_id: sid.clone(),
                        shot_origin: (s_pos.x, s_pos.y),
                        is_made: made,
                        is_three: three,
                        contest_intensity: 0.0,
                    });
                    // ## 投篮犯规是独立事实（evidence/problem.md §23.9）
                    //
                    // 与 `HoopArrival` 并列发出，而不是用命中与否反推：
                    // and-one（犯规且命中）与投篮犯规（犯规且不中）都要能表达，
                    // 且犯规的后果（罚球、个人/团队犯规计数、犯满离场）
                    // 必须走与突破犯规同一条处理链，避免第二套口径。
                    if was_fouled {
                        if let Some(ref fid) = fouler {
                            self.journal.pending_events.push(GameEvent::Foul {
                                fouled_player_id: sid.clone(),
                                fouler_id: fid.clone(),
                                is_shooting: true,
                            });
                        }
                    }
                    if three {
                        self.ledger.box_score.fg3_attempts += 1;
                        if made {
                            self.ledger.box_score.fg3_made += 1;
                        }
                    } else {
                        self.ledger.box_score.fg2_attempts += 1;
                        if made {
                            self.ledger.box_score.fg2_made += 1;
                        }
                    }
                    // ## 投篮结果回写到士气状态机（D27 修复不可达分支）
                    //
                    // `PlayerModulationState::record_shot` 先前在生产代码与测试中
                    // 零调用，因此 `consecutive_makes` 恒为 0，
                    // `update_stamina_with_rules` 的 `HotHand` 分支
                    // （阈值 `hot_hand_makes`）永远不可达——`hot_hand_bias` 无效。
                    // 每次出手结算（进或不进）都在此立即回写，使连中/连铁
                    // 真正累积。
                    if let Some(state) = self.observations.modulation.get_mut(&sid) {
                        state.record_shot(made);
                    }
                    match ResolutionLayer::resolve_shot_arrival(made, three, &sid) {
                        ResolutionOutcome::Score { points, .. } => {
                            if is_home {
                                self.ledger.home_score += points;
                            } else {
                                self.ledger.away_score += points;
                            }
                            let baseline = Court::nearest_boundary_with_geometry(
                                h_pos,
                                self.config.rules.court,
                            );
                            self.ball.ball_pos_3d = (h_pos, self.config.rules.rim_height_ft);
                            self.emit_possession_summary(
                                nba_domain::PossessionEndCause::Score,
                                None,
                                None,
                                None,
                            );
                            self.transition_phase(SubPhase::DeadBallReset);
                            self.start_inbound_transition(
                                baseline,
                                (h_pos, self.config.rules.rim_height_ft),
                            );
                        }
                        ResolutionOutcome::Miss { .. } => {
                            let shooter_name = self
                                .systems
                                .physics
                                .get_player(&sid)
                                .map(|p| p.jersey.clone())
                                .unwrap_or_else(|| sid.clone());
                            self.journal.current_event = Some("SHOT_MISSED".to_string());
                            self.journal.current_callout =
                                Some(format!("砸框而出！{} 投篮不中，争抢篮板！", shooter_name));
                            let shot_flight_seconds = *duration;
                            self.transition_phase(SubPhase::FlightAndRebound);
                            // 双通道路由（ADR-017 第三步）：探针判「力度过大
                            // 越过筐」——出手 → 筐延长线穿过板面且弦外推 z
                            // 处于板高内 → 走打板路径（瞄准点为筐心，散射
                            // 由弦几何承载）；不足量仍是近筐沿反射。
                            let landing_spot = if BallisticsEngine::compute_backboard_contact_probe(
                                s_pos,
                                h_pos,
                                &self.config.rules,
                            ) {
                                BallisticsEngine::compute_rebound_landing_bank(
                                    s_pos,
                                    h_pos,
                                    shot_flight_seconds,
                                    &mut self.systems.rng,
                                    &self.config.rules,
                                )
                            } else {
                                BallisticsEngine::compute_rebound_landing(
                                    s_pos,
                                    h_pos,
                                    shot_flight_seconds,
                                    &mut self.systems.rng,
                                    &self.config.rules,
                                )
                            };
                            // 反弹起点 = 触点（近筐沿或板面，第三步双通道）。
                            let rebound_from = (
                                landing_spot.contact_pos,
                                landing_spot.contact_z,
                            );
                            new_ball_state = Some(BallTrajectoryKind::RimRebound {
                                from_pos: rebound_from.0,
                                from_z: rebound_from.1,
                                hoop_pos: h_pos,
                                target_landing: landing_spot.landing_pos,
                                start_time: current_t,
                                duration: landing_spot.flight_duration,
                                peak_z: landing_spot.peak_z,
                                last_touch_team: self.flow.possession,
                            });
                            // ## 篮板冲抢指派（evidence/problem.md §23.8）
                            //
                            // 实测：球在空中时守方朝球靠近的速率是攻方的**约 42 倍**
                            // （0.0042 vs 0.0001 ft/tick），且两者绝对值都极小——
                            // 即双方都几乎没有抢篮板行为，攻方几乎为零。后果是
                            // ORB% 仅 0.07–0.13（真实 0.245），每次不中直接换手，
                            // 回合被压成「一次性进攻」并与节奏过快同向。
                            //
                            // 此处显式指派：双方球员向球的落点邻域移动（守方优先，
                            // 攻方按 offensive_rebound 属性加权），使篮板真的被争抢，
                            // 而不是靠判定公式凭空产生归属。
                            self.assign_rebound_pursuit(landing_spot.landing_pos, current_t);
                        }
                        _ => {}
                    }
                }
            }
            BallTrajectoryKind::RimRebound {
                target_landing,
                start_time,
                duration,
                ..
            } => {
                let tau = if *duration <= f32::EPSILON {
                    1.0
                } else {
                    ((current_t - start_time) / duration).clamp(0.0, 1.0)
                };
                if tau >= 1.0 {
                    let reb_pos = *target_landing;
                    let max_reach = self.config.rules.player_radius_ft
                        + self.config.rules.defender_reach_ft
                        + 1.5;
                    let maybe_reb_id = self.try_resolve_rebounder(reb_pos, max_reach);

                    if let Some(reb_id) = maybe_reb_id {
                        let reb_name = self
                            .systems
                            .physics
                            .get_player(&reb_id)
                            .map(|p| p.jersey.clone())
                            .unwrap_or(reb_id.clone());
                        let original_offense = if is_home { "home" } else { "away" };
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
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
                            pos: reb_pos,
                            vel: Vec2::ZERO,
                            z: self.ball.ball_pos_3d.1,
                            vel_z: 0.0,
                            last_touch_team: self.flow.possession,
                        });
                        self.journal.current_event = Some("LOOSE_BALL".to_string());
                        self.journal.current_callout =
                            Some("篮板球弹出无人抢到，双方争夺地板球！".to_string());
                    }
                }
            }
            BallTrajectoryKind::LooseBall {
                pos, vel, z, vel_z, ..
            } => {
                // 第一性原理：球飞出边界就是**出界事实**，应触发裁定，
                // 而不是被 `clamp_playable` 硬夹回场内。
                //
                // 此前把界外松球直接夹回边界，单 tick 产生数英尺位移，
                // 被 L1 判为 `BALL_SPEED`（实测 122 ft/s > 85 上限，
                // seed 1/12/16 各 1–2 条 Hard）。球没有"贴边弹回"这种
                // 物理；出界必须是一个显式状态转移。
                let raw_next = *pos + *vel * dt;
                let margin = self.config.rules.player_radius_ft;
                let out_of_bounds = raw_next.x < margin
                    || raw_next.x > self.config.rules.court.width_ft - margin
                    || raw_next.y < margin
                    || raw_next.y > self.config.rules.court.height_ft - margin;
                if out_of_bounds {
                    // 出界：球权交给最后触球方的对手，进入发球程序。
                    // 放在主循环之后统一执行（此处不能提前 return，
                    // 否则会跳过账本提交与不变量检查）。
                    loose_ball_out_of_bounds = Some((
                        Court::nearest_boundary_with_geometry(*pos, self.config.rules.court),
                        (*pos, *z),
                    ));
                }
                let next_pos = if out_of_bounds {
                    // 本 tick 不再推进球的位置：出界点就是事实位置。
                    *pos
                } else {
                    raw_next
                };
                let mut next_vel_z = *vel_z - self.config.rules.ball_gravity_ftps2 * dt;
                let mut next_z = *z + next_vel_z * dt;
                let mut next_vel = *vel * self.config.rules.ball_velocity_retention;
                if next_z <= 0.0 {
                    // 地面碰撞反弹：反弹恢复系数 e = 0.70，地面摩擦衰减
                    next_z = 0.0;
                    next_vel_z = (-next_vel_z * 0.70).max(0.0);
                    next_vel *= 0.85;
                }
                let reach =
                    self.config.rules.player_radius_ft + self.config.rules.defender_reach_ft;
                let home_jumper = self.select_jumper_id(Possession::Home);
                let away_jumper = self.select_jumper_id(Possession::Away);
                let mut candidates =
                    self.systems
                        .physics
                        .query_nearby(*pos, reach, &nba_physics::EntityFilter::Any);
                // 真实规则：跳球员在球触地或被其他人触及前，严禁直接控球
                if *z > 0.5 {
                    candidates.retain(|id| *id != home_jumper && *id != away_jumper);
                }
                candidates.sort();
                // 球-人身体接触（ADR-017 第三步）：本 tick 预检已命中时，
                // 弹开优先于捡起——球被弹开而不是被收下，弹开后的
                // 松球态已在函数顶部写入 `new_ball_state`，此处跳过
                // 本 tick 的掌控判定与位置积分（弹开球态就是本 tick 的
                // 权威下一态）。
                let bounce_active = matches!(new_ball_state, Some(BallTrajectoryKind::LooseBall { .. }));
                if bounce_active {
                    // 弹开路径：不再覆盖 new_ball_state。
                } else if let Some(player_id) = candidates.into_iter().next() {
                    self.journal
                        .pending_events
                        .push(GameEvent::LooseBallSecured {
                            player_id: player_id.clone(),
                            position: (next_pos.x, next_pos.y),
                        });
                    loose_ball_secured_player = Some((player_id, next_pos, next_z));
                } else {
                    new_ball_state = Some(BallTrajectoryKind::LooseBall {
                        pos: next_pos,
                        vel: next_vel,
                        z: next_z,
                        vel_z: next_vel_z,
                        last_touch_team: self.flow.possession,
                    });
                }
            }
            BallTrajectoryKind::Dead { .. } => {}
        }
        BallFlightOutcome {
            new_ball_state,
            steal_triggered_defender,
            loose_ball_secured_player,
            live_ball_triggered,
            loose_ball_out_of_bounds,
            drive_kickout_action,
            drive_pullup_action,
            blocked_shot_event,
        }
    }
}
