//! 按回合回放：帧流确定，且只包含目标回合。

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{replay_possession, run_game};

#[test]
fn replaying_a_possession_twice_yields_identical_frames() {
    let calibration = workspace_calibration_dir();
    let report = run_game(7, &calibration);
    let target = report
        .possessions
        .iter()
        .find(|record| record.points > 0)
        .expect("a game must contain a scoring possession")
        .id;

    let first = replay_possession(7, &calibration, target).expect("first replay");
    let second = replay_possession(7, &calibration, target).expect("second replay");

    assert!(!first.frames.is_empty(), "回放必须产出帧");
    assert_eq!(
        first.ndjson(),
        second.ndjson(),
        "同一回合两次回放必须逐字节相同"
    );
    assert!(
        first.frames.len() < report.ticks as usize / 10,
        "单回合帧数 {} 不应接近整场的 {} tick",
        first.frames.len(),
        report.ticks
    );
}

#[test]
fn different_possessions_produce_different_frame_streams() {
    let calibration = workspace_calibration_dir();
    let report = run_game(7, &calibration);
    let mut scoring = report.possessions.iter().filter(|record| record.points > 0);
    let first = scoring.next().expect("first scoring possession").id;
    let second = scoring.next().expect("second scoring possession").id;

    let first_frames = replay_possession(7, &calibration, first).expect("first replay");
    let second_frames = replay_possession(7, &calibration, second).expect("second replay");
    assert_ne!(
        first_frames.ndjson(),
        second_frames.ndjson(),
        "不同回合的帧流不应相同"
    );
}

#[test]
fn replay_rejects_an_unknown_possession() {
    let calibration = workspace_calibration_dir();
    let error = replay_possession(7, &calibration, u64::MAX).expect_err("未知回合必须报错");
    assert!(
        error.contains("no possession"),
        "错误信息应指出缺失的回合：{error}"
    );
}
