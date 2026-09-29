//! 犯规与罚则账本（charter §5.2-§5.3）：按事件流独立重建的检查器本体。

use super::LedgerViolation;
use nba_protocol::StreamTick;

/// 犯规账本（charter §5.3）：从事件流独立重建个人累计、球队累计、
/// 犯满离场、bonus 罚则与罚球对应关系，任何一项与事件载荷矛盾即系统说谎。
///
/// 重建只依赖流内事实：
/// - 个人累计：`Foul.personal_foul_count` 必须等于按事件顺序重建的该球员第 N 次犯规；
/// - 球队累计：计入球队账的犯规（种类区分见 §5.2：进攻犯规、双方犯规、
///   技术犯规不进球队账）按节累计，节关闭时必须等于帧计数器
///   `team_fouls_home + team_fouls_away` 在该节观测到的峰值；
///   单犯规帧的 `period_team_foul_count` 必须能在帧计数器上观测到；
/// - bonus：`penalty.is_bonus` 必须等于 `period_team_foul_count >= bonus 阈值`；
/// - 罚球对应：`penalty.free_throw_count` 次 `FreeThrowAttempt` 必须以该犯规
///   事件为因果父、罚球人是被侵犯人；新的带罚球犯规或回合总结出现时，
///   上一犯规的罚球必须已经完成；
/// - 犯满离场：达到 `max_personal_fouls` 的球员必须被 `Substitution` 换下，
///   不得再犯规，也不得再被换上。
///
/// 限值来源（quality.md §1.1 单一事实源）：犯满上限与 bonus 阈值取自帧内
/// `FrameRules`（紧凑流首条携带、向前继承），检查器不自带副本。
pub(super) fn check_foul_conservation(ticks: &[StreamTick], out: &mut Vec<LedgerViolation>) {
    use std::collections::{HashMap, HashSet};

    /// 单节球队账。
    #[derive(Default)]
    struct PeriodAccount {
        /// 计入球队账的犯规数（按事件顺序重建）。
        counted: u32,
        /// 帧计数器 `home + away` 在本节的观测峰值。
        observed_peak: u32,
    }

    let mut limits: Option<(u8, u32)> = None;
    let mut personal: HashMap<String, u32> = HashMap::new();
    let mut periods: HashMap<u32, PeriodAccount> = HashMap::new();
    let mut last_period: Option<u32> = None;
    // 犯满球员 → 是否已观察到换下事实。
    let mut fouled_out: HashMap<String, bool> = HashMap::new();
    // 球员 → 队伍（换人事实的 team 字段）。
    let mut player_teams: HashMap<String, String> = HashMap::new();
    // 引擎发布的板凳耗尽事实（`BENCH_DEPLETED retain:<player>`）。
    let mut bench_depleted_retains: Option<HashSet<String>> = None;
    // 全部 FOUL 事件 ID（罚球因果父必须是带罚球的 FOUL）。
    let mut foul_ids: HashSet<u64> = HashSet::new();
    // 带罚球的 FOUL event_id → (被侵犯人, 剩余应尝试次数)。
    let mut pending: HashMap<u64, (String, u32)> = HashMap::new();

    for tick in ticks {
        let frame = &tick.frame;
        if limits.is_none() && frame.rules.is_present() {
            limits = Some((
                frame.rules.max_personal_fouls,
                frame.rules.bonus_fouls_per_period,
            ));
        }
        if let Some(prev) = last_period {
            if prev != frame.period {
                let account = periods.get(&prev);
                let counted = account.map(|a| a.counted).unwrap_or(0);
                let peak = account.map(|a| a.observed_peak).unwrap_or(0);
                if counted != peak {
                    out.push(LedgerViolation::new(
                        "FOUL_CONSERVATION",
                        format!(
                            "period {prev} team fouls: {counted} counted from events vs {peak} observed in frame counters"
                        ),
                    ));
                }
            }
        }
        last_period = Some(frame.period);
        let account = periods.entry(frame.period).or_default();
        account.observed_peak = account
            .observed_peak
            .max(frame.team_fouls_home.saturating_add(frame.team_fouls_away));

        let fouls_in_frame = frame
            .event_log
            .iter()
            .filter(|event| event.kind == "FOUL")
            .count();

        for event in &frame.event_log {
            match event.kind.as_str() {
                "FOUL" => {
                    let Some(payload) = event.data.as_ref().and_then(|d| d.get("Foul")) else {
                        continue;
                    };
                    let fouled = payload
                        .get("fouled_player_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let fouler = payload
                        .get("fouler_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    foul_ids.insert(event.event_id);
                    let count = personal.entry(fouler.to_string()).or_insert(0);
                    *count += 1;
                    if let Some(claimed) =
                        payload.get("personal_foul_count").and_then(|v| v.as_u64())
                    {
                        if claimed != *count as u64 {
                            let mut v = LedgerViolation::new(
                                "FOUL_CONSERVATION",
                                format!(
                                    "personal foul count for {fouler} mismatch: reconstructed {count} vs event {claimed}"
                                ),
                            );
                            v.tick = Some(event.sequence);
                            out.push(v);
                        }
                    }
                    // 球队账种类区分：进攻犯规、双方犯规与技术犯规不进球队账。
                    let kind = payload.get("foul_kind").and_then(|v| v.as_str());
                    let counts_team =
                        !matches!(kind, Some("offensive") | Some("double") | Some("technical"));
                    let team_count = payload
                        .get("period_team_foul_count")
                        .and_then(|v| v.as_u64());
                    if counts_team {
                        periods.get_mut(&frame.period).unwrap().counted += 1;
                        if fouls_in_frame == 1 {
                            if let Some(c) = team_count {
                                let visible = frame.team_fouls_home as u64 == c
                                    || frame.team_fouls_away as u64 == c;
                                if !visible {
                                    let mut v = LedgerViolation::new(
                                        "FOUL_CONSERVATION",
                                        format!(
                                            "period team foul count {c} not visible in frame counters ({}, {})",
                                            frame.team_fouls_home, frame.team_fouls_away
                                        ),
                                    );
                                    v.tick = Some(event.sequence);
                                    out.push(v);
                                }
                            }
                        }
                    }
                    if let (Some(c), Some((_, bonus_limit))) = (team_count, limits) {
                        let claimed_bonus = payload
                            .get("penalty")
                            .and_then(|p| p.get("is_bonus"))
                            .and_then(|v| v.as_bool());
                        if let Some(claimed) = claimed_bonus {
                            let expected = c >= bonus_limit as u64;
                            if claimed != expected {
                                let mut v = LedgerViolation::new(
                                    "FOUL_CONSERVATION",
                                    format!(
                                        "bonus flag for fouler {fouler} mismatch: team count {c} vs limit {bonus_limit} expects {expected}, event claims {claimed}"
                                    ),
                                );
                                v.tick = Some(event.sequence);
                                out.push(v);
                            }
                        }
                    }
                    // 罚球对应登记（charter §5.3 逐次罚球）：同一死球窗口内
                    // 的新判罚合法地排队（罚球程序按序执行），故不在此检查
                    // 「上一判罚是否完成」；未完成的罚球由回合总结与流末
                    // 检查兜底——两者出现即说明队列在程序结束前被丢弃。
                    let ft_count = payload
                        .get("penalty")
                        .and_then(|p| p.get("free_throw_count"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    if ft_count > 0 {
                        pending.insert(event.event_id, (fouled.to_string(), ft_count as u32));
                    }
                    // 犯满：达到上限必须换下；已犯满者不得再犯规。
                    // 板凳耗尽时犯满者留场继续犯规是合法例外（
                    // `BENCH_DEPLETED retain` 事实），不报。
                    if let Some((max_personal, _)) = limits {
                        if *count == max_personal as u32 {
                            fouled_out.entry(fouler.to_string()).or_insert(false);
                        } else if *count > max_personal as u32 {
                            let bench_depleted = bench_depleted_retains
                                .as_ref()
                                .is_some_and(|set| set.contains(fouler));
                            if !bench_depleted {
                                let mut v = LedgerViolation::new(
                                    "FOUL_CONSERVATION",
                                    format!(
                                        "fouled-out player {fouler} committed another foul ({count} > {max_personal})"
                                    ),
                                );
                                v.tick = Some(event.sequence);
                                out.push(v);
                            }
                            fouled_out.entry(fouler.to_string()).or_insert(false);
                        }
                    }
                }
                "FREE_THROW" => {
                    let shooter = event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("FreeThrowAttempt"))
                        .and_then(|ft| ft.get("shooter_id"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    match event.parent_event_id {
                        Some(parent) => {
                            if let Some((expected_shooter, remaining)) = pending.get_mut(&parent) {
                                if *remaining == 0 {
                                    let mut v = LedgerViolation::new(
                                        "FOUL_CONSERVATION",
                                        format!(
                                            "free throw by {shooter} exceeds the penalty of foul event {parent}"
                                        ),
                                    );
                                    v.tick = Some(event.sequence);
                                    out.push(v);
                                } else {
                                    if shooter != *expected_shooter {
                                        let mut v = LedgerViolation::new(
                                            "FOUL_CONSERVATION",
                                            format!(
                                                "free throw shooter mismatch: penalty awards {expected_shooter}, attempt by {shooter}"
                                            ),
                                        );
                                        v.tick = Some(event.sequence);
                                        out.push(v);
                                    }
                                    *remaining -= 1;
                                }
                            } else if !foul_ids.contains(&parent) {
                                let mut v = LedgerViolation::new(
                                    "FOUL_CONSERVATION",
                                    format!(
                                        "free throw by {shooter} parented to non-foul event {parent}"
                                    ),
                                );
                                v.tick = Some(event.sequence);
                                out.push(v);
                            } else {
                                let mut v = LedgerViolation::new(
                                    "FOUL_CONSERVATION",
                                    format!(
                                        "free throw by {shooter} parented to foul event {parent} that awards no attempts"
                                    ),
                                );
                                v.tick = Some(event.sequence);
                                out.push(v);
                            }
                        }
                        None => {
                            let mut v = LedgerViolation::new(
                                "FOUL_CONSERVATION",
                                format!("free throw by {shooter} without causal parent"),
                            );
                            v.tick = Some(event.sequence);
                            out.push(v);
                        }
                    }
                }
                "SUBSTITUTION" => {
                    let Some(payload) = event.data.as_ref().and_then(|d| d.get("Substitution"))
                    else {
                        continue;
                    };
                    let team = payload.get("team").and_then(|v| v.as_str()).unwrap_or("");
                    let out_player = payload
                        .get("out_player")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let in_player = payload
                        .get("in_player")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    // 球员队伍归属（替补池枯竭豁免的判据输入）：换人事实
                    // 的 team 字段给出出/入场球员的队属。
                    if !team.is_empty() {
                        if !out_player.is_empty() {
                            player_teams.insert(out_player.to_string(), team.to_string());
                        }
                        if !in_player.is_empty() {
                            player_teams.insert(in_player.to_string(), team.to_string());
                        }
                    }
                    if let Some(substituted) = fouled_out.get_mut(out_player) {
                        *substituted = true;
                    }
                    if fouled_out.contains_key(in_player) {
                        let mut v = LedgerViolation::new(
                            "FOUL_CONSERVATION",
                            format!("fouled-out player {in_player} re-entered the game"),
                        );
                        v.tick = Some(event.sequence);
                        out.push(v);
                    }
                }
                "ENFORCEMENT_APPLIED" => {
                    let Some(payload) = event
                        .data
                        .as_ref()
                        .and_then(|d| d.get("EnforcementApplied"))
                    else {
                        continue;
                    };
                    let constraint = payload
                        .get("constraint_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let action = payload.get("action").and_then(|v| v.as_str()).unwrap_or("");
                    if constraint == "BENCH_DEPLETED" {
                        if let Some(retained) = action.strip_prefix("retain:") {
                            bench_depleted_retains
                                .get_or_insert_with(HashSet::new)
                                .insert(retained.to_string());
                        }
                    }
                }
                "POSSESSION_SUMMARY" => {
                    for (id, (shooter, remaining)) in &pending {
                        if *remaining > 0 {
                            out.push(LedgerViolation::new(
                                "FOUL_CONSERVATION",
                                format!(
                                    "free throws of foul event {id} for {shooter} left unresolved before possession end"
                                ),
                            ));
                        }
                    }
                    pending.clear();
                }
                _ => {}
            }
        }
    }

    // 流末结算：最后一节的球队账、未完成的罚球、未换下的犯满球员。
    if let Some(prev) = last_period {
        let account = periods.get(&prev);
        let counted = account.map(|a| a.counted).unwrap_or(0);
        let peak = account.map(|a| a.observed_peak).unwrap_or(0);
        if counted != peak {
            out.push(LedgerViolation::new(
                "FOUL_CONSERVATION",
                format!(
                    "period {prev} team fouls: {counted} counted from events vs {peak} observed in frame counters"
                ),
            ));
        }
    }
    for (id, (shooter, remaining)) in &pending {
        if *remaining > 0 {
            out.push(LedgerViolation::new(
                "FOUL_CONSERVATION",
                format!(
                    "free throws of foul event {id} for {shooter} never attempted ({remaining} left)"
                ),
            ));
        }
    }
    for (player, substituted) in &fouled_out {
        if !*substituted {
            // 替补池枯竭豁免（真实规则边界），两种流内事实任一成立即合法：
            // 1. 引擎发布的 `BENCH_DEPLETED retain:<player>`（候选空时的事实）；
            // 2. 同队犯满人数达到 5 人（旧口径：8-10 人名单 5 人犯满时
            //    可换的未犯满替补已枯竭）。
            let bench_depleted_fact = bench_depleted_retains
                .as_ref()
                .is_some_and(|set| set.contains(player.as_str()));
            let team = player_teams.get(player);
            let teammates_fouled_out = team
                .map(|t| {
                    fouled_out
                        .iter()
                        .filter(|(other, _)| {
                            player_teams.get(*other).map(|ot| ot == t).unwrap_or(false)
                        })
                        .count()
                })
                .unwrap_or(0);
            if !bench_depleted_fact && teammates_fouled_out < 5 {
                out.push(LedgerViolation::new(
                    "FOUL_CONSERVATION",
                    format!("fouled-out player {player} never left the court"),
                ));
            }
        }
    }
}
