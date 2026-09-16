//! 防守方案必须改变比赛过程（round-6 审计：`DefensiveTactic` 零影响）。
//!
//! ## 缺陷
//!
//! `home_defensive_tactic` / `away_defensive_tactic` 在全仓库只有 6 处引用 =
//! 2 处字段声明 + 2 处赋值 + 2 处 `name_zh()`（生成**展示字符串**）。物理层与
//! 决策管线从不消费它。独立复现（6 种方案各跑一场全场）：
//!
//! ```text
//! def_man_conservative   ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! def_man_pressure       ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! def_switch_heavy       ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! def_drop_coverage      ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! def_hedge_recover      ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! def_zone_23            ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
//! ```
//!
//! **逐字节相同，包括 2-3 联防**——即 `gap.md §21` 第 2 条标准
//! （「它会打篮球，而不是播放战术动画」）不成立。
//!
//! ## 判定口径
//!
//! 不是"对象的哈希不同"（展示字符串也会让哈希不同——见
//! `impact_assessment.md §30.4` 记录的错误中间结论），而是：
//! **统计过程量必须出现差异**。本测试对每场比赛聚合位置类指标，断言
//! 不同方案之间的差异既有统计意义、又朝 schema 声明的方向。
//!
//! 同时断言**不是任意乱动**：`def_zone_23` 必须比 `def_man_pressure`
//! 更收缩（防守人平均更靠近篮筐），否则"联防"只是换了个名字。

use nba_domain::GameRules;
use nba_engine::MatchEngine;
use nba_engine::MatchSetup;

/// 一场比赛聚合出的防守侧过程量。
#[derive(Debug, Clone, Copy)]
struct DefenseProfile {
    /// 防守人（非持球方）平均距自家篮筐距离（ft）。越小 = 越收缩。
    mean_defender_dist_to_hoop: f32,
    /// 防守人平均离对位人的距离代理：全体防守人两两最小距离均值（ft）。
    /// 越大 = 越分散（联防/收缩通常更分散于近筐区）。
    mean_defender_spacing: f32,
    /// 失误（由防守造成）总数。
    turnovers: u32,
    /// 总得分。
    score: u32,
}

fn profile(scheme: &str, seed: u64) -> DefenseProfile {
    let rules = GameRules::default();
    let mut setup = MatchSetup::builtin(rules.clone());
    setup.home_lineup.defense_tactic = scheme.to_string();
    setup.away_lineup.defense_tactic = scheme.to_string();
    let mut engine = MatchEngine::with_setup(setup, seed);
    // 防守方案是否**因果**是结构性质（方案不同 → 几何/结果不同），
    // 不需要整场；`1q` 已足够证伪「零影响」，且快约 4.5 倍。
    engine.set_scope("1q").expect("1q scope is valid");

    let mut sum_dist = 0.0f64;
    let mut sum_spacing = 0.0f64;
    let mut samples = 0u64;
    let mut ticks = 0usize;

    while !engine.is_finished() && ticks < 300_000 {
        let tick = engine.step();
        let offenders = tick.frame.possession_team.clone();
        // 仅采样活球阶段：死球/发球/罚球的站位由程序决定，不反映防守体系。
        let live = tick.frame.game_flow == "LiveBall";
        if live {
            let (hoop_x, hoop_y) = {
                let c = &rules.court;
                // 防守方保护的是**进攻方正在攻击的那个篮筐**。
                // 引擎口径：`attacking_right = (possession == Possession::Home)`
                // （见 match_engine.rs 的 `is_backcourt` 调用点）。
                // 因此 home 进攻时攻右篮，away 进攻时攻左篮。
                if offenders == "home" {
                    (c.hoop_right_x_ft, c.hoop_y_ft)
                } else {
                    (c.hoop_left_x_ft, c.hoop_y_ft)
                }
            };
            let defenders: Vec<(f32, f32)> = tick
                .frame
                .players
                .iter()
                .filter(|p| p.on_court && p.team != offenders)
                .map(|p| (p.x * rules.court.width_ft, p.y * rules.court.height_ft))
                .collect();
            if !defenders.is_empty() {
                let mut d = 0.0f64;
                for (x, y) in &defenders {
                    d += (((x - hoop_x).powi(2) + (y - hoop_y).powi(2)) as f64).sqrt();
                }
                sum_dist += d / defenders.len() as f64;
                // 两两最小距离（分散度代理）
                let mut min_pair = f64::MAX;
                for i in 0..defenders.len() {
                    for j in (i + 1)..defenders.len() {
                        let dx = (defenders[i].0 - defenders[j].0) as f64;
                        let dy = (defenders[i].1 - defenders[j].1) as f64;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist < min_pair {
                            min_pair = dist;
                        }
                    }
                }
                if min_pair != f64::MAX {
                    sum_spacing += min_pair;
                }
                samples += 1;
            }
        }
        ticks += 1;
    }

    let snap = engine.engine_snapshot();
    DefenseProfile {
        mean_defender_dist_to_hoop: if samples > 0 {
            (sum_dist / samples as f64) as f32
        } else {
            0.0
        },
        mean_defender_spacing: if samples > 0 {
            (sum_spacing / samples as f64) as f32
        } else {
            0.0
        },
        turnovers: snap.box_score.turnovers,
        score: snap.game.home_score + snap.game.away_score,
    }
}

#[test]
fn defensive_scheme_changes_defensive_geometry() {
    let seeds: [u64; 4] = [42, 1, 7, 100];
    let schemes = ["def_man_conservative", "def_zone_23", "def_drop_coverage"];

    let mut by_scheme: Vec<(&str, Vec<DefenseProfile>)> = Vec::new();
    for s in schemes {
        let mut v = Vec::new();
        for seed in seeds {
            v.push(profile(s, seed));
        }
        by_scheme.push((s, v));
    }

    for (s, v) in &by_scheme {
        let mean = v.iter().map(|p| p.mean_defender_dist_to_hoop).sum::<f32>() / v.len() as f32;
        let sp = v.iter().map(|p| p.mean_defender_spacing).sum::<f32>() / v.len() as f32;
        eprintln!(
            "{s:<22} mean_def_dist_to_hoop={mean:6.2}ft  mean_min_spacing={sp:5.2}ft  \
             TO={:>3} score={:>3}",
            v.iter().map(|p| p.turnovers).sum::<u32>(),
            v.iter().map(|p| p.score).sum::<u32>(),
        );
    }

    // 门 1：不同方案必须产生不同的防守几何（否则方案是装饰）。
    let man = by_scheme
        .iter()
        .find(|(s, _)| *s == "def_man_conservative")
        .map(|(_, v)| v.iter().map(|p| p.mean_defender_dist_to_hoop).sum::<f32>() / v.len() as f32)
        .unwrap();
    let zone = by_scheme
        .iter()
        .find(|(s, _)| *s == "def_zone_23")
        .map(|(_, v)| v.iter().map(|p| p.mean_defender_dist_to_hoop).sum::<f32>() / v.len() as f32)
        .unwrap();
    let drop = by_scheme
        .iter()
        .find(|(s, _)| *s == "def_drop_coverage")
        .map(|(_, v)| v.iter().map(|p| p.mean_defender_dist_to_hoop).sum::<f32>() / v.len() as f32)
        .unwrap();

    let spread = (man - zone).abs().max((man - drop).abs());
    assert!(
        spread > 0.25,
        "defensive scheme must change defensive geometry: \
         man={man:.3} zone={zone:.3} drop={drop:.3} (max spread {spread:.3} ft). \
         A scheme that only renames a display string is a decorative parameter \
         (round-6 audit: all 6 schemes produced byte-identical games)"
    );

    // 门 2：方向必须符合 schema 语义——联防比盯人更收缩。
    assert!(
        zone < man,
        "2-3 zone must sit closer to the basket than man-to-man: \
         zone={zone:.3}ft vs man={man:.3}ft"
    );

    // 门 3：沉退防守也必须比盯人更收缩（drop 的本义）。
    assert!(
        drop < man,
        "drop coverage must sit closer to the basket than man-to-man: \
         drop={drop:.3}ft vs man={man:.3}ft"
    );
}

#[test]
fn defensive_scheme_changes_game_outcome_distribution() {
    // 更强的门：过程改变必须传导到结果量（失误/得分分布），
    // 否则防守只是"站位好看"。用 4 seed 聚合，断言不是逐字节相同。
    let seeds: [u64; 4] = [42, 1, 7, 100];
    let mut press_total = 0u32;
    let mut zone_total = 0u32;
    for seed in seeds {
        press_total += profile("def_man_pressure", seed).turnovers;
        zone_total += profile("def_zone_23", seed).turnovers;
    }
    assert_ne!(
        press_total, zone_total,
        "press and zone must not produce identical turnover totals \
         (both were {press_total}); defensive schemes must be causal, not decorative"
    );
}
