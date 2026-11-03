//! GameFlowState 转换、时钟映射、进攻时钟重置与犯规计账守卫。

use nba_domain::event::FoulKind;
use nba_domain::rules::{counts_toward_personal_limit, counts_toward_team_period_fouls, GameRules};
use nba_domain::GameFlowState;

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

#[test]
fn game_end_is_absorbing() {
    for next in ALL {
        if next != GameFlowState::GameEnd {
            assert!(!GameFlowState::GameEnd.can_transition_to(next));
        }
    }
}

#[test]
fn game_end_is_reachable_from_every_mid_state() {
    use std::collections::VecDeque;
    for start in ALL {
        if matches!(start, GameFlowState::GameEnd | GameFlowState::Pregame) {
            continue;
        }
        let mut visited = [false; 10];
        let mut queue = VecDeque::from([start]);
        visited[index_of(start)] = true;
        let mut reachable = false;
        while let Some(current) = queue.pop_front() {
            if current == GameFlowState::GameEnd {
                reachable = true;
                break;
            }
            for next in ALL {
                if !visited[index_of(next)] && current.can_transition_to(next) {
                    visited[index_of(next)] = true;
                    queue.push_back(next);
                }
            }
        }
        assert!(reachable, "GameEnd is unreachable from {start:?}");
    }
}

fn index_of(state: GameFlowState) -> usize {
    ALL.iter()
        .position(|candidate| *candidate == state)
        .expect("state is listed")
}

#[test]
fn opening_lifecycle_edges_match_the_flow_table() {
    assert!(GameFlowState::Pregame.can_transition_to(GameFlowState::TipOff));
    assert!(GameFlowState::Pregame.can_transition_to(GameFlowState::GameEnd));
    for state in ALL {
        if !matches!(state, GameFlowState::Pregame | GameFlowState::TipOff) {
            assert!(!state.can_transition_to(GameFlowState::TipOff));
            assert!(!state.can_transition_to(GameFlowState::Pregame));
        }
    }
    assert!(GameFlowState::TipOff.can_transition_to(GameFlowState::LiveBall));
    assert!(GameFlowState::TipOff.can_transition_to(GameFlowState::DeadBall));
    assert!(!GameFlowState::TipOff.can_transition_to(GameFlowState::Overtime));
}

#[test]
fn overtime_cannot_return_to_regulation_live_ball() {
    assert!(GameFlowState::LiveBall.can_transition_to(GameFlowState::Overtime));
    assert!(!GameFlowState::Overtime.can_transition_to(GameFlowState::LiveBall));
}

#[test]
fn same_state_assignment_is_idempotent() {
    for state in ALL {
        assert!(state.can_transition_to(state));
    }
}

#[test]
fn game_flow_clock_mapping_marks_only_live_play_as_running() {
    for state in ALL {
        let running = matches!(state, GameFlowState::LiveBall | GameFlowState::Overtime);
        assert_eq!(state.advances_game_clock(), running);
        assert_eq!(state.advances_shot_clock(), running);
    }
}

#[test]
fn offensive_rebound_shot_clock_requires_rim_contact() {
    let league = nba_domain::LeagueProfile::nba();
    assert_eq!(league.shot_clock_after_offensive_rebound(8.0, true), 14.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(16.0, true), 16.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(0.4, true), 14.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(8.0, false), 8.0);
    assert_eq!(league.shot_clock_after_offensive_rebound(2.5, false), 2.5);
}

#[test]
fn profile_bonus_thresholds_match_the_current_scalar_rules() {
    let nba = nba_domain::LeagueProfile::nba();
    assert_eq!(nba.bonus_free_throws_for(4, 2, 350.0), 0);
    assert_eq!(nba.bonus_free_throws_for(5, 2, 350.0), 2);
    assert_eq!(nba.bonus_free_throws_for(1, 4, 90.0), 0);
    assert_eq!(nba.bonus_free_throws_for(2, 4, 90.0), 2);

    let fiba = nba_domain::LeagueProfile::fiba();
    assert_eq!(fiba.bonus_free_throws_for(4, 2, 350.0), 0);
    assert_eq!(fiba.bonus_free_throws_for(5, 2, 350.0), 2);
    assert_eq!(fiba.bonus_free_throws_for(4, 4, 90.0), 0);
    assert_eq!(fiba.bonus_free_throws_for(5, 4, 90.0), 2);
}

#[test]
fn foul_accounting_separates_personal_and_team_period_counters() {
    assert!(counts_toward_personal_limit(FoulKind::Offensive));
    assert!(!counts_toward_team_period_fouls(FoulKind::Offensive));
    assert!(counts_toward_personal_limit(FoulKind::Shooting));
    assert!(counts_toward_team_period_fouls(FoulKind::Shooting));
    assert!(counts_toward_personal_limit(FoulKind::Personal));
    assert!(counts_toward_team_period_fouls(FoulKind::Personal));
    assert!(!counts_toward_personal_limit(FoulKind::Technical));
    assert!(!counts_toward_team_period_fouls(FoulKind::Technical));
}

#[test]
fn changed_league_threshold_changes_bonus_result() {
    let mut rules = GameRules::default();
    rules.league.fouls.bonus_fouls_per_period = 3;
    rules.league.fouls.bonus_fouls_final = 2;
    assert_eq!(rules.league.bonus_free_throws_for(2, 2, 400.0), 0);
    assert_eq!(rules.league.bonus_free_throws_for(3, 2, 400.0), 2);
    assert_eq!(rules.league.bonus_free_throws_for(1, 4, 90.0), 0);
    assert_eq!(rules.league.bonus_free_throws_for(2, 4, 90.0), 2);
}
