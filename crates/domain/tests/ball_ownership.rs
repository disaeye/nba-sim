//! 权威球态的归属语义与迁移表守卫。
//!
//! 球态是比赛的单一事实源：持球人、飞行、松球、裁判控球与比赛用球离场
//! 七种状态互相正交，迁移表在 `nba_domain::match_model` 里给出。这里做
//! 两件事：
//!
//! 1. 迁移矩阵的穷举核对：声明表与实现逐一一致；
//! 2. 归属语义：持球人只能从真正持球的球态读出，飞行期间没有持球人。

use glam::Vec2;
use nba_domain::action_window::DribbleMoveKind;
use nba_domain::match_model::{
    ball_state_transition_allowed, transition_ball_state, BallState, BallStateKind, PassKind,
    ShotContext,
};

fn held() -> BallState {
    BallState::Held { holder: 0 }
}

fn dribble() -> BallState {
    BallState::DribbleFlight {
        holder: 0,
        from_pos: Vec2::ZERO,
        to_pos: Vec2::ONE,
        start: 0.0,
        duration: 0.4,
        move_kind: DribbleMoveKind::Crossover,
    }
}

fn pass() -> BallState {
    BallState::PassFlight {
        pass_id: 1,
        from: 0,
        to: 1,
        kind: PassKind::Chest,
        from_pos: Vec2::ZERO,
        to_pos: Vec2::ONE,
        start: 0.0,
        duration: 0.5,
        inbound: false,
    }
}

fn shot() -> BallState {
    BallState::ShotFlight {
        shot_id: 1,
        shooter: 0,
        context: ShotContext::FieldGoal,
        from_pos: Vec2::ZERO,
        target: Vec2::ONE,
        start: 0.0,
        aperture_ft: 0.75,
        rim_in_probability: 0.2,
        aim_offset: Vec2::ZERO,
    }
}

fn loose() -> BallState {
    BallState::Loose {
        pos: Vec2::ZERO,
        vel: Vec2::ONE,
    }
}

fn official() -> BallState {
    BallState::ControlledByOfficial {
        holder: 0,
        inbound_spot: Vec2::ZERO,
        authorized_at: 0.0,
    }
}

fn out_of_play() -> BallState {
    BallState::OutOfPlay {
        pos: Vec2::ZERO,
        reason: "out_of_bounds",
    }
}

/// 全部七种球态的代表实例。顺序与 [`BallStateKind::ALL`] 一致。
fn all_variants() -> Vec<(&'static str, BallState)> {
    vec![
        ("Held", held()),
        ("DribbleFlight", dribble()),
        ("PassFlight", pass()),
        ("ShotFlight", shot()),
        ("Loose", loose()),
        ("ControlledByOfficial", official()),
        ("OutOfPlay", out_of_play()),
    ]
}

/// 声明的合法边。
///
/// 这里**有意重复一遍**声明表：测试的价值来自「独立表达期望」。直接调用
/// 实现函数会让测试变成同义反复，无法发现表被改错。
const DECLARED_LEGAL: &[(&str, &str)] = &[
    // 持球：可以起运球、传球、投篮、失去控制、交给裁判或出界。
    ("Held", "Held"),
    ("Held", "DribbleFlight"),
    ("Held", "PassFlight"),
    ("Held", "ShotFlight"),
    ("Held", "Loose"),
    ("Held", "ControlledByOfficial"),
    ("Held", "OutOfPlay"),
    // 运球：可以收球（回到持球）、传球、投篮、丢球或出界。
    ("DribbleFlight", "DribbleFlight"),
    ("DribbleFlight", "Held"),
    ("DribbleFlight", "PassFlight"),
    ("DribbleFlight", "ShotFlight"),
    ("DribbleFlight", "Loose"),
    ("DribbleFlight", "OutOfPlay"),
    // 传球飞行：可以被接住（持球）、被打断（松球）或出界。
    ("PassFlight", "PassFlight"),
    ("PassFlight", "Held"),
    ("PassFlight", "Loose"),
    ("PassFlight", "OutOfPlay"),
    // 投篮飞行：可以弹出（松球）或出界。
    ("ShotFlight", "ShotFlight"),
    ("ShotFlight", "Loose"),
    ("ShotFlight", "OutOfPlay"),
    // 松球：可以被控制（持球）、交给裁判、继续滚动或出界。
    ("Loose", "Loose"),
    ("Loose", "Held"),
    ("Loose", "ControlledByOfficial"),
    ("Loose", "OutOfPlay"),
    // 裁判控球：可以交给发球人（持球）、直接发球（传球飞行）或出界。
    ("ControlledByOfficial", "ControlledByOfficial"),
    ("ControlledByOfficial", "Held"),
    ("ControlledByOfficial", "PassFlight"),
    ("ControlledByOfficial", "OutOfPlay"),
    // 比赛用球离场：回到裁判控球或直接被控制。
    ("OutOfPlay", "OutOfPlay"),
    ("OutOfPlay", "ControlledByOfficial"),
    ("OutOfPlay", "Held"),
];

#[test]
fn every_state_edge_matches_the_declared_table() {
    let variants = all_variants();
    assert_eq!(
        variants.len(),
        BallStateKind::ALL.len(),
        "every declared ball-state kind needs a representative instance"
    );

    let legal: std::collections::HashSet<(&str, &str)> = DECLARED_LEGAL.iter().copied().collect();
    assert_eq!(
        legal.len(),
        DECLARED_LEGAL.len(),
        "DECLARED_LEGAL contains duplicate entries"
    );

    let mut mismatches: Vec<String> = Vec::new();
    for (from_name, from) in &variants {
        for (to_name, to) in &variants {
            let expected_ok = legal.contains(&(*from_name, *to_name));
            let actual_ok = transition_ball_state(from, to.clone()).is_ok();
            if expected_ok != actual_ok {
                mismatches.push(format!(
                    "{from_name} -> {to_name}: declared {}, actual {}",
                    if expected_ok { "legal" } else { "illegal" },
                    if actual_ok { "legal" } else { "illegal" }
                ));
            }
        }
    }

    assert!(
        mismatches.is_empty(),
        "state-edge matrix disagrees with the declared transition table:\n  {}",
        mismatches.join("\n  ")
    );

    let total = variants.len() * variants.len();
    assert_eq!(legal.len(), 31, "declared legal edge count changed");
    assert_eq!(
        total - legal.len(),
        18,
        "declared illegal edge count changed"
    );
}

#[test]
fn the_illegal_edges_are_the_basketball_impossibilities() {
    // 飞行中的球不能凭空变成另一种飞行：传球与投篮之间没有直接边。
    assert!(!ball_state_transition_allowed(
        BallStateKind::PassFlight,
        BallStateKind::ShotFlight
    ));
    assert!(!ball_state_transition_allowed(
        BallStateKind::ShotFlight,
        BallStateKind::PassFlight
    ));
    // 投篮飞行不能被直接拿住：必须先成为松球。
    assert!(transition_ball_state(&shot(), held()).is_err());
    // 传球飞行不能直接变成投篮。
    assert!(transition_ball_state(&pass(), shot()).is_err());
    // 裁判控球的球不能直接进入投篮飞行。
    assert!(transition_ball_state(&official(), shot()).is_err());
    // 比赛用球离场不能直接进入飞行。
    assert!(transition_ball_state(&out_of_play(), pass()).is_err());
}

#[test]
fn the_holder_is_only_read_from_states_that_carry_control() {
    assert_eq!(held().holder(), Some(0));
    assert_eq!(dribble().holder(), Some(0));
    assert_eq!(official().holder(), Some(0));
    assert_eq!(pass().holder(), None);
    assert_eq!(shot().holder(), None);
    assert_eq!(loose().holder(), None);
    assert_eq!(out_of_play().holder(), None);
}

#[test]
fn only_the_three_flight_states_count_as_in_flight() {
    assert!(dribble().is_in_flight());
    assert!(pass().is_in_flight());
    assert!(shot().is_in_flight());
    assert!(!held().is_in_flight());
    assert!(!loose().is_in_flight());
    assert!(!official().is_in_flight());
    assert!(!out_of_play().is_in_flight());
}

#[test]
fn every_kind_has_a_stable_name() {
    for kind in BallStateKind::ALL {
        assert!(!kind.as_str().is_empty());
    }
    assert_eq!(held().name(), "held");
    assert_eq!(shot().name(), "shot_flight");
}
