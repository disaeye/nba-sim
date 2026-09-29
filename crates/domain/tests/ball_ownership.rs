use glam::Vec2;
use nba_domain::{transition_ball_state, BallState, Possession};

fn held() -> BallState {
    BallState::Held {
        carrier_id: "H_01".into(),
    }
}

#[test]
fn legal_edges_are_accepted() {
    let cases: Vec<(BallState, BallState)> = vec![
        (
            held(),
            BallState::Pass {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                target_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
                peak_z: 6.0,
                inbound: false,
            },
        ),
        (
            held(),
            BallState::Drive {
                driver_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                target_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.0,
                move_kind: None,
            },
        ),
        (
            held(),
            BallState::Shot {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                hoop_pos: Vec2::ONE,
                aim_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.2,
                is_three: false,
                peak_z: 14.0,
                make_probability: 0.5,
                contest_intensity: 0.0,
            },
        ),
        (
            held(),
            BallState::ControlTransfer {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                carrier_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.3,
                target_pos: Vec2::ONE,
                target_z: 4.0,
            },
        ),
        (
            held(),
            BallState::Dead {
                pos: Vec2::ZERO,
                z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
        ),
        (
            held(),
            BallState::InboundTransfer {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                baseline_pos: Vec2::ZERO,
                inbounder_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
            },
        ),
        (
            BallState::Pass {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                target_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
                peak_z: 6.0,
                inbound: false,
            },
            held(),
        ),
        (
            BallState::Pass {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                target_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
                peak_z: 6.0,
                inbound: false,
            },
            BallState::LooseBall {
                pos: Vec2::ZERO,
                vel: Vec2::ZERO,
                z: 4.0,
                vel_z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
        ),
        (
            BallState::Shot {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                hoop_pos: Vec2::ONE,
                aim_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.2,
                is_three: false,
                peak_z: 14.0,
                make_probability: 0.5,
                contest_intensity: 0.0,
            },
            BallState::RimRebound {
                from_pos: Vec2::ZERO,
                from_z: 10.0,
                hoop_pos: Vec2::ONE,
                target_landing: Vec2::ZERO,
                start_time: 0.0,
                duration: 1.0,
                peak_z: 12.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
        ),
        (
            BallState::RimRebound {
                from_pos: Vec2::ZERO,
                from_z: 10.0,
                hoop_pos: Vec2::ONE,
                target_landing: Vec2::ZERO,
                start_time: 0.0,
                duration: 1.0,
                peak_z: 12.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
            held(),
        ),
        (
            BallState::LooseBall {
                pos: Vec2::ZERO,
                vel: Vec2::ZERO,
                z: 4.0,
                vel_z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
            held(),
        ),
        (
            BallState::LooseBall {
                pos: Vec2::ZERO,
                vel: Vec2::ZERO,
                z: 4.0,
                vel_z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
            BallState::LooseBall {
                pos: Vec2::ONE,
                vel: Vec2::ZERO,
                z: 2.0,
                vel_z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
        ),
        (
            BallState::ControlTransfer {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                carrier_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.3,
                target_pos: Vec2::ONE,
                target_z: 4.0,
            },
            held(),
        ),
        (
            BallState::Dead {
                pos: Vec2::ZERO,
                z: 4.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
            BallState::FreeThrowSetup {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                from_z: 0.0,
                to_pos: Vec2::ONE,
                to_z: 4.0,
                start_time: 0.0,
                duration: 1.0,
                forced_result: None,
                is_final: false,
            },
        ),
        (
            BallState::FreeThrowSetup {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                to_z: 4.0,
                start_time: 0.0,
                duration: 1.0,
                forced_result: None,
                is_final: false,
            },
            BallState::FreeThrow {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ONE,
                hoop_pos: Vec2::new(2.0, 1.0),
                aim_pos: Vec2::new(2.0, 1.0),
                start_time: 1.0,
                duration: 1.0,
                peak_z: 14.0,
                is_final: false,
            },
        ),
        (
            BallState::FreeThrow {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                hoop_pos: Vec2::ONE,
                aim_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.0,
                peak_z: 14.0,
                is_final: true,
            },
            BallState::RimRebound {
                from_pos: Vec2::ONE,
                from_z: 10.0,
                hoop_pos: Vec2::ONE,
                target_landing: Vec2::ZERO,
                start_time: 1.0,
                duration: 1.0,
                peak_z: 12.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
        ),
        (
            BallState::Dead {
                pos: Vec2::ZERO,
                z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: None,
            },
            BallState::InboundTransfer {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                baseline_pos: Vec2::ZERO,
                inbounder_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
            },
        ),
        (
            BallState::InboundTransfer {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                baseline_pos: Vec2::ZERO,
                inbounder_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
            },
            BallState::InboundReady {
                baseline_pos: Vec2::ZERO,
                inbounder_id: "H_02".into(),
            },
        ),
        (
            BallState::InboundReady {
                baseline_pos: Vec2::ZERO,
                inbounder_id: "H_02".into(),
            },
            BallState::Pass {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                target_id: "H_01".into(),
                start_time: 0.0,
                duration: 0.6,
                peak_z: 8.0,
                inbound: true,
            },
        ),
        (
            BallState::Drive {
                driver_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                target_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.0,
                move_kind: None,
            },
            held(),
        ),
    ];
    for (i, (cur, next)) in cases.iter().enumerate() {
        assert!(
            transition_ball_state(cur, next.clone()).is_ok(),
            "case {} {:?} -> {:?} must be legal",
            i,
            cur.phase(),
            next.phase()
        );
    }
}

#[test]
fn illegal_edges_are_rejected() {
    // 飞行中的投篮不能被直接拿住：必须经 RimRebound/LooseBall/Dead。
    let shot = BallState::Shot {
        shooter_id: "H_01".into(),
        from_pos: Vec2::ZERO,
        hoop_pos: Vec2::ONE,
        aim_pos: Vec2::ONE,
        start_time: 0.0,
        duration: 1.2,
        is_three: false,
        peak_z: 14.0,
        make_probability: 0.5,
        contest_intensity: 0.0,
    };
    assert!(transition_ball_state(&shot, held()).is_err());
    assert!(transition_ball_state(&shot, held()).is_err());
    // 死球不能直接进入活球飞行：必须经发球程序。
    let dead = BallState::Dead {
        pos: Vec2::ZERO,
        z: 0.0,
        last_touch_team: Possession::Home,
        last_touch_player: None,
    };
    assert!(transition_ball_state(
        &dead,
        BallState::Pass {
            from_pos: Vec2::ZERO,
            from_z: 4.0,
            to_pos: Vec2::ONE,
            target_id: "H_01".into(),
            start_time: 0.0,
            duration: 0.5,
            peak_z: 6.0,
            inbound: false,
        }
    )
    .is_err());
    assert!(transition_ball_state(
        &dead,
        BallState::Shot {
            shooter_id: "H_01".into(),
            from_pos: Vec2::ZERO,
            hoop_pos: Vec2::ONE,
            aim_pos: Vec2::ONE,
            start_time: 0.0,
            duration: 1.0,
            is_three: false,
            peak_z: 10.0,
            make_probability: 0.5,
            contest_intensity: 0.0,
        }
    )
    .is_err());
    // 篮板飞行不能直接变成投篮。
    let reb = BallState::RimRebound {
        from_pos: Vec2::ZERO,
        from_z: 10.0,
        hoop_pos: Vec2::ONE,
        target_landing: Vec2::ZERO,
        start_time: 0.0,
        duration: 1.0,
        peak_z: 12.0,
        last_touch_team: Possession::Home,
        last_touch_player: Some("H_01".into()),
    };
    assert!(transition_ball_state(
        &reb,
        BallState::Shot {
            shooter_id: "H_01".into(),
            from_pos: Vec2::ZERO,
            hoop_pos: Vec2::ONE,
            aim_pos: Vec2::ONE,
            start_time: 0.0,
            duration: 1.0,
            is_three: false,
            peak_z: 10.0,
            make_probability: 0.5,
            contest_intensity: 0.0,
        }
    )
    .is_err());
    // 传球飞行不能二次出手。
    let pass = BallState::Pass {
        from_pos: Vec2::ZERO,
        from_z: 4.0,
        to_pos: Vec2::ONE,
        target_id: "H_02".into(),
        start_time: 0.0,
        duration: 0.5,
        peak_z: 6.0,
        inbound: false,
    };
    assert!(transition_ball_state(
        &pass,
        BallState::Shot {
            shooter_id: "H_02".into(),
            from_pos: Vec2::ONE,
            hoop_pos: Vec2::ONE,
            aim_pos: Vec2::ONE,
            start_time: 0.0,
            duration: 1.0,
            is_three: false,
            peak_z: 10.0,
            make_probability: 0.5,
            contest_intensity: 0.0,
        }
    )
    .is_err());
}

// ============================================================================
// 穷举状态边覆盖（D8.3 / gap.md §5.3）
//
// ## 为什么需要它
//
// 上面的 `legal_edges_are_accepted` / `illegal_edges_are_rejected` 是**样例**
// 测试：它们挑选了若干代表性边，因而无法回答「声明表是否被完整执行」。
// 状态边测试使用 11 个球态变体，样例测试只覆盖其中一部分组合。
//
// `gap.md` §5.3 要求「状态机必须列出合法边、非法边和每条边的责任事件」。
// 本测试用**穷举矩阵**兑现该要求：对全部 11×11 = 121 种组合断言
// `transition_ball_state` 的接受/拒绝与声明表**逐一一致**。
//
// 这样任何一处边增删都会让矩阵失配，而不是悄悄通过样例测试。
// ============================================================================

/// 构造除罚球准备外的 11 个球态变体代表实例。
fn all_variants() -> Vec<(&'static str, BallState)> {
    vec![
        (
            "Held",
            BallState::Held {
                carrier_id: "H_01".into(),
            },
        ),
        (
            "Drive",
            BallState::Drive {
                driver_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                target_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.0,
                move_kind: None,
            },
        ),
        (
            "Pass",
            BallState::Pass {
                from_pos: Vec2::ZERO,
                from_z: 4.0,
                to_pos: Vec2::ONE,
                target_id: "H_02".into(),
                start_time: 0.0,
                duration: 0.5,
                peak_z: 8.0,
                inbound: false,
            },
        ),
        (
            "Shot",
            BallState::Shot {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                hoop_pos: Vec2::ONE,
                aim_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.2,
                is_three: false,
                peak_z: 14.0,
                make_probability: 0.5,
                contest_intensity: 0.0,
            },
        ),
        (
            "FreeThrow",
            BallState::FreeThrow {
                shooter_id: "H_01".into(),
                from_pos: Vec2::ZERO,
                hoop_pos: Vec2::ONE,
                aim_pos: Vec2::ONE,
                start_time: 0.0,
                duration: 1.0,
                peak_z: 14.0,
                is_final: false,
            },
        ),
        (
            "RimRebound",
            BallState::RimRebound {
                from_pos: Vec2::ZERO,
                from_z: 10.0,
                hoop_pos: Vec2::ONE,
                target_landing: Vec2::ONE,
                start_time: 0.0,
                duration: 0.8,
                peak_z: 11.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
        ),
        (
            "LooseBall",
            BallState::LooseBall {
                pos: Vec2::ZERO,
                vel: Vec2::ONE,
                z: 2.0,
                vel_z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
        ),
        (
            "ControlTransfer",
            BallState::ControlTransfer {
                from_pos: Vec2::ZERO,
                from_z: 6.0,
                target_pos: Vec2::ONE,
                target_z: 6.0,
                carrier_id: "H_01".into(),
                start_time: 0.0,
                duration: 0.3,
            },
        ),
        (
            "Dead",
            BallState::Dead {
                pos: Vec2::ZERO,
                z: 0.0,
                last_touch_team: Possession::Home,
                last_touch_player: Some("H_01".into()),
            },
        ),
        (
            "InboundTransfer",
            BallState::InboundTransfer {
                from_pos: Vec2::ZERO,
                from_z: 0.0,
                baseline_pos: Vec2::ONE,
                inbounder_id: "H_01".into(),
                start_time: 0.0,
                duration: 0.5,
            },
        ),
        (
            "InboundReady",
            BallState::InboundReady {
                baseline_pos: Vec2::ONE,
                inbounder_id: "H_01".into(),
            },
        ),
    ]
}

/// 声明表（`edge_allowed`）中允许的边，按变体名列出。
///
/// 这里**有意重复一遍**声明表：测试的价值正来自于「独立表达期望」。
/// 若直接调用 `edge_allowed`，测试就变成同义反复，无法发现表被改错。
const DECLARED_LEGAL: &[(&str, &str)] = &[
    // Held
    ("Held", "Held"),
    ("Held", "Drive"),
    ("Held", "Pass"),
    ("Held", "Shot"),
    ("Held", "ControlTransfer"),
    ("Held", "LooseBall"),
    ("Held", "Dead"),
    ("Held", "InboundTransfer"),
    ("Held", "RimRebound"),
    // Drive
    ("Drive", "Held"),
    ("Drive", "Pass"),
    ("Drive", "Shot"),
    ("Drive", "RimRebound"),
    ("Drive", "InboundTransfer"),
    ("Drive", "LooseBall"),
    ("Drive", "Dead"),
    // Pass
    ("Pass", "Pass"),
    ("Pass", "ControlTransfer"),
    ("Pass", "Held"),
    ("Pass", "LooseBall"),
    ("Pass", "Dead"),
    ("Pass", "InboundTransfer"),
    // Shot
    ("Shot", "InboundTransfer"),
    ("Shot", "Dead"),
    ("Shot", "RimRebound"),
    ("Shot", "LooseBall"),
    // FreeThrowSetup
    ("FreeThrowSetup", "FreeThrow"),
    ("FreeThrowSetup", "Dead"),
    ("FreeThrowSetup", "FreeThrowSetup"),
    // FreeThrow
    ("FreeThrow", "FreeThrowSetup"),
    ("FreeThrow", "RimRebound"),
    ("FreeThrow", "LooseBall"),
    ("FreeThrow", "Dead"),
    ("FreeThrow", "FreeThrow"),
    // RimRebound
    ("RimRebound", "Held"),
    ("RimRebound", "ControlTransfer"),
    ("RimRebound", "Pass"),
    ("RimRebound", "RimRebound"),
    ("RimRebound", "LooseBall"),
    ("RimRebound", "Dead"),
    ("RimRebound", "InboundTransfer"),
    // LooseBall
    ("LooseBall", "Held"),
    ("LooseBall", "ControlTransfer"),
    ("LooseBall", "LooseBall"),
    ("LooseBall", "Dead"),
    ("LooseBall", "InboundTransfer"),
    // ControlTransfer
    ("ControlTransfer", "Held"),
    ("ControlTransfer", "Dead"),
    ("ControlTransfer", "InboundTransfer"),
    // Dead
    ("Dead", "FreeThrowSetup"),
    ("Dead", "RimRebound"),
    ("Dead", "InboundTransfer"),
    ("Dead", "FreeThrow"),
    ("Dead", "Dead"),
    // InboundTransfer
    ("InboundTransfer", "InboundReady"),
    ("InboundTransfer", "InboundTransfer"),
    ("InboundTransfer", "Dead"),
    // InboundReady
    ("InboundReady", "Pass"),
    ("InboundReady", "InboundTransfer"),
    ("InboundReady", "Dead"),
];

#[test]
fn every_state_edge_matches_the_declared_table() {
    let mut variants = all_variants();
    variants.push((
        "FreeThrowSetup",
        BallState::FreeThrowSetup {
            shooter_id: "H_01".into(),
            from_pos: Vec2::ZERO,
            from_z: 4.0,
            to_pos: Vec2::ONE,
            to_z: 4.0,
            start_time: 0.0,
            duration: 1.0,
            forced_result: None,
            is_final: false,
        },
    ));
    assert_eq!(
        variants.len(),
        12,
        "expected 12 ball-state variants; update DECLARED_LEGAL when adding one"
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
        "state-edge matrix disagrees with the declared transition table \
         (gap.md §5.3 requires legal and illegal edges to be enumerated):\n  {}",
        mismatches.join("\n  ")
    );

    // 完整性自检：144 种组合必须全部分类。
    // 数字变化说明声明表被改动——此时应显式更新本测试与 DECLARED_LEGAL，
    // 而不是让它静默通过。
    let total = variants.len() * variants.len();
    assert_eq!(legal.len(), 60, "declared legal edge count changed");
    assert_eq!(
        total - legal.len(),
        84,
        "declared illegal edge count changed"
    );
}
