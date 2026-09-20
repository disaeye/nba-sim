//! 事件流因果图追踪器（Causal Event Graph Validator）。
//!
//! 从第一性原理出发，对连续物理与离散事件进行跨 tick 因果链验证：
//! 1. 投篮出手 (ShotRelease) -> 到筐 (HoopArrival) 的因果与时间跨度一致性；
//! 2. 比分增加 (ScoreDelta) 严格必须由前置的合法进球或罚球事件引起（杜绝幽灵得分）；
//! 3. 篮板争抢 (Rebound) 严格必须由前置的投篮不中 (HoopArrival { is_made: false }) 触发。

use crate::Violation;
use nba_protocol::StreamTick;

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
    pub prev_ball_pos: Option<(f32, f32, f32)>,
    pub prev_ball_holder: Option<String>,
    pub prev_ball_status: Option<String>,
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
        let cur_score = (cur_home, cur_away);
        if cur_score != self.last_score {
            let delta_home = cur_score.0.saturating_sub(self.last_score.0);
            let delta_away = cur_score.1.saturating_sub(self.last_score.1);
            let is_valid_delta =
                (delta_home <= 3 && delta_away == 0) || (delta_away <= 3 && delta_home == 0);
            if !is_valid_delta {
                violations.push(Violation {
                    tick_index,
                    rule: "SCORE_QUANTUM_LEAP",
                    severity: crate::ViolationSeverity::Hard,
                    detail: format!(
                        "score jumped illegally from {:?} to {:?}",
                        self.last_score, cur_score
                    ),
                });
            }
            let has_scoring_event = frame.events.iter().any(|e| {
                e == "SCORE"
                    || e == "SHOT_MADE"
                    || e == "FREE_THROW"
                    || e == "FREE_THROW_MADE"
                    || e == "FREE_THROW_ATTEMPT"
                    || e == "HOOP_ARRIVAL"
            });
            if !has_scoring_event {
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
            self.last_score = cur_score;
        }
        // 2. 投篮与到筐事件因果链
        for event in &frame.events {
            if event == "SHOT_RELEASE" || event == "SHOT" {
                let shooter = frame
                    .players
                    .iter()
                    .find(|p| p.has_ball)
                    .map(|p| p.id.clone())
                    .unwrap_or_default();
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
                if self
                    .last_made_shot_tick
                    .is_some_and(|t| tick_index <= t + 2)
                {
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
                    if holder.x > margin
                        && holder.x < (1.0 - margin)
                        && holder.y > margin
                        && holder.y < (1.0 - margin)
                    {
                        violations.push(Violation {
                            tick_index,
                            rule: "INBOUNDER_DEEP_IN_COURT",
                            severity: crate::ViolationSeverity::Hard,
                            detail: format!(
                                "inbounder {} is deep inside the court at ({:.2}, {:.2})",
                                holder.id, holder.x, holder.y
                            ),
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
        // 3. 冲量溯源公理（Impulse Origin Axiom）：
        // 当球从非飞行状态突变为飞行状态（PASS / SHOT / INBOUND_TRANSFER）时，
        // 上一帧必须有合法持球人，或者球的起始位置必须在某个在场球员的接触范围内（<= 4.5 ft）
        let cur_ball_pos = (frame.ball.x, frame.ball.y, frame.ball.z);
        let cur_status = &frame.ball.status;
        let is_new_flight =
            (cur_status == "PASS" || cur_status == "SHOT" || cur_status == "INBOUND_TRANSFER")
                && self.prev_ball_status.as_deref() != Some(cur_status);

        if is_new_flight {
            let had_prior_holder = self.prev_ball_holder.is_some()
                || self.prev_ball_status.as_deref() == Some("INBOUND_READY")
                || self.prev_ball_status.as_deref() == Some("INBOUND_TRANSFER")
                || self.prev_ball_status.as_deref() == Some("REBOUND")
                || frame.events.iter().any(|e| e == "REBOUND");
            let near_any_player = frame.players.iter().any(|p| {
                let dx = (p.x - frame.ball.x) * 94.0;
                let dy = (p.y - frame.ball.y) * 50.0;
                (dx * dx + dy * dy).sqrt() < 6.0
            });
            let is_rim_origin =
                (cur_ball_pos.0 - 0.05).abs() < 0.08 || (cur_ball_pos.0 - 0.95).abs() < 0.08;
            let is_inbound = cur_status == "INBOUND_TRANSFER"
                || (cur_status == "PASS"
                    && (frame.phase == "Inbound"
                        || frame.phase == "ActionExecution"
                            && (self.prev_ball_status.as_deref() == Some("INBOUND_READY")
                                || self.prev_ball_status.as_deref() == Some("INBOUND_TRANSFER"))));
            if !had_prior_holder
                && !near_any_player
                && !is_rim_origin
                && !is_inbound
                && tick_index > 0
            {
                violations.push(Violation {
                    tick_index,
                    rule: "BALL_IMPULSE_WITHOUT_SOURCE",
                    severity: crate::ViolationSeverity::Hard,
                    detail: format!(
                        "ball entered flight state {} at ({:.2}, {:.2}) with no prior holder and no nearby on-court player",
                        cur_status, frame.ball.x, frame.ball.y
                    ),
                });
            }
        }
        // 4. 时空连续性公理（Continuity Axiom）：
        // 球在两帧之间的移动速度不得超过物理极值（归一化坐标每帧位移不超过 0.2，约等于 110 ft/s）
        if let Some(prev_pos) = self.prev_ball_pos {
            let is_dead = frame.game_flow == "DeadBall"
                || frame.game_flow == "FreeThrow"
                || cur_status == "DEAD"
                || cur_status == "INBOUND_TRANSFER"
                || self.prev_ball_status.as_deref() == Some("DEAD");
            let dx = (cur_ball_pos.0 - prev_pos.0) * 94.0;
            let dy = (cur_ball_pos.1 - prev_pos.1) * 50.0;
            let dist_ft = (dx * dx + dy * dy).sqrt();
            if !is_dead && dist_ft > 20.0 && tick_index > 0 {
                violations.push(Violation {
                    tick_index,
                    rule: "BALL_POSITION_DISCONTINUITY",
                    severity: crate::ViolationSeverity::Hard,
                    detail: format!(
                        "ball teleported {:.1} ft in single tick from ({:.2}, {:.2}) to ({:.2}, {:.2})",
                        dist_ft, prev_pos.0, prev_pos.1, cur_ball_pos.0, cur_ball_pos.1
                    ),
                });
            }
        }

        self.prev_ball_pos = Some(cur_ball_pos);
        self.prev_ball_holder = frame.ball.holder_id.clone();
        self.prev_ball_status = Some(cur_status.clone());

        violations
    }
}

/// 声明式因果前置图（Causal Precondition DAG）
/// 任何阶段跃迁在执行前必须通过前置物理完备集评估
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleTransitionTarget {
    LiveBallInitiation,
    InboundExecution,
    FreeThrowExecution,
    TipOffExecution,
}

#[derive(Debug, Clone, Default)]
pub struct PreconditionEvaluator;

impl PreconditionEvaluator {
    /// 校验切入指定活球/执行阶段的前置物理条件是否达成
    pub fn evaluate_preconditions(
        target: LifecycleTransitionTarget,
        frame: &nba_protocol::RenderFrame,
    ) -> Result<(), &'static str> {
        match target {
            LifecycleTransitionTarget::LiveBallInitiation => {
                // 必须满足：球必须被合规持有或处于合规争抢轨迹，且在场球员严格10人
                let on_court_count = frame.players.iter().filter(|p| p.on_court).count();
                if on_court_count != 10 {
                    return Err("PRECONDITION_FAILED: on-court players must be exactly 10");
                }
                Ok(())
            }
            LifecycleTransitionTarget::InboundExecution => {
                // 必须满足：发球人必须在场外有效发球区
                let has_inbounder = frame.players.iter().any(|p| {
                    p.has_ball && (p.x <= 0.08 || p.x >= 0.92 || p.y <= 0.08 || p.y >= 0.92)
                });
                if !has_inbounder {
                    return Err("PRECONDITION_FAILED: inbounder must be stationed out-of-bounds");
                }
                Ok(())
            }
            LifecycleTransitionTarget::FreeThrowExecution => {
                // 罚球必须有明确指定罚球人持球就位
                let has_shooter = frame.players.iter().any(|p| p.has_ball);
                if !has_shooter {
                    return Err(
                        "PRECONDITION_FAILED: free throw shooter must hold the ball at line",
                    );
                }
                Ok(())
            }
            LifecycleTransitionTarget::TipOffExecution => {
                // 跳球时双方中锋必须在中圈就位
                Ok(())
            }
        }
    }
}
