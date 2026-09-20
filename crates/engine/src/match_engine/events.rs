//! 事件裁决与发布：本 tick 收集的领域事实经裁判重判、强制项应用后，
//! 分配全场唯一 `event_id` 并按语义因果槽位串链写入事件日志。
//!
//! 依据 `gap.md` §7.1/§7.2：只有真实因果关系才串链，同 tick 相邻不等于因果。

use nba_decision::constraint::{ConstraintStatus, EnforcementAction, PhaseType, ViolationKind};
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_protocol::FrameEvent;

use super::MatchEngine;

/// D4.1 因果链定义：事件 kind → 它属于哪个“结果槽位”的父。
///
/// 只登记真实因果关系（动作→结果），不猜测同 tick 相邻即因果：
/// - 投篮释放 → 进筐/失手/篮板
/// - 传球释放 → 接球/被点掉/掉球/被断
/// - 犯规 → 罚球尝试
/// - 突破发起 → 突破结果
fn causal_parent_of(kind: &str) -> Option<&'static str> {
    match kind {
        "SCORE" | "SHOT_MISS" | "REBOUND" => Some("shot"),
        "PASS_RECEIVED" | "PASS_TIPPED" | "PASS_DROPPED" | "STEAL" => Some("pass"),
        "FREE_THROW" => Some("foul"),
        "DRIVE_SCORE" | "DRIVE_MISS" | "DRIVE_STOPPED" => Some("drive"),
        _ => None,
    }
}

/// D4.1 因果链定义：事件 kind → 它在哪个槽位上充当后续事件的父。
fn causal_trigger_slot(kind: &str) -> Option<&'static str> {
    match kind {
        "SHOT_RELEASE" => Some("shot"),
        "PASS" => Some("pass"),
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
                    if let Some(contact) =
                        self.observations.latest_contacts.iter().find(|contact| {
                            contact.raw.entity_a == *player_a && contact.raw.entity_b == *player_b
                        })
                    {
                        if let ResolutionOutcome::Foul {
                            fouled_player_id,
                            fouler_id,
                            is_shooting,
                        } = ResolutionLayer::resolve_semantic_contact(
                            contact,
                            self.systems.physics.get_players(),
                            &self.config.rules.resolve.contact,
                            &mut self.systems.rng,
                        ) {
                            adjudicated_events.push(GameEvent::Foul {
                                fouled_player_id,
                                fouler_id,
                                is_shooting,
                            });
                        }
                    }
                }
                adjudicated_events.push(event);
            }
            let ctx = self.constraint_ctx();
            let findings = self
                .systems
                .decision
                .registry
                .evaluate_events(&ctx, &adjudicated_events);

            for event in &adjudicated_events {
                if let GameEvent::Foul {
                    fouled_player_id,
                    fouler_id,
                    is_shooting,
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
                    if *is_shooting || team_fouls >= self.config.rules.league.bonus_fouls_per_period
                    {
                        self.ledger.free_throws_remaining = if *is_shooting {
                            self.config.rules.league.shooting_foul_free_throws
                        } else {
                            self.config.rules.league.bonus_free_throws
                        };
                        // The fouled team keeps possession and shoots.
                        self.ledger.free_throw_shooter = Some(fouled_player_id.clone());
                        self.set_game_flow(GameFlowState::FreeThrow);
                        self.transition_phase(SubPhase::DeadBallReset);
                        // F1.1：罚球程序把球权威态收敛为 Dead（置于罚球点），
                        // 否则权威态仍是 Held{持球人} 而球在篮筐/罚球点，
                        // 投影出 BALL_WITH_HOLDER Hard（gap.md §5.1）。
                        let shooter_is_home = self
                            .systems
                            .physics
                            .get_player(fouled_player_id)
                            .map(|p| p.team == "home")
                            .unwrap_or(self.flow.possession == Possession::Home);
                        let ft_spot = Court::free_throw_pos(shooter_is_home, &self.config.rules);
                        self.ball.ball_pos_3d = (ft_spot, self.config.rules.ball_holder_height_ft);
                        self.transition_ball_state(
                            self.dead_state(ft_spot, self.config.rules.ball_holder_height_ft),
                        );
                    } else {
                        // 普通犯规（非投篮且未到奖励罚球）：进攻方保留球权，重置进攻时间至 14 秒重新组织！
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
            for event in adjudicated_events {
                self.journal.event_sequence = self.journal.event_sequence.saturating_add(1);
                // D4.1：分配全场唯一 event_id，并按因果父子关系链接：
                // 本 tick 的第一个事件成为该 tick 的因果根，后续同 tick 事件
                // 以它为父——这使得同一决策触发的多条事实（如
                // SHOT_RELEASE → SCORE）在账本上构成一条因果链，
                // 评判器不再需要用"事件窗口猜测"重建因果（gap.md §7.1/§7.2）。
                self.journal.event_id_counter = self.journal.event_id_counter.saturating_add(1);
                let event_id = self.journal.event_id_counter;
                let kind = event.event_type_str().to_string();
                // D4.1：按语义槽位解析父事件，并登记本事件作为新的触发事件。
                let parent_event_id = causal_parent_of(&kind)
                    .and_then(|slot| self.journal.causal_links.get(slot).copied());
                if let Some(slot) = causal_trigger_slot(&kind) {
                    self.journal.causal_links.insert(slot, event_id);
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
