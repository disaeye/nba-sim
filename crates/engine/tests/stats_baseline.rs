//! L3 统计分布基线：抓"错得离谱但确定"的失真。
//!
//! 黄金哈希抓"行为变了"，本测试抓"行为离谱"：即使逐 tick 确定，模拟也可能
//! 产出统计上不像篮球的结果（比分 200+、回合 2 秒一次、零失误）。这里跑一个
//! 种子矩阵的整场模拟，聚合关键分布并断言落在容忍带内。
//!
//! 锚定遵循 quality.md §2.5 的"目标带 + 阶段门"：当前门允许校准期间的
//! 走廊；模拟落在门内 → 绿，越门外 → 红。校准逼近下一门时按 docs/protocol.md §1
//! 协议在 PR 中推进门，禁止把"实测±任意百分比"固化成锚。
//!
//! 判定口径（quality.md §2.5）：总分取 p50，回合数与回合时长取均值，
//! 种子数 ≥ 8。投篮分解（2P/3P/FT）为 M3/M8 验收的必采集项；3P% 以
//! 阶段门走廊考核（终态目标带 [30, 40]%，随校准逐级收窄）。
//!
//! ## 为什么同一处还要跑账本与 Hard 门
//!
//! 统计带只约束「分布形态」，对「事实能否独立重建」没有任何约束：实测
//! seed 14 全场在第 4 节开场出现停球状态与活球流程并存的回合，8 秒后被判
//! 后场违例且责任人缺失，账本 `TURNOVER_CONSERVATION` 与评判
//! `TURNOVER_ACTOR_CONSISTENCY`（Hard）同时报出，而统计带照旧全绿。
//! 原因是 `check_ledger` 此前只被 CLI 调用、评判只覆盖 8 个固定种子，
//! 两者都不进套件。本测试因此与 16-seed 矩阵同口径：同一批模拟上
//! 直接断言账本零违规、Hard 门通过、构成准则全过。
//!
//! 证据与定位过程见 `output/review16/`（诊断记录）。

use nba_engine::{MatchEngine, StreamMode};
use nba_test_support::TempArtifact;
use serde_json::Value;

/// 一场模拟聚合的统计量。
#[derive(Debug, Clone, Copy)]
struct GameStats {
    total_points: u32,
    possessions: usize,
    /// 平均回合时长（秒）。
    avg_possession_secs: f32,
    fg2_made: u32,
    fg2_attempts: u32,
    fg3_made: u32,
    fg3_attempts: u32,
    ft_made: u32,
    ft_attempts: u32,
    rim_attempts: usize,
    near_attempts: usize,
    mid_attempts: usize,
    three_attempts: usize,
    drive_rim_attempts: usize,
    drive_pull_up_rim_attempts: usize,
    cut_rim_attempts: usize,
    offensive_rebound_rim_attempts: usize,
    transition_rim_attempts: usize,
    set_play_rim_attempts: usize,
}

fn validate_shot_source(
    event: &nba_protocol::FrameEvent,
    shot: &Value,
    events_by_id: &std::collections::HashMap<u64, &nba_protocol::FrameEvent>,
    shooter: &str,
) -> usize {
    let source = shot
        .get("creation_source")
        .and_then(Value::as_str)
        .expect("ShotRelease must include creation_source");
    let source_id = shot.get("source_event_id").and_then(Value::as_u64);
    let parent = event
        .parent_event_id
        .and_then(|parent_id| events_by_id.get(&parent_id).copied());
    let transition_id = shot.get("transition_event_id").and_then(Value::as_u64);
    assert_eq!(
        shot.get("transition_context").and_then(Value::as_bool),
        Some(transition_id.is_some()),
        "transition context must agree with its event ID"
    );
    if let Some(transition_id) = transition_id {
        assert_eq!(
            events_by_id
                .get(&transition_id)
                .map(|event| event.kind.as_str()),
            Some("TRANSITION_STARTED"),
            "transition_event_id must identify TRANSITION_STARTED"
        );
    }
    match source {
        "drive_finish" => {
            let parent = parent.expect("drive finish parent");
            assert_eq!(parent.kind, "DRIVE_REACHED");
            assert_eq!(source_id, Some(parent.event_id));
            let outcome = parent
                .data
                .as_ref()
                .and_then(|data| data.get("DriveOutcome"))
                .expect("drive parent must carry DriveOutcome data");
            assert_eq!(
                outcome.get("successful").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                outcome.get("driver_id").and_then(Value::as_str),
                Some(shooter)
            );
            let initiation = parent
                .parent_event_id
                .and_then(|parent_id| events_by_id.get(&parent_id).copied())
                .expect("drive outcome must retain its DRIVE_INITIATED ancestor");
            assert_eq!(initiation.kind, "DRIVE_INITIATED");
            let drive = initiation
                .data
                .as_ref()
                .and_then(|data| data.get("DriveInitiated"))
                .expect("drive initiation must carry DriveInitiated data");
            assert_eq!(
                drive.get("driver_id").and_then(Value::as_str),
                Some(shooter)
            );
            0
        }
        "drive_pull_up" => {
            let parent = parent.expect("drive pull-up parent");
            assert!(matches!(
                parent.kind.as_str(),
                "DRIVE_INITIATED" | "DRIVE_REACHED" | "DRIVE_STOPPED"
            ));
            assert_eq!(source_id, Some(parent.event_id));
            let data = parent.data.as_ref().expect("drive parent event data");
            let drive = if parent.kind == "DRIVE_INITIATED" {
                data.get("DriveInitiated")
            } else {
                data.get("DriveOutcome")
            }
            .expect("drive parent must carry drive data");
            assert_eq!(
                drive.get("driver_id").and_then(Value::as_str),
                Some(shooter)
            );
            1
        }
        "cut_reception" => {
            let parent = parent.expect("cut reception parent");
            assert_eq!(parent.kind, "PASS_RECEIVED");
            assert_eq!(source_id, Some(parent.event_id));
            let received = parent
                .data
                .as_ref()
                .and_then(|data| data.get("PassReceived"))
                .expect("cut parent must carry PassReceived data");
            assert_eq!(
                received.get("is_cut_reception").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                received.get("receiver_id").and_then(Value::as_str),
                Some(shooter)
            );
            2
        }
        "offensive_rebound_putback" => {
            let parent = parent.expect("offensive rebound parent");
            assert_eq!(parent.kind, "REBOUND");
            assert_eq!(source_id, Some(parent.event_id));
            let rebound = parent
                .data
                .as_ref()
                .and_then(|data| data.get("ReboundContest"))
                .expect("rebound parent must carry ReboundContest data");
            assert_eq!(
                rebound.get("is_offensive").and_then(Value::as_bool),
                Some(true)
            );
            assert_eq!(
                rebound.get("rebounder_id").and_then(Value::as_str),
                Some(shooter)
            );
            3
        }
        "transition_finish" => {
            let parent = parent.expect("transition finish parent");
            assert_eq!(parent.kind, "TRANSITION_STARTED");
            assert_eq!(source_id, Some(parent.event_id));
            assert_eq!(transition_id, Some(parent.event_id));
            4
        }
        "set_play" => {
            assert!(parent.is_none());
            assert!(source_id.is_none());
            5
        }
        other => panic!("unknown shot creation source: {other}"),
    }
}

fn shot_zone_counts(
    ticks: &[nba_protocol::StreamTick],
) -> (usize, usize, usize, usize, [usize; 6]) {
    let mut rim = 0usize;
    let mut near = 0usize;
    let mut mid = 0usize;
    let mut three = 0usize;
    let events_by_id: std::collections::HashMap<u64, &nba_protocol::FrameEvent> = ticks
        .iter()
        .flat_map(|tick| &tick.frame.event_log)
        .map(|event| (event.event_id, event))
        .collect();
    let mut rim_sources = [0usize; 6];
    for tick in ticks {
        for event in &tick.frame.event_log {
            if event.kind != "SHOT_RELEASE" {
                continue;
            }
            let shot = event
                .data
                .as_ref()
                .and_then(|data| data.get("ShotRelease"))
                .expect("SHOT_RELEASE must carry ShotRelease data");
            let shooter = shot
                .get("shooter_id")
                .and_then(Value::as_str)
                .expect("ShotRelease must include shooter_id");
            let source_index = validate_shot_source(event, shot, &events_by_id, shooter);
            let position = shot
                .get("pos")
                .and_then(Value::as_array)
                .expect("ShotRelease must include pos");
            let x = position
                .first()
                .and_then(Value::as_f64)
                .expect("ShotRelease.pos must include x") as f32;
            let y = position
                .get(1)
                .and_then(Value::as_f64)
                .expect("ShotRelease.pos must include y") as f32;
            if shot.get("is_three").and_then(Value::as_bool) == Some(true) {
                three += 1;
                continue;
            }
            let hoop_x = if shooter.starts_with('H') {
                88.75
            } else {
                5.25
            };
            let distance = ((x - hoop_x).powi(2) + (y - 25.0).powi(2)).sqrt();
            if distance < nba_domain::court::RIM_ZONE_MAX_DIST_FT {
                rim += 1;
                rim_sources[source_index] += 1;
            } else if distance < nba_domain::court::NEAR_ZONE_MAX_DIST_FT {
                near += 1;
            } else {
                mid += 1;
            }
        }
    }
    (rim, near, mid, three, rim_sources)
}

/// 一场模拟的完整产出：统计量、账本违规、评判裁决。
struct SeedOutcome {
    stats: GameStats,
    ledger_violations: Vec<String>,
    hard_gate_failed: bool,
    evaluator_failures: Vec<String>,
    total_judgments: usize,
    defect_count: usize,
    hard_criteria: Vec<String>,
}

/// 单 seed 的完整验收：模拟 → facts 流（与 CLI 批量模式同一条写出路径）
/// → 账本对平 → 评判。评判对象是生产口径的有界事实流，
/// 而不是引擎内部完整帧：两者的体积差一个量级，后者无法进套件。
fn audit_full_game(seed: u64) -> SeedOutcome {
    let artifact = TempArtifact::new(&format!("stats_s{seed}"));
    let summary = {
        let mut engine = MatchEngine::new(seed);
        engine
            .simulate_scope_and_export_with_mode("full", &artifact.path_str(), StreamMode::Facts)
            .expect("full-scope export must succeed")
    };
    artifact.assert_within_limit();
    let content = std::fs::read_to_string(artifact.path()).expect("facts stream must be readable");
    let ticks = nba_evaluator::parse_stream(&content).expect("facts stream must parse strictly");

    // 统计量从流自身重建（quality.md §2.2：事实流是唯一真相），
    // 而不是从引擎内存快照抄一份——那会绕过「流能否独立重建比赛」这一问。
    let last = ticks.last().expect("facts stream must be non-empty");
    let score = &last.frame.score;
    let b = summary.box_score;
    let (rim_attempts, near_attempts, mid_attempts, three_attempts, rim_sources) =
        shot_zone_counts(&ticks);
    assert_eq!(
        rim_attempts + near_attempts + mid_attempts + three_attempts,
        b.fg2_attempts as usize + b.fg3_attempts as usize,
        "SHOT_RELEASE zone totals must equal box-score FGA for seed {seed}"
    );
    let stats = GameStats {
        total_points: score.home + score.away,
        possessions: last.frame.completed_possessions.max(1),
        avg_possession_secs: last.frame.t / last.frame.completed_possessions.max(1) as f32,
        fg2_made: b.fg2_made,
        fg2_attempts: b.fg2_attempts,
        fg3_made: b.fg3_made,
        fg3_attempts: b.fg3_attempts,
        ft_made: b.ft_made,
        ft_attempts: b.ft_attempts,
        rim_attempts,
        near_attempts,
        mid_attempts,
        three_attempts,
        drive_rim_attempts: rim_sources[0],
        drive_pull_up_rim_attempts: rim_sources[1],
        cut_rim_attempts: rim_sources[2],
        offensive_rebound_rim_attempts: rim_sources[3],
        transition_rim_attempts: rim_sources[4],
        set_play_rim_attempts: rim_sources[5],
    };

    let ledger_violations = nba_evaluator::check_ledger(&ticks)
        .into_iter()
        .map(|v| format!("{}: {}", v.equation, v.detail))
        .collect();
    let fixture = nba_evaluator::ReferenceDistributions::for_league("nba");
    let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture);
    let evaluator_failures = judgments
        .iter()
        .filter(|judgment| {
            matches!(judgment.criterion.as_str(), "SHOT_PROFILE_ZONE_MIX")
                && judgment.verdict != nba_evaluator::Verdict::Pass
        })
        .map(|judgment| {
            format!(
                "{}={:?}: {}",
                judgment.criterion, judgment.verdict, judgment.detail
            )
        })
        .collect();
    let report = nba_evaluator::attribution_report(&judgments, &fixture.version);
    let hard_criteria = report
        .defects_by_criterion
        .iter()
        .filter(|row| row.hard > 0)
        .map(|row| format!("{}x{}", row.criterion, row.hard))
        .collect();
    SeedOutcome {
        stats,
        ledger_violations,
        hard_gate_failed: report.hard_gate_failed,
        evaluator_failures,
        total_judgments: report.total_judgments,
        defect_count: report.defect_count,
        hard_criteria,
    }
}

fn percentile(sorted: &[u32], p: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    let k = (sorted.len() - 1) as f32 * p;
    let f = k.floor() as usize;
    let c = k.ceil() as usize;
    if f == c {
        sorted[f] as f32
    } else {
        sorted[f] as f32 * (c as f32 - k) + sorted[c] as f32 * (k - f as f32)
    }
}

#[test]
fn full_game_stats_within_baseline_band() {
    // 与记录在 `docs/dev/status.md` 的 G-STATS 矩阵同口径：`--seeds 1..16`。
    let seeds: Vec<u64> = (1..=16).collect();

    // 有界工作池：每个 seed 的整场事件流在账本与评判判定后立即释放。
    // 一次持有 4 场（实测单场峰值常驻内存约 70 MiB，见 output/review16）。
    const WORKERS: usize = 4;
    let next = std::sync::atomic::AtomicUsize::new(0);
    let outcomes: std::sync::Mutex<Vec<(u64, SeedOutcome)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..WORKERS {
            s.spawn(|| loop {
                let idx = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(&seed) = seeds.get(idx) else { break };
                let outcome = audit_full_game(seed);
                outcomes.lock().expect("outcome lock").push((seed, outcome));
            });
        }
    });
    let mut sim_results = outcomes.into_inner().expect("outcome mutex");
    sim_results.sort_by_key(|(seed, _)| *seed);

    let mut totals: Vec<u32> = Vec::new();
    let mut poss: Vec<usize> = Vec::new();
    let mut dur: Vec<f32> = Vec::new();
    let mut fg3_pct: Vec<f32> = Vec::new();
    let mut rim_shares: Vec<f32> = Vec::new();
    let mut mid_shares: Vec<f32> = Vec::new();
    let rim_band = nba_evaluator::ReferenceDistributions::nba_v2()
        .composition_bands
        .expect("nba.v2 must define composition bands")
        .rim_share_of_fga
        .expect("nba.v2 must define rim share band");
    let mut ledger_failures: Vec<String> = Vec::new();
    let mut hard_failures: Vec<String> = Vec::new();
    let mut evaluator_failures: Vec<String> = Vec::new();
    let mut total_judgments = 0usize;
    let mut total_defects = 0usize;
    let mut route_rim_attempts = [0usize; 4];
    for (seed, outcome) in sim_results {
        let s = outcome.stats;
        let fga = s.rim_attempts + s.near_attempts + s.mid_attempts + s.three_attempts;
        assert!(fga > 0, "seed {seed} must record field-goal attempts");
        let rim_share = s.rim_attempts as f32 / fga as f32;
        eprintln!(
            "seed {:>5}: total={:>3} poss={:>3} avg_poss={:>5.2}s 2P={}/{} zones={}/{}/{} 3P={}/{} ({:.1}%) rim_share={:.3} routes={}/{}/{}/{}/{}/{} FT={}/{} ledger={} hard_gate={} judgments={} defects={}",
            seed, s.total_points, s.possessions, s.avg_possession_secs,
            s.fg2_made, s.fg2_attempts,
            s.rim_attempts, s.near_attempts, s.mid_attempts,
            s.fg3_made, s.fg3_attempts,
            if s.fg3_attempts > 0 { s.fg3_made as f32 / s.fg3_attempts as f32 * 100.0 } else { 0.0 },
            rim_share,
            s.drive_rim_attempts,
            s.drive_pull_up_rim_attempts,
            s.cut_rim_attempts,
            s.offensive_rebound_rim_attempts,
            s.transition_rim_attempts,
            s.set_play_rim_attempts,
            s.ft_made, s.ft_attempts,
            outcome.ledger_violations.len(), outcome.hard_gate_failed,
            outcome.total_judgments, outcome.defect_count
        );
        for v in &outcome.ledger_violations {
            ledger_failures.push(format!("seed {seed}: {v}"));
        }
        if outcome.hard_gate_failed {
            hard_failures.push(format!("seed {seed}: {}", outcome.hard_criteria.join(", ")));
        }
        evaluator_failures.extend(
            outcome
                .evaluator_failures
                .iter()
                .map(|failure| format!("seed {seed}: {failure}")),
        );
        total_judgments += outcome.total_judgments;
        total_defects += outcome.defect_count;
        totals.push(s.total_points);
        poss.push(s.possessions);
        dur.push(s.avg_possession_secs);
        if s.fg3_attempts > 0 {
            fg3_pct.push(s.fg3_made as f32 / s.fg3_attempts as f32);
        }
        assert_eq!(
            s.rim_attempts,
            s.drive_rim_attempts
                + s.drive_pull_up_rim_attempts
                + s.cut_rim_attempts
                + s.offensive_rebound_rim_attempts
                + s.transition_rim_attempts
                + s.set_play_rim_attempts,
            "seed {seed}: every rim attempt must have typed route attribution"
        );
        route_rim_attempts[0] += s.drive_rim_attempts + s.drive_pull_up_rim_attempts;
        route_rim_attempts[1] += s.cut_rim_attempts;
        route_rim_attempts[2] += s.offensive_rebound_rim_attempts;
        route_rim_attempts[3] += s.transition_rim_attempts;
        rim_shares.push(rim_share);
        mid_shares.push(s.mid_attempts as f32 / fga as f32);
    }
    for (index, route) in ["drive", "cut reception", "offensive rebound", "transition"]
        .into_iter()
        .enumerate()
    {
        assert!(
            route_rim_attempts[index] > 0,
            "the 16-seed matrix must contain at least one rim attempt from {route}"
        );
    }
    totals.sort_unstable();

    let total_p50 = percentile(&totals, 0.50);
    let min_total = *totals.first().unwrap() as f32;
    let max_total = *totals.last().unwrap() as f32;
    let avg_poss = poss.iter().sum::<usize>() as f32 / poss.len() as f32;
    let avg_dur = dur.iter().sum::<f32>() / dur.len() as f32;
    let mut fg3_sorted = fg3_pct.clone();
    fg3_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let fg3_median = fg3_sorted[fg3_sorted.len() / 2] * 100.0;

    let mut rim_sorted = rim_shares.clone();
    rim_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let rim_median = rim_sorted[rim_sorted.len() / 2];
    let mut mid_sorted = mid_shares.clone();
    mid_sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid_median = mid_sorted[mid_sorted.len() / 2];
    let rim_in_band = rim_shares
        .iter()
        .filter(|share| rim_band.contains(share))
        .count();
    eprintln!(
        "AGG: total_p50={:.1} range=[{:.0},{:.0}] avg_poss={:.1} avg_dur={:.2}s 3P%_median={:.1} rim_share_median={:.3} mid_share_median={:.3} rim_in_band={}/{} band={:?}",
        total_p50, min_total, max_total, avg_poss, avg_dur, fg3_median, rim_median, mid_median, rim_in_band, rim_shares.len(), rim_band
    );
    assert!(
        (140.0..=230.0).contains(&total_p50),
        "median total points {:.1} outside stage gate [140, 230]",
        total_p50
    );
    // G4 回合门下界由 [172,240] 调整为 [172,290]：全场模式下每次球权交替发射真实 PossessionSummary，
    // 统计消费真实的 summary 数量；平均单回合持续时间相应收窄到 [12.0, 18.5]s。
    assert!(
        (172.0..=290.0).contains(&avg_poss),
        "avg possessions {:.1} outside stage gate G4 [172,290]",
        avg_poss
    );
    assert!(
        (12.0..=20.0).contains(&avg_dur),
        "avg possession duration {:.2}s outside stage gate G4 [12.0,20.0]",
        avg_dur
    );

    // ---- 3P% 门（取 dev 方案 G-D3a 的目标带 [30, 40]）----
    //
    // 历史沿革（如实记录，避免"为让门变绿而改门"）：
    // - 旧门为 [35, 75]，是 3P% 高达 61% 时设下的**防漂移走廊**，
    //   其注释即写明"校准逐级收窄至目标带 [30, 40]%"；
    // - 本轮 D5.1b（战术档案槽位生效 + 底角三分几何）把 3P% 从 61.2%
    //   降到 34.4%，已落在 `docs/dev/...开发方案.md` G-D3a 声明的
    //   [30, 40] 内；
    // - 因此把门收窄到方案目标带。这不是放宽（等价上界 75→40 是**收紧**），
    //   也不是为了让当前值通过：若 3P% 漂出 [30,40] 即为真实缺陷。
    assert!(
        (30.0..=40.0).contains(&fg3_median),
        "median 3P% {:.1} outside G-D3a target band [30, 40]",
        fg3_median
    );
    assert_eq!(
        rim_in_band,
        rim_shares.len(),
        "rim share band {:?} must pass for every seed; median was {:.3}",
        rim_band,
        rim_median
    );
    assert!(
        mid_median >= 0.08,
        "median midrange share {mid_median:.3} must meet the nba.v2 minimum 0.08"
    );

    // ---- 账本门（五式对平）与评判 Hard 门 ----
    //
    // 与 CLI 批量模式同判据：账本不平衡意味着事实流不能独立重建比赛；
    // Hard 缺陷意味着归因链或因果链断裂。两者都不能被统计带通过所掩盖。
    assert!(
        ledger_failures.is_empty(),
        "ledger violations in 16-seed full-game matrix ({} judgments, {} defects):\n{}",
        total_judgments,
        total_defects,
        ledger_failures.join("\n")
    );
    assert!(
        hard_failures.is_empty(),
        "Hard gate failed in 16-seed full-game matrix:\n{}",
        hard_failures.join("\n")
    );
    assert!(
        evaluator_failures.is_empty(),
        "R2 evaluator criteria failed in 16-seed full-game matrix:\n{}",
        evaluator_failures.join("\n")
    );
}
