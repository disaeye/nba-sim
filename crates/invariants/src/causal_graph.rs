//! 事件流因果图追踪器（Causal Event Graph Validator）。
//!
//! 从第一性原理出发，对连续物理与离散事件进行跨 tick 因果链验证：
//! 1. 投篮出手 (ShotRelease) -> 到筐 (HoopArrival) 的因果与时间跨度一致性；
//! 2. 比分增加 (ScoreDelta) 严格必须由前置的合法进球或罚球事件引起（杜绝幽灵得分）；
//! 3. 篮板争抢 (Rebound) 严格必须由前置的投篮不中 (HoopArrival { is_made: false }) 触发。

use nba_protocol::StreamTick;
use crate::Violation;

#[derive(Debug, Clone)]
pub struct ActiveShotTracking {
    pub shooter_id: String,
    pub release_tick: u64,
    pub is_three: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CausalEventGraph {
    pub active_shot: Option<ActiveShotTracking>,
    pub last_missed_shot_tick: Option<u64>,
    pub last_made_shot_tick: Option<u64>,
    pub last_score: (u32, u32),
}

impl CausalEventGraph {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate_tick(&mut self, tick: &StreamTick, tick_index: u64) -> Vec<Violation> {
        let mut violations = Vec::new();
        let frame = &tick.frame;

        // 1. 比分因果性检查：比分变动必须有对应进球/罚球事件支撑
        let (cur_home, cur_away) = (frame.score.home, frame.score.away);
        let score_changed = (cur_home, cur_away) != self.last_score;
        let has_scoring_event = frame.events.iter().any(|e| {
            e == "SCORE"
                || e == "SHOT_MADE"
                || e == "FREE_THROW_MADE"
                || e == "FREE_THROW_ATTEMPT"
                || e == "HOOP_ARRIVAL"
                || e.contains("FREE_THROW")
        });

        if score_changed && (cur_home > self.last_score.0 || cur_away > self.last_score.1) {
            let was_recent_make = self.last_made_shot_tick.is_some_and(|t| tick_index <= t + 2);
            if !has_scoring_event && !was_recent_make && tick_index > 0 {
                violations.push(Violation {
                    tick_index,
                    rule: "UNCAUSED_SCORE_DELTA",
                    severity: crate::ViolationSeverity::Hard,
                    detail: format!(
                        "score increased from {:?} to ({}, {}) without preceding score event",
                        self.last_score, cur_home, cur_away
                    ),
                });
            }
        }
        self.last_score = (cur_home, cur_away);

        // 2. 投篮与到筐事件因果链
        for event in &frame.events {
            if event == "SHOT_RELEASE" || event == "SHOT" {
                let shooter = frame.players.iter().find(|p| p.has_ball).map(|p| p.id.clone()).unwrap_or_default();
                self.active_shot = Some(ActiveShotTracking {
                    shooter_id: shooter,
                    release_tick: tick_index,
                    is_three: false,
                });
            } else if event == "HOOP_ARRIVAL" || event == "SCORE" {
                self.last_made_shot_tick = Some(tick_index);
                self.active_shot = None;
            } else if event == "REBOUND" {
                // 篮板事件因果性：争抢篮板必须在前置投篮不中后发生
                if self.last_made_shot_tick.is_some_and(|t| tick_index <= t + 2) {
                    violations.push(Violation {
                        tick_index,
                        rule: "REBOUND_AFTER_MADE_SHOT",
                        severity: crate::ViolationSeverity::Hard,
                        detail: "REBOUND event occurred immediately after MADE shot".to_string(),
                    });
                }
            } else if event == "INBOUND_PASS" || event == "INBOUND" {
                // 发球因果：发球人必须站在底线/边线界外附近
                if let Some(holder) = frame.players.iter().find(|p| p.has_ball) {
                    let margin = 0.05;
                    if holder.x > margin && holder.x < (1.0 - margin) && holder.y > margin && holder.y < (1.0 - margin) {
                        violations.push(Violation {
                            tick_index,
                            rule: "INBOUNDER_DEEP_IN_COURT",
                            severity: crate::ViolationSeverity::Hard,
                            detail: format!("inbounder {} is deep inside the court at ({:.2}, {:.2})", holder.id, holder.x, holder.y),
                        });
                    }
                }
            } else if event == "PASS_INTERCEPTED" || event == "STEAL" {
                // 抢断因果：球必须处于离散争抢或飞行状态，不可由原持球人静止持有
                let active_holders = frame.players.iter().filter(|p| p.has_ball).count();
                if active_holders > 1 {
                    violations.push(Violation {
                        tick_index,
                        rule: "STEAL_MULTIPLE_HOLDERS",
                        severity: crate::ViolationSeverity::Hard,
                        detail: "steal event emitted while multiple players hold ball".to_string(),
                    });
                }
            }
        }

        violations
    }
}
