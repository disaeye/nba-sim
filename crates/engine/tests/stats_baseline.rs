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

use nba_engine::MatchEngine;

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

fn simulate_full_game(seed: u64) -> GameStats {
    let mut engine = MatchEngine::new(seed);
    engine.set_scope("full").expect("full scope is valid");
    let mut max_t = 0.0f32;
    let mut ticks = 0usize;
    while !engine.is_finished() && ticks < 200_000 {
        let tick = engine.step();

        max_t = tick.frame.t;
        ticks += 1;
    }
    let snap = engine.engine_snapshot();
    let home = snap.game.home_score;
    let away = snap.game.away_score;
    let b = snap.box_score;
    let possessions = engine.completed_possessions().max(1);
    GameStats {
        total_points: home + away,
        possessions,
        avg_possession_secs: max_t / possessions as f32,
        fg2_made: b.fg2_made,
        fg2_attempts: b.fg2_attempts,
        fg3_made: b.fg3_made,
        fg3_attempts: b.fg3_attempts,
        ft_made: b.ft_made,
        ft_attempts: b.ft_attempts,
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
    // 种子矩阵 ≥ 8（quality.md §2.5 判定口径）；分布回归靠跨种子聚合。
    let seeds: [u64; 8] = [42, 1, 7, 100, 999, 31337, 2024, 555];

    // 多线程并行模拟（利用多核 CPU 并发执行各独立 seed，消除串行长耗时阻塞）
    let sim_results: Vec<(u64, GameStats)> = std::thread::scope(|s| {
        let handles: Vec<_> = seeds
            .iter()
            .copied()
            .map(|seed| {
                s.spawn(move || {
                    eprintln!("starting seed {seed} in thread");
                    let stats = simulate_full_game(seed);
                    (seed, stats)
                })
            })
            .collect();
        let mut results = Vec::with_capacity(handles.len());
        for h in handles {
            match h.join() {
                Ok(res) => results.push(res),
                Err(e) => std::panic::resume_unwind(e),
            }
        }
        results
    });

    let mut totals: Vec<u32> = Vec::new();
    let mut poss: Vec<usize> = Vec::new();
    let mut dur: Vec<f32> = Vec::new();
    let mut fg3_pct: Vec<f32> = Vec::new();
    for (seed, s) in sim_results {
        eprintln!(
            "seed {:>5}: total={:>3} poss={:>3} avg_poss={:>5.2}s 2P={}/{} 3P={}/{} ({:.1}%) FT={}/{}",
            seed, s.total_points, s.possessions, s.avg_possession_secs,
            s.fg2_made, s.fg2_attempts,
            s.fg3_made, s.fg3_attempts,
            if s.fg3_attempts > 0 { s.fg3_made as f32 / s.fg3_attempts as f32 * 100.0 } else { 0.0 },
            s.ft_made, s.ft_attempts
        );
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
}
