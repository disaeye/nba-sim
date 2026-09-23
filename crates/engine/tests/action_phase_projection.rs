//! 动作阶段协议与真实引擎帧投影验证。

use nba_domain::{GameFlowState, GameRules};
use nba_engine::MatchEngine;
use nba_protocol::RenderPlayer;

fn find_player<'a>(players: &'a [RenderPlayer], player_id: &str) -> &'a RenderPlayer {
    players
        .iter()
        .find(|player| player.id == player_id)
        .expect("render frame should contain the selected roster player")
}

#[test]
fn active_action_phases_are_projected_and_round_trip_through_serde() {
    let rules = GameRules {
        tick_seconds: 0.04,
        tip_off_duration_seconds: 0.0,
        jump_shot_prep_seconds: 0.08,
        jump_shot_exec_seconds: 0.08,
        jump_shot_follow_seconds: 0.16,
        ..Default::default()
    };

    let mut engine = MatchEngine::with_rules(2501, rules);
    engine.force_possession_for_test(nba_domain::Possession::Home);
    engine.set_game_flow_for_test(GameFlowState::LiveBall);
    let shooter_pos = engine
        .physics()
        .get_player("H_01")
        .expect("home shooter should exist")
        .pos_ft;
    let ball_height = engine.rules().ball_holder_height_ft;
    engine.set_ball_pos_for_test(shooter_pos, ball_height);
    engine.execute_shot_for_test("H_01", shooter_pos, false);

    let preparation = engine.snapshot();
    let preparation_player = find_player(&preparation.frame.players, "H_01");
    assert_eq!(
        preparation_player.action_phase.as_deref(),
        Some("Preparation")
    );
    let encoded = serde_json::to_value(preparation_player).expect("player should serialize");
    assert_eq!(encoded["action_phase"], "Preparation");
    let decoded: RenderPlayer = serde_json::from_value(encoded).expect("player should deserialize");
    assert_eq!(decoded.action_phase.as_deref(), Some("Preparation"));

    engine.step();
    let preparation = engine.snapshot();
    assert_eq!(
        find_player(&preparation.frame.players, "H_01")
            .action_phase
            .as_deref(),
        Some("Preparation")
    );

    engine.step();
    let execution = engine.snapshot();
    let execution_player = find_player(&execution.frame.players, "H_01");
    assert_eq!(execution_player.action_phase.as_deref(), Some("Execution"));
    let encoded = serde_json::to_value(execution_player).expect("player should serialize");
    assert_eq!(encoded["action_phase"], "Execution");
    let decoded: RenderPlayer = serde_json::from_value(encoded).expect("player should deserialize");
    assert_eq!(decoded.action_phase.as_deref(), Some("Execution"));

    engine.step();
    engine.step();
    let follow_through = engine.snapshot();
    assert_eq!(
        find_player(&follow_through.frame.players, "H_01")
            .action_phase
            .as_deref(),
        Some("FollowThrough")
    );
}

#[test]
fn frame_without_an_active_window_projects_none_and_omits_the_serde_field() {
    let engine = MatchEngine::new(2502);
    let frame = engine.snapshot();
    assert!(!frame.frame.players.is_empty());
    assert!(frame
        .frame
        .players
        .iter()
        .all(|player| player.action_phase.is_none()));

    let player = find_player(&frame.frame.players, "H_01");
    let mut encoded = serde_json::to_value(player).expect("player should serialize");
    assert!(encoded.get("action_phase").is_none());
    encoded
        .as_object_mut()
        .expect("serialized player should be a JSON object")
        .remove("action_phase");
    let decoded: RenderPlayer =
        serde_json::from_value(encoded).expect("missing action_phase should deserialize");
    assert_eq!(decoded.action_phase, None);
}
