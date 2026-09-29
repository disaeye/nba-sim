//! 事件裁决与发布：本 tick 收集的领域事实经裁判重判、强制项应用后，
//! 分配全场唯一 `event_id` 并按语义因果槽位串链写入事件日志。
//!
//! 依据 `gap.md` §7.1/§7.2：只有真实因果关系才串链，同 tick 相邻不等于因果。

use nba_decision::constraint::{ConstraintStatus, EnforcementAction, PhaseType, ViolationKind};
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_protocol::FrameEvent;

use super::MatchEngine;

/// D4.1 因果链定义：事件 kind → 它属于哪个“结果槽位”的父。
///
/// 只登记真实因果关系（动作→结果），不猜测同 tick 相邻即因果：
/// - 投篮释放 → 到达篮筐平面 → 触筐/得分/投失/篮板
/// - 传球释放 → 接球/被点掉/掉球/被断
/// - 犯规 → 罚球尝试
/// - 突破发起 → 突破结果
fn causal_parent_of(kind: &str) -> Option<&'static str> {
    match kind {
        "SHOT_TRAJECTORY_ARRIVAL" => Some("shot"),
        "SHOT_RELEASE" => Some("shot_creation"),
        "BLOCKED_SHOT" => Some("shot"),
        "SHOT_CONTACT" | "SCORE" | "SHOT_MISS" | "REBOUND" => Some("shot_arrival"),
        "PASS_RECEIVED" | "PASS_TIPPED" | "PASS_DROPPED" | "STEAL" => Some("pass"),
        "FOUL" => Some("contact"),
        "FREE_THROW" => Some("foul"),
        "DRIVE_SCORE" | "DRIVE_MISS" | "DRIVE_STOPPED" | "DRIVE_REACHED" => Some("drive"),
        _ => None,
    }
}

/// D4.1 因果链定义：事件 kind → 它在哪个槽位上充当后续事件的父。
fn causal_trigger_slot(kind: &str) -> Option<&'static str> {
    match kind {
        "DRIVE_REACHED" | "DRIVE_STOPPED" => Some("drive"),
        "PASS_RECEIVED" => Some("pass_received"),
        "REBOUND" => Some("rebound"),
        "TRANSITION_STARTED" => Some("transition"),
        "SHOT_RELEASE" => Some("shot"),
        "SHOT_TRAJECTORY_ARRIVAL" => Some("shot_arrival"),
        "PASS" => Some("pass"),
        "CONTACT_BUMP" | "SCREEN_CONTACT" => Some("contact"),
        "FOUL" => Some("foul"),
        "DRIVE_INITIATED" => Some("drive"),
        _ => None,
    }
}

impl MatchEngine {
    pub(crate) fn publish_events(&mut self) {
        while !self.journal.pending_events.is_empty() {
            let events = std::mem::take(&mut self.journal.pending_events);
            let mut adjudicated_events = Vec::with_capacity(events.len() + 2);
            for event in events {
                if let GameEvent::Contact {
                    player_a, player_b, ..
                } = &event
                {
                    adjudicated_events.push(event.clone());
                    // 节间/终场停表期间不裁决犯规：比赛时钟已停，接触
                    // 不构成比赛事实；此前节间接触会被判成犯规并把罚球
                    // 程序带进下一节（实测 seed 1：节间两次犯规 → 四罚
                    // 跨节执行，回合时长账目跨节膨胀 148s）。
                    let in_break = matches!(
                        self.flow.game_flow,
                        GameFlowState::QuarterEnd
                            | GameFlowState::Halftime
                            | GameFlowState::GameEnd
                    );
                    if in_break {
                        continue;
                    }
                    if let Some(contact) =
                        self.observations.latest_contacts.iter().find(|contact| {
                            (contact.raw.entity_a == *player_a && contact.raw.entity_b == *player_b)
                                || (contact.raw.entity_a == *player_b
                                    && contact.raw.entity_b == *player_a)
                        })
                    {
                        if let ResolutionOutcome::Foul {
                            fouled_player_id,
                            fouler_id,
                            is_shooting,
                        } = ResolutionLayer::resolve_semantic_contact_in_context(
                            contact,
                            self.systems.physics.get_players(),
                            &self.config.rules.resolve.contact,
                            &self.config.rules.resolve.drive,
                            &mut self.systems.rng,
                            matches!(
                                self.ball.ball_state,
                                nba_physics::ballistics::BallTrajectoryKind::LooseBall { .. }
                            ),
                        ) {
                            adjudicated_events.push(GameEvent::Foul {
                                fouled_player_id,
                                fouler_id,
                                is_shooting,
                                foul_kind: None,
                                personal_foul_count: None,
                                period_team_foul_count: None,
                                penalty: None,
                            });
                        }
                    }
                    continue;
                }
                adjudicated_events.push(event);
            }
            let ctx = self.constraint_ctx();
            let findings = self
                .systems
                .decision
                .registry
                .evaluate_events(&ctx, &adjudicated_events);
            let possession_at_whistle = self.flow.possession;
            let offensive_fouler_ids: std::collections::HashSet<String> = adjudicated_events
                .iter()
                .filter_map(|event| {
                    let GameEvent::Foul { fouler_id, .. } = event else {
                        return None;
                    };
                    let player = self.systems.physics.get_player(fouler_id)?;
                    let is_offense = match possession_at_whistle {
                        Possession::Home => player.team == "home",
                        Possession::Away => player.team == "away",
                    };
                    is_offense.then(|| fouler_id.clone())
                })
                .collect();
            let mut offensive_foul_turnover_applied = false;

            for event in &adjudicated_events {
                if let GameEvent::Foul {
                    fouled_player_id,
                    fouler_id,
                    is_shooting,
                    ..
                } = event
                {
                    // 团队犯规是比赛级事实，必须计入箱体。
                    //
                    // 此前 `box_score.fouls` **零自增点**，CLI 恒定打印
                    // `Fouls: 0`（与 §23.10 的 `turnovers` 同类缺陷）：
                    // 声明的字段与事件事实脱钩。
                    //
                    // 口径说明：bonus 判定用的是 `team_fouls_{home,away}`
                    // （下方 `team_fouls >= bonus_fouls_per_period`），
                    // 不是本字段；此处是 CLI/批量工件消费的**比赛汇总**，
                    // 两者都源于同一个 `GameEvent::Foul`。
                    self.ledger.box_score.fouls = self.ledger.box_score.fouls.saturating_add(1);
                    if let Some(state) = self.observations.modulation.get_mut(fouled_player_id) {
                        state.catch_equilibrium = (state.catch_equilibrium
                            - self.config.rules.semantics.contact_minor_speed_ratio * 0.2)
                            .clamp(0.4, 1.0);
                    }
                    if let Some(state) = self.observations.modulation.get_mut(fouler_id) {
                        state.turnover_count = state.turnover_count.saturating_add(1);
                    }
                    let fouler_is_home = self
                        .systems
                        .physics
                        .get_player(fouler_id)
                        .map(|p| p.team == "home")
                        .unwrap_or(false);
                    let team_fouls = if fouler_is_home {
                        self.ledger.team_fouls_home = self.ledger.team_fouls_home.saturating_add(1);
                        self.ledger.team_fouls_home
                    } else {
                        self.ledger.team_fouls_away = self.ledger.team_fouls_away.saturating_add(1);
                        self.ledger.team_fouls_away
                    };
                    if let Some(player) = self.systems.physics.get_player_mut(fouler_id) {
                        player.foul_count = player.foul_count.saturating_add(1);
                    }
                    // 犯满离场（charter 7：个人犯满上限为联赛档案参数）。
                    if self.systems.physics.get_player(fouler_id).is_some_and(|p| {
                        p.foul_count >= self.config.rules.league.max_personal_fouls
                    }) {
                        self.forced_substitution(fouler_id);
                    }
                    self.journal.current_event = Some(
                        if *is_shooting {
                            "SHOOTING_FOUL"
                        } else {
                            "FOUL"
                        }
                        .to_string(),
                    );

                    let fouler_on_offense = offensive_fouler_ids.contains(fouler_id);
                    if !fouler_on_offense
                        && (*is_shooting
                            || team_fouls >= self.config.rules.league.bonus_fouls_per_period)
                    {
                        let award = if *is_shooting {
                            self.config.rules.league.shooting_foul_free_throws
                        } else {
                            self.config.rules.league.bonus_free_throws
                        };
                        // 球已离手在飞，或被犯规者本人的出手仍在动作窗口中
                        // 未离手（正在投篮动作中被侵犯，真实规则允许动作
                        // 完成：命中即加罚、不中即罚球）时，哨响不得截断
                        // 出手（R1.2）：程序入队，出手按自己的弹道到筐结算
                        // （命中计分、不中发 SHOT_MISS），结算点消费队头启动
                        // 罚球。直接置死球会把已释放的出手凭空丢弃——无
                        // 结果事实、命中分丢失。
                        let pending_release_is_fouled_shot = self
                            .observations
                            .pending_shot_release
                            .as_ref()
                            .is_some_and(|p| p.shooter_id == *fouled_player_id);
                        let shot_in_flight =
                            matches!(self.ball.ball_state, BallTrajectoryKind::Shot { .. });
                        let continuation = shot_in_flight || pending_release_is_fouled_shot;
                        if self.ledger.free_throws_remaining > 0
                            || self.ledger.free_throw_shooter.is_some()
                            || continuation
                        {
                            // 逐次罚球（charter §5.3）：当前程序进行中时，新判罚
                            // 进入队列，不覆写进行中的程序（此前实现会直接覆写，
                            // 先判罚的罚球被静默丢弃——犯规账本必报不平衡）。
                            // 判罚事件 ID 在发布循环分配后回填（见 events.rs）。
                            self.ledger.free_throw_queue.push_back((
                                fouled_player_id.clone(),
                                award,
                                None,
                            ));
                        } else if let Some(pending) = self.observations.pending_shot_release.take()
                        {
                            // 哨响时另有他人未离手的挂起出手：哨后出手不成立，
                            // 窗口取消、挂起静默作废（不发布 ShotRelease 事实，
                            // 否则事件流出现无结果的出手），罚球程序立即开始。
                            self.observations.active_windows.remove(&pending.shooter_id);
                            self.ledger.free_throws_remaining = award;
                            // The fouled team keeps possession and shoots.
                            self.ledger.free_throw_shooter = Some(fouled_player_id.clone());
                            self.ledger.free_throw_source_foul = None;
                            self.set_game_flow(GameFlowState::FreeThrow);
                            self.transition_phase(SubPhase::DeadBallReset);
                            // 哨响时传球仍在飞行：补发 PASS_DROPPED 终结后再进入死球。
                            self.emit_whistle_pass_drop();
                            // 犯规裁判哨响后，球从活球位置进入罚球程序。
                            let (foul_pos, foul_height) = self.ball.ball_pos_3d;
                            self.transition_ball_state(self.dead_state(foul_pos, foul_height));
                        } else {
                            self.ledger.free_throws_remaining = award;
                            // The fouled team keeps possession and shoots.
                            self.ledger.free_throw_shooter = Some(fouled_player_id.clone());
                            self.ledger.free_throw_source_foul = None;
                            self.set_game_flow(GameFlowState::FreeThrow);
                            self.transition_phase(SubPhase::DeadBallReset);
                            // 哨响时传球仍在飞行：补发 PASS_DROPPED 终结后再进入死球。
                            self.emit_whistle_pass_drop();
                            // 犯规裁判哨响后，球从活球位置进入罚球程序。
                            let (foul_pos, foul_height) = self.ball.ball_pos_3d;
                            self.transition_ball_state(self.dead_state(foul_pos, foul_height));
                        }
                    } else {
                        // 防守犯规后进攻方保留球权；进攻犯规后球权交给对方。
                        self.cancel_all_active_windows(
                            nba_domain::event::ActionCancellationReason::PreemptedByFoul,
                        );
                        let fouler_fouled_out = self
                            .systems
                            .physics
                            .get_player(fouler_id)
                            .is_some_and(|player| {
                                player.foul_count >= self.config.rules.league.max_personal_fouls
                            });
                        if fouler_on_offense && fouler_fouled_out {
                            if !offensive_foul_turnover_applied {
                                // 犯满进攻犯规者若继续持球，会阻塞强制换人；先转入死球，
                                // 清除球态中的持球人引用，再执行犯满换人。
                                offensive_foul_turnover_applied = true;
                                let carrier_has_ball = matches!(
                                    &self.ball.ball_state,
                                    BallTrajectoryKind::Held { carrier_id }
                                        if carrier_id == fouler_id
                                ) || matches!(
                                    &self.ball.ball_state,
                                    BallTrajectoryKind::Drive { driver_id, .. }
                                        if driver_id == fouler_id
                                );
                                if carrier_has_ball {
                                    let (pos, height) = self.ball.ball_pos_3d;
                                    self.transition_ball_state(self.dead_state(pos, height));
                                }
                                if let BallTrajectoryKind::Dead {
                                    last_touch_player, ..
                                } = &mut self.ball.ball_state
                                {
                                    if last_touch_player.as_deref() == Some(fouler_id.as_str()) {
                                        *last_touch_player = None;
                                    }
                                }
                                self.substitute(
                                    fouler_id,
                                    nba_domain::SubstitutionReason::FoulTrouble,
                                    None,
                                );
                                self.emit_possession_summary(
                                    nba_domain::PossessionEndCause::TurnoverOffensiveFoul,
                                    None,
                                    Some(fouler_id.clone()),
                                    None,
                                );
                                self.emit_whistle_pass_drop();
                                let current_ball_3d = self.ball.ball_pos_3d;
                                let baseline = Court::nearest_boundary_with_geometry(
                                    current_ball_3d.0,
                                    self.config.rules.court,
                                );
                                self.start_inbound_transition(baseline, current_ball_3d);
                            }
                            continue;
                        }
                        // 防守方非投篮犯规：进攻方保留球权，重置进攻时间至 14 秒重新组织！
                        //
                        // ## 球在飞行中时不得重置子阶段（实测修复）
                        //
                        // 本条之前无条件把子阶段设为 `Initiation`。当犯规发生时
                        // 球已在飞行（`ball_state` 为 `Shot`/`Pass`），则该重置会在
                        // 球触地前把子阶段拉回 `Initiation`，之后弹道裁决再执行
                        // `Initiation -> FlightAndRebound`——这是评判器认为非法的迁移
                        // （`nba.v2` 的合法表里 `Initiation` 只允许到
                        // `ActionExecution`/`DeadBallReset`/`ShotAttempt`）。
                        //
                        // 实测（seed 12）：tick 51056 出手 → 51060 非投篮犯规
                        // （子阶段被设为 `Initiation`，而 `ball_status` 仍为 `SHOT`）
                        // → 51080 球触地发 `SHOT_MISS`，产生 1 条 Hard
                        // `PHASE_TRANSITION_LEGALITY`。
                        //
                        // 飞行的球必须先按自己的裁决触地（续到 `ShotAttempt`/
                        // `FlightAndRebound`），提前重置子阶段会伪造一个不存在的
                        // 阶段序列；因此仅在球不在飞行时重置子阶段，
                        // 否则保留原子阶段，只重置进攻时间。
                        self.clock.shot_clock = self.clock.shot_clock.max(14.0);
                        // 飞行的定义与 `ConstraintContext::is_ball_in_flight` 一致：
                        // 控制转移、传球、投篮、松球、篮板均属飞行；仅 Held 与 Dead 不算。
                        let in_flight = matches!(
                            self.ball_phase(),
                            nba_domain::BallPhase::ControlTransfer
                                | nba_domain::BallPhase::PassFlight
                                | nba_domain::BallPhase::ShotFlight
                                | nba_domain::BallPhase::Loose
                                | nba_domain::BallPhase::Rebound
                        );
                        // ## 罚球程序进行中时同样不得重置子阶段
                        //
                        // 罚球间隙（球为 `Dead`，所以 `in_flight` 为假）再判一次
                        // 非投篮犯规时，本条会把子阶段拉回 `Initiation`；而罚球
                        // 不中后的篮板裁决随后执行 `Initiation -> FlightAndRebound`，
                        // 这是合法表禁止的迁移（`Initiation` 只允许到
                        // `ActionExecution`/`DeadBallReset`/`ShotAttempt`）。
                        //
                        // 实测（seed 42 tick 39675）：罚球程序进行中判一次
                        // 非投篮犯规 → 子阶段回 `Initiation` → tick 39731 罚球不中
                        // 进入篮板，产生一条 Hard `PHASE_TRANSITION_LEGALITY`。
                        //
                        // 罚球程序属于投篮族，其子阶段不应被无球犯规打断；
                        // 只需重置进攻时间，与球在飞行时的处理一致。
                        let free_throw_program_active = self.flow.game_flow
                            == GameFlowState::FreeThrow
                            && self.ledger.free_throws_remaining > 0;
                        if !in_flight && !free_throw_program_active {
                            self.transition_phase(SubPhase::Initiation);
                        }
                    }
                }
            }

            let mut applied_keys = std::collections::HashSet::new();
            for finding in &findings {
                let key = finding.summary();
                // 只有 Violate 才触发强制项：Flagged 是咨询性信号（如非持球人的
                // BOUNDARY_CROSSING），不能生成 RuleViolation 事件——否则每个
                // 边界事实都会附带一条伪 VIOLATION/ENFORCEMENT_APPLIED。
                let is_violate = matches!(finding.result.status, ConstraintStatus::Violate { .. });
                if is_violate
                    && finding.enforcement != EnforcementAction::None
                    && applied_keys.insert(key)
                {
                    self.apply_enforcement(finding);
                }
            }

            self.journal.current_event_types.extend(
                adjudicated_events
                    .iter()
                    .map(|event| event.event_type_str().to_string()),
            );
            for mut event in adjudicated_events {
                self.journal.event_sequence = self.journal.event_sequence.saturating_add(1);
                // D4.1：分配全场唯一 event_id，并按因果父子关系链接：
                // 本 tick 的第一个事件成为该 tick 的因果根，后续同 tick 事件
                // 以它为父——这使得同一决策触发的多条事实（如
                // SHOT_RELEASE → SCORE）在账本上构成一条因果链，
                // 评判器不再需要用"事件窗口猜测"重建因果（gap.md §7.1/§7.2）。
                self.journal.event_id_counter = self.journal.event_id_counter.saturating_add(1);
                let event_id = self.journal.event_id_counter;
                let kind = event.event_type_str().to_string();
                if let GameEvent::Foul {
                    fouled_player_id,
                    fouler_id,
                    is_shooting,
                    foul_kind,
                    personal_foul_count,
                    period_team_foul_count,
                    penalty,
                } = &mut event
                {
                    let p_count = self
                        .systems
                        .physics
                        .get_player(fouler_id)
                        .map(|p| p.foul_count)
                        .unwrap_or(1);
                    let fouler_is_home = self
                        .systems
                        .physics
                        .get_player(fouler_id)
                        .map(|p| p.team == "home")
                        .unwrap_or(false);
                    let t_count = if fouler_is_home {
                        self.ledger.team_fouls_home
                    } else {
                        self.ledger.team_fouls_away
                    };
                    *personal_foul_count = Some(p_count);
                    *period_team_foul_count = Some(t_count as u8);
                    *foul_kind = Some(if *is_shooting {
                        nba_domain::FoulKind::Shooting
                    } else {
                        nba_domain::FoulKind::Personal
                    });
                    // 罚则载荷取**本次犯规自己的判罚**：投篮犯规给投篮犯规
                    // 罚球数，非投篮但球队已达 bonus 给 bonus 罚球数——与
                    // 该判罚是否已进入队列无关（队列只影响执行时机），
                    // 犯规账本据此核对逐次罚球。
                    let awarded_free_throws = if offensive_fouler_ids.contains(fouler_id) {
                        0
                    } else if *is_shooting {
                        self.config.rules.league.shooting_foul_free_throws
                    } else if t_count >= self.config.rules.league.bonus_fouls_per_period {
                        self.config.rules.league.bonus_free_throws
                    } else {
                        0
                    };
                    *penalty = Some(nba_domain::FoulPenalty {
                        free_throw_count: awarded_free_throws,
                        retains_possession: awarded_free_throws > 0,
                        is_bonus: t_count >= self.config.rules.league.bonus_fouls_per_period,
                    });
                    // 判罚事件 ID 回填到罚球程序/队列：罚球事件的因果父必须
                    // 指向判罚它的那次犯规，而不是因果槽里的最近犯规
                    // （队列存在时两者会分离，账本会误报跨判罚错配）。
                    // 同一 tick 内同一投篮被判多次犯规时，队列会有多条同
                    // 罚球手的未认领项：按 FIFO 取最早未认领的一条，与
                    // 发布顺序、程序执行顺序保持一致（取队尾会把首程序的
                    // 判罚错配给末程序，首程序父链悬空）。
                    if awarded_free_throws > 0 {
                        if self.ledger.free_throw_shooter.as_deref()
                            == Some(fouled_player_id.as_str())
                            && self.ledger.free_throws_remaining > 0
                            && self.ledger.free_throw_source_foul.is_none()
                        {
                            self.ledger.free_throw_source_foul = Some(event_id);
                        } else if let Some(entry) =
                            self.ledger
                                .free_throw_queue
                                .iter_mut()
                                .find(|(shooter, _, src)| {
                                    *shooter == *fouled_player_id && src.is_none()
                                })
                        {
                            entry.2 = Some(event_id);
                        }
                    }
                }
                // D4.1：按语义槽位解析父事件，并登记本事件作为新的触发事件。
                let parent_event_id = if kind == "SHOT_RELEASE" {
                    let GameEvent::ShotRelease {
                        creation_source,
                        transition_context,
                        transition_event_id,
                        source_event_id,
                        ..
                    } = &mut event
                    else {
                        unreachable!("SHOT_RELEASE event kind must carry ShotRelease data")
                    };
                    assert_eq!(
                        *transition_context,
                        transition_event_id.is_some(),
                        "transition context and transition event ID must agree"
                    );
                    if let Some(source_id) = transition_event_id {
                        assert_eq!(
                            self.journal.causal_links.get("transition").copied(),
                            Some(*source_id),
                            "transition event ID must match the current transition causal link"
                        );
                    }
                    match creation_source {
                        nba_domain::ShotCreationSource::DriveFinish => {
                            assert!(source_event_id.is_none());
                            let source_id = self
                                .journal
                                .causal_links
                                .get("drive")
                                .copied()
                                .expect("drive finish must follow a published drive outcome");
                            *source_event_id = Some(source_id);
                            Some(source_id)
                        }
                        nba_domain::ShotCreationSource::DrivePullUp => {
                            let source_id = source_event_id
                                .expect("drive pull-up must include its DriveInitiated event ID");
                            assert_eq!(
                                self.journal.causal_links.get("drive").copied(),
                                Some(source_id),
                                "drive pull-up source must match the current drive causal link"
                            );
                            Some(source_id)
                        }
                        nba_domain::ShotCreationSource::CutReception => {
                            let source_id = source_event_id
                                .expect("cut reception must include its PassReceived event ID");
                            assert_eq!(
                                self.journal.causal_links.get("pass_received").copied(),
                                Some(source_id),
                                "cut source must match the current PassReceived causal link"
                            );
                            Some(source_id)
                        }
                        nba_domain::ShotCreationSource::OffensiveReboundPutback => {
                            let source_id = source_event_id
                                .expect("putback must include its offensive Rebound event ID");
                            assert_eq!(
                                self.journal.causal_links.get("rebound").copied(),
                                Some(source_id),
                                "putback source must match the current Rebound causal link"
                            );
                            Some(source_id)
                        }
                        nba_domain::ShotCreationSource::TransitionFinish => {
                            let source_id = transition_event_id
                                .expect("transition finish must include transition_event_id");
                            assert_eq!(
                                self.journal.causal_links.get("transition").copied(),
                                Some(source_id),
                                "transition source must match the current transition causal link"
                            );
                            Some(source_id)
                        }
                        nba_domain::ShotCreationSource::SetPlay => {
                            assert!(source_event_id.is_none());
                            None
                        }
                    }
                } else if kind == "TRANSITION_STARTED" {
                    None
                } else if kind == "FREE_THROW" {
                    // 罚球的因果父是结算时冻结的判罚事件 ID（FIFO 与罚球
                    // 事件一一对应）：队列切换同 tick 时读当前 source 会
                    // 错配到下一程序的判罚。
                    self.ledger
                        .free_throw_event_parents
                        .pop_front()
                        .filter(|parent| *parent != 0)
                } else {
                    causal_parent_of(&kind)
                        .and_then(|slot| self.journal.causal_links.get(slot).copied())
                };
                if kind == "SHOT_RELEASE" {
                    let GameEvent::ShotRelease { is_three, .. } = &event else {
                        unreachable!("SHOT_RELEASE event kind must carry ShotRelease data")
                    };
                    self.ledger.box_score.fg2_attempts += u32::from(!is_three);
                    self.ledger.box_score.fg3_attempts += u32::from(*is_three);
                }
                if kind == "TRANSITION_STARTED" {
                    self.journal.causal_links.insert("transition", event_id);
                }
                if let Some(slot) = causal_trigger_slot(&kind) {
                    self.journal.causal_links.insert(slot, event_id);
                }
                if let GameEvent::PassDropped { receiver_id, .. }
                | GameEvent::PassTipped { receiver_id, .. }
                | GameEvent::PassIntercepted { receiver_id, .. } = &event
                {
                    self.observations.cut_route_players.remove(receiver_id);
                }
                if let GameEvent::PassReceived {
                    receiver_id,
                    is_cut_reception: true,
                    ..
                } = &event
                {
                    self.observations
                        .cut_route_players
                        .insert(receiver_id.clone());
                }
                match &event {
                    GameEvent::TransitionStarted { .. } => {
                        self.possession_ctx.transition_start_time = Some(self.clock.current_time);
                        self.possession_ctx.last_transition_event_parent = Some(event_id);
                    }
                    GameEvent::DriveOutcome {
                        successful: true, ..
                    } => {}
                    GameEvent::PassReceived {
                        receiver_id,
                        is_cut_reception,
                        ..
                    } => {
                        self.possession_ctx.last_pass_received_time = Some(self.clock.current_time);
                        self.possession_ctx.pass_receiver_decision_player =
                            Some(receiver_id.clone());
                        if *is_cut_reception {
                            self.possession_ctx.last_cut_reception_time =
                                Some(self.clock.current_time);
                            self.possession_ctx.last_cut_reception_player =
                                Some(receiver_id.clone());
                            self.possession_ctx.last_cut_reception_event_parent = Some(event_id);
                        }
                    }
                    GameEvent::ReboundContest {
                        rebounder_id,
                        is_offensive: true,
                        ..
                    } => {
                        let is_live_recovery = matches!(
                            self.ball.ball_state,
                            BallTrajectoryKind::Held { ref carrier_id }
                                | BallTrajectoryKind::ControlTransfer { ref carrier_id, .. }
                                if carrier_id == rebounder_id
                        );
                        if is_live_recovery {
                            self.possession_ctx.last_offensive_rebound_time =
                                Some(self.clock.current_time);
                            self.possession_ctx.last_offensive_rebound_player =
                                Some(rebounder_id.clone());
                            self.possession_ctx.last_offensive_rebound_event_parent =
                                Some(event_id);
                        }
                    }
                    _ => {}
                }
                let data = serde_json::to_value(&event).ok();
                self.journal.current_event_log.push(FrameEvent {
                    sequence: self.journal.event_sequence,
                    time: (self.clock.current_time * 100.0).round() / 100.0,
                    kind,
                    data,
                    event_id,
                    parent_event_id,
                });
            }
            if let Some(primary) = self.journal.current_event_types.last() {
                self.journal.current_event = Some(primary.clone());
            }
        }
    }

    fn apply_enforcement(&mut self, finding: &nba_decision::constraint::ConstraintFinding) {
        match &finding.enforcement {
            EnforcementAction::None
            | EnforcementAction::BlockAction
            | EnforcementAction::ModifyAction { .. }
            | EnforcementAction::DelayAction { .. }
            | EnforcementAction::TriggerFoul { .. }
            | EnforcementAction::ResetPosition => {}
            EnforcementAction::Violation { kind } => {
                let action = kind.as_str().to_string();
                self.cancel_all_active_windows(
                    nba_domain::event::ActionCancellationReason::PreemptedByViolation,
                );
                self.journal
                    .current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.journal.pending_events.push(GameEvent::RuleViolation {
                    constraint_id: finding.constraint.id.to_string(),
                    reason: action.clone(),
                });
                self.journal
                    .pending_events
                    .push(GameEvent::EnforcementApplied {
                        constraint_id: finding.constraint.id.to_string(),
                        action,
                    });
            }
            EnforcementAction::ChangePossession { reason } => {
                self.journal
                    .current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, reason));
                self.journal
                    .pending_events
                    .push(GameEvent::EnforcementApplied {
                        constraint_id: finding.constraint.id.to_string(),
                        action: format!("CHANGE_POSSESSION:{}", reason),
                    });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
            EnforcementAction::EndPhase { next } => {
                let action = format!("END_PHASE:{}", next.as_str());
                self.journal
                    .current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.journal
                    .pending_events
                    .push(GameEvent::EnforcementApplied {
                        constraint_id: finding.constraint.id.to_string(),
                        action,
                    });
                if *next == PhaseType::DeadBallReset {
                    self.set_game_flow(GameFlowState::DeadBall);
                    self.transition_phase(SubPhase::DeadBallReset);
                }
            }
            EnforcementAction::Turnover { reason } => {
                self.journal
                    .current_enforcements
                    .push(format!("{}:TURNOVER:{}", finding.constraint.id, reason));
                self.journal
                    .pending_events
                    .push(GameEvent::EnforcementApplied {
                        constraint_id: finding.constraint.id.to_string(),
                        action: format!("TURNOVER:{}", reason),
                    });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
        }
    }
}
