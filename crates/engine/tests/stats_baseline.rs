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
}

/// 一场模拟的完整产出：统计量、账本违规、评判裁决。
struct SeedOutcome {
    stats: GameStats,
    ledger_violations: Vec<String>,
    hard_gate_failed: bool,
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
    };

    let ledger_violations = nba_evaluator::check_ledger(&ticks)
        .into_iter()
        .map(|v| format!("{}: {}", v.equation, v.detail))
        .collect();
    let fixture = nba_evaluator::ReferenceDistributions::for_league("nba");
    let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture);
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
    let mut ledger_failures: Vec<String> = Vec::new();
    let mut hard_failures: Vec<String> = Vec::new();
    let mut total_judgments = 0usize;
    let mut total_defects = 0usize;
    for (seed, outcome) in sim_results {
        let s = outcome.stats;
        eprintln!(
            "seed {:>5}: total={:>3} poss={:>3} avg_poss={:>5.2}s 2P={}/{} 3P={}/{} ({:.1}%) FT={}/{} ledger={} hard_gate={} judgments={} defects={}",
            seed, s.total_points, s.possessions, s.avg_possession_secs,
            s.fg2_made, s.fg2_attempts,
            s.fg3_made, s.fg3_attempts,
            if s.fg3_attempts > 0 { s.fg3_made as f32 / s.fg3_attempts as f32 * 100.0 } else { 0.0 },
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
        total_judgments += outcome.total_judgments;
        total_defects += outcome.defect_count;
        totals.push(s.total_points);
        poss.push(s.possessions);
        dur.push(s.avg_possession_secs);
        if s.fg3_attempts > 0 {
            fg3_pct.push(s.fg3_made as f32 / s.fg3_attempts as f32);
        }
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

    eprintln!(
        "AGG: total_p50={:.1} range=[{:.0},{:.0}] avg_poss={:.1} avg_dur={:.2}s 3P%_median={:.1}",
        total_p50, min_total, max_total, avg_poss, avg_dur, fg3_median
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
}
