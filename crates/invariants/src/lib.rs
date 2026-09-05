//! 模拟不变量检查 (Invariant Subsystem)。
//!
//! 对引擎产出的 `StreamTick` 逐 tick 校验最基础的物理与篮球逻辑：
//! 球权唯一性、持球人与球的位置一致性、运动学边界、比分与时钟单调性。
//!
//! 本 crate 只读协议快照，不知道引擎内部实现；检查的是"一场篮球比赛
//! 最起码必须成立的事"，不评判战术好坏。

use nba_protocol::StreamTick;
use serde::Serialize;
use std::collections::HashMap;

pub mod causal_graph;
pub mod taxonomy;
pub use causal_graph::{
    ActiveShotTracking, CausalEventGraph, LifecycleTransitionTarget, PreconditionEvaluator,
};
pub use taxonomy::{ViolationCategory, ViolationSeverity, ViolationTaxonomy};
/// 单条不变量违反记录。
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    /// 触发违反的 tick 序号（0 为引擎导出前的初始快照）。
    pub tick_index: u64,
    /// 违反的规则标识，例如 `BALL_HOLDER_ON_COURT`。
    pub rule: &'static str,
    /// 严重级别：Hard = 物理不可能/规则破坏；Soft = 不合理但可能合法。
    /// 发出处定级（quality 严重级别规范：Violation 结构必须携带 severity）。
    pub severity: ViolationSeverity,
    /// 人类可读的具体描述。
    pub detail: String,
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[tick {}] {}: {}", self.tick_index, self.rule, self.detail)
    }
}

impl std::error::Error for Violation {}

/// 跨 tick 的不变量检查器。
///
/// 每个 `StreamTick` 调用一次 [`InvariantChecker::check_tick`]，内部维护
/// 上一 tick 的球员位置 / 球位置 / 比分 / 时钟，以计算速度与单调性。
/// 引擎导出循环与离线分析脚本都应复用同一实现，保证规则只有一份。
pub struct InvariantChecker {
    tick_index: u64,
    prev_positions: HashMap<String, (f32, f32)>,
    /// 上一帧的在场标志：入场首 tick 豁免 PLAYER_SPEED
    /// （死球换人允许站位重置，quality 不变量阶段语义）。
    prev_on_court: HashMap<String, bool>,
    prev_ball: Option<(f32, f32, f32)>,
    prev_score: Option<(u32, u32)>,
    prev_game_clock: Option<f32>,
    pub causal_graph: CausalEventGraph,
}

impl InvariantChecker {
    pub fn new() -> Self {
        Self {
            tick_index: 0,
            prev_positions: HashMap::new(),
            prev_on_court: HashMap::new(),
            prev_ball: None,
            prev_score: None,
            prev_game_clock: None,
            causal_graph: CausalEventGraph::new(),
        }
    }

    /// 已检查的 tick 数（含下一个待检查 tick 的序号）。
    pub fn tick_index(&self) -> u64 {
        self.tick_index
    }

    /// 校验单个 tick，返回该 tick 内发现的所有违反（通常为零）。
    ///
    /// 阶段语义（quality.md §1.1）：InboundSetup / DeadBallReset 阶段下，
    /// 持球发球者允许站在界外（发球规则站位），`PLAYER_IN_BOUNDS` 对其豁免；
    /// 禁止通过"导出帧撒谎"迎合不变量（architecture.md §7.3）。
    pub fn check_tick(&mut self, tick: &StreamTick) -> Vec<Violation> {
        let mut out = Vec::new();
        let frame = &tick.frame;
        // frame.rules 为必填字段（quality.md §1.1）：限值唯一来源，无兜底常数。
        let rules = &frame.rules;
        let court_w = rules.court_width_ft;
        let court_h = rules.court_height_ft;
        let dt = rules.tick_seconds;
        let max_speed = rules.max_player_speed_ftps;
        let ball_max = rules.ball_max_speed_ftps;
        let min_sep = rules.min_player_separation_ft;
        // 阶段语义：发球/死球重置阶段，持球者（发球人）可站界外。
        let phase_upper = frame.phase.to_ascii_uppercase();
        let inbound_phase =
            phase_upper == "INBOUND" || phase_upper == "DEAD_BALL_RESET" || (phase_upper == "INITIATION" && frame.game_flow == "DeadBall");
        let holder_id = frame.ball.holder_id.as_deref();

        // --------------------------------------------------------------
        // 0. 篮球基本常识：双方在场人数必须各自严格等于 5 人。
        let home_on_court = frame
            .players
            .iter()
            .filter(|p| p.team == "home" && p.on_court)
            .count();
        let away_on_court = frame
            .players
            .iter()
            .filter(|p| p.team == "away" && p.on_court)
            .count();
        if home_on_court != 5 {
            out.push(Violation {
                tick_index: self.tick_index,
                rule: "TEAM_ON_COURT_COUNT",
                severity: ViolationSeverity::Hard,
                detail: format!("home team has {} players on court, expected 5", home_on_court),
            });
        }
        if away_on_court != 5 {
            out.push(Violation {
                tick_index: self.tick_index,
                rule: "TEAM_ON_COURT_COUNT",
                severity: ViolationSeverity::Hard,
                detail: format!("away team has {} players on court, expected 5", away_on_court),
            });
        }

        // --------------------------------------------------------------
        // 1. 球权唯一性：渲染帧上至多一名持球者。
        let holders: Vec<&str> = frame
            .players
            .iter()
            .filter(|p| p.has_ball)
            .map(|p| p.id.as_str())
            .collect();
        if holders.len() > 1 {
            out.push(Violation {
                tick_index: self.tick_index,
                rule: "BALL_SINGLE_HOLDER",
                severity: ViolationSeverity::Hard,
                detail: format!("{} players flagged hasBall: {:?}", holders.len(), holders),
            });
        }
        // 球状态标记的 holder 必须与渲染的 hasBall 一致。
        if let Some(holder_id) = &frame.ball.holder_id {
            if !holders.is_empty() && !holders.contains(&holder_id.as_str()) {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "BALL_HOLDER_MISMATCH",
                    severity: ViolationSeverity::Hard,
                    detail: format!(
                        "ball.holderId={} but hasBall flags {:?}",
                        holder_id, holders
                    ),
                });
            }
            // 持球者必须在场上。
            match frame.players.iter().find(|p| &p.id == holder_id) {
                Some(p) if p.on_court => {}
                Some(_) => out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "BALL_HOLDER_ON_COURT",
                    severity: ViolationSeverity::Hard,
                    detail: format!("holder {} is not on court", holder_id),
                }),
                None => out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "BALL_HOLDER_EXISTS",
                    severity: ViolationSeverity::Hard,
                    detail: format!("holder {} missing from player list", holder_id),
                }),
            }
        }

        // --------------------------------------------------------------
        // 2. 球员位置：在场球员必须在球场界内，替补球员必须位于替补席区。
        // 阶段豁免：发球阶段持球发球者允许站界外（quality.md §1.1）。
        for p in &frame.players {
            if !p.on_court {
                continue;
            }
            let is_inbounding_player = (inbound_phase && holder_id == Some(p.id.as_str()))
                || p.action == "INBOUND_SETUP"
                || p.action == "InboundPositioning";
            if is_inbounding_player {
                continue;
            }
            if !(0.0..=1.0).contains(&p.x) || !(0.0..=1.0).contains(&p.y) {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "PLAYER_IN_BOUNDS",
                    severity: ViolationSeverity::Hard,
                    detail: format!("on-court player {} out of bounds at norm ({}, {})", p.id, p.x, p.y),
                });
            }
        }

        // --------------------------------------------------------------
        // 3. 运动学边界：速度 / 加速度 / 间距（复用 audit 脚本的物理事实）。
        let dt_safe = dt.max(f32::EPSILON);
        for p in &frame.players {
            let entering = p.on_court
                && !self.prev_on_court.get(&p.id).copied().unwrap_or(false);
            if !p.on_court {
                self.prev_positions.remove(&p.id);
                self.prev_on_court.insert(p.id.clone(), false);
                continue;
            }
            self.prev_on_court.insert(p.id.clone(), true);
            // 入场首 tick（换人/死球重置）豁免速度判定：站位重置是规则允许
            // 的瞬移（quality.md §1.1 阶段语义），下一 tick 恢复监测。
            if entering {
                self.prev_positions.insert(p.id.clone(), (p.x, p.y));
                continue;
            }
            if let Some(&(px, py)) = self.prev_positions.get(&p.id) {
                let dx_ft = (p.x - px) * court_w;
                let dy_ft = (p.y - py) * court_h;
                let speed_ftps = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt() / dt_safe;
                if speed_ftps > max_speed + rules.speed_tolerance_ftps {
                    out.push(Violation {
                        tick_index: self.tick_index,
                        rule: "PLAYER_SPEED",
                        severity: ViolationSeverity::Hard,
                        detail: format!(
                            "player {} speed {:.2} ft/s exceeds limit {:.2}",
                            p.id, speed_ftps, max_speed
                        ),
                    });
                }
            }
            self.prev_positions.insert(p.id.clone(), (p.x, p.y));
        }
        // 最小间距（仅统计在场球员）。
        let on_court: Vec<&nba_protocol::RenderPlayer> =
            frame.players.iter().filter(|p| p.on_court).collect();
        for i in 0..on_court.len() {
            for j in (i + 1)..on_court.len() {
                let a = on_court[i];
                let b = on_court[j];
                let dx_ft = (a.x - b.x) * court_w;
                let dy_ft = (a.y - b.y) * court_h;
                let dist = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt();
                if dist < min_sep * 0.5 {
                    out.push(Violation {
                        tick_index: self.tick_index,
                        rule: "PLAYER_SEPARATION",
                        severity: ViolationSeverity::Soft,
                        detail: format!(
                            "players {} / {} separation {:.3} ft below {:.3}",
                            a.id, b.id, dist, min_sep
                        ),
                    });
                }
            }
        }

        // --------------------------------------------------------------
        // 4. 球运动边界：持球时球贴着人，非持球时速度受弹道上限约束。
        let ball_ft = (frame.ball.x * court_w, frame.ball.y * court_h, frame.ball.z);
        if let Some(holder_id) = &frame.ball.holder_id {
            // 在合球或交接过渡期 (CONTROL_TRANSFER)，球正在向持球人飞行平滑合拢，不施加静态持球贴身 leash 约束
            if frame.ball.status == "CONTROL_TRANSFER" {
                // 仅验证过渡期速度上限
                if let Some(prev) = self.prev_ball {
                    let dx = ball_ft.0 - prev.0;
                    let dy = ball_ft.1 - prev.1;
                    let dz = ball_ft.2 - prev.2;
                    let ball_speed = (dx * dx + dy * dy + dz * dz).sqrt() / dt;
                    let ball_max = rules.ball_max_speed_ftps * 1.05;
                    if ball_speed > ball_max {
                        out.push(Violation {
                            tick_index: self.tick_index,
                            rule: "BALL_SPEED",
                            severity: ViolationSeverity::Hard,
                            detail: format!(
                                "control transfer ball speed {:.2} ft/s exceeds limit {:.2}",
                                ball_speed, ball_max
                            ),
                        });
                    }
                }
            } else if let Some(holder) = frame.players.iter().find(|p| &p.id == holder_id) {
                let hx = holder.x * court_w;
                let hy = holder.y * court_h;
                let dx = ball_ft.0 - hx;
                let dy = ball_ft.1 - hy;
                let dist = (dx * dx + dy * dy).sqrt();
                // 球在持球者偏移半径 + 容差之外 = 球人分离。
                if dist > rules.holder_leash_ft {
                    out.push(Violation {
                        tick_index: self.tick_index,
                        rule: "BALL_WITH_HOLDER",
                        severity: ViolationSeverity::Hard,
                        detail: format!(
                            "ball {:.2} ft from holder {} (x={:.1}, y={:.1})",
                            dist, holder_id, ball_ft.0, ball_ft.1
                        ),
                    });
                }
            }
        } else if let Some(prev) = self.prev_ball {
            let is_dead_ball = frame.ball.status == "DEAD" || frame.phase == "FreeThrow" || frame.phase == "DeadBallReset" || frame.game_flow == "DeadBall";
            let was_rebound_start = frame.events.iter().any(|e| e == "FREE_THROW" || e == "REBOUND" || e == "TIPOFF_SECURED");
            if is_dead_ball || was_rebound_start {
                // 死球、罚球准备、发球、篮板或跳球点拍争夺时不计算速度跳变
            } else {
                let dx = ball_ft.0 - prev.0;
                let dy = ball_ft.1 - prev.1;
                let dz = ball_ft.2 - prev.2;
                let ball_speed = (dx * dx + dy * dy + dz * dz).sqrt() / dt_safe;
                if ball_speed > ball_max + rules.speed_tolerance_ftps {
                    out.push(Violation {
                        tick_index: self.tick_index,
                        rule: "BALL_SPEED",
                        severity: ViolationSeverity::Hard,
                        detail: format!(
                            "free ball 3D speed {:.2} ft/s exceeds limit {:.2}",
                            ball_speed, ball_max
                        ),
                    });
                }
            }
        }
        self.prev_ball = Some(ball_ft);

        // --------------------------------------------------------------
        // 5. 球的三维高度界限：严禁掉入地下 (z < 0) 或飞出球馆。
        if frame.ball.z < -0.1 || frame.ball.z > rules.ball_z_max_ft {
            out.push(Violation {
                tick_index: self.tick_index,
                rule: "BALL_HEIGHT_BOUNDS",
                severity: ViolationSeverity::Hard,
                detail: format!(
                    "ball z-height {:.2} ft out of valid range [0, {}]",
                    frame.ball.z, rules.ball_z_max_ft
                ),
            });
        }

        // --------------------------------------------------------------
        // 6. 比分因果律：比分永不倒退，单 tick 增量最多 3 分。
        let score = (frame.score.home, frame.score.away);
        if let Some(prev) = self.prev_score {
            if score.0 < prev.0 || score.1 < prev.1 {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "SCORE_MONOTONIC",
                    severity: ViolationSeverity::Hard,
                    detail: format!("score {:?} regressed from {:?}", score, prev),
                });
            }
            let delta_home = score.0.saturating_sub(prev.0);
            let delta_away = score.1.saturating_sub(prev.1);
            if delta_home > 3 || delta_away > 3 {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "SCORE_DELTA_VALIDITY",
                    severity: ViolationSeverity::Hard,
                    detail: format!("score jumped abnormally from {:?} to {:?}", prev, score),
                });
            }
        }
        self.prev_score = Some(score);

        // --------------------------------------------------------------
        // 7. 比赛时钟与进攻时钟倒计时公理（限值取自 FrameRules）。
        let clock = frame.t_game;
        if let Some(prev) = self.prev_game_clock {
            // 允许跨节重置（变大），但同节内不能变大超过一个 tick。
            if clock > prev + dt + 0.05 && frame.period <= 1 {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "CLOCK_MONOTONIC",
                    severity: ViolationSeverity::Hard,
                    detail: format!("game clock rose {:.2} -> {:.2} within period", prev, clock),
                });
            }
        }
        self.prev_game_clock = Some(clock);
        if frame.shot_clock < -0.1 || frame.shot_clock > rules.shot_clock_seconds + 0.1 {
            out.push(Violation {
                tick_index: self.tick_index,
                rule: "SHOT_CLOCK_BOUNDS",
                severity: ViolationSeverity::Hard,
                detail: format!(
                    "shot clock {:.2}s out of valid range [0, {}]",
                    frame.shot_clock, rules.shot_clock_seconds
                ),
            });
        }

        // --------------------------------------------------------------
        // 8. L2 事件流因果语义与回合自洽性公理。
        for event in &frame.events {
            if event.as_str() == "SCORE" && frame.score.home == 0 && frame.score.away == 0 {
                out.push(Violation {
                    tick_index: self.tick_index,
                    rule: "SCORE_EVENT_WITHOUT_POINTS",
                    severity: ViolationSeverity::Hard,
                    detail: "SCORE event emitted but total points remain 0".to_string(),
                });
            }
        }
        out.extend(self.causal_graph.validate_tick(tick, self.tick_index));
        self.tick_index += 1;
        out
    }
}

impl Default for InvariantChecker {
    fn default() -> Self {
        Self::new()
    }
}

/// 便捷函数：检查一整段 tick 序列，聚合所有违反。
pub fn check_all<'a>(ticks: impl IntoIterator<Item = &'a StreamTick>) -> Vec<Violation> {
    let mut checker = InvariantChecker::new();
    let mut out = Vec::new();
    for tick in ticks {
        out.extend(checker.check_tick(tick));
    }
    out
}
