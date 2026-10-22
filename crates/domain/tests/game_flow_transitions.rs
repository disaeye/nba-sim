//! GameFlowState 转换表穷举守卫（③：第二批·守卫加固）与联赛规则语义守卫。
//!
//! 前半部分：转换表是"合法宏观生命周期的唯一权威"。任何非法边必须被拒绝。
//! 后半部分：进攻时钟重置、bonus 阈值矩阵与犯规计账是数据驱动的纯函数，
//! 可用同一份测试直接卡住区间——它们不依赖引擎状态，改错一个数就会红。

use nba_domain::event::FoulKind;
use nba_domain::league::LeagueProfile;
use nba_domain::rules::{
    counts_toward_personal_limit, counts_toward_team_period_fouls,
};
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

/// 双向可达性烟雾：从 Pregame 出发，合法路径必须能抵达 GameEnd（无缺失出口）。
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

/// 前场篮板后的进攻时钟：只有球触筐才重置，且只补不足、不削减剩余时间。
///
/// - 触筐且剩 8s → 补到 14s；
/// - 触筐且剩 16s → 保持 16s（分秒不扣）；
/// - 三不沾/被封盖且被攻方抢到 → 原读数继续走。
#[test]
fn shot_clock_resets_to_fourteen_only_when_the_ball_touches_the_rim() {
    let league = LeagueProfile::nba();
    assert_eq!(league.shot_clock_after_offensive_rebound(8.0, true), 14.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(16.0, true), 16.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(0.4, true), 14.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(8.0, false), 8.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(2.5, false), 2.5);
}

/// bonus 阈值矩阵：常规时段第 5 犯，第四节与加时最后两分钟第 2 犯。
#[test]
fn bonus_threshold_matrix_matches_the_nba_rule() {
    let league = LeagueProfile::nba();
    // 常规时段：第 4 犯发边线球，第 5 犯两罚。
    assert_eq!(league.bonus_free_throws_for(4, 4, 350.0), 0);
    assert_eq!(league.bonus_free_throws_for(5, 4, 350.0), 2);
    // 第四节最后两分钟：第 1 犯发边线球，第 2 犯两罚。
    assert_eq!(league.bonus_free_throws_for(1, 4, 90.0), 0);
    assert_eq!(league.bonus_free_throws_for(2, 4, 90.0), 2);
    // 同一种切在第二节不适用（第二节没有最后两分钟特例）。
    assert_eq!(league.bonus_free_throws_for(1, 2, 90.0), 0);
    assert_eq!(league.bonus_free_throws_for(4, 2, 90.0), 0);
    // 加时同样适用最后两分钟特例。
    assert_eq!(league.bonus_free_throws_for(2, 5, 90.0), 2);
    // FIBA 没有这段特例，第四节最后两分钟仍需第 5 犯。
    let fiba = LeagueProfile::fiba();
    assert_eq!(fiba.bonus_free_throws_for(2, 4, 90.0), 0);
    assert_eq!(fiba.bonus_free_throws_for(5, 4, 90.0), 2);
}

/// 进攻犯规计入个人累计（满 6 犯仍离场），但不计入球队单节犯规（不进 bonus）。
#[test]
fn offensive_fouls_count_toward_the_personal_limit_but_not_the_bonus() {
    assert!(counts_toward_personal_limit(FoulKind::Offensive));
    assert!(!counts_toward_team_period_fouls(FoulKind::Offensive));
    // 投篮与普通犯规两本账都计。
    assert!(counts_toward_personal_limit(FoulKind::Shooting));
    assert!(counts_toward_team_period_fouls(FoulKind::Shooting));
    assert!(counts_toward_personal_limit(FoulKind::Personal));
    assert!(counts_toward_team_period_fouls(FoulKind::Personal));
    // 技术犯规两本账都不计。
    assert!(!counts_toward_personal_limit(FoulKind::Technical));
    assert!(!counts_toward_team_period_fouls(FoulKind::Technical));
}
