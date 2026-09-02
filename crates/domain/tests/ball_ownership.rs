use nba_domain::{transition_ball_state, BallState, Possession};
use glam::Vec2;

fn held() -> BallState {
    BallState::Held { carrier_id: "H_1".into() }
}

#[test]
fn legal_edges_are_accepted() {
    let cases: Vec<(BallState, BallState)> = vec![
        (held(), BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_2".into(), start_time: 0.0, duration: 0.5, peak_z: 6.0, inbound: false, receive_success: true }),
        (held(), BallState::Drive { driver_id: "H_1".into(), from_pos: Vec2::ZERO, target_pos: Vec2::ONE, start_time: 0.0, duration: 1.0, successful: true, finish_made: false, fouler_id: None }),
        (held(), BallState::Shot { shooter_id: "H_1".into(), from_pos: Vec2::ZERO, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.2, is_made: true, is_three: false, peak_z: 14.0 }),
        (held(), BallState::ControlTransfer { from_pos: Vec2::ZERO, from_z: 4.0, carrier_id: "H_2".into(), start_time: 0.0, duration: 0.3 }),
        (held(), BallState::Dead { pos: Vec2::ZERO, z: 0.0, last_touch_team: Possession::Home }),
        (held(), BallState::InboundTransfer { from_pos: Vec2::ZERO, from_z: 4.0, baseline_pos: Vec2::ZERO, inbounder_id: "H_2".into(), start_time: 0.0, duration: 0.5 }),
        (BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_2".into(), start_time: 0.0, duration: 0.5, peak_z: 6.0, inbound: false, receive_success: true }, held()),
        (BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_2".into(), start_time: 0.0, duration: 0.5, peak_z: 6.0, inbound: false, receive_success: false }, BallState::LooseBall { pos: Vec2::ZERO, vel: Vec2::ZERO, z: 4.0, vel_z: 0.0, last_touch_team: Possession::Home }),
        (BallState::Shot { shooter_id: "H_1".into(), from_pos: Vec2::ZERO, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.2, is_made: false, is_three: false, peak_z: 14.0 }, BallState::RimRebound { from_pos: Vec2::ZERO, from_z: 10.0, hoop_pos: Vec2::ONE, target_landing: Vec2::ZERO, start_time: 0.0, duration: 1.0, peak_z: 12.0, last_touch_team: Possession::Home }),
        (BallState::RimRebound { from_pos: Vec2::ZERO, from_z: 10.0, hoop_pos: Vec2::ONE, target_landing: Vec2::ZERO, start_time: 0.0, duration: 1.0, peak_z: 12.0, last_touch_team: Possession::Home }, held()),
        (BallState::LooseBall { pos: Vec2::ZERO, vel: Vec2::ZERO, z: 4.0, vel_z: 0.0, last_touch_team: Possession::Home }, held()),
        (BallState::LooseBall { pos: Vec2::ZERO, vel: Vec2::ZERO, z: 4.0, vel_z: 0.0, last_touch_team: Possession::Home }, BallState::LooseBall { pos: Vec2::ONE, vel: Vec2::ZERO, z: 2.0, vel_z: 0.0, last_touch_team: Possession::Home }),
        (BallState::ControlTransfer { from_pos: Vec2::ZERO, from_z: 4.0, carrier_id: "H_2".into(), start_time: 0.0, duration: 0.3 }, held()),
        (BallState::Dead { pos: Vec2::ZERO, z: 0.0, last_touch_team: Possession::Home }, BallState::InboundTransfer { from_pos: Vec2::ZERO, from_z: 4.0, baseline_pos: Vec2::ZERO, inbounder_id: "H_2".into(), start_time: 0.0, duration: 0.5 }),
        (BallState::InboundTransfer { from_pos: Vec2::ZERO, from_z: 4.0, baseline_pos: Vec2::ZERO, inbounder_id: "H_2".into(), start_time: 0.0, duration: 0.5 }, BallState::InboundReady { baseline_pos: Vec2::ZERO, inbounder_id: "H_2".into() }),
        (BallState::InboundReady { baseline_pos: Vec2::ZERO, inbounder_id: "H_2".into() }, BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_1".into(), start_time: 0.0, duration: 0.6, peak_z: 8.0, inbound: true, receive_success: true }),
        (BallState::Drive { driver_id: "H_1".into(), from_pos: Vec2::ZERO, target_pos: Vec2::ONE, start_time: 0.0, duration: 1.0, successful: false, finish_made: false, fouler_id: None }, held()),
    ];
    for (i, (cur, next)) in cases.iter().enumerate() {
        assert!(
            transition_ball_state(cur, next.clone()).is_ok(),
            "case {} {:?} -> {:?} must be legal",
            i, cur.phase(), next.phase()
        );
    }
}

#[test]
fn illegal_edges_are_rejected() {
    // 飞行中的投篮不能被直接拿住：必须经 RimRebound/LooseBall/Dead。
    let shot = BallState::Shot { shooter_id: "H_1".into(), from_pos: Vec2::ZERO, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.2, is_made: false, is_three: false, peak_z: 14.0 };
    assert!(transition_ball_state(&shot, held()).is_err());
    assert!(transition_ball_state(&shot, held()).is_err());
    // 死球不能直接进入活球飞行：必须经发球程序。
    let dead = BallState::Dead { pos: Vec2::ZERO, z: 0.0, last_touch_team: Possession::Home };
    assert!(transition_ball_state(&dead, BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_1".into(), start_time: 0.0, duration: 0.5, peak_z: 6.0, inbound: false, receive_success: true }).is_err());
    assert!(transition_ball_state(&dead, BallState::Shot { shooter_id: "H_1".into(), from_pos: Vec2::ZERO, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.0, is_made: true, is_three: false, peak_z: 10.0 }).is_err());
    // 篮板飞行不能直接变成投篮。
    let reb = BallState::RimRebound { from_pos: Vec2::ZERO, from_z: 10.0, hoop_pos: Vec2::ONE, target_landing: Vec2::ZERO, start_time: 0.0, duration: 1.0, peak_z: 12.0, last_touch_team: Possession::Home };
    assert!(transition_ball_state(&reb, BallState::Shot { shooter_id: "H_1".into(), from_pos: Vec2::ZERO, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.0, is_made: false, is_three: false, peak_z: 10.0 }).is_err());
    // 传球飞行不能二次出手。
    let pass = BallState::Pass { from_pos: Vec2::ZERO, to_pos: Vec2::ONE, target_id: "H_2".into(), start_time: 0.0, duration: 0.5, peak_z: 6.0, inbound: false, receive_success: true };
    assert!(transition_ball_state(&pass, BallState::Shot { shooter_id: "H_2".into(), from_pos: Vec2::ONE, hoop_pos: Vec2::ONE, start_time: 0.0, duration: 1.0, is_made: false, is_three: false, peak_z: 10.0 }).is_err());
}

