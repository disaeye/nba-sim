//! 验证业务身份与独立观测事实（validation.md §3.2–§3.3）。
//!
//! 本模块只投影已发布的领域事实：业务身份按实例生命周期分配，观测载荷从
//! 生产事实推导。派生事件不参与球态转移、规则裁决或记账，仅供验证器
//! 只读取证。

use nba_domain::{DefenseResponsibility, GameEvent};
use nba_protocol::EventRelationKind;

use super::MatchEngine;

/// 一次事件发射的业务身份集合。`None` 字段不写入帧事件；
/// 集合由 `assign_business_identity` 按事件种类填充。
#[derive(Debug, Default)]
pub(crate) struct BusinessIdentity {
    pub(crate) action: Option<String>,
    pub(crate) shot: Option<String>,
    pub(crate) pass: Option<String>,
    pub(crate) loose_ball: Option<String>,
    pub(crate) foul_program: Option<String>,
    pub(crate) responsibility: Option<String>,
    pub(crate) movement_intent: Option<String>,
    pub(crate) play_instance: Option<String>,
}

impl MatchEngine {
    /// 事件身份由单一发布入口按实例生命周期分配：创建事件取号、
    /// 延续事件携带、终结事件清算；关系只从引擎已计算的事实推导。
    pub(crate) fn assign_business_identity(
        &mut self,
        event: &GameEvent,
        event_id: u64,
        parent_event_id: Option<u64>,
    ) -> (BusinessIdentity, Vec<nba_protocol::EventRelation>) {
        let mut ids = BusinessIdentity::default();
        let mut relations = Vec::new();
        if let Some(parent) = parent_event_id.filter(|parent| *parent != 0 && *parent < event_id) {
            relations.push(relation(EventRelationKind::CausedBy, parent));
        }
        match event {
            GameEvent::ActionStarted { player_id, .. } => {
                let id = self.next_business_id("action");
                self.journal
                    .active_business
                    .insert(format!("action:{player_id}"), id.clone());
                ids.action = Some(id);
            }
            GameEvent::ActionCompleted { player_id, .. }
            | GameEvent::ActionCancelled { player_id, .. }
            | GameEvent::ActionFailed { player_id, .. } => {
                ids.action = self.take_business_id(&format!("action:{player_id}"));
            }
            GameEvent::ShotRelease { .. } => {
                let id = self.next_business_id("shot");
                self.journal
                    .active_business
                    .insert("shot".to_string(), id.clone());
                ids.shot = Some(id);
                if let Some(play_event) = self
                    .journal
                    .causal_links
                    .get("play_activation")
                    .copied()
                    .filter(|play_event| *play_event != 0)
                {
                    relations.push(relation(EventRelationKind::InfluencedBy, play_event));
                }
            }
            GameEvent::HoopArrival { .. }
            | GameEvent::ShotContact { .. }
            | GameEvent::ShotTrajectoryArrival { .. } => {
                ids.shot = self.journal.active_business.get("shot").cloned();
            }
            GameEvent::ShotAttemptResolved {
                scoring_decision, ..
            } => {
                // 进球回合的 RESOLVED 派生发布晚于 PossessionSummary（后者
                // 已 close "shot" 实例），此处 get 不到时回退最近关闭的
                // shot 身份——命中结算的事件信封必须携带 shot_id（验证器
                // 四区命中记账与因果配对依赖它）。
                ids.shot = self
                    .journal
                    .active_business
                    .get("shot")
                    .cloned()
                    .or_else(|| self.journal.last_closed_shot_id.clone());
                self.close_business_key("shot");
                if scoring_decision != "fga_fgm" {
                    self.open_loose_ball(&mut ids);
                }
            }
            GameEvent::BlockedShot { .. } => {
                ids.shot = self.journal.active_business.get("shot").cloned();
                if let Some(shot_event) = self
                    .journal
                    .causal_links
                    .get("shot")
                    .copied()
                    .filter(|shot_event| *shot_event != 0)
                {
                    relations.push(relation(EventRelationKind::CausedBy, shot_event));
                }
                self.open_loose_ball(&mut ids);
            }
            GameEvent::PassRelease { .. } => {
                let id = self.next_business_id("pass");
                self.journal
                    .active_business
                    .insert("pass".to_string(), id.clone());
                ids.pass = Some(id);
            }
            GameEvent::PassLandingCorrected { .. } => {
                ids.pass = self.journal.active_business.get("pass").cloned();
            }
            GameEvent::PassReceived { .. } | GameEvent::PassIntercepted { .. } => {
                ids.pass = self.take_business_id("pass");
            }
            GameEvent::PassDropped { .. } | GameEvent::PassTipped { .. } => {
                ids.pass = self.take_business_id("pass");
                self.open_loose_ball(&mut ids);
            }
            GameEvent::BallPokedLoose { .. } => self.open_loose_ball(&mut ids),
            GameEvent::LooseBallSecured { .. } => {
                ids.loose_ball = self.take_business_id("loose_ball");
            }
            GameEvent::ReboundContest { .. } => {
                ids.loose_ball = self.take_business_id("loose_ball");
                if let Some(parent) = parent_event_id.filter(|parent| *parent != 0) {
                    relations.push(relation(EventRelationKind::RecoveredFrom, parent));
                }
            }
            GameEvent::Foul { penalty, .. } => {
                if penalty.as_ref().is_some_and(|p| p.free_throw_count > 0) {
                    let id = self.next_business_id("foul_program");
                    self.journal
                        .active_business
                        .insert("foul_program".to_string(), id.clone());
                    ids.foul_program = Some(id);
                } else {
                    ids.foul_program = self.journal.active_business.get("foul_program").cloned();
                }
            }
            GameEvent::FreeThrowAttempt { attempt, .. } => {
                ids.foul_program = self.journal.active_business.get("foul_program").cloned();
                if let Some(parent) = parent_event_id.filter(|parent| *parent != 0) {
                    relations.push(relation(EventRelationKind::AwardedBy, parent));
                }
                if *attempt > 1 {
                    if let Some(previous) = self
                        .journal
                        .causal_links
                        .get("foul_ft")
                        .copied()
                        .filter(|previous| *previous != 0 && *previous != event_id)
                    {
                        relations.push(relation(EventRelationKind::Continues, previous));
                    }
                }
            }
            GameEvent::TransitionStarted { .. } => {
                if let Some(parent) = ["rebound", "steal", "loose_ball"]
                    .iter()
                    .find_map(|slot| self.journal.causal_links.get(slot).copied())
                    .filter(|parent| *parent != 0)
                {
                    relations.push(relation(EventRelationKind::CreatedBy, parent));
                }
            }
            GameEvent::PlayActivated { .. } => {
                let id = self.next_business_id("play_instance");
                self.journal
                    .active_business
                    .insert("play_instance".to_string(), id.clone());
                self.journal
                    .causal_links
                    .insert("play_activation", event_id);
                ids.play_instance = Some(id);
            }
            GameEvent::PlayWindowEnded { .. } => {
                ids.play_instance = self.take_business_id("play_instance");
            }
            GameEvent::MovementIntent { player_id, .. } => {
                let key = format!("movement_intent:{player_id}");
                let id = self.next_business_id("movement_intent");
                self.journal.active_business.insert(key, id.clone());
                ids.movement_intent = Some(id);
            }
            GameEvent::MovementOutcome { player_id, .. } => {
                ids.movement_intent =
                    self.take_business_id(&format!("movement_intent:{player_id}"));
            }
            GameEvent::ResponsibilityStarted { defender_id, .. } => {
                let key = format!("responsibility:{defender_id}");
                let id = self.next_business_id("responsibility");
                self.journal.active_business.insert(key, id.clone());
                ids.responsibility = Some(id);
            }
            GameEvent::ResponsibilityEnded { defender_id, .. } => {
                ids.responsibility =
                    self.take_business_id(&format!("responsibility:{defender_id}"));
            }
            GameEvent::DefenseResponsibilityChanged { defender_id, .. } => {
                ids.responsibility = self
                    .journal
                    .active_business
                    .get(&format!("responsibility:{defender_id}"))
                    .cloned();
            }
            GameEvent::PossessionSummary(_) => {
                self.close_business_key("shot");
                self.close_business_key("pass");
                self.close_business_key("loose_ball");
                self.close_business_key("foul_program");
            }
            _ => {}
        }
        (ids, relations)
    }

    /// 从既有生产事实派生验证器所需的观测载荷：统计记账声明、投篮终局、
    /// 控制建立与防守责任生命周期。这些事件只声明事实，不驱动状态。
    pub(crate) fn derive_observation_events(
        &self,
        event: &GameEvent,
        business_ids: &BusinessIdentity,
    ) -> Vec<GameEvent> {
        let mut out = Vec::new();
        let team_of = |player_id: &str| {
            self.systems
                .physics
                .get_player(player_id)
                .map(|player| player.team.clone())
                .unwrap_or_default()
        };
        match event {
            GameEvent::ShotRelease {
                shooter_id,
                is_three,
                ..
            } => {
                let team = team_of(shooter_id);
                out.push(stat(
                    team,
                    Some(shooter_id.clone()),
                    "FGA",
                    business_ids.shot.clone(),
                    "shot_release_counts_fga",
                ));
                if *is_three {
                    out.push(stat(
                        team_of(shooter_id),
                        Some(shooter_id.clone()),
                        "3PA",
                        business_ids.shot.clone(),
                        "three_point_shot_release",
                    ));
                }
            }
            GameEvent::HoopArrival {
                shooter_id,
                is_made,
                is_three,
                ..
            } => {
                let physical_outcome = if *is_made {
                    "made"
                } else if self.tick_has_shot_contact("Rim") {
                    "miss_rim"
                } else if self.tick_has_shot_contact("Backboard") {
                    "miss_backboard"
                } else {
                    "miss_no_rim"
                };
                out.push(GameEvent::ShotAttemptResolved {
                    shooter_id: shooter_id.clone(),
                    physical_outcome: physical_outcome.to_string(),
                    scoring_decision: if *is_made { "fga_fgm" } else { "fga_miss" }.to_string(),
                    rule_basis: if *is_made {
                        "hoop_arrival_made"
                    } else {
                        "hoop_arrival_miss"
                    }
                    .to_string(),
                    is_three: *is_three,
                });
                if *is_made {
                    out.push(stat(
                        team_of(shooter_id),
                        Some(shooter_id.clone()),
                        "FGM",
                        business_ids.shot.clone(),
                        "valid_field_goal_made",
                    ));
                    out.push(GameEvent::StatAttribution {
                        team: team_of(shooter_id),
                        player_id: Some(shooter_id.clone()),
                        stat_kind: "PTS".to_string(),
                        amount: if *is_three { 3 } else { 2 },
                        source_business_id: business_ids.shot.clone(),
                        rule_basis: Some("valid_field_goal_points".to_string()),
                    });
                    if *is_three {
                        out.push(stat(
                            team_of(shooter_id),
                            Some(shooter_id.clone()),
                            "3PM",
                            business_ids.shot.clone(),
                            "valid_three_point_make",
                        ));
                    }
                }
            }
            GameEvent::BlockedShot {
                shooter_id,
                blocker_id,
                is_three,
                ..
            } => {
                out.push(GameEvent::ShotAttemptResolved {
                    shooter_id: shooter_id.clone(),
                    physical_outcome: "blocked".to_string(),
                    scoring_decision: "fga_blocked".to_string(),
                    rule_basis: "legal_blocked_attempt_counts_fga".to_string(),
                    is_three: *is_three,
                });
                out.push(stat(
                    team_of(blocker_id),
                    Some(blocker_id.clone()),
                    "BLK",
                    business_ids.shot.clone(),
                    "legal_block_contact",
                ));
            }
            GameEvent::PassIntercepted {
                defender_id,
                position,
                ..
            } => {
                out.push(stat(
                    team_of(defender_id),
                    Some(defender_id.clone()),
                    "STL",
                    business_ids.pass.clone(),
                    "defender_controlled_intercept",
                ));
                out.push(GameEvent::ControlEstablished {
                    player_id: defender_id.clone(),
                    team: team_of(defender_id),
                    source: "steal".to_string(),
                    position: *position,
                });
            }
            GameEvent::PassReceived {
                receiver_id,
                position,
                ..
            } => {
                out.push(GameEvent::ControlEstablished {
                    player_id: receiver_id.clone(),
                    team: team_of(receiver_id),
                    source: "pass_reception".to_string(),
                    position: *position,
                });
            }
            GameEvent::LooseBallSecured {
                player_id,
                position,
            } => {
                out.push(GameEvent::ControlEstablished {
                    player_id: player_id.clone(),
                    team: team_of(player_id),
                    source: "loose_ball_recovery".to_string(),
                    position: *position,
                });
            }
            GameEvent::ReboundContest {
                rebounder_id,
                landing_pos,
                is_offensive,
            } => {
                let team = team_of(rebounder_id);
                out.push(stat(
                    team.clone(),
                    Some(rebounder_id.clone()),
                    if *is_offensive { "ORB" } else { "DRB" },
                    business_ids
                        .loose_ball
                        .clone()
                        .or_else(|| business_ids.shot.clone()),
                    if *is_offensive {
                        "offensive_rebound_control"
                    } else {
                        "defensive_rebound_control"
                    },
                ));
                out.push(GameEvent::ControlEstablished {
                    player_id: rebounder_id.clone(),
                    team,
                    source: "rebound".to_string(),
                    position: *landing_pos,
                });
            }
            GameEvent::Foul {
                fouler_id,
                foul_kind,
                ..
            } => {
                out.push(stat(
                    team_of(fouler_id),
                    Some(fouler_id.clone()),
                    "PF",
                    business_ids.foul_program.clone(),
                    "personal_foul_event",
                ));
                if matches!(foul_kind, Some(nba_domain::FoulKind::Offensive)) {
                    out.push(stat(
                        team_of(fouler_id),
                        Some(fouler_id.clone()),
                        "TOV",
                        business_ids.foul_program.clone(),
                        "offensive_foul_turnover",
                    ));
                }
            }
            GameEvent::FreeThrowAttempt {
                shooter_id, made, ..
            } => {
                out.push(stat(
                    team_of(shooter_id),
                    Some(shooter_id.clone()),
                    "FTA",
                    business_ids.foul_program.clone(),
                    "free_throw_attempt",
                ));
                if *made {
                    out.push(stat(
                        team_of(shooter_id),
                        Some(shooter_id.clone()),
                        "FTM",
                        business_ids.foul_program.clone(),
                        "free_throw_made",
                    ));
                    out.push(GameEvent::StatAttribution {
                        team: team_of(shooter_id),
                        player_id: Some(shooter_id.clone()),
                        stat_kind: "PTS".to_string(),
                        amount: 1,
                        source_business_id: business_ids.foul_program.clone(),
                        rule_basis: Some("free_throw_point".to_string()),
                    });
                }
            }
            GameEvent::PossessionSummary(summary) if summary.terminal_event.is_turnover() => {
                out.push(stat(
                    summary.offense_team.clone(),
                    summary.turnover_player_id.clone(),
                    "TOV",
                    None,
                    "possession_turnover_terminal",
                ));
            }
            _ => {}
        }
        out
    }

    fn open_loose_ball(&mut self, ids: &mut BusinessIdentity) {
        let id = self.next_business_id("loose_ball");
        self.journal
            .active_business
            .insert("loose_ball".to_string(), id.clone());
        ids.loose_ball = Some(id);
    }

    fn next_business_id(&mut self, instance_type: &'static str) -> String {
        let counter = self
            .journal
            .business_id_counters
            .entry(instance_type)
            .or_insert(0);
        *counter += 1;
        self.journal.open_business_instances += 1;
        format!("{instance_type}-{counter:06}")
    }

    fn take_business_id(&mut self, key: &str) -> Option<String> {
        let value = self.journal.active_business.remove(key);
        if value.is_some() {
            self.journal.open_business_instances =
                self.journal.open_business_instances.saturating_sub(1);
        }
        value
    }

    fn close_business_key(&mut self, key: &str) {
        let closed = self.take_business_id(key);
        if key == "shot" {
            self.journal.last_closed_shot_id = closed;
        }
    }

    /// 本 tick 是否存在指定表面的投篮接触事实（Rim/Backboard 用于
    /// 投失分类：触筐投失、触板投失、未触筐投失）。
    fn tick_has_shot_contact(&self, surface: &str) -> bool {
        self.journal.current_event_log.iter().any(|entry| {
            entry.kind == "SHOT_CONTACT"
                && entry
                    .data
                    .as_ref()
                    .and_then(|data| data.get("surface"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| value.eq_ignore_ascii_case(surface))
        })
    }

    /// 流尾部的未完成业务实例计数（validation.md §3.1）：stream 导出与
    /// 验证 runner 的直接推进通道共用同一只读访问器。
    pub fn open_business_instances(&self) -> u64 {
        self.journal.open_business_instances
    }

    /// 防守责任变更时的责任生命周期事实（validation.md §3.3）：
    /// 旧责任登记结束，新责任登记开始；首次登记只发开始事实。
    pub(crate) fn derive_responsibility_lifecycle(
        &self,
        defender_id: &str,
        old_responsibility: Option<DefenseResponsibility>,
        new_responsibility: DefenseResponsibility,
    ) -> Vec<GameEvent> {
        let mut out = Vec::new();
        if let Some(old) = old_responsibility {
            out.push(GameEvent::ResponsibilityEnded {
                defender_id: defender_id.to_string(),
                responsibility: old,
                end_reason: "switch".to_string(),
                achieved: true,
            });
        }
        out.push(GameEvent::ResponsibilityStarted {
            defender_id: defender_id.to_string(),
            responsibility: new_responsibility,
            offensive_player_id: None,
        });
        out
    }
}

fn relation(kind: EventRelationKind, event_id: u64) -> nba_protocol::EventRelation {
    nba_protocol::EventRelation { kind, event_id }
}

fn stat(
    team: String,
    player_id: Option<String>,
    stat_kind: &str,
    source_business_id: Option<String>,
    rule_basis: &str,
) -> GameEvent {
    GameEvent::StatAttribution {
        team,
        player_id,
        stat_kind: stat_kind.to_string(),
        amount: 1,
        source_business_id,
        rule_basis: Some(rule_basis.to_string()),
    }
}
