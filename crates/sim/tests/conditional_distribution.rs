use std::collections::BTreeMap;

use nba_domain::rules::GameRules;
use nba_sim_core::params::{workspace_calibration_dir, workspace_root};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

const ZONES: [&str; 4] = ["rim", "near", "mid", "three"];

/// 对抗强度分档：松、一般、紧。表里的对抗敏感度是连续量，
/// 校验必须落在分档后的格子上才能看出分层偏差。
fn contest_bucket(contest: f32) -> u8 {
    if contest < 0.2 {
        0
    } else if contest < 0.45 {
        1
    } else {
        2
    }
}

/// 命中率按「区域 × 对抗」分层校验：每一格里实测命中率要落在模型
/// 预测均值附近，偏差由二项抽样误差给出上界。
#[test]
fn conditional_make_rate_matches_the_model_in_every_zone_and_contest_bucket() {
    struct Bucket {
        count: u32,
        made: u32,
        sum: f64,
        fouled: u32,
        blocked: u32,
        reconstructed: f64,
    }
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let and_one = f64::from(params.policy.w("and_one_rate"));
    let mut buckets: BTreeMap<(u8, u8), Bucket> = BTreeMap::new();
    for seed in 1..=72u64 {
        let mut world = World::new(
            MatchSetup::builtin(GameRules::default()),
            params.clone(),
            seed,
        );
        while !world.finished {
            world.step(TICK_SECONDS);
        }
        for sample in &world.shot_samples {
            let entry = buckets
                .entry((sample.zone, contest_bucket(sample.contest)))
                .or_insert(Bucket {
                    count: 0,
                    made: 0,
                    sum: 0.0,
                    fouled: 0,
                    blocked: 0,
                    reconstructed: 0.0,
                });
            entry.count += 1;
            if sample.made {
                entry.made += 1;
            }
            if sample.fouled {
                entry.fouled += 1;
            }
            if sample.blocked {
                entry.blocked += 1;
            }
            let p_foul = f64::from(sample.p_foul);
            let p_block = f64::from(sample.p_block);
            entry.reconstructed += p_foul * and_one
                + (1.0 - p_foul) * (1.0 - p_block) * f64::from(sample.p_make_cond);
            entry.sum += f64::from(sample.p_make);
        }
    }
    let mut checked = 0;
    for ((zone, contest), bucket) in &buckets {
        if bucket.count < 100 {
            continue;
        }
        checked += 1;
        let count = f64::from(bucket.count);
        let observed = f64::from(bucket.made) / count;
        let predicted = bucket.sum / count;
        let error = (predicted * (1.0 - predicted) / count).sqrt();
        let band = 2.5 * error + 0.01;
        assert!(
            (observed - predicted).abs() <= band,
            "{} 对抗档 {contest}：{} 次出手实测 {observed:.3}，模型 {predicted:.3}（按三个概率重建 {:.3}），超出 ±{band:.3}（犯规 {}，封盖 {}）",
            ZONES[*zone as usize],
            bucket.count,
            bucket.reconstructed / count,
            bucket.fouled,
            bucket.blocked
        );
    }
    assert!(
        checked >= 8,
        "分层样本不足：只有 {checked} 个区域×对抗组合达到 100 次出手"
    );
}

/// 冻结的三张表必须覆盖全部区域与对抗项，否则分层校验会静默跳过格子。
#[test]
fn shot_make_table_covers_every_zone_and_contest_term() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    for zone in ZONES {
        assert!(
            params.shot_make.league_zone_make.contains_key(zone),
            "命中率表缺少区域 {zone}"
        );
        assert!(
            params.shot_make.skill_spread.contains_key(zone),
            "命中率表缺少区域技能跨度 {zone}"
        );
    }
    assert!(
        params.shot_make.contest_sensitivity > f32::default(),
        "命中率表缺少对抗敏感度"
    );
    assert!(
        params.shot_make.contest_center > f32::default(),
        "命中率表缺少对抗中心"
    );
    for (kind, sides) in &params.shot_make.cells {
        assert!(
            sides.contains_key("two") || sides.contains_key("three"),
            "{kind} 没有任何距离格子"
        );
        for (side, cells) in sides {
            assert!(!cells.is_empty(), "{kind}/{side} 的距离格子为空");
            assert!(
                cells.iter().any(|cell| cell.samples > 0),
                "{kind}/{side} 的格子样本量全为零"
            );
        }
    }
}

/// 第一阶段的三张表必须有冻结锁，且锁里包含全部三张表。
#[test]
fn the_first_stage_tables_are_locked() {
    let lock = workspace_root()
        .join("data")
        .join("calibration")
        .join("frozen.lock.json");
    let text = std::fs::read_to_string(&lock).expect("冻结锁必须存在");
    let value: serde_json::Value = serde_json::from_str(&text).expect("冻结锁是合法 JSON");
    let tables = value
        .get("tables")
        .and_then(|tables| tables.as_object())
        .expect("冻结锁必须有 tables");
    for name in ["shot_make", "rebound", "freethrow"] {
        let digest = tables
            .get(name)
            .and_then(|digest| digest.as_str())
            .unwrap_or_else(|| panic!("冻结锁缺少 {name}"));
        assert_eq!(digest.len(), 64, "{name} 的哈希长度不是 SHA-256");
    }
}
