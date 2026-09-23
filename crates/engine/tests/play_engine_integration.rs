//! Play 引擎集成：触发 → 同 tick 效用 → 槽位动词目标 → 生命周期。
//!
//! 覆盖 tactics.md §2.2.4 的完整运行时链路：选板器激活（`PlayActivated`
//! 事件）、决策追踪携带 Play 偏好/抑制（`DecisionDebug.play_adjustments`）、
//! 命中规则把槽位球员目标改写为动词目标、回合结束后的激活清理与冷却。
//! 场景注入一个触发谓词在半场阵地必然成立的 PlaySpec（内联 JSON），
//! 引擎从正常开球推进，不强制任何初始态——强制态会与真实流程竞争并
//! 提前终局，掩盖真实链路。

use nba_domain::GameRules;
use nba_engine::{MatchEngine, MatchSetup};
use nba_protocol::StreamTick;

const PLAY_ID: &str = "integration_halfcourt_spot_up_v1";

/// 构造一台注入「半场持球即触发」Play 的引擎（自然开球流程）。
fn engine_with_play(seed: u64) -> MatchEngine {
    let play_json = r#"{
        "schema_version": 1,
        "id": "integration_halfcourt_spot_up_v1",
        "name_zh": "集成测试半场即触发",
        "kind": "play",
        "rules": [
            {
                "id": "spot_up_when_halfcourt_carrier",
                "when": [
                    { "predicate": "carrier_possed" },
                    { "predicate": "halfcourt_possession" }
                ],
                "then": { "verb": "SpotUp", "slot": "top" },
                "carrier_preferences": [
                    { "action_family": "Shoot", "bonus": 0.4 }
                ]
            }
        ],
        "triggers": [
            {
                "when": [
                    { "predicate": "carrier_possed" },
                    { "predicate": "halfcourt_possession" }
                ],
                "window_seconds": 600.0,
                "cooldown_seconds": 3.0
            }
        ]
    }"#;
    let play = nba_domain::PlaySpec::from_json(play_json).expect("integration PlaySpec is valid");
    let mut setup = MatchSetup::builtin(GameRules::default());
    setup.home_playbook = vec![play];
    setup.validate().expect("setup with custom play is valid");
    MatchEngine::with_setup(setup, seed)
}

fn is_activated(engine: &MatchEngine) -> bool {
    engine
        .active_play_id_for_test(nba_domain::Possession::Home)
        .is_some()
}

fn run_until_activation(engine: &mut MatchEngine, max_ticks: u64) -> Option<(u64, StreamTick)> {
    for tick in 0..max_ticks {
        let stream = engine.step();
        if is_activated(engine) {
            return Some((tick, stream));
        }
        if engine.is_finished() {
            return None;
        }
    }
    None
}

#[test]
fn trigger_activates_play_and_emits_observability() {
    let mut engine = engine_with_play(4101);
    let (activated_tick, stream) = run_until_activation(&mut engine, 5000)
        .expect("halfcourt play must activate within 5000 natural ticks");
    // 激活事件随 step 的 tick 事件流发布（pending_events 在 step 末尾被消费）。
    assert!(
        stream
            .frame
            .event_log
            .iter()
            .any(|event| event.kind == "PLAY_ACTIVATED"),
        "activation tick must emit PLAY_ACTIVATED in its event log (tick {activated_tick})"
    );
    assert_eq!(
        engine
            .active_play_id_for_test(nba_domain::Possession::Home)
            .as_deref(),
        Some(PLAY_ID),
        "engine must report the active play after activation"
    );
}

#[test]
fn activation_projects_play_adjustments_into_decision_debug() {
    let mut engine = engine_with_play(4201);
    let _ = run_until_activation(&mut engine, 5000).expect("play must activate");
    // 激活后继续推进直到一次持球决策产生 trace，再断言投影。
    let mut traced = false;
    for _ in 0..2000u64 {
        let _ = engine.step();
        if let Some(trace) = engine.last_decision_trace_for_test() {
            if trace.active_play_id.as_deref() == Some(PLAY_ID) {
                let shoot_adjustment = trace
                    .play_adjustments
                    .iter()
                    .find(|adj| adj.action_family == "Shoot")
                    .expect(
                        "Shoot family must appear in play adjustments while the play is active",
                    );
                assert!(
                    shoot_adjustment.bonus > 0.0,
                    "Shoot bonus from the play rule must be positive, got {}",
                    shoot_adjustment.bonus
                );
                traced = true;
                break;
            }
        }
        if engine.is_finished() {
            break;
        }
    }
    assert!(
        traced,
        "a post-activation decision must carry the play id in its trace"
    );
}

#[test]
fn matched_rule_rewrites_slot_target_with_verb_action() {
    let mut engine = engine_with_play(4301);
    let _ = run_until_activation(&mut engine, 5000).expect("play must activate");
    // 激活后规则每 tick 命中，top 槽位球员的动作标签被改写为动词目标。
    let mut observed = None;
    for _ in 0..2000u64 {
        let _ = engine.step();
        if let Some(actor) = engine
            .physics()
            .get_players()
            .values()
            .find(|p| p.on_court && p.team == "home" && p.action.starts_with("PLAY_"))
        {
            observed = Some((actor.id.clone(), actor.action.clone()));
            break;
        }
        if engine.is_finished() {
            break;
        }
    }
    let (actor_id, action) =
        observed.expect("matched play rule must retarget its slot actor with a PLAY_ action");
    assert_eq!(action, "PLAY_SpotUp");
    assert!(engine.physics().get_player(&actor_id).is_some());
}

#[test]
fn possession_end_clears_activation_and_starts_cooldown() {
    let mut engine = engine_with_play(4401);
    let _ = run_until_activation(&mut engine, 5000).expect("play must activate");
    let start_possession = engine.possession_for_test();
    let entries = engine.home_play_book_entries_for_test();
    assert!(
        entries
            .iter()
            .any(|entry| entry.play_id == PLAY_ID
                && entry.phase == nba_decision::PlayBookPhase::Active),
        "activation book must hold the play as Active while it runs"
    );
    // 驱动到下一次球权翻转：回合结束后激活清空、簿记进入 Cooldown。
    let mut flipped = false;
    for _ in 0..20000u64 {
        let _ = engine.step();
        if engine.is_finished() {
            break;
        }
        if engine.possession_for_test() != start_possession {
            flipped = true;
            break;
        }
    }
    assert!(flipped, "possession must flip within 20000 ticks");
    assert_eq!(
        engine.active_play_id_for_test(start_possession),
        None,
        "possession end must clear the active play"
    );
    let entries = engine.home_play_book_entries_for_test();
    assert!(
        entries.iter().any(|entry| entry.play_id == PLAY_ID
            && entry.phase == nba_decision::PlayBookPhase::Cooldown),
        "activation book must move the play to Cooldown after the possession (entries: {entries:?})"
    );
}
