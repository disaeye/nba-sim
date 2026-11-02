//! 按回合回放：帧流确定，且只包含目标回合。

use std::sync::OnceLock;

use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{replay_possession, run_game, GameReport};

/// 三个测试都用种子 7：完整比赛报告只算一次；每次回放仍然
/// 是从头逐 tick 重算的真实路径，不做任何缓存。
fn seed_seven() -> &'static GameReport {
    static REPORT: OnceLock<GameReport> = OnceLock::new();
    REPORT.get_or_init(|| run_game(7, &workspace_calibration_dir()))
}

#[test]
fn replaying_a_possession_twice_yields_identical_frames() {
    let report = seed_seven();
    let target = report
        .possessions
        .iter()
        .find(|record| record.points > 0)
        .expect("a game must contain a scoring possession")
        .id;

    // 两次回放并发执行，各自从头重放目标回合。
    let (first, second) = std::thread::scope(|scope| {
        let left = scope
            .spawn(|| replay_possession(7, &workspace_calibration_dir(), target).expect("replay"));
        let right = scope
            .spawn(|| replay_possession(7, &workspace_calibration_dir(), target).expect("replay"));
        (
            left.join().expect("replay thread must not panic"),
            right.join().expect("replay thread must not panic"),
        )
    });

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
    let report = seed_seven();
    let mut scoring = report.possessions.iter().filter(|record| record.points > 0);
    let first_id = scoring.next().expect("first scoring possession").id;
    let second_id = scoring.next().expect("second scoring possession").id;

    let (first, second) = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            replay_possession(7, &workspace_calibration_dir(), first_id).expect("replay")
        });
        let right = scope.spawn(|| {
            replay_possession(7, &workspace_calibration_dir(), second_id).expect("replay")
        });
        (
            left.join().expect("replay thread must not panic"),
            right.join().expect("replay thread must not panic"),
        )
    });
    assert_ne!(first.ndjson(), second.ndjson(), "不同回合的帧流不应相同");
}

#[test]
fn replay_rejects_an_unknown_possession() {
    let error =
        replay_possession(7, &workspace_calibration_dir(), u64::MAX).expect_err("未知回合必须报错");
    assert!(
        error.contains("no possession"),
        "错误信息应指出缺失的回合：{error}"
    );
}
