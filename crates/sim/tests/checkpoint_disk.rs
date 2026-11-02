//! 磁盘检查点：写入、重放恢复与指纹校验。
//!
//! 世界状态不直接序列化；检查点用「种子 + tick + 计数指纹」描述，
//! 恢复时按同一条路径重放并在指纹处停下。分叉的检查点必须就地报错。

use std::path::PathBuf;

use nba_domain::rules::GameRules;
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::{MatchSetup, SessionService, SessionState};

mod common;

fn checkpoint_path(name: &str) -> PathBuf {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".work")
        .join("checkpoints");
    std::fs::create_dir_all(&dir).expect("checkpoint dir must be creatable");
    dir.join(name)
}

fn params() -> Params {
    Params::load(&workspace_calibration_dir()).expect("calibration tables load")
}

fn configured(seed: u64) -> SessionService {
    let mut service = SessionService::new(seed);
    service.setup(params(), seed).expect("match setup");
    service.start().expect("match starts");
    service
}

fn finish(service: &mut SessionService) -> (u32, u32, usize) {
    while service.state() != SessionState::Finished {
        service.fast_forward(60.0).expect("session advances");
    }
    let report = service.report().expect("completed report");
    (report.home_score, report.away_score, report.events.len())
}

#[test]
fn disk_checkpoint_resumes_the_same_game() {
    let seed = 211;
    // 参照：一次跑完的比赛。
    let mut reference = configured(seed);
    let expected = finish(&mut reference);

    // 检查点：跑到一半写盘，再从磁盘恢复并跑完。
    let mut partial = configured(seed);
    partial.fast_forward(45.0).expect("first half advances");
    let path = checkpoint_path("resume.json");
    let descriptor = partial.save_descriptor(&path).expect("checkpoint writes");
    assert!(descriptor.tick > 0, "checkpoint must be taken mid-game");

    let mut restored = SessionService::new(seed);
    let setup = MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0));
    let reloaded = restored
        .restore_from_descriptor(&path, setup, params().into())
        .expect("checkpoint restores");
    assert_eq!(reloaded.tick, descriptor.tick);
    assert_eq!(reloaded.latest_event_id, descriptor.latest_event_id);
    let actual = finish(&mut restored);
    assert_eq!(
        actual, expected,
        "从检查点恢复的比赛必须与一次跑完的比赛完全一致"
    );
}

#[test]
fn diverged_checkpoint_is_rejected() {
    let seed = 223;
    let mut partial = configured(seed);
    partial.fast_forward(30.0).expect("advances");
    let path = checkpoint_path("diverged.json");
    let descriptor = partial.save_descriptor(&path).expect("checkpoint writes");

    // 篡改事件游标：重放不可能复现这个值，必须报错而不是继续跑。
    let mut tampered = descriptor.clone();
    tampered.latest_event_id += 7;
    let text = serde_json::to_string_pretty(&tampered).expect("serialize");
    std::fs::write(&path, text).expect("write tampered checkpoint");

    let mut restored = SessionService::new(seed);
    let setup = MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0));
    let error = restored
        .restore_from_descriptor(&path, setup, params().into())
        .expect_err("diverged checkpoint must be rejected");
    assert!(
        error.contains("diverged"),
        "错误必须指出重放分叉：{error}"
    );
}

#[test]
fn unsupported_checkpoint_version_is_rejected() {
    let seed = 227;
    let mut partial = configured(seed);
    partial.fast_forward(20.0).expect("advances");
    let path = checkpoint_path("version.json");
    let descriptor = partial.save_descriptor(&path).expect("checkpoint writes");

    let mut future = descriptor;
    future.version = 2;
    let text = serde_json::to_string_pretty(&future).expect("serialize");
    std::fs::write(&path, text).expect("write future checkpoint");

    let mut restored = SessionService::new(seed);
    let setup = MatchSetup::builtin(GameRules::default().shortened(120.0, 60.0));
    let error = restored
        .restore_from_descriptor(&path, setup, params().into())
        .expect_err("unknown version must be rejected");
    assert!(
        error.contains("unsupported checkpoint version"),
        "错误必须指出不支持的版本：{error}"
    );
}
