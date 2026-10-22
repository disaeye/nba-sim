//! 基线指纹：行为中性改动的对照工件。
//!
//! 指纹覆盖箱体、出手分区计数、回合结束原因分布与事实流构成。同种子两次
//! 运行必须逐位相同；记录过的基线必须与当前行为逐字段一致，差异按字段
//! 路径报告，便于定位是哪一个量先动。
//!
//! 记录基线：`cargo test --test baseline_fingerprint -- --ignored record_baseline_fingerprint`
//! 工件路径：`.work/baseline/fingerprint.json`（已被 `.gitignore` 忽略）

use std::collections::BTreeMap;
use std::path::PathBuf;

use nba_sim_core::action::Fact;
use nba_sim_core::params::{workspace_calibration_dir, workspace_root};
use nba_sim_core::{run_game, GameReport};
use serde_json::{json, Map, Value};

const BASELINE_SEEDS: u64 = 16;
const BASELINE_PATH: &str = ".work/baseline/fingerprint.json";

fn baseline_path() -> PathBuf {
    workspace_root().join(BASELINE_PATH)
}

fn fact_kind(fact: &Fact) -> &'static str {
    match fact {
        Fact::PossessionStarted { .. } => "possession_started",
        Fact::Pass { .. } => "pass",
        Fact::Screen { .. } => "screen",
        Fact::Drive { .. } => "drive",
        Fact::PostUp { .. } => "post_up",
        Fact::Shot { .. } => "shot",
        Fact::Foul { .. } => "foul",
        Fact::FreeThrow { .. } => "free_throw",
        Fact::Rebound { .. } => "rebound",
        Fact::Turnover { .. } => "turnover",
        Fact::Substitution { .. } => "substitution",
        Fact::Timeout { .. } => "timeout",
        Fact::PeriodEnd { .. } => "period_end",
        Fact::PlayActivated { .. } => "play_activated",
        Fact::PlayWindowEnded { .. } => "play_window_ended",
        Fact::GameEnd => "game_end",
    }
}

/// 单名球员的一行箱体，按固定次序排列；秒数取整到毫秒，避免浮点格式漂移。
fn player_row(line: &nba_sim_core::PlayerLine) -> Vec<i64> {
    vec![
        (line.seconds * 1000.0).round() as i64,
        line.points as i64,
        line.fgm as i64,
        line.fga as i64,
        line.tpm as i64,
        line.tpa as i64,
        line.ftm as i64,
        line.fta as i64,
        line.oreb as i64,
        line.dreb as i64,
        line.assists as i64,
        line.steals as i64,
        line.blocks as i64,
        line.turnovers as i64,
        line.fouls as i64,
        line.plus_minus as i64,
    ]
}

fn fingerprint(report: &GameReport) -> Value {
    let mut end_reasons: BTreeMap<&'static str, u32> = BTreeMap::new();
    for record in &report.possessions {
        *end_reasons.entry(record.ended_by).or_default() += 1;
    }
    let mut event_kinds: BTreeMap<&'static str, u32> = BTreeMap::new();
    for event in &report.events {
        *event_kinds.entry(fact_kind(&event.fact)).or_default() += 1;
    }
    json!({
        "seed": report.seed,
        "score": [report.home_score, report.away_score],
        "ticks": report.ticks,
        "possessions": report.possessions.len(),
        "zone_counts": report.shot_zone_counts,
        "zone_makes": report.shot_zone_makes,
        "creation_counts": report.creation_counts,
        "end_reasons": end_reasons,
        "event_kinds": event_kinds,
        "box": {
            "home": report.box_score.home.iter().map(player_row).collect::<Vec<_>>(),
            "away": report.box_score.away.iter().map(player_row).collect::<Vec<_>>(),
        },
    })
}

/// FNV-1a 64：不依赖 `DefaultHasher`，后者不保证跨版本稳定。
fn digest(value: &Value) -> String {
    let canonical = serde_json::to_string(value).expect("fingerprint serializes");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in canonical.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:#018x}")
}

fn collect() -> Map<String, Value> {
    let calibration = workspace_calibration_dir();
    let mut entries = Map::new();
    for seed in 1..=BASELINE_SEEDS {
        let report = run_game(seed, &calibration);
        let fields = fingerprint(&report);
        entries.insert(
            seed.to_string(),
            json!({ "digest": digest(&fields), "fields": fields }),
        );
    }
    entries
}

/// 按字段路径报告第一处差异；两侧结构不同时报告形状差异。
fn first_difference(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(left), Value::Object(right)) => {
            for (key, left_value) in left {
                match right.get(key) {
                    Some(right_value) => {
                        if let Some(found) =
                            first_difference(left_value, right_value, &format!("{path}.{key}"))
                        {
                            return Some(found);
                        }
                    }
                    None => return Some(format!("{path}.{key}: 基线有该字段，当前结果没有")),
                }
            }
            for key in right.keys() {
                if !left.contains_key(key) {
                    return Some(format!("{path}.{key}: 当前结果多出该字段，基线没有"));
                }
            }
            None
        }
        (Value::Array(left), Value::Array(right)) => {
            if left.len() != right.len() {
                return Some(format!(
                    "{path}: 长度不同，基线 {} 项，当前 {} 项",
                    left.len(),
                    right.len()
                ));
            }
            for (index, (left_value, right_value)) in left.iter().zip(right.iter()).enumerate() {
                if let Some(found) =
                    first_difference(left_value, right_value, &format!("{path}[{index}]"))
                {
                    return Some(found);
                }
            }
            None
        }
        _ if expected == actual => None,
        _ => Some(format!("{path}: 基线 {expected}，当前 {actual}")),
    }
}

#[test]
fn same_seed_produces_identical_fingerprint() {
    let calibration = workspace_calibration_dir();
    for seed in [1u64, 7] {
        let first = digest(&fingerprint(&run_game(seed, &calibration)));
        let second = digest(&fingerprint(&run_game(seed, &calibration)));
        assert_eq!(first, second, "seed {seed} 两次运行的指纹不一致");
    }
}

#[test]
fn baseline_fingerprint_matches_recorded_artifact() {
    let path = baseline_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "基线工件 {} 不可读（{error}）；先运行 \
             `cargo test --test baseline_fingerprint -- --ignored record_baseline_fingerprint`",
            path.display()
        )
    });
    let artifact: Value = serde_json::from_str(&text).expect("基线工件必须是合法 JSON");
    let recorded = artifact
        .get("entries")
        .and_then(Value::as_object)
        .expect("基线工件必须含 entries 对象");

    let current = collect();
    let mut failures = Vec::new();
    for (seed, entry) in &current {
        let Some(baseline_entry) = recorded.get(seed) else {
            failures.push(format!("seed {seed}: 基线工件没有这个种子"));
            continue;
        };
        let expected_fields = baseline_entry
            .get("fields")
            .expect("基线条目必须含 fields");
        let actual_fields = entry.get("fields").expect("当前条目必须含 fields");
        if let Some(difference) = first_difference(expected_fields, actual_fields, "fields") {
            failures.push(format!("seed {seed}: {difference}"));
        }
        let expected_digest = baseline_entry
            .get("digest")
            .and_then(Value::as_str)
            .expect("基线条目必须含 digest");
        let actual_digest = entry
            .get("digest")
            .and_then(Value::as_str)
            .expect("当前条目必须含 digest");
        if expected_digest != actual_digest && !failures.iter().any(|line| line.contains(&format!("seed {seed}:"))) {
            failures.push(format!(
                "seed {seed}: 摘要不同，基线 {expected_digest}，当前 {actual_digest}"
            ));
        }
    }
    for seed in recorded.keys() {
        if !current.contains_key(seed) {
            failures.push(format!("seed {seed}: 基线工件多出这个种子"));
        }
    }

    assert!(
        failures.is_empty(),
        "指纹与基线工件不一致（共 {} 处）：\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "记录基线时手动运行；常规测试只做比对"]
fn record_baseline_fingerprint() {
    let path = baseline_path();
    let entries = collect();
    let artifact = json!({
        "seeds": BASELINE_SEEDS,
        "calibration": "data/calibration",
        "entries": entries,
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("基线工件目录必须可创建");
    }
    let text = serde_json::to_string_pretty(&artifact).expect("基线工件序列化");
    std::fs::write(&path, format!("{text}\n")).expect("基线工件必须可写入");
    println!("已写入基线工件 {}", path.display());
}
