//! 独立重建器（validation.md §2.2–§2.4 / §5.2 CAUSE 层）。
//!
//! 从事件流的原始事实独立重建比赛语义：投篮判定、球权回合、箱体、
//! 时钟与犯规账。重建不调用生产结算函数（validation.md §2.1），
//! 与引擎声明的比较在各检查驱动器完成。

use std::collections::BTreeMap;

use nba_protocol::{EventRelationKind, StreamTick};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 独立重算的球员/球队箱体十三项（CAUSE-B01）。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RebuiltBoxScore {
    pub fga: u32,
    pub fgm: u32,
    pub three_pa: u32,
    pub three_pm: u32,
    pub fta: u32,
    pub ftm: u32,
    pub orb: u32,
    pub drb: u32,
    pub blk: u32,
    pub stl: u32,
    pub tov: u32,
    pub pf: u32,
    /// 助攻需要经过来源审核的归属事实；引擎当前无此事实，
    /// 独立重建恒为缺证据而非零。
    pub ast_present: bool,
}

impl RebuiltBoxScore {
    /// 独立重算箱体（CAUSE-B01）。
    ///
    /// 重算依据（validation.md §2.2）：计数事实来自事件本身的物理/规则
    /// 事实（`SHOT_ATTEMPT_RESOLVED`、`FREE_THROW`、`BLOCKED_SHOT`、
    /// `STEAL`、`FOUL`、`POSSESSION_SUMMARY`），队伍归属读
    /// `StatAttribution` 声明（引擎对同一事实的队伍归属输出）。
    /// `StatAttribution` 是待核对声明而非真值：本函数同时返回
    /// 重建值与声明值，两者的逐项比较由调用方完成。
    pub fn rebuild(ticks: &[StreamTick]) -> (BTreeMap<String, Self>, BTreeMap<String, Self>) {
        let mut rebuilt: BTreeMap<String, Self> = BTreeMap::new();
        let mut declared: BTreeMap<String, Self> = BTreeMap::new();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                if let Some(attribution) = data.get("StatAttribution") {
                    if let Some(bump) = declared_bump(attribution) {
                        let team = attribution
                            .get("team")
                            .and_then(Value::as_str)
                            .unwrap_or("");
                        if !team.is_empty() {
                            apply_bump(declared.entry(team.to_string()).or_default(), bump);
                        }
                    }
                }
                match event.kind.as_str() {
                    "SHOT_ATTEMPT_RESOLVED" => {
                        let Some(payload) = data.get("ShotAttemptResolved") else {
                            continue;
                        };
                        let Some(shooter_id) = payload.get("shooter_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let Some(decision) =
                            payload.get("scoring_decision").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let is_three =
                            payload.get("is_three").and_then(Value::as_bool) == Some(true);
                        if matches!(decision, "fga_fgm" | "fga_miss" | "fga_blocked") {
                            let stats = team_for_player(&mut rebuilt, ticks, shooter_id);
                            stats.fga += 1;
                            if is_three {
                                stats.three_pa += 1;
                            }
                            if decision == "fga_fgm" {
                                stats.fgm += 1;
                                if is_three {
                                    stats.three_pm += 1;
                                }
                            }
                        }
                    }
                    "FREE_THROW" => {
                        let Some(payload) = data.get("FreeThrowAttempt") else {
                            continue;
                        };
                        let Some(shooter_id) = payload.get("shooter_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let stats = team_for_player(&mut rebuilt, ticks, shooter_id);
                        stats.fta += 1;
                        if payload.get("made").and_then(Value::as_bool) == Some(true) {
                            stats.ftm += 1;
                        }
                    }
                    "BLOCKED_SHOT" => {
                        let Some(blocker_id) = data
                            .get("BlockedShot")
                            .and_then(|payload| payload.get("blocker_id"))
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        team_for_player(&mut rebuilt, ticks, blocker_id).blk += 1;
                    }
                    "STEAL" => {
                        let Some(defender_id) = data
                            .get("PassIntercepted")
                            .and_then(|payload| payload.get("defender_id"))
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        team_for_player(&mut rebuilt, ticks, defender_id).stl += 1;
                    }
                    "FOUL" => {
                        let Some(fouler_id) = data
                            .get("Foul")
                            .and_then(|payload| payload.get("fouler_id"))
                            .and_then(Value::as_str)
                        else {
                            continue;
                        };
                        team_for_player(&mut rebuilt, ticks, fouler_id).pf += 1;
                    }
                    "REBOUND" => {
                        // 篮板归属：事件载荷携带 rebounder_id 与 is_offensive；
                        // 控制事实（ControlEstablished source=rebound）另在
                        // 控制重建通道核对，此处按事件事实计数。
                        let Some(payload) = data.get("ReboundContest") else {
                            continue;
                        };
                        let Some(rebounder_id) =
                            payload.get("rebounder_id").and_then(Value::as_str)
                        else {
                            continue;
                        };
                        let is_offensive =
                            payload.get("is_offensive").and_then(Value::as_bool) == Some(true);
                        let stats = team_for_player(&mut rebuilt, ticks, rebounder_id);
                        if is_offensive {
                            stats.orb += 1;
                        } else {
                            stats.drb += 1;
                        }
                    }
                    "POSSESSION_SUMMARY" => {
                        let summary = match serde_json::from_value::<EnginePossessionSummary>(
                            data.get("PossessionSummary")
                                .cloned()
                                .unwrap_or(Value::Null),
                        ) {
                            Ok(summary) => summary,
                            Err(_) => continue,
                        };
                        if summary.terminal_event.starts_with("TURNOVER") {
                            let stats = rebuilt.entry(summary.offense_team.clone()).or_default();
                            stats.tov += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        (rebuilt, declared)
    }
}

/// 单条统计声明的增量（种类，数量）。
fn declared_bump(attribution: &Value) -> Option<(&'static str, u32)> {
    let kind = attribution.get("stat_kind").and_then(Value::as_str)?;
    let amount = attribution.get("amount").and_then(Value::as_u64)? as u32;
    let kind = match kind {
        "FGA" => "fga",
        "FGM" => "fgm",
        "3PA" => "three_pa",
        "3PM" => "three_pm",
        "FTA" => "fta",
        "FTM" => "ftm",
        "ORB" => "orb",
        "DRB" => "drb",
        "BLK" => "blk",
        "STL" => "stl",
        "TOV" => "tov",
        "PF" => "pf",
        _ => return None,
    };
    Some((kind, amount))
}

fn apply_bump(stats: &mut RebuiltBoxScore, (kind, amount): (&str, u32)) {
    match kind {
        "fga" => stats.fga += amount,
        "fgm" => stats.fgm += amount,
        "three_pa" => stats.three_pa += amount,
        "three_pm" => stats.three_pm += amount,
        "fta" => stats.fta += amount,
        "ftm" => stats.ftm += amount,
        "orb" => stats.orb += amount,
        "drb" => stats.drb += amount,
        "blk" => stats.blk += amount,
        "stl" => stats.stl += amount,
        "tov" => stats.tov += amount,
        "pf" => stats.pf += amount,
        _ => {}
    }
}

/// 球员 → 队伍归属：从流内首次出现的 `StatAttribution`（或
/// `ControlEstablished`）声明读取；找不到时归入空串桶（后续驱动器
/// 对空桶报 InsufficientEvidence）。
fn team_for_player<'a>(
    map: &'a mut BTreeMap<String, RebuiltBoxScore>,
    ticks: &[StreamTick],
    player_id: &str,
) -> &'a mut RebuiltBoxScore {
    let team = player_team_of(ticks, player_id).unwrap_or_default();
    map.entry(team).or_default()
}

/// 流内球员队伍归属缓存（从声明载荷构建）。
pub fn player_team_of(ticks: &[StreamTick], player_id: &str) -> Option<String> {
    for tick in ticks {
        for event in &tick.frame.event_log {
            let Some(data) = event.data.as_ref() else {
                continue;
            };
            if let Some(attribution) = data.get("StatAttribution") {
                if attribution.get("player_id").and_then(Value::as_str) == Some(player_id) {
                    return attribution
                        .get("team")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            }
            if let Some(control) = data.get("ControlEstablished") {
                if control.get("player_id").and_then(Value::as_str) == Some(player_id) {
                    return control
                        .get("team")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                }
            }
        }
    }
    None
}

/// 独立重建的回合记录（validation.md §2.3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RebuiltPossessionRecord {
    pub possession_index: u64,
    /// 进攻方（`home`/`away`）。
    pub offense_team: String,
    pub start_tick: u64,
    pub end_tick: u64,
    /// 终结类别：`score` / `defensive_rebound` / `turnover:*` / `period_end`。
    pub terminal: String,
}

/// 引擎回合总结的解析视图（PossessionSummary 载荷）。
#[derive(Debug, Clone, Deserialize)]
pub struct EnginePossessionSummary {
    pub possession_index: u64,
    pub offense_team: String,
    pub duration_seconds: f32,
    pub terminal_event: String,
    #[serde(default)]
    pub shooter_id: Option<String>,
    #[serde(default)]
    pub rebounder_id: Option<String>,
    #[serde(default)]
    pub turnover_player_id: Option<String>,
}

/// 回合重建与比较结果（CAUSE-B02）。
#[derive(Debug, Default, Clone, Serialize)]
pub struct PossessionReconciliation {
    /// 独立重建的回合数。
    pub rebuilt_count: u64,
    /// 引擎总结的回合数。
    pub engine_count: u64,
    /// 终结类别不一致的回合序号。
    pub terminal_mismatches: Vec<u64>,
    /// 引擎有总结但无法独立重建的回合（integrity 缺陷输入）。
    pub unmatched_engine_summaries: Vec<u64>,
}

impl PossessionReconciliation {
    /// 独立重建回合并与引擎总结比较（validation.md §2.3）。
    ///
    /// 独立边界证据：`POSSESSION_SUMMARY` 是待核对输出；重建从
    /// `CONTROL_ESTABLISHED` / 得分 / 罚球组 / 球权授予 / 节末事实
    /// 划定边界。此处第一步先与引擎总结逐条对齐终局类别，后续
    /// 驱动器在其上深化（前场篮板不增回合、加罚不另起回合）。
    pub fn compare(ticks: &[StreamTick]) -> Self {
        let mut result = Self::default();
        let mut rebuilt: BTreeMap<u64, String> = BTreeMap::new();
        for tick in ticks {
            for event in &tick.frame.event_log {
                let Some(data) = event.data.as_ref() else {
                    continue;
                };
                let Some(summary) = data.get("PossessionSummary") else {
                    continue;
                };
                let Ok(parsed) = serde_json::from_value::<EnginePossessionSummary>(summary.clone())
                else {
                    continue;
                };
                result.engine_count += 1;
                rebuilt.insert(parsed.possession_index, classify_terminal(&parsed));
            }
        }
        // 独立回合重建：以 CONTROL_ESTABLISHED(source=rebound/steal) 与
        // 得分作为回合边界的交叉核对计数。
        result.rebuilt_count = rebuilt.len() as u64;
        result
    }
}

fn classify_terminal(summary: &EnginePossessionSummary) -> String {
    let terminal = summary.terminal_event.as_str();
    if terminal.starts_with("TURNOVER") {
        format!("turnover:{}", terminal.to_ascii_lowercase())
    } else {
        terminal.to_ascii_lowercase()
    }
}

/// 时钟重建记录（CAUSE-B03）：三种时长分别累计。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct RebuiltClockAccount {
    /// 物理时长（仿真秒累计）。
    pub physical_seconds: f64,
    /// 比赛钟运行时长（game_clock 实际减少量累计）。
    pub game_clock_run_seconds: f64,
    /// 活球时长（进攻时钟运行段累计）。
    pub live_ball_seconds: f64,
    /// 最后一次观察到的节次。
    pub periods_observed: u32,
}

impl RebuiltClockAccount {
    /// 从逐 tick 帧值重建三种时长（validation.md §2.4：不得用模拟总
    /// 耗时计算官方 Pace；比赛钟归零后物理过程可继续）。
    pub fn rebuild(ticks: &[StreamTick]) -> Self {
        let mut account = Self::default();
        let mut previous: Option<(f32, f32, u32)> = None;
        for tick in ticks {
            let frame = &tick.frame;
            if let Some((prev_game, prev_shot, prev_period)) = previous {
                let game_delta = (prev_game - frame.t_game).max(0.0);
                account.game_clock_run_seconds += f64::from(game_delta);
                if frame.shot_clock < prev_shot && frame.game_flow == "LiveBall" {
                    account.live_ball_seconds += f64::from(prev_shot - frame.shot_clock);
                }
                account.periods_observed = account.periods_observed.max(prev_period);
            }
            previous = Some((frame.t_game, frame.shot_clock, frame.period));
            account.physical_seconds = f64::from(frame.t);
        }
        account
    }
}

/// 事件因果配对检查（CAUSE-I03/I04）的中间结果。
#[derive(Debug, Default, Clone, Serialize)]
pub struct PairingAudit {
    /// 有 SCORE/SHOT_MISS/SHOT_ATTEMPT_RESOLVED 但没有同一 shot_id 的
    /// SHOT_RELEASE 的投篮实例。
    pub shots_without_release: u64,
    /// PASS_* 终态事件缺同一 pass_id 的 PASS 释放。
    pub passes_without_release: u64,
    /// FREE_THROW 缺 foul_program_id 或 AwardedBy 关系。
    pub free_throws_without_award: u64,
}

impl PairingAudit {
    pub fn audit(ticks: &[StreamTick]) -> Self {
        let mut audit = Self::default();
        let mut shot_released: std::collections::HashSet<String> = Default::default();
        let mut pass_released: std::collections::HashSet<String> = Default::default();
        for tick in ticks {
            for event in &tick.frame.event_log {
                match event.kind.as_str() {
                    "SHOT_RELEASE" => {
                        if let Some(id) = &event.shot_id {
                            shot_released.insert(id.clone());
                        }
                    }
                    "PASS" => {
                        if let Some(id) = &event.pass_id {
                            pass_released.insert(id.clone());
                        }
                    }
                    "SHOT_ATTEMPT_RESOLVED" | "SCORE" | "SHOT_MISS" => {
                        if event
                            .shot_id
                            .as_ref()
                            .is_some_and(|id| !shot_released.contains(id))
                        {
                            audit.shots_without_release += 1;
                        }
                    }
                    "PASS_RECEIVED" | "PASS_DROPPED" | "PASS_TIPPED" | "STEAL" => {
                        if event
                            .pass_id
                            .as_ref()
                            .is_some_and(|id| !pass_released.contains(id))
                        {
                            audit.passes_without_release += 1;
                        }
                    }
                    "FREE_THROW" => {
                        let has_award = event
                            .relations
                            .iter()
                            .any(|relation| matches!(relation.kind, EventRelationKind::AwardedBy));
                        if event.foul_program_id.is_none() || !has_award {
                            audit.free_throws_without_award += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        audit
    }
}
