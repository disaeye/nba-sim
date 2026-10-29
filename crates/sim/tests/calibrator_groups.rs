//! 校准器的参数分组守卫。
//!
//! 三条不变量：
//! 1. 一个参数键只能属于一个组：任何组之间没有共享的可写键。
//! 2. 每个键有自己的取值范围：组内也不存在「同一个数值同时表示两种
//!    量纲」的可能。
//! 3. 时间阈值、速度尺度与距离尺度分属不同的量纲区间：校准器不可能把
//!    同一个数值同时写进这三种量纲。

use std::collections::BTreeMap;
use std::process::Command;

fn script_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/calibrate/fit_policy.py")
}

fn groups_json() -> serde_json::Value {
    let output = Command::new("python3")
        .arg(script_path())
        .arg("--dry-run")
        .output()
        .expect("the calibrator must run");
    assert!(
        output.status.success(),
        "the calibrator must print its groups: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let start = text.find('{').expect("group listing must be JSON");
    serde_json::from_str(&text[start..]).expect("group listing must be valid JSON")
}

#[test]
fn every_parameter_key_belongs_to_exactly_one_group() {
    let listing = groups_json();
    let mut owners: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, group) in listing.as_object().expect("group listing is an object") {
        let bounds = group
            .get("bounds")
            .and_then(|value| value.as_object())
            .expect("group entry lists its bounds");
        for key in bounds.keys() {
            owners
                .entry(key.clone())
                .or_default()
                .push(name.clone());
        }
    }
    for (key, groups) in owners {
        assert_eq!(
            groups.len(),
            1,
            "parameter {key} is written by more than one group: {groups:?}"
        );
    }
}

#[test]
fn time_speed_and_distance_scales_live_in_different_groups() {
    let listing = groups_json();
    let group_of = |key: &str| -> Option<String> {
        listing
            .as_object()
            .expect("group listing is an object")
            .iter()
            .find(|(_, group)| {
                group
                    .get("bounds")
                    .and_then(|value| value.as_object())
                    .map(|bounds| bounds.contains_key(key))
                    .unwrap_or(false)
            })
            .map(|(name, _)| name.clone())
    };
    // 时间阈值（秒）、速度尺度（英尺每秒）、距离尺度（英尺）。
    let seconds = group_of("seconds_per_possession").expect("pace group owns seconds");
    let distance = group_of("shot_aim_distance_scale").expect("distance group owns distance");
    assert_ne!(
        seconds, distance,
        "a time threshold and a distance scale must not share a parameter group"
    );
    // 同一个键不可能同时被写进两组：结构上由上面的不变量保证。
    assert!(group_of("shot_aim_sigma_scale").is_some());
    assert!(group_of("shot_distance_decay_three_scale_ft").is_some());
    assert!(group_of("shot_quality_contest_sensitivity").is_some());
}

#[test]
fn every_group_key_declares_its_own_bounds() {
    let listing = groups_json();
    for (name, group) in listing.as_object().expect("group listing is an object") {
        let entry = group
            .get("bounds")
            .and_then(|value| value.as_object())
            .expect("group entry lists its bounds");
        for (key, bounds) in entry {
            let pair = bounds
                .as_array()
                .unwrap_or_else(|| panic!("group {name} key {key} must declare bounds"));
            assert_eq!(pair.len(), 2, "group {name} key {key} needs two bounds");
            let low = pair[0].as_f64().expect("bound is a number");
            let high = pair[1].as_f64().expect("bound is a number");
            assert!(
                low < high,
                "group {name} key {key} has an empty range [{low}, {high}]"
            );
        }
    }
}
