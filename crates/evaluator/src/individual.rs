//! 个体评判准则簇（R6：attributes.md §2.6 倾向与个体身份）。
//!
//! 四条准则全部只读**事件流与帧投影**里已有的事实，逐条给出：
//!
//! | 准则 | 判据 | 证据不足时 |
//! |------|------|-----------|
//! | `USAGE_CONCENTRATION` | 两队分别计算球员使用回合份额 | 投篮、罚球或失误归属缺失 → `InsufficientEvidence` |
//! | `ASSIST_PARENT_CHAIN` | 进球的因果父链是否经由接球事实（助攻可重建） | 流中无 `PASS_RECEIVED` 父链 → `InsufficientEvidence` |
//! | `MATCHUP_RESPONSIBILITY` | 弱侧协防责任是否在同一回合恢复 | 无责任转移或协防事实 → `InsufficientEvidence` |
//! | `LATE_GAME_STAMINA` | 第四节末段在场上球员的体能是否仍高于归零线 | 帧投影无 `stm` → `InsufficientEvidence` |
//!
//! 依据 `docs/dev/current/plan.md` §7 与 dev 方案 §5.1：证据不足时保持
//! `InsufficientEvidence`（不计分、不给假结论），不得为了「有结论」而编造。

use std::collections::{HashMap, HashSet};

use crate::report::Judgment;

/// 个体评判的输入证据：从事件流与帧投影重建（由 `lib.rs` 组装）。
pub(crate) struct IndividualEvidence {
    /// 按球队和球员聚合的使用回合权重（投篮出手 + 0.44 倍罚球出手 + 失误）。
    pub(crate) usage: HashMap<(String, String), f32>,
    /// 使用率相关事件是否具备完整的球员归属。
    pub(crate) usage_evidence_complete: bool,
    /// 进球总数及其中沿父链经过接球事实的数量。
    pub(crate) total_made: usize,
    pub(crate) made_via_pass_reception: usize,
    /// 用于区分流中缺少助攻来源事实与因果链不完整。
    pub(crate) pass_receptions: usize,
    /// 观察到的责任迁移和协防分配数量。
    pub(crate) responsibility_changes: usize,
    pub(crate) help_evidence_present: bool,
    pub(crate) help_assignments: usize,
    /// 同一回合内恢复原有责任的数量。
    pub(crate) help_assignments_restored: usize,
    /// 第四节末段在场球员的体能最低值、样本数与字段可用性。
    pub(crate) late_period_min_stamina: Option<f32>,
    pub(crate) late_period_players: usize,
    pub(crate) stamina_fact_present: bool,
}

/// 使用率集中度判据的范围：最小份额用于发现球权过度平均，最高份额用于发现过度集中。
const USAGE_TOP_MIN: f32 = 0.20;
const USAGE_TOP_MAX: f32 = 0.45;
/// 证据门槛：样本过少时判 InsufficientEvidence。
const MIN_USAGE_EVENTS: f32 = 20.0;
/// 助攻父链判据的证据门槛。
const MIN_MADE_SHOTS: usize = 10;

pub(crate) fn evaluate_individual_criteria(
    out: &mut Vec<Judgment>,
    idx: u64,
    ev: IndividualEvidence,
) {
    evaluate_usage_concentration(out, idx, &ev);
    evaluate_assist_parent_chain(out, idx, &ev);
    evaluate_matchup_responsibility(out, idx, &ev);
    evaluate_late_game_stamina(out, idx, &ev);
}

fn evaluate_usage_concentration(out: &mut Vec<Judgment>, idx: u64, ev: &IndividualEvidence) {
    let present_teams: HashSet<&str> = ev.usage.keys().map(|(team, _)| team.as_str()).collect();
    if present_teams.len() < 2 {
        out.push(Judgment::insufficient(
            "USAGE_CONCENTRATION",
            "decision",
            idx,
        ));
        return;
    }
    let mut team_totals: HashMap<&str, f32> = HashMap::new();
    for ((team, _), count) in &ev.usage {
        *team_totals.entry(team.as_str()).or_default() += count;
    }
    if !ev.usage_evidence_complete
        || team_totals.len() != 2
        || team_totals.values().any(|count| *count < MIN_USAGE_EVENTS)
    {
        out.push(Judgment::insufficient(
            "USAGE_CONCENTRATION",
            "decision",
            idx,
        ));
        return;
    }
    let top = ev
        .usage
        .iter()
        .filter_map(|((team, player), count)| {
            let team_total = team_totals.get(team.as_str())?;
            Some((team.as_str(), player.as_str(), *count / team_total))
        })
        .max_by(|left, right| left.2.total_cmp(&right.2));
    let Some((_team, top_id, top_share)) = top else {
        out.push(Judgment::insufficient(
            "USAGE_CONCENTRATION",
            "decision",
            idx,
        ));
        return;
    };
    if (USAGE_TOP_MIN..=USAGE_TOP_MAX).contains(&top_share) {
        out.push(Judgment::pass("USAGE_CONCENTRATION", "decision", idx));
    } else {
        out.push(Judgment::defect(
            "USAGE_CONCENTRATION",
            "soft",
            format!(
                "top usage {top_id} at {:.3} outside [{USAGE_TOP_MIN}, {USAGE_TOP_MAX}] \
                 ({} teams, {} team-player records)",
                top_share,
                team_totals.len(),
                ev.usage.len()
            ),
            "decision",
            idx,
        ));
    }
}

fn evaluate_assist_parent_chain(out: &mut Vec<Judgment>, idx: u64, ev: &IndividualEvidence) {
    if ev.total_made < MIN_MADE_SHOTS {
        out.push(Judgment::insufficient(
            "ASSIST_PARENT_CHAIN",
            "decision",
            idx,
        ));
        return;
    }
    if ev.pass_receptions == 0 {
        // 流里没有任何接球事实：助攻无法从流重建，保持证据不足
        // （不得把「没有助攻事实」判成「助攻率为 0」）。
        out.push(Judgment::insufficient(
            "ASSIST_PARENT_CHAIN",
            "decision",
            idx,
        ));
        return;
    }
    let share = ev.made_via_pass_reception as f32 / ev.total_made as f32;
    // 助攻父链份额过低时，表示进球缺少可追溯的接球因果来源。
    const MIN_ASSIST_PARENT_SHARE: f32 = 0.35;
    if share >= MIN_ASSIST_PARENT_SHARE {
        out.push(Judgment::pass("ASSIST_PARENT_CHAIN", "decision", idx));
    } else {
        out.push(Judgment::defect(
            "ASSIST_PARENT_CHAIN",
            "soft",
            format!(
                "only {}/{} made baskets ({:.2}) trace a pass reception in their causal chain",
                ev.made_via_pass_reception, ev.total_made, share
            ),
            "decision",
            idx,
        ));
    }
}

fn evaluate_matchup_responsibility(out: &mut Vec<Judgment>, idx: u64, ev: &IndividualEvidence) {
    if ev.responsibility_changes == 0 || !ev.help_evidence_present || ev.help_assignments == 0 {
        out.push(Judgment::insufficient(
            "MATCHUP_RESPONSIBILITY",
            "decision",
            idx,
        ));
        return;
    }
    let restored_share = ev.help_assignments_restored as f32 / ev.help_assignments as f32;
    // 真实协防里，弱侧协防人多数在数秒内回到原对位；长期不回说明责任
    // 链断裂（人或球丢了再也回不来）。
    const MIN_RESTORED_SHARE: f32 = 0.50;
    if restored_share >= MIN_RESTORED_SHARE {
        out.push(Judgment::pass("MATCHUP_RESPONSIBILITY", "decision", idx));
    } else {
        out.push(Judgment::defect(
            "MATCHUP_RESPONSIBILITY",
            "soft",
            format!(
                "only {}/{} help assignments ({:.2}) were restored to the original matchup",
                ev.help_assignments_restored, ev.help_assignments, restored_share
            ),
            "decision",
            idx,
        ));
    }
}

fn evaluate_late_game_stamina(out: &mut Vec<Judgment>, idx: u64, ev: &IndividualEvidence) {
    if !ev.stamina_fact_present || ev.late_period_players == 0 {
        out.push(Judgment::insufficient("LATE_GAME_STAMINA", "decision", idx));
        return;
    }
    let Some(min_stamina) = ev.late_period_min_stamina else {
        out.push(Judgment::insufficient("LATE_GAME_STAMINA", "decision", idx));
        return;
    };
    // 第四节末段仍应有球员保有可用体能：全队跑空说明体能模型与轮换脱节。
    const MIN_LATE_STAMINA: f32 = 0.15;
    if min_stamina >= MIN_LATE_STAMINA {
        out.push(Judgment::pass("LATE_GAME_STAMINA", "decision", idx));
    } else {
        out.push(Judgment::defect(
            "LATE_GAME_STAMINA",
            "soft",
            format!(
                "lowest on-court stamina in the late game is {min_stamina:.3}, \
                 below {MIN_LATE_STAMINA} ({} players observed)",
                ev.late_period_players
            ),
            "decision",
            idx,
        ));
    }
}

/// 从事件流重建个体证据（由 `lib.rs` 调用）。
///
/// 使用率从 `SHOT_RELEASE`、`FREE_THROW` 和 `POSSESSION_SUMMARY` 中按攻方及球员身份聚合；
/// 助攻父链从 `SCORE` 沿因果链接查找接球事实；对位与体能读取责任事件和完整帧投影。
pub(crate) fn collect_individual_evidence(
    ticks: &[nba_protocol::StreamTick],
) -> IndividualEvidence {
    let mut usage: HashMap<(String, String), f32> = HashMap::new();
    let mut usage_evidence_complete = true;
    // 事件 id → 类型（用于父链解析）。
    let mut kinds: HashMap<u64, String> = HashMap::new();
    // 事件 id → 因果父。
    let mut parents: HashMap<u64, u64> = HashMap::new();
    let mut made_total = 0usize;
    let mut made_via_pass = 0usize;
    let mut pass_receptions = 0usize;
    let mut responsibility_changes = 0usize;
    let mut help_evidence_present = false;
    let mut help_assignments = 0usize;
    let mut help_restored = 0usize;
    // 同一 possession 中，防守人离开与恢复的对位记录。
    let mut help_open: HashSet<(u32, String, String)> = HashSet::new();
    let mut stamina_fact_present = false;
    let mut late_min: Option<f32> = None;
    let mut late_players = 0usize;

    for tick in ticks {
        let frame = &tick.frame;
        // 帧投影体能：第四节（及加时）末段（比赛钟 <= 5 分钟）在场上球员。
        if frame.players.iter().any(|p| p.on_court) {
            for player in frame.players.iter().filter(|p| p.on_court) {
                if frame.period >= 4 && tick.game_clock <= LATE_GAME_WINDOW_SECONDS {
                    stamina_fact_present = true;
                    late_players += 1;
                    let stamina = player.stm / player.stm_max.max(f32::EPSILON);
                    late_min = Some(match late_min {
                        Some(current) => current.min(stamina),
                        None => stamina,
                    });
                }
            }
        }
        for event in &frame.event_log {
            kinds.insert(event.event_id, event.kind.clone());
            if let Some(parent) = event.parent_event_id {
                parents.insert(event.event_id, parent);
            }
            let Some(payload) = event.data.as_ref() else {
                if matches!(
                    event.kind.as_str(),
                    "SHOT_RELEASE" | "FREE_THROW" | "POSSESSION_SUMMARY"
                ) {
                    usage_evidence_complete = false;
                }
                continue;
            };
            match event.kind.as_str() {
                "SHOT_RELEASE" => {
                    if let Some(shooter) = payload
                        .get("ShotRelease")
                        .and_then(|d| d.get("shooter_id"))
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                    {
                        *usage
                            .entry((frame.possession_team.clone(), shooter.to_string()))
                            .or_default() += 1.0;
                    } else {
                        usage_evidence_complete = false;
                    }
                }
                "FREE_THROW" => {
                    if let Some(shooter) = payload
                        .get("FreeThrowAttempt")
                        .and_then(|d| d.get("shooter_id"))
                        .and_then(|v| v.as_str())
                        .filter(|id| !id.is_empty())
                    {
                        *usage
                            .entry((frame.possession_team.clone(), shooter.to_string()))
                            .or_default() += 0.44;
                    } else {
                        usage_evidence_complete = false;
                    }
                }
                // 进球事实的 kind 是 `SCORE`（`HoopArrival` 是载荷键：
                // `event_type_str` 按 `is_made` 分流为 SCORE / SHOT_MISS）。
                "SCORE" | "HOOP_ARRIVAL" => {
                    let Some(arrival) = payload.get("HoopArrival") else {
                        continue;
                    };
                    if !arrival
                        .get("is_made")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        continue;
                    }
                    if frame.possession_team.is_empty() {
                        usage_evidence_complete = false;
                    }
                    made_total += 1;
                    // 父链：进球 → 轨迹到达 → 出手 →（可选）接球事实。
                    // 向上追溯：只有「出手/轨迹到达」是合法中转；遇到
                    // 突破/转换/无父即终止（接球后突破不算助攻）。
                    let mut ancestor = parents.get(&event.event_id).copied();
                    let mut hops = 0;
                    while let Some(id) = ancestor {
                        hops += 1;
                        if hops > MAX_PARENT_CHAIN_HOPS {
                            break;
                        }
                        match kinds.get(&id).map(String::as_str) {
                            Some("PASS_RECEIVED") => {
                                made_via_pass += 1;
                                break;
                            }
                            Some("SHOT_RELEASE")
                            | Some("SHOT_TRAJECTORY_ARRIVAL")
                            | Some("HOOP_ARRIVAL") => {
                                ancestor = parents.get(&id).copied();
                            }
                            _ => break,
                        }
                    }
                }
                "PASS_RECEIVED" => {
                    pass_receptions += 1;
                }
                "POSSESSION_SUMMARY" => {
                    if let Some(summary) = payload.get("PossessionSummary") {
                        let is_turnover = summary
                            .get("terminal_event")
                            .and_then(|value| value.as_str())
                            .is_some_and(|cause| cause.starts_with("TURNOVER"));
                        if is_turnover {
                            if let Some(actor) = summary
                                .get("turnover_player_id")
                                .and_then(|v| v.as_str())
                                .filter(|id| !id.is_empty())
                            {
                                let team = summary
                                    .get("offense_team")
                                    .and_then(|value| value.as_str())
                                    .filter(|team| !team.is_empty());
                                if let Some(team) = team {
                                    *usage
                                        .entry((team.to_string(), actor.to_string()))
                                        .or_default() += 1.0;
                                } else {
                                    usage_evidence_complete = false;
                                }
                            } else {
                                usage_evidence_complete = false;
                            }
                        }
                    }
                }
                "DEFENSE_RESPONSIBILITY_CHANGED" => {
                    let Some(change) = payload.get("DefenseResponsibilityChanged") else {
                        continue;
                    };
                    responsibility_changes += 1;
                    let defender = change
                        .get("defender_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let offensive = change
                        .get("offensive_player_id")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string();
                    let new_resp = change
                        .get("new_responsibility")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    let key = (frame.possession_id, defender, offensive);
                    if matches!(new_resp, "help" | "rotate") {
                        help_evidence_present = true;
                        if help_open.insert(key) {
                            help_assignments += 1;
                        }
                    } else if matches!(new_resp, "recover" | "primary_matchup")
                        && help_open.remove(&key)
                    {
                        help_restored += 1;
                    }
                }
                _ => {}
            }
        }
    }
    IndividualEvidence {
        usage,
        usage_evidence_complete,
        total_made: made_total,
        made_via_pass_reception: made_via_pass,
        pass_receptions,
        responsibility_changes,
        help_evidence_present,
        help_assignments,
        help_assignments_restored: help_restored,
        late_period_min_stamina: late_min,
        late_period_players: late_players,
        stamina_fact_present,
    }
}

/// 第四节末段窗口（秒）：比赛钟低于本值视为「末段」。
const LATE_GAME_WINDOW_SECONDS: f32 = 300.0;
/// 父链追溯的最大跳数（防止流成环时死循环）。
const MAX_PARENT_CHAIN_HOPS: u32 = 8;

#[cfg(test)]
mod tests {
    use super::*;
    use nba_protocol::{FrameEvent, RenderFrame, RenderPlayer, StreamTick};
    use std::collections::HashMap as Map;

    fn event(kind: &str, data: serde_json::Value, id: u64, parent: Option<u64>) -> FrameEvent {
        FrameEvent {
            event_id: id,
            parent_event_id: parent,
            sequence: id,
            time: 0.0,
            kind: kind.to_string(),
            data: Some(data),
        }
    }

    fn tick_with(events: Vec<FrameEvent>, players: Vec<RenderPlayer>, period: u32) -> StreamTick {
        let frame = RenderFrame {
            t: 0.0,
            t_game: 0.0,
            shot_clock: 24.0,
            period,
            phase: "LiveBall".to_string(),
            possession_id: 1,
            possession_team: "home".to_string(),
            home_team: Default::default(),
            away_team: Default::default(),
            score: Default::default(),
            players,
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
            game_clock: 120.0,
            keyframe_index: None,
        }
    }

    fn on_court_player(id: &str, stamina: f32) -> RenderPlayer {
        RenderPlayer {
            id: id.to_string(),
            jersey: id.to_string(),
            team: "home".to_string(),
            position: String::new(),
            offensive_role: String::new(),
            defensive_role: String::new(),
            x: 47.0,
            y: 25.0,
            zone: "Paint".to_string(),
            has_ball: false,
            on_court: true,
            action: "SpotUp".to_string(),
            action_phase: None,
            slot: String::new(),
            morale: "Normal".to_string(),
            stm: stamina,
            stm_max: 1.0,
            foul_count: 0,
            target_x: None,
            target_y: None,
            facing_x: None,
            facing_y: None,
            potential_target_x: None,
            potential_target_y: None,
            potential_action: None,
            potential_threat_ratio: None,
            potential_void_ratio: None,
        }
    }

    fn verdict_of(out: &[Judgment], criterion: &str) -> String {
        out.iter()
            .find(|j| j.criterion == criterion)
            .map(|j| format!("{:?}", j.verdict))
            .unwrap_or_else(|| "MISSING".to_string())
    }

    /// 正例：使用率集中、助攻父链完整、责任成对、末节体能尚可 → 四条全过。
    #[test]
    fn individual_criteria_pass_on_a_balanced_stream() {
        let mut events = Vec::new();
        let mut id = 1u64;
        // 12 次接球后进球（父链：SCORE ← 轨迹到达 ← 出手 ← 接球）。
        for _ in 0..12 {
            let reception = id;
            let pass = reception;
            id += 1;
            events.push(event(
                "PASS",
                serde_json::json!({"PassRelease": {"passer_id": "H_02", "receiver_id": "H_03"}}),
                pass,
                None,
            ));
            events.push(event(
                "PASS_RECEIVED",
                serde_json::json!({"PassReceived": {"receiver_id": "H_03"}}),
                reception,
                Some(pass),
            ));
            let release = id;
            let arrival = id + 1;
            let score = id + 2;
            id += 3;
            events.push(event(
                "SHOT_RELEASE",
                serde_json::json!({"ShotRelease": {"shooter_id": "H_03"}}),
                release,
                Some(reception),
            ));
            events.push(event(
                "SHOT_TRAJECTORY_ARRIVAL",
                serde_json::json!({"ShotTrajectoryArrival": {}}),
                arrival,
                Some(release),
            ));
            events.push(event(
                "SCORE",
                serde_json::json!({"HoopArrival": {"is_made": true, "shooter_id": "H_03"}}),
                score,
                Some(arrival),
            ));
        }
        // 四名主队球员分别承担 12 次出手，共 60 个加权使用事件。
        for shooter in ["H_01", "H_02", "H_04"] {
            for _ in 0..12 {
                events.push(event(
                    "SHOT_RELEASE",
                    serde_json::json!({"ShotRelease": {"shooter_id": shooter}}),
                    id,
                    None,
                ));
                id += 1;
            }
        }
        // 责任转移成对：help → recover。
        events.push(event(
            "DEFENSE_RESPONSIBILITY_CHANGED",
            serde_json::json!({"DefenseResponsibilityChanged": {
                "defender_id": "A_02", "offensive_player_id": "H_02",
                "new_responsibility": "help"
            }}),
            id,
            None,
        ));
        id += 1;
        events.push(event(
            "DEFENSE_RESPONSIBILITY_CHANGED",
            serde_json::json!({"DefenseResponsibilityChanged": {
                "defender_id": "A_02", "offensive_player_id": "H_02",
                "new_responsibility": "recover"
            }}),
            id,
            None,
        ));
        let players = vec![on_court_player("H_03", 0.6), on_court_player("A_02", 0.5)];
        let mut away_events = vec![event(
            "POSSESSION_SUMMARY",
            serde_json::json!({"PossessionSummary": {
                "offense_team": "away",
                "terminal_event": "TURNOVER_STEAL",
                "turnover_player_id": "A_01"
            }}),
            id,
            None,
        )];
        id += 1;
        for shooter in ["A_01", "A_02", "A_03", "A_04"] {
            for _ in 0..12 {
                away_events.push(event(
                    "SHOT_RELEASE",
                    serde_json::json!({"ShotRelease": {"shooter_id": shooter}}),
                    id,
                    None,
                ));
                id += 1;
            }
        }
        let mut away_tick = tick_with(away_events, vec![], 4);
        away_tick.frame.possession_team = "away".to_string();
        let ticks = vec![tick_with(events, players, 4), away_tick];
        let ev = collect_individual_evidence(&ticks);
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        for criterion in [
            "USAGE_CONCENTRATION",
            "ASSIST_PARENT_CHAIN",
            "MATCHUP_RESPONSIBILITY",
            "LATE_GAME_STAMINA",
        ] {
            assert_eq!(
                verdict_of(&out, criterion),
                "Pass",
                "{criterion} should pass on a balanced stream"
            );
        }
    }

    /// 负面对照①：流里有接球事实，但进球父链断在出手处 → 助攻父链报缺陷。
    #[test]
    fn assist_parent_chain_defect_when_chain_stops_at_release() {
        let mut events = Vec::new();
        // 12 次接球事实（存在助攻证据）。
        for i in 0..12u64 {
            events.push(event(
                "PASS",
                serde_json::json!({"PassRelease": {"passer_id": "H_02", "receiver_id": "H_04"}}),
                i * 4 + 1,
                None,
            ));
            events.push(event(
                "PASS_RECEIVED",
                serde_json::json!({"PassReceived": {"receiver_id": "H_04"}}),
                i * 4 + 2,
                Some(i * 4 + 1),
            ));
        }
        // 12 次进球，父链只回到出手（没有接球祖先）。
        for i in 0..12u64 {
            let release = 100 + i * 2;
            events.push(event(
                "SHOT_RELEASE",
                serde_json::json!({"ShotRelease": {"shooter_id": "H_03"}}),
                release,
                None,
            ));
            events.push(event(
                "SCORE",
                serde_json::json!({"HoopArrival": {"is_made": true, "shooter_id": "H_03"}}),
                release + 1,
                Some(release),
            ));
        }
        let ticks = vec![tick_with(events, vec![], 4)];
        let ev = collect_individual_evidence(&ticks);
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(verdict_of(&out, "ASSIST_PARENT_CHAIN"), "Defect");
    }

    /// 负面对照②：帧投影无球员 → 末节体能判证据不足（不得编造结论）。
    #[test]
    fn late_game_stamina_is_insufficient_without_player_projection() {
        let ticks = vec![tick_with(vec![], vec![], 4)];
        let ev = collect_individual_evidence(&ticks);
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(
            verdict_of(&out, "LATE_GAME_STAMINA"),
            "InsufficientEvidence"
        );
    }

    /// 负面对照②b：流里既无接球事实也无进球 → 助攻父链同样证据不足。
    #[test]
    fn assist_parent_chain_is_insufficient_without_pass_facts() {
        let ticks = vec![tick_with(vec![], vec![], 4)];
        let ev = collect_individual_evidence(&ticks);
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(
            verdict_of(&out, "ASSIST_PARENT_CHAIN"),
            "InsufficientEvidence"
        );
    }

    /// 负面对照③：末节体能跌破归零线 → 报缺陷（有证据时的真实判定）。
    #[test]
    fn late_game_stamina_defect_when_exhausted() {
        let players = vec![on_court_player("H_03", 0.05)];
        let ticks = vec![tick_with(vec![], players, 4)];
        let ev = collect_individual_evidence(&ticks);
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(verdict_of(&out, "LATE_GAME_STAMINA"), "Defect");
    }

    /// 负面对照④：使用率全部压在一人身上 → 报缺陷。
    #[test]
    fn usage_concentration_defect_when_one_player_owns_everything() {
        let mut usage: Map<(String, String), f32> = Map::new();
        *usage
            .entry(("home".to_string(), "H_01".to_string()))
            .or_default() += 60.0;
        *usage
            .entry(("away".to_string(), "A_01".to_string()))
            .or_default() += 60.0;
        let ev = IndividualEvidence {
            usage,
            usage_evidence_complete: true,
            total_made: 60,
            made_via_pass_reception: 0,
            pass_receptions: 0,
            responsibility_changes: 0,
            help_evidence_present: false,
            help_assignments: 0,
            help_assignments_restored: 0,
            late_period_min_stamina: None,
            late_period_players: 0,
            stamina_fact_present: false,
        };
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(verdict_of(&out, "USAGE_CONCENTRATION"), "Defect");
    }

    /// 负面对照⑤：责任转移从不恢复 → 报缺陷。
    #[test]
    fn matchup_responsibility_defect_when_help_never_restores() {
        let ev = IndividualEvidence {
            usage: Map::new(),
            usage_evidence_complete: true,
            total_made: 0,
            made_via_pass_reception: 0,
            pass_receptions: 0,
            responsibility_changes: 9,
            help_evidence_present: true,
            help_assignments: 9,
            help_assignments_restored: 0,
            late_period_min_stamina: None,
            late_period_players: 0,
            stamina_fact_present: false,
        };
        let mut out = Vec::new();
        evaluate_individual_criteria(&mut out, 0, ev);
        assert_eq!(verdict_of(&out, "MATCHUP_RESPONSIBILITY"), "Defect");
    }
}
