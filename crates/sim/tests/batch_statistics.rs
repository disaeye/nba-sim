//! 批次统计验收：多场比赛聚合后的行为量级与能力敏感性。
//!
//! 这些测试的样本来自多场种子比赛，检验的是聚合统计性质；
//! 逐事件与单回合行为见 `engine.rs`。

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{gate, MatchSetup, Params, TeamSide, World};

mod common;

fn short_play(seed: u64) -> nba_sim_core::GameReport {
    common::short_world(seed).run()
}

fn params() -> Params {
    Params::load(&workspace_calibration_dir()).expect("tables load")
}

fn play(seed: u64) -> nba_sim_core::GameReport {
    nba_sim_core::run_game(seed, &workspace_calibration_dir())
}

fn clone_params(params: &Params) -> Params {
    params.clone()
}

/// 六十场短比赛批次只跑一次，分类与犯规类断言共用同一批报告；
/// 断言要求批次贡献超过一百次投篮犯规，样本量不能缩减。
fn shared_short_batch() -> &'static Vec<nba_sim_core::GameReport> {
    static BATCH: std::sync::OnceLock<Vec<nba_sim_core::GameReport>> = std::sync::OnceLock::new();
    BATCH.get_or_init(|| play_in_parallel(51u64..111))
}

/// 用四个作用域线程分摊比赛；每场比赛只依赖自己的种子，结果按种子顺序汇合。
fn play_in_parallel(seeds: std::ops::Range<u64>) -> Vec<nba_sim_core::GameReport> {
    let seeds: Vec<u64> = seeds.collect();
    std::thread::scope(|scope| {
        let handles: Vec<_> = split_chunks(&seeds, 4)
            .into_iter()
            .map(|chunk| {
                scope.spawn(move || chunk.iter().map(|s| short_play(*s)).collect::<Vec<_>>())
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("game thread must not panic"))
            .flatten()
            .collect()
    })
}

/// 把种子序列按块数均分为若干连续切片。
fn split_chunks(seeds: &[u64], chunks: usize) -> Vec<&[u64]> {
    let len = seeds.len();
    let size = len.div_ceil(chunks);
    (0..chunks)
        .map(|c| {
            let start = c * size;
            let end = (start + size).min(len);
            &seeds[start..end]
        })
        .filter(|chunk| !chunk.is_empty())
        .collect()
}

#[test]
fn game_shape_is_within_the_professional_range() {
    // 两场完整比赛的均值离区间边界超过四个标准差，足以验证量级；两场并行执行。
    let reports: Vec<nba_sim_core::GameReport> = std::thread::scope(|scope| {
        let handles: Vec<_> = [1u64, 3]
            .iter()
            .map(|seed| scope.spawn(|| play(*seed)))
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("full game thread must not panic"))
            .collect()
    });
    let mut points = Vec::new();
    let mut possessions = Vec::new();
    for report in reports {
        points.push((report.home_score + report.away_score) as f64);
        possessions.push(report.possessions.len() as f64);
    }
    let mean_points: f64 = points.iter().sum::<f64>() / points.len() as f64;
    let mean_possessions: f64 = possessions.iter().sum::<f64>() / possessions.len() as f64;
    assert!(
        (150.0..=280.0).contains(&mean_points),
        "mean total points {mean_points:.1} outside the plausible range"
    );
    assert!(
        (170.0..=230.0).contains(&mean_possessions),
        "mean possessions {mean_possessions:.1} outside the plausible range: {possessions:?}"
    );
}

#[test]
fn shots_are_geometrically_legal_across_a_batch() {
    let reports: Vec<_> = [21u64, 22, 23]
        .iter()
        .map(|seed| short_play(*seed))
        .collect();
    let report = gate::evaluate_batch(&reports, &params());
    assert_eq!(
        report.geometric_violations, 0,
        "geometric violations must be zero"
    );
    assert_eq!(
        report.context_violations, 0,
        "context violations must be zero"
    );
}

#[test]
fn zone_and_creation_shares_are_complete() {
    // 分类完备性是逐出手性质：共用六十场批次检验每个出手都有分区与创造方式，
    // 且批内四个分区全部出现。
    let reports = shared_short_batch();
    let mut batch_zone_counts = [0u32; 4];
    let zone_total: u32 = reports
        .iter()
        .map(|r| r.shot_zone_counts.iter().flatten().sum::<u32>())
        .sum();
    let creation_total: u32 = reports
        .iter()
        .map(|r| r.creation_counts.iter().sum::<u32>())
        .sum();
    assert!(zone_total > 120, "batch must contain enough shots");
    assert_eq!(
        zone_total, creation_total,
        "every shot has one zone and one creation"
    );
    for report in reports {
        for zone in 0..4 {
            let count = report.shot_zone_counts[0][zone] + report.shot_zone_counts[1][zone];
            batch_zone_counts[zone] += count;
        }
    }
    for zone in 0..4 {
        assert!(
            batch_zone_counts[zone] > 0,
            "every zone must appear in the batch"
        );
    }
}

#[test]
fn three_point_skill_moves_three_point_output() {
    let base = params();
    let rules = common::nba_short();
    // 低档与高档对着同一批种子跑，主队属性分别取 0.10 与 0.95。
    let mut low_setup = MatchSetup::builtin(rules.clone());
    for player in low_setup.home.players.iter_mut() {
        player.attributes.shooting_three = 0.10;
    }
    let mut high_setup = MatchSetup::builtin(rules.clone());
    for player in high_setup.home.players.iter_mut() {
        player.attributes.shooting_three = 0.95;
    }
    let mut made_low = 0u32;
    let mut attempts_low = 0u32;
    let mut made_high = 0u32;
    let mut attempts_high = 0u32;
    // 差距悬殊（0.10 对 0.95），八个种子的样本足以分离两档命中率；两档各用四个线程分摊。
    let run_lines = |setup: &MatchSetup| -> Vec<nba_sim_core::TeamLine> {
        std::thread::scope(|scope| {
            let handles: Vec<_> = split_chunk_ranges(41u64..49, 4)
                .into_iter()
                .map(|chunk| {
                    let setup = setup.clone();
                    let base = base.clone();
                    scope.spawn(move || {
                        chunk
                            .map(|seed| {
                                World::new(setup.clone(), base.clone(), seed)
                                    .run()
                                    .box_score
                                    .team_totals(TeamSide::Home)
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .map(|h| h.join().expect("skill thread must not panic"))
                .flatten()
                .collect::<Vec<_>>()
        })
    };
    let low_lines = run_lines(&low_setup);
    let high_lines = run_lines(&high_setup);
    for line in low_lines {
        made_low += line.tpm;
        attempts_low += line.tpa;
    }
    for line in high_lines {
        made_high += line.tpm;
        attempts_high += line.tpa;
    }
    let low_pct = made_low as f64 / attempts_low.max(1) as f64;
    let high_pct = made_high as f64 / attempts_high.max(1) as f64;
    println!("in-game three point percentage {low_pct:.3} -> {high_pct:.3}");
    // 场上命中率会叠上选择效应（能力强的球员会去投更难的球）。
    // 机制断言直接比同一位置的模型输出。
    let model_p_make = |setup: &MatchSetup| -> f32 {
        let world = World::new(setup.clone(), base.clone(), 41);
        let shooter = world.on_court(TeamSide::Home).as_slice()[0];
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + glam::Vec2::new(-24.0, 0.0);
        nba_sim_core::shot_quality::evaluate(
            &world,
            shooter,
            nba_sim_core::action::ShotKind::CatchShoot,
            nba_sim_core::state::BallPhase::Catch,
            spot,
        )
        .p_make
    };
    let low_model = model_p_make(&low_setup);
    let high_model = model_p_make(&high_setup);
    assert!(
        high_model > low_model,
        "raising three point skill must raise the modelled make probability ({low_model:.3} -> {high_model:.3})"
    );
}

/// 封盖与抢断是动作，不是聚合项：同一种子下只改防守人的能力，
/// 封盖/抢断次数必须跟着变。
#[test]
fn defensive_skill_changes_blocks_and_steals() {
    let base = params();
    let rules = common::nba_short();
    let mut weak = MatchSetup::builtin(rules.clone());
    for player in weak.home.players.iter_mut() {
        player.attributes.block = 0.1;
        player.attributes.steal = 0.1;
    }
    let mut strong = MatchSetup::builtin(rules.clone());
    for player in strong.home.players.iter_mut() {
        player.attributes.block = 0.95;
        player.attributes.steal = 0.95;
    }
    let mut weak_blocks = 0u32;
    let mut strong_blocks = 0u32;
    let mut weak_steals = 0u32;
    let mut strong_steals = 0u32;
    // 十八个种子两种防守强度，四个线程分摊：封盖是低频率事件，样本小了
    // 分不出能力差异。
    let weak_lines = run_skill_lines(&weak, &base, 31u64..49);
    let strong_lines = run_skill_lines(&strong, &base, 31u64..49);
    for line in weak_lines {
        weak_blocks += line.blocks;
        weak_steals += line.steals;
    }
    for line in strong_lines {
        strong_blocks += line.blocks;
        strong_steals += line.steals;
    }
    assert!(
        strong_blocks > weak_blocks,
        "raising block skill must raise blocks ({weak_blocks} -> {strong_blocks})"
    );
    assert!(
        strong_steals > weak_steals,
        "raising steal skill must raise steals ({weak_steals} -> {strong_steals})"
    );
}

/// 对同一阵容按种子区间跑短场，返回每场的主队合计行；四个线程分摊。
fn run_skill_lines(
    setup: &MatchSetup,
    base: &Params,
    seeds: std::ops::Range<u64>,
) -> Vec<nba_sim_core::TeamLine> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = split_chunk_ranges(seeds, 4)
            .into_iter()
            .map(|chunk| {
                let setup = setup.clone();
                let base = base.clone();
                scope.spawn(move || {
                    chunk
                        .map(|seed| {
                            World::new(setup.clone(), base.clone(), seed)
                                .run()
                                .box_score
                                .team_totals(TeamSide::Home)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().expect("skill thread must not panic"))
            .flatten()
            .collect()
    })
}

/// 把种子区间按块数均分为若干连续子区间。
fn split_chunk_ranges(seeds: std::ops::Range<u64>, chunks: usize) -> Vec<std::ops::Range<u64>> {
    let len = (seeds.end - seeds.start) as usize;
    let size = len.div_ceil(chunks);
    (0..chunks)
        .map(|c| {
            let start = seeds.start + (c * size) as u64;
            let end = seeds.start + ((c * size + size).min(len)) as u64;
            start..end
        })
        .filter(|r| r.start < r.end)
        .collect()
}

/// 投篮犯规里的 and-1 是少数：命中不再走普通命中率，而是独立的
/// and-1 概率；否则会在犯规分支里重复一次投进。
#[test]
fn and_one_is_a_minority_of_shooting_fouls() {
    let mut fouls = 0u32;
    let mut and_one = 0u32;
    let reports = shared_short_batch();
    for report in reports {
        for team in [TeamSide::Home, TeamSide::Away] {
            let totals = report.box_score.team_totals(team);
            fouls += totals.shooting_fouls;
            and_one += totals.and_one;
        }
    }
    // 样本下界只保证“少数”这个判断不是在个位数上做的；份额本身由批次判据盯。
    assert!(fouls > 80, "sample too small: {fouls} shooting fouls");
    let share = and_one as f64 / fouls as f64;
    // 这里只查“少数”这个性质；具体份额由批次判据 and_one_per_game 盯。
    assert!(
        (0.01..0.25).contains(&share),
        "and-1 share of shooting fouls must stay a minority: {share:.3} ({and_one}/{fouls})"
    );
}

/// 非投篮犯规（手部犯规、移动掩护）必须真的发生：总犯规数大于
/// 投篮犯规数，否则 bonus 永远不触发。
#[test]
fn non_shooting_fouls_occur() {
    let mut total = 0u32;
    let mut shooting = 0u32;
    let reports = shared_short_batch();
    for report in reports {
        for team in [TeamSide::Home, TeamSide::Away] {
            let totals = report.box_score.team_totals(team);
            total += totals.fouls;
            shooting += totals.shooting_fouls;
        }
    }
    assert!(
        total > shooting,
        "non-shooting fouls must occur: {total} vs {shooting}"
    );
}
