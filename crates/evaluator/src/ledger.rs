//! D0.2 四式账本平衡检查器（dev 方案 §3.2）。
//!
//! 账本不是锦上添花，是"系统是否在说真话"的判定器：比分、球权、
//! 时间、犯规都是事件序列的**派生计数**，派生量与事件序列不一致
//! 即系统说谎。四式任何一式不平衡 = Hard `ledger_violation`。
//!
//! 输入为逐 tick 事件流（facts 模式足够），输出结构化违反记录，
//! 由 CLI 写入 `ledger_violations.ndjson`。

use nba_protocol::StreamTick;
use serde::{Deserialize, Serialize};

mod foul_ledger;
use foul_ledger::check_foul_conservation;

/// 时间守恒式的端点容差（秒）：回合时长求和允许超出墙钟的上界，
/// 覆盖逐 tick 端点取整误差。属账本判定口径常数，随检查器定义集中于此。
const TIME_SUM_WALL_TOLERANCE_SECONDS: f32 = 2.0;

/// 单条账本不平衡记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerViolation {
    /// 平衡式 ID：SCORE_CONSERVATION / POSSESSION_CONSERVATION /
    /// TIME_CONSERVATION / FOUL_CONSERVATION。
    pub equation: String,
    pub detail: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub possession: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tick: Option<u64>,
}

impl LedgerViolation {
    fn new(equation: &str, detail: impl Into<String>) -> Self {
        Self {
            equation: equation.to_string(),
            detail: detail.into(),
            possession: None,
            tick: None,
        }
    }
}

/// 从事件流独立重建四本账并检查平衡。
/// 账本对平的方程数（供调用方报告用，避免硬编码数量在加式后漂移）。
///
/// 新增一条守恒式时必须同步此值：`crates/evaluator/src/ledger.rs`
/// 的 `check_ledger()` 是唯一权威，本常量只是它的声明式投影。
pub const LEDGER_EQUATION_COUNT: usize = 5;

pub fn check_ledger(ticks: &[StreamTick]) -> Vec<LedgerViolation> {
    let mut violations = Vec::new();
    if ticks.is_empty() {
        return violations;
    }
    check_score_conservation(ticks, &mut violations);
    check_possession_conservation(ticks, &mut violations);
    check_time_conservation(ticks, &mut violations);
    check_foul_conservation(ticks, &mut violations);
    check_turnover_conservation(ticks, &mut violations);
    violations
}

/// 失误终结的责任可追溯性：每条 `TURNOVER*` 回合终结都必须携带
/// 责任球员或可归因的抢断事实。
///
/// 为什么需要（evidence/problem.md §23.10 的同类缺陷）：
/// `box_score.turnovers` 曾只有两个自增点而失误终结有五种，导致箱体
/// 低估约 4 倍（seed42 实测 12 vs 52），并使守恒式
/// `possessions ≈ FGA + TO + 0.44·FTA − OREB` 的 TO 项失真。
/// 该缺陷长期存活的原因是：没有任何检查对平「汇总字段」与「事件事实」。
///
/// 本函数只做它**能看见**的那部分：事实流内部的一致性——失误终结不能
/// 缺少责任归属（`turnover_player_id`），否则归因链断裂。
/// 「箱体等于事件流」的跨天对比在内核对中不可行（`StreamTick` 不带
/// 箱体快照），因此那一条由 `crates/engine/tests/` 的回归测试承担。
fn check_turnover_conservation(ticks: &[StreamTick], out: &mut Vec<LedgerViolation>) {
    for tick in ticks {
        for event in &tick.frame.event_log {
            if event.kind != "POSSESSION_SUMMARY" {
                continue;
            }
            let Some(summary) = event.data.as_ref().and_then(|d| d.get("PossessionSummary")) else {
                continue;
            };
            let terminal = summary
                .get("terminal_event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if !terminal.starts_with("TURNOVER") {
                continue;
            }
            let has_player = summary
                .get("turnover_player_id")
                .map(|v| !v.is_null())
                .unwrap_or(false);
            if !has_player {
                let idx = summary
                    .get("possession_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(u64::MAX);
                let mut v = LedgerViolation::new(
                    "TURNOVER_CONSERVATION",
                    format!("turnover terminal `{terminal}` at possession {idx} lacks responsible player"),
                );
                v.possession = Some(idx);
                out.push(v);
            }
        }
    }
}

/// 得分守恒：终场比分 = Σ 得分事件载荷。
///
/// 得分**事实**只有两类（账本原则：事件只陈述已发生的事实）：
/// - `SCORE`：载荷 `HoopArrival { is_made: true, is_three, shooter_id }`，逐次 +2/+3；
/// - `FREE_THROW`：载荷 `FreeThrowAttempt.made`，逐次 +1。
///
/// `DRIVE_SCORE` 不是得分事实：`DriveOutcome.finish_made` 是终结判定的
/// **预定结果**，实际得分由后续 ShotRelease → HoopArrival 完成并入账
/// （D0 审计实证：17 次 DRIVE_SCORE 事件帧比分均未随之变化，得分全部
/// 经由同 tick/后续 HoopArrival 记录）。该标签的事件语义缺陷
/// （决策与事实混淆）登记为 F-系列后续项，本账本不重复计数。
fn check_score_conservation(ticks: &[StreamTick], out: &mut Vec<LedgerViolation>) {
    let mut home_from_events = 0u32;
    let mut away_from_events = 0u32;
    let mut last_score: Option<(u32, u32)> = None;
    for tick in ticks {
        for event in &tick.frame.event_log {
            match event.kind.as_str() {
                "SCORE" | "SHOT_MISS" => {
                    let Some(arrival) = event.data.as_ref().and_then(|d| d.get("HoopArrival"))
                    else {
                        let mut v = LedgerViolation::new(
                            "SCORE_CONSERVATION",
                            format!(
                                "tick {} {} fact without HoopArrival payload",
                                tick.frame.t, event.kind
                            ),
                        );
                        v.tick = Some(tick.frame.event_sequence);
                        out.push(v);
                        continue;
                    };
                    let Some(made) = arrival.get("is_made").and_then(|v| v.as_bool()) else {
                        let mut v = LedgerViolation::new(
                            "SCORE_CONSERVATION",
                            format!(
                                "tick {} {} fact without boolean is_made outcome",
                                tick.frame.t, event.kind
                            ),
                        );
                        v.tick = Some(tick.frame.event_sequence);
                        out.push(v);
                        continue;
                    };
                    let expected_made = event.kind == "SCORE";
                    if made != expected_made {
                        let mut v = LedgerViolation::new(
                            "SCORE_CONSERVATION",
                            format!(
                                "tick {} {} fact contradicts HoopArrival.is_made={made}",
                                tick.frame.t, event.kind
                            ),
                        );
                        v.tick = Some(tick.frame.event_sequence);
                        out.push(v);
                        continue;
                    }
                    if !made {
                        continue;
                    }
                    let points = if arrival
                        .get("is_three")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        3
                    } else {
                        2
                    };
                    match arrival
                        .get("shooter_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                    {
                        s if s.starts_with("H_") => home_from_events += points,
                        s if s.starts_with("A_") => away_from_events += points,
                        other => {
                            out.push(LedgerViolation::new(
                                "SCORE_CONSERVATION",
                                format!(
                                    "tick {} score with unknown shooter team prefix '{other}'",
                                    tick.frame.t
                                ),
                            ));
                        }
                    }
                }
                "FREE_THROW" => {
                    let Some(ft) = event.data.as_ref().and_then(|d| d.get("FreeThrowAttempt"))
                    else {
                        continue;
                    };
                    let made = ft.get("made").and_then(|v| v.as_bool()).unwrap_or(false);
                    if !made {
                        continue;
                    }
                    match ft.get("shooter_id").and_then(|v| v.as_str()).unwrap_or("") {
                        s if s.starts_with("H_") => home_from_events += 1,
                        s if s.starts_with("A_") => away_from_events += 1,
                        other => {
                            out.push(LedgerViolation::new(
                                "SCORE_CONSERVATION",
                                format!(
                                    "tick {} made free throw with unknown shooter '{other}'",
                                    tick.frame.t
                                ),
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
        last_score = Some((tick.frame.score.home, tick.frame.score.away));
    }
    if let Some((home, away)) = last_score {
        if home != home_from_events || away != away_from_events {
            out.push(LedgerViolation::new(
                "SCORE_CONSERVATION",
                format!(
                    "final score (home={home} away={away}) != Σ score events (home={home_from_events} away={away_from_events})"
                ),
            ));
        }
    }
}

/// 球权守恒：回合总结序列完整（possession_index 连续递增、无重复），
/// 且每次球权转移恰有一个显式终结原因（D0.1 已保证枚举无兜底）。
fn check_possession_conservation(ticks: &[StreamTick], out: &mut Vec<LedgerViolation>) {
    let mut expected_index = 0u64;
    let mut seen_any = false;
    for tick in ticks {
        for event in &tick.frame.event_log {
            if event.kind != "POSSESSION_SUMMARY" {
                continue;
            }
            let Some(summary) = event.data.as_ref().and_then(|d| d.get("PossessionSummary")) else {
                continue;
            };
            let idx = summary
                .get("possession_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(u64::MAX);
            if seen_any && idx != expected_index {
                let mut v = LedgerViolation::new(
                    "POSSESSION_CONSERVATION",
                    format!("possession index gap: expected {expected_index}, got {idx}"),
                );
                v.possession = Some(idx);
                out.push(v);
                expected_index = idx;
            }
            // 终结原因必须可解析为合法枚举（无 UNATTRIBUTED_END）。
            let terminal = summary
                .get("terminal_event")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if terminal.is_empty() || terminal == "UNATTRIBUTED_END" {
                let mut v = LedgerViolation::new(
                    "POSSESSION_CONSERVATION",
                    format!("possession {idx} has unattributed or empty terminal cause"),
                );
                v.possession = Some(idx);
                out.push(v);
            }
            seen_any = true;
            expected_index = expected_index.max(idx).saturating_add(1);
        }
    }
}

/// 时间守恒：Σ 回合时长 ≤ 仿真墙钟总时长（`t` 单调）+ 容差；
/// 且每条回合时长为非负、非零填充。
///
/// 注意口径：`t_game` 是节内倒计时（每节重置），不能跨节求差；
/// 墙钟 `t` 才是全程单调的仿真时间，`duration_seconds` 的权威来源
/// 也是 `current_time`（= `t`）差值（dev 方案 §3.2 D0.3）。
fn check_time_conservation(ticks: &[StreamTick], out: &mut Vec<LedgerViolation>) {
    let mut sum_duration = 0f32;
    let mut wall_start: Option<f32> = None;
    let mut wall_end: Option<f32> = None;
    for tick in ticks {
        if wall_start.is_none() {
            wall_start = Some(tick.frame.t);
        }
        wall_end = Some(tick.frame.t);
        for event in &tick.frame.event_log {
            if event.kind != "POSSESSION_SUMMARY" {
                continue;
            }
            let Some(summary) = event.data.as_ref().and_then(|d| d.get("PossessionSummary")) else {
                continue;
            };
            // 哨兵：解析失败标为负值，随后被负值检查捕获（非行为阈值）。
            let duration = summary
                .get("duration_seconds")
                .and_then(|v| v.as_f64())
                .unwrap_or(f64::NEG_INFINITY) as f32;
            let idx = summary
                .get("possession_index")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if duration < 0.0 {
                let mut v = LedgerViolation::new(
                    "TIME_CONSERVATION",
                    format!("possession {idx} has negative duration {duration}"),
                );
                v.possession = Some(idx);
                out.push(v);
            }
            // 时长必须来自单调时间差：不得出现零时长伪装（D0.3 已删除
            // .max(0.1) 填充，此处回归监督）。真实亚 tick 回合允许存在，
            // 但恰好 0.0 只可能是填充。
            if duration == 0.0 {
                let mut v = LedgerViolation::new(
                    "TIME_CONSERVATION",
                    format!("possession {idx} has zero duration (clock fill suspected)"),
                );
                v.possession = Some(idx);
                out.push(v);
            }
            sum_duration += duration;
        }
    }
    if let (Some(start), Some(end)) = (wall_start, wall_end) {
        let wall_elapsed = (end - start).abs();
        // 回合时长之和不应超过墙钟总时长（死球区间也占墙钟时间，故取 ≤）。
        // 端点容差：全程回合数 × 一个 tick 的取整误差，取保守上界。
        let tolerance = TIME_SUM_WALL_TOLERANCE_SECONDS;
        if sum_duration > wall_elapsed + tolerance {
            out.push(LedgerViolation::new(
                "TIME_CONSERVATION",
                format!(
                    "Σ possession durations {sum_duration:.2}s exceeds wall-clock elapsed {wall_elapsed:.2}s (+{tolerance}s tolerance)"
                ),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba_protocol::{RenderFrame, StreamTick};

    /// 构造最小合成 tick（含事件日志与比分），用于负面对照。
    fn synthetic_tick(
        t: f32,
        t_game: f32,
        home: u32,
        away: u32,
        events: Vec<nba_protocol::FrameEvent>,
    ) -> StreamTick {
        let frame = RenderFrame {
            t,
            t_game,
            shot_clock: 24.0,
            period: 1,
            phase: "LiveBall".to_string(),
            possession_id: 0,
            possession_team: "home".to_string(),
            home_team: Default::default(),
            away_team: Default::default(),
            score: nba_protocol::RenderScore { home, away },
            players: vec![],
            ball: Default::default(),
            event_type: None,
            events: vec![],
            game_flow: "LiveBall".to_string(),
            team_fouls_home: 0,
            team_fouls_away: 0,
            free_throws_remaining: 0,
            defensive_tactic: None,
            event_sequence: 0,
            event_log: events,
            simulation_complete: false,
            completed_possessions: 0,
            target_possessions: 0,
            callout: None,
            intensity: None,
            debug: None,
            rules: Default::default(),
            stream_projection: Default::default(),
            potential_field: vec![],
        };
        StreamTick {
            frame,
            tactical_set: String::new(),
            game_clock: t_game,
            keyframe_index: None,
        }
    }

    fn event(kind: &str, data: serde_json::Value, seq: u64) -> nba_protocol::FrameEvent {
        nba_protocol::FrameEvent {
            event_id: seq,
            parent_event_id: None,
            sequence: seq,
            time: 0.0,
            kind: kind.to_string(),
            data: Some(data),
        }
    }

    /// 犯规账本专用合成 tick：帧计数器与比分可配置（球队累计/得分重建依赖）。
    fn foul_tick(
        t: f32,
        period: u32,
        home: u32,
        away: u32,
        team_fouls_home: u32,
        team_fouls_away: u32,
        events: Vec<nba_protocol::FrameEvent>,
    ) -> StreamTick {
        let mut tick = synthetic_tick(t, 720.0 - t, home, away, events);
        tick.frame.period = period;
        tick.frame.team_fouls_home = team_fouls_home;
        tick.frame.team_fouls_away = team_fouls_away;
        tick
    }

    /// 合成犯规事实：个人/球队累计、bonus 与罚球次数可配置。
    fn foul_event(
        seq: u64,
        fouled: &str,
        fouler: &str,
        personal: u64,
        team: u64,
        bonus: bool,
        free_throws: u64,
    ) -> nba_protocol::FrameEvent {
        event(
            "FOUL",
            serde_json::json!({
                "Foul": {
                    "fouled_player_id": fouled,
                    "fouler_id": fouler,
                    "is_shooting": free_throws > 0,
                    "foul_kind": if free_throws > 0 { "shooting" } else { "personal" },
                    "personal_foul_count": personal,
                    "period_team_foul_count": team,
                    "penalty": {
                        "free_throw_count": free_throws,
                        "retains_possession": free_throws > 0,
                        "is_bonus": bonus
                    }
                }
            }),
            seq,
        )
    }

    /// 合成罚球事实：因果父指向判罚它的犯规事件。
    fn free_throw_event(
        seq: u64,
        parent: u64,
        shooter: &str,
        made: bool,
    ) -> nba_protocol::FrameEvent {
        let mut e = event(
            "FREE_THROW",
            serde_json::json!({
                "FreeThrowAttempt": {"shooter_id": shooter, "attempt": 1, "made": made}
            }),
            seq,
        );
        e.parent_event_id = Some(parent);
        e
    }

    /// 合成换人事实。
    fn substitution_event(seq: u64, out_player: &str, in_player: &str) -> nba_protocol::FrameEvent {
        event(
            "SUBSTITUTION",
            serde_json::json!({
                "Substitution": {
                    "team": "away",
                    "out_player": out_player,
                    "in_player": in_player,
                    "reason": "FoulTrouble"
                }
            }),
            seq,
        )
    }

    /// 正面对照：完整犯规账（个人/球队累计、bonus 罚球、犯满换下）不得报账。
    #[test]
    fn foul_ledger_balanced_game_passes() {
        let possession_summary = |seq: u64| {
            event(
                "POSSESSION_SUMMARY",
                serde_json::json!({"PossessionSummary": {"possession_index": 0, "offense_team": "home", "start_clock": 720.0, "end_clock": 700.0, "duration_seconds": 20.0, "passes_count": 1, "terminal_event": "FREE_THROW", "shooter_id": "H_1"}}),
                seq,
            )
        };
        let ticks = vec![
            // 第一节：客队 5 次犯规，第 5 次进入 bonus，两罚完成后回合结束。
            foul_tick(
                4.0,
                1,
                0,
                0,
                0,
                1,
                vec![foul_event(1, "H_1", "A_1", 1, 1, false, 0)],
            ),
            foul_tick(
                8.0,
                1,
                0,
                0,
                0,
                2,
                vec![foul_event(2, "H_2", "A_2", 1, 2, false, 0)],
            ),
            foul_tick(
                12.0,
                1,
                0,
                0,
                0,
                3,
                vec![foul_event(3, "H_3", "A_3", 1, 3, false, 0)],
            ),
            foul_tick(
                16.0,
                1,
                0,
                0,
                0,
                4,
                vec![foul_event(4, "H_4", "A_4", 1, 4, false, 0)],
            ),
            foul_tick(
                20.0,
                1,
                0,
                0,
                0,
                5,
                vec![foul_event(5, "H_1", "A_5", 1, 5, true, 2)],
            ),
            foul_tick(
                24.0,
                1,
                1,
                0,
                0,
                5,
                vec![free_throw_event(6, 5, "H_1", true)],
            ),
            foul_tick(
                28.0,
                1,
                1,
                0,
                0,
                5,
                vec![free_throw_event(7, 5, "H_1", false)],
            ),
            foul_tick(30.0, 1, 1, 0, 0, 5, vec![possession_summary(8)]),
            // 第二节：A_1 犯满（累计 6 次），必须被换下。
            foul_tick(
                704.0,
                2,
                1,
                0,
                0,
                1,
                vec![foul_event(9, "H_1", "A_1", 2, 1, false, 0)],
            ),
            foul_tick(
                708.0,
                2,
                1,
                0,
                0,
                2,
                vec![foul_event(10, "H_2", "A_1", 3, 2, false, 0)],
            ),
            foul_tick(
                712.0,
                2,
                1,
                0,
                0,
                3,
                vec![foul_event(11, "H_3", "A_1", 4, 3, false, 0)],
            ),
            foul_tick(
                716.0,
                2,
                1,
                0,
                0,
                4,
                vec![foul_event(12, "H_4", "A_1", 5, 4, false, 0)],
            ),
            foul_tick(
                720.0,
                2,
                1,
                0,
                0,
                5,
                vec![
                    foul_event(13, "H_5", "A_1", 6, 5, true, 0),
                    substitution_event(14, "A_1", "A_6"),
                ],
            ),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations.is_empty(),
            "balanced foul ledger must not report: {violations:?}"
        );
    }

    /// 负面对照：个人累计与事件载荷不一致必须变红。
    #[test]
    fn negative_control_personal_foul_count_lie_detected() {
        let ticks = vec![foul_tick(
            4.0,
            1,
            0,
            0,
            0,
            1,
            vec![foul_event(1, "H_1", "A_1", 2, 1, false, 0)],
        )];
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "FOUL_CONSERVATION"
                    && v.detail.contains("personal foul count")),
            "personal count lie must be caught: {violations:?}"
        );
    }

    /// 负面对照：单犯规帧的球队计数必须能在帧计数器上观测到。
    #[test]
    fn negative_control_team_count_not_visible_in_frame_detected() {
        let ticks = vec![foul_tick(
            4.0,
            1,
            0,
            0,
            0,
            1,
            // 声称球队第 3 次犯规，帧计数器却是 1。
            vec![foul_event(1, "H_1", "A_1", 1, 3, false, 0)],
        )];
        let violations = check_ledger(&ticks);
        assert!(
            violations.iter().any(|v| v.equation == "FOUL_CONSERVATION"
                && v.detail.contains("not visible in frame counters")),
            "invisible team count must be caught: {violations:?}"
        );
    }

    /// 负面对照：节内球队累计与帧计数器峰值不一致必须变红。
    #[test]
    fn negative_control_team_period_conservation_detected() {
        let ticks = vec![
            foul_tick(
                4.0,
                1,
                0,
                0,
                0,
                1,
                vec![foul_event(1, "H_1", "A_1", 1, 1, false, 0)],
            ),
            // 帧计数器声称 3 次球队犯规，事件流只有 1 次。
            foul_tick(8.0, 1, 0, 0, 0, 3, vec![]),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations.iter().any(|v| v.equation == "FOUL_CONSERVATION"
                && v.detail.contains("counted from events vs")),
            "period team conservation lie must be caught: {violations:?}"
        );
    }

    /// 负面对照：bonus 标志与球队累计/阈值矛盾必须变红。
    #[test]
    fn negative_control_bonus_flag_lie_detected() {
        let ticks = vec![foul_tick(
            4.0,
            1,
            0,
            0,
            0,
            2,
            // 球队第 2 次犯规（阈值 5）却声明 bonus。
            vec![foul_event(1, "H_1", "A_1", 1, 2, true, 0)],
        )];
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "FOUL_CONSERVATION" && v.detail.contains("bonus flag")),
            "bonus flag lie must be caught: {violations:?}"
        );
    }

    /// 负面对照：判罚的罚球必须全部完成且罚球人是被侵犯人。
    #[test]
    fn negative_control_free_throw_correspondence_detected() {
        let ticks = vec![
            foul_tick(
                4.0,
                1,
                0,
                0,
                0,
                5,
                vec![foul_event(1, "H_1", "A_5", 1, 5, true, 2)],
            ),
            // 只执行了一次罚球，回合即告结束：剩余一次被跳过。
            foul_tick(
                8.0,
                1,
                1,
                0,
                0,
                5,
                vec![free_throw_event(2, 1, "H_1", true)],
            ),
            foul_tick(
                12.0,
                1,
                1,
                0,
                0,
                5,
                vec![event(
                    "POSSESSION_SUMMARY",
                    serde_json::json!({"PossessionSummary": {"possession_index": 0, "offense_team": "home", "start_clock": 720.0, "end_clock": 700.0, "duration_seconds": 20.0, "passes_count": 0, "terminal_event": "FREE_THROW", "shooter_id": "H_1"}}),
                    3,
                )],
            ),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations.iter().any(|v| v.equation == "FOUL_CONSERVATION"
                && v.detail.contains("unresolved before possession end")),
            "skipped free throw must be caught: {violations:?}"
        );
    }

    #[test]
    fn negative_control_free_throw_shooter_mismatch_detected() {
        let ticks = vec![
            foul_tick(
                4.0,
                1,
                0,
                0,
                0,
                5,
                vec![foul_event(1, "H_1", "A_5", 1, 5, true, 2)],
            ),
            // 罚球人是 H_2，罚则判给被侵犯人 H_1。
            foul_tick(
                8.0,
                1,
                0,
                0,
                0,
                5,
                vec![free_throw_event(2, 1, "H_2", true)],
            ),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "FOUL_CONSERVATION" && v.detail.contains("shooter mismatch")),
            "free throw shooter lie must be caught: {violations:?}"
        );
    }

    /// 负面对照：犯满球员必须被换下。
    #[test]
    fn negative_control_foul_out_requires_substitution_detected() {
        // A_1 连续 6 次犯规（个人累计 1..6，球队累计同步），事后没有换人事实。
        let mut ticks = Vec::new();
        for i in 1u64..=6 {
            ticks.push(foul_tick(
                i as f32 * 4.0,
                1,
                0,
                0,
                0,
                i as u32,
                vec![foul_event(i, "H_1", "A_1", i, i, i >= 5, 0)],
            ));
        }
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "FOUL_CONSERVATION"
                    && v.detail.contains("never left the court")),
            "missing foul-out substitution must be caught: {violations:?}"
        );
    }

    /// 负面对照：犯满球员不得再犯规，也不得再被换上。
    #[test]
    fn negative_control_fouled_out_player_cannot_return_detected() {
        // A_1 连续 6 次犯规后被换下，随后又被换上并再次犯规。
        let mut ticks = Vec::new();
        for i in 1u64..=6 {
            let mut events = vec![foul_event(i, "H_1", "A_1", i, i, i >= 5, 0)];
            if i == 6 {
                events.push(substitution_event(7, "A_1", "A_6"));
            }
            ticks.push(foul_tick(i as f32 * 4.0, 1, 0, 0, 0, i as u32, events));
        }
        // 已犯满的 A_1 再被换上场。
        ticks.push(foul_tick(
            28.0,
            1,
            0,
            0,
            0,
            6,
            vec![substitution_event(8, "A_6", "A_1")],
        ));
        // 已犯满的 A_1 又犯规（个人累计 7）。
        ticks.push(foul_tick(
            32.0,
            1,
            0,
            0,
            0,
            7,
            vec![foul_event(9, "H_2", "A_1", 7, 7, true, 0)],
        ));
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "FOUL_CONSERVATION"
                    && v.detail.contains("re-entered the game")),
            "fouled-out re-entry must be caught: {violations:?}"
        );
        assert!(
            violations.iter().any(|v| v.equation == "FOUL_CONSERVATION"
                && v.detail.contains("committed another foul")),
            "fouled-out extra foul must be caught: {violations:?}"
        );
    }

    /// 正面对照：比分与事件一致的流不得报账。
    #[test]
    fn balanced_ledger_passes() {
        let ticks = vec![
            synthetic_tick(
                0.0,
                720.0,
                0,
                0,
                vec![event(
                    "SCORE",
                    serde_json::json!({"HoopArrival": {"shooter_id": "H_1", "shot_origin": [0.5, 0.5], "is_made": true, "is_three": false, "contest_intensity": 0.0}}),
                    0,
                )],
            ),
            synthetic_tick(
                10.0,
                710.0,
                2,
                0,
                vec![event(
                    "POSSESSION_SUMMARY",
                    serde_json::json!({"PossessionSummary": {"possession_index": 0, "offense_team": "home", "start_clock": 720.0, "end_clock": 710.0, "duration_seconds": 10.0, "passes_count": 1, "terminal_event": "SCORE", "shooter_id": "H_1"}}),
                    2,
                )],
            ),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations.is_empty(),
            "balanced ledger must not report: {violations:?}"
        );
    }

    /// 负面对照（dev 方案 §2.3 #1）：终场比分与得分事件不一致必须变红。
    #[test]
    fn negative_control_score_conservation_detects_lie() {
        let ticks = vec![
            synthetic_tick(0.0, 720.0, 0, 0, vec![]),
            // 终场 10 分，但流中没有任何得分事件。
            synthetic_tick(100.0, 620.0, 10, 0, vec![]),
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "SCORE_CONSERVATION"),
            "injected score lie must be caught: {violations:?}"
        );
    }

    /// 负面对照：回合索引跳变（缺回合）必须被球权守恒捕获。
    #[test]
    fn negative_control_shot_outcome_kind_must_match_arrival_fact() {
        let ticks = vec![synthetic_tick(
            0.0,
            720.0,
            0,
            0,
            vec![
                event(
                    "SCORE",
                    serde_json::json!({"HoopArrival": {"shooter_id": "H_1", "is_made": false, "is_three": false}}),
                    0,
                ),
                event(
                    "SHOT_MISS",
                    serde_json::json!({"HoopArrival": {"shooter_id": "A_1", "is_made": true, "is_three": false}}),
                    1,
                ),
            ],
        )];
        let violations = check_ledger(&ticks);
        assert_eq!(
            violations
                .iter()
                .filter(|violation| violation.equation == "SCORE_CONSERVATION")
                .count(),
            2,
            "SCORE and SHOT_MISS kinds must agree with HoopArrival.is_made"
        );
    }

    #[test]
    fn negative_control_possession_gap_detected() {
        let summary = |idx: u64| {
            event(
                "POSSESSION_SUMMARY",
                serde_json::json!({"PossessionSummary": {"possession_index": idx, "offense_team": "home", "start_clock": 720.0, "end_clock": 700.0, "duration_seconds": 20.0, "passes_count": 1, "terminal_event": "SCORE", "shooter_id": "H_1"}}),
                idx,
            )
        };
        let ticks = vec![
            synthetic_tick(0.0, 720.0, 0, 0, vec![summary(0)]),
            synthetic_tick(20.0, 700.0, 2, 0, vec![summary(2)]), // 跳过 1
        ];
        let violations = check_ledger(&ticks);
        assert!(
            violations
                .iter()
                .any(|v| v.equation == "POSSESSION_CONSERVATION"),
            "possession gap must be caught: {violations:?}"
        );
    }

    /// 负面对照：零时长回合（时钟填充）必须被时间守恒捕获。
    #[test]
    fn negative_control_zero_duration_detected() {
        let ticks = vec![synthetic_tick(
            0.0,
            720.0,
            0,
            0,
            vec![event(
                "POSSESSION_SUMMARY",
                serde_json::json!({"PossessionSummary": {"possession_index": 0, "offense_team": "home", "start_clock": 720.0, "end_clock": 720.0, "duration_seconds": 0.0, "passes_count": 0, "terminal_event": "TURNOVER_STEAL", "turnover_player_id": "H_1"}}),
                0,
            )],
        )];
        let violations = check_ledger(&ticks);
        assert!(
            violations.iter().any(|v| v.equation == "TIME_CONSERVATION"),
            "zero-duration possession must be caught: {violations:?}"
        );
    }
}
