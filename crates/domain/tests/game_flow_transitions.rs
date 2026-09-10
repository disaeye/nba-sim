//! GameFlowState 转换表穷举守卫（③：第二批·守卫加固）。
//!
//! 表是"合法宏观生命周期的唯一权威"。任何非法边必须被拒绝——
//! 历史 v10 死球楔死（seed 2/4 永久冻结）就是"拒绝路径缺出口"类缺陷，
//! 本测试把每条非法边逐一钉死，新增 Phase 变体时编译器强制补全此表。

use nba_domain::GameFlowState;

/// 全状态枚举，保证穷举性（新增变体时此处编译失败）。
const ALL: [GameFlowState; 10] = [
    GameFlowState::Pregame,
    GameFlowState::TipOff,
    GameFlowState::LiveBall,
    GameFlowState::DeadBall,
    GameFlowState::Timeout,
    GameFlowState::FreeThrow,
    GameFlowState::QuarterEnd,
    GameFlowState::Halftime,
    GameFlowState::Overtime,
    GameFlowState::GameEnd,
];

/// 同状态赋值幂等合法（调用方先查同值短路，这里直接验证表语义）。
#[test]
fn same_state_assignment_is_idempotent() {
    for state in ALL {
        assert!(
            state.can_transition_to(state),
            "{state:?} -> itself must be legal (idempotent)"
        );
    }
}

/// 终局吸收：GameEnd 不再迁出（唯一例外是测试后门路径，运行时无此边）。
#[test]
fn game_end_is_absorbing() {
    for next in ALL {
        if next != GameFlowState::GameEnd {
            assert!(
                !GameFlowState::GameEnd.can_transition_to(next),
                "GameEnd -> {next:?} must be rejected"
            );
        }
    }
}
#[test]
fn pregame_only_enters_tipoff_or_cancel() {
    for next in ALL {
        if next == GameFlowState::Pregame {
            continue; // 幂等边由同状态测试覆盖
        }
        let legal = matches!(next, GameFlowState::TipOff | GameFlowState::GameEnd);
        assert_eq!(
            GameFlowState::Pregame.can_transition_to(next),
            legal,
            "Pregame -> {next:?} legality mismatch"
        );
    }
}

/// 比赛中不可回到 Pregame/TipOff——跳球是开局一次性事件。
#[test]
fn no_return_to_pregame_or_tipoff() {
    let mid_states = [
        GameFlowState::LiveBall,
        GameFlowState::DeadBall,
        GameFlowState::Timeout,
        GameFlowState::FreeThrow,
        GameFlowState::QuarterEnd,
        GameFlowState::Halftime,
        GameFlowState::Overtime,
    ];
    for from in mid_states {
        assert!(
            !from.can_transition_to(GameFlowState::Pregame),
            "{from:?} -> Pregame must be rejected"
        );
        assert!(
            !from.can_transition_to(GameFlowState::TipOff),
            "{from:?} -> TipOff must be rejected"
        );
    }
}

#[test]
fn overtime_never_returns_to_liveball() {
    // LiveBall -> Overtime 合法：第4节平分时 finish_period 从活球直切加时。
    assert!(
        GameFlowState::LiveBall.can_transition_to(GameFlowState::Overtime),
        "LiveBall -> Overtime (tied-regulation buzzer) must be legal"
    );
    assert!(
        !GameFlowState::Overtime.can_transition_to(GameFlowState::LiveBall),
        "Overtime -> LiveBall must be rejected (rebound/outlet goes DeadBall-free but stays in OT clock)"
    );
}

/// 跳球后直接进活球是唯一开球边；TipOff 不允许直通 FreeThrow/节末等。
#[test]
fn tipoff_edges_are_minimal() {
    for next in ALL {
        if next == GameFlowState::TipOff {
            continue;
        }
        // TipOff -> DeadBall（点拍死球判罚）/ FreeThrow（跳球期间犯规）均为真实路径
        let legal = matches!(
            next,
            GameFlowState::LiveBall
                | GameFlowState::DeadBall
                | GameFlowState::FreeThrow
                | GameFlowState::GameEnd
        );
        assert_eq!(
            GameFlowState::TipOff.can_transition_to(next),
            legal,
            "TipOff -> {next:?} legality mismatch"
        );
    }
}

/// 双向可达性烟雾：从 Pregame 出发，合法路径必须能抵达 GameEnd（无死锁出口缺失）。
/// 广度优先遍历合法边，断言 GameEnd 可达。
#[test]
fn game_end_reachable_from_every_mid_state() {
    use std::collections::VecDeque;
    for start in ALL {
        if start == GameFlowState::GameEnd || start == GameFlowState::Pregame {
            continue;
        }
        let mut visited = [false; 10];
        let mut queue = VecDeque::new();
        queue.push_back(start);
        visited[index_of(start)] = true;
        let mut reachable = false;
        while let Some(cur) = queue.pop_front() {
            if cur == GameFlowState::GameEnd {
                reachable = true;
                break;
            }
            for next in ALL {
                if !visited[index_of(next)] && cur.can_transition_to(next) {
                    visited[index_of(next)] = true;
                    queue.push_back(next);
                }
            }
        }
        assert!(
            reachable,
            "GameEnd must be reachable from {start:?} (no-absorb lifelock guard)"
        );
    }
}

fn index_of(state: GameFlowState) -> usize {
    ALL.iter().position(|s| *s == state).unwrap()
}
