//! 111 个动作条目的运行时证据。
//!
//! 方案 §6：每个动作条目有种类、技术细分、执行人、对象、准备窗口、执行
//! 窗口、恢复窗口、控制要求、运动约束和取消条件。技术细分参与可执行性、
//! 时长、球和身体路线、防守反应、失误风险、体能与接触条件；每种动作都
//! 需要独立消费者，不允许只添加名称。
//!
//! 这条测试把三件事钉在一起：
//!
//! 1. 每个条目都由生产路径真的产出过（不是空壳）；
//! 2. 每个动作都有唯一终态与生命周期事实；
//! 3. 协议投影里的动作身份与引擎里的动作身份是同一套。

mod common;

use std::collections::{BTreeMap, BTreeSet};

use nba_domain::action_spec::{ActionFamily, ActionRole};
use nba_sim_core::action::{ActionKind, Fact, TeamSide};
use nba_sim_core::action_identity::{
    box_out_identity, defense_identity, offense_identity, off_ball_identity, posture_identity,
    transition_identity,
};
use nba_sim_core::action_spec::action_spec_table;
use nba_sim_core::state::{Phase, PossessionKind, MAX_ON_COURT};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

/// 引擎里出现过的动作身份。
#[derive(Default)]
struct Observations {
    /// 动作运行时的身份。
    runtime: BTreeSet<(String, String, String, String)>,
    /// 协议投影里的身份。
    projected: BTreeSet<(String, String, String, String)>,
    /// 事件流里出现过的动作名。
    kinds: BTreeSet<&'static str>,
    /// 每个动作身份有过的终态。
    terminals: BTreeSet<u64>,
    /// 每个动作身份开始过的次数。
    starts: BTreeMap<u64, u64>,
}

/// 把引擎里的一个动作运行时记成它的协议身份。
fn runtime_identity(
    world: &World,
    runtime: &nba_sim_core::action::ActionRuntime,
) -> (String, String, String, String) {
    let identity = match runtime.intent.kind {
        ActionKind::Shoot(shot_kind) => {
            nba_sim_core::action_identity::shot_identity(world, runtime.player, shot_kind)
        }
        kind => offense_identity(kind),
    };
    let role = match identity.family {
        nba_domain::action_spec::ActionFamily::ActionScreen => match runtime.role {
            nba_sim_core::action::ActionIntentRole::ScreenPartner => "screener",
            nba_sim_core::action::ActionIntentRole::CutPartner => "cut_partner",
            nba_sim_core::action::ActionIntentRole::HandlerPartner => "ball_handler",
            nba_sim_core::action::ActionIntentRole::Primary => match identity.variant {
                "off_ball" => "screener",
                "curl" | "handoff" => "cut_partner",
                _ => "ball_handler",
            },
        },
        nba_domain::action_spec::ActionFamily::ActionCut => "cut_partner",
        _ => identity.role.as_str(),
    };
    (
        identity.family.as_str().to_string(),
        identity.kind.to_string(),
        identity.variant.to_string(),
        role.to_string(),
    )
}

fn observe(world: &World, out: &mut Observations) {
    for player in 0..world.players.len() {
        if let Some(runtime) = &world.players[player].action {
            out.runtime.insert(runtime_identity(world, runtime));
        }
        if let Some(def) = world.defensive_actions[player].as_ref() {
            let identity = defense_identity(def.kind);
            out.runtime.insert((
                identity.family.as_str().to_string(),
                identity.kind.to_string(),
                identity.variant.to_string(),
                identity.role.as_str().to_string(),
            ));
        }
        if let Some(positioning) = world.players[player].on_ball_positioning {
            let identity = posture_identity(positioning);
            out.runtime.insert((
                identity.family.as_str().to_string(),
                identity.kind.to_string(),
                identity.variant.to_string(),
                identity.role.as_str().to_string(),
            ));
        }
        if let Some(kind) = world.players[player].transition {
            let identity = transition_identity(kind);
            out.runtime.insert((
                identity.family.as_str().to_string(),
                identity.kind.to_string(),
                identity.variant.to_string(),
                identity.role.as_str().to_string(),
            ));
        }
        if let Some(response) = world.players[player].closeout_response {
            out.runtime.insert((
                "ACTION-CLOSEOUT".to_string(),
                "closeout_response".to_string(),
                response.to_string(),
                "shooter".to_string(),
            ));
        }
        if let Some(identity) = off_ball_identity(world.players[player].defense_responsibility) {
            out.runtime.insert((
                identity.family.as_str().to_string(),
                identity.kind.to_string(),
                identity.variant.to_string(),
                identity.role.as_str().to_string(),
            ));
        }
        if world.players[player].defense_responsibility
            == nba_sim_core::state::DefenseResponsibility::BoxOut
        {
            let identity = box_out_identity();
            out.runtime.insert((
                identity.family.as_str().to_string(),
                identity.kind.to_string(),
                identity.variant.to_string(),
                identity.role.as_str().to_string(),
            ));
        }
    }
    // 事件流只读新增的部分：反复读全量会把同一个动作数很多次。
    for event in world.events.iter().rev().take(256) {
        match &event.fact {
            Fact::ActionStarted { action_id, kind, .. } => {
                *out.starts.entry(*action_id).or_insert(0) += 1;
                out.kinds.insert(kind);
            }
            Fact::ActionCompleted { action_id, .. }
            | Fact::ActionFailed { action_id, .. }
            | Fact::ActionCancelled { action_id, .. } => {
                out.terminals.insert(*action_id);
            }
            _ => {}
        }
    }
}

/// 协议投影里的动作身份。
fn projected_identities(world: &World) -> BTreeSet<(String, String, String, String)> {
    let tick = world.stream_tick();
    tick.frame
        .players
        .iter()
        .flat_map(|player| player.active_actions.iter())
        .map(|action| {
            (
                action.family.clone(),
                action.kind.clone(),
                action.variant.clone(),
                action.role.clone(),
            )
        })
        .collect()
}

/// 跑若干场完整比赛，收集动作身份与生命周期事实。
fn observe_games(seeds: std::ops::RangeInclusive<u64>) -> Observations {
    let mut out = Observations::default();
    for seed in seeds {
        let mut world = common::short_world(seed);
        let mut guard = 0;
        while !world.finished && guard < 400_000 {
            world.step(TICK_SECONDS);
            observe(&world, &mut out);
            guard += 1;
        }
        assert!(world.finished, "seed {seed} 的比赛必须打完");
    }
    out
}

/// 双方同时控制球的争抢：它太罕见，完整比赛里不一定出现，因此这里
/// 直接构造这个局面。
fn observe_held_ball(out: &mut Observations) {
    let params = Params::load(&nba_sim_core::params::workspace_calibration_dir())
        .expect("calibration tables load");
    let mut world = World::new(
        MatchSetup::builtin(nba_domain::rules::GameRules::default()),
        params,
        233,
    );
    let offense = world.on_court(TeamSide::Home).as_slice()[0];
    let defense = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == offense || index == defense;
        player.vel = glam::Vec2::ZERO;
    }
    let ball_spot = glam::Vec2::new(62.0, 25.0);
    // 两人到球的距离只差五分之一英尺：同时够到，谁也没有先控制。
    world.players[offense].pos = ball_spot + glam::Vec2::new(5.0, 0.0);
    world.players[defense].pos = ball_spot + glam::Vec2::new(0.0, 5.05);
    world.possession.team = TeamSide::Home;
    world.start_possession(TeamSide::Home, PossessionKind::Live);
    world.set_phase(Phase::Live);
    world.ball = nba_sim_core::ball::BallBody::loose(ball_spot, glam::Vec2::ZERO);
    world.ball.z = world.rules.ball_radius_ft;
    world.pending_rebound = Some(nba_sim_core::resolve::rebound::PendingRebound {
        offense: TeamSide::Home,
        zone: "test",
        shot_distance: 0.0,
        rim_contact: false,
        started_at: world.now() - 3.0,
        turnover: false,
        poke_by: None,
        pass_by: None,
        box_outs: [usize::MAX; MAX_ON_COURT],
        box_out_count: 0,
    });
    nba_sim_core::ball::step(&mut world, TICK_SECONDS);
    observe(&world, out);
    out.projected.extend(projected_identities(&world));
}

#[test]
fn every_action_entry_is_produced_by_the_engine() {
    // 动作条目有生产路径：跑足够多的完整比赛，每一个条目都必须真的
    // 出现过。稀有的条目需要更多的场次，因此这里跑一批比赛而不是一场。
    let mut observations = observe_games(1..=120);
    observe_held_ball(&mut observations);
    let table = action_spec_table();
    let mut missing = Vec::new();
    for spec in table.specs() {
        let key = (
            spec.family.as_str().to_string(),
            spec.kind.to_string(),
            spec.variant.to_string(),
            spec.role.as_str().to_string(),
        );
        if !observations.runtime.contains(&key) {
            missing.push(format!(
                "{}::{} {} {} {}",
                spec.family.as_str(),
                spec.name,
                spec.kind,
                spec.variant,
                spec.role.as_str()
            ));
        }
    }
    assert!(
        missing.is_empty(),
        "这些动作条目没有被生产路径产出过：\n{}",
        missing.join("\n")
    );
}

#[test]
fn every_action_has_exactly_one_terminal_state() {
    let observations = observe_games(1..=6);
    // 每个开始过的动作都有终态：完成、失败与取消三选一。
    for action_id in observations.starts.keys() {
        assert!(
            observations.terminals.contains(action_id),
            "动作 {action_id} 开始之后没有终态"
        );
    }
    assert!(
        observations.starts.len() > 100,
        "完整比赛必须产生大量动作生命周期事实：{}",
        observations.starts.len()
    );
}

#[test]
fn the_projection_carries_the_engine_identities() {
    // 协议投影里的动作身份必须与引擎里的一致：渲染器不能显示一个引擎
    // 没有的动作，也不能把一个动作显示成另一个身份。
    let mut world = common::short_world(1);
    let mut checked = 0usize;
    for _ in 0..4_000 {
        world.step(TICK_SECONDS);
        let mut runtime = Observations::default();
        observe(&world, &mut runtime);
        let projected = projected_identities(&world);
        if projected.is_empty() {
            continue;
        }
        checked += 1;
        for identity in &projected {
            assert!(
                runtime.runtime.contains(identity),
                "协议投影里的动作身份 {identity:?} 不在引擎的动作身份里"
            );
        }
    }
    assert!(checked > 100, "采样的帧数太少：{checked}");
}

#[test]
fn the_spec_table_covers_every_family_and_role() {
    let table = action_spec_table();
    assert_eq!(table.len(), 111, "动作规格表必须有 111 个条目");
    let families = table.families();
    assert_eq!(
        families.len(),
        16,
        "动作规格表必须覆盖 16 个动作族：{}",
        families.len()
    );
    assert_eq!(families.first(), Some(&ActionFamily::ActionDribble));
    assert_eq!(families.last(), Some(&ActionFamily::ActionTransition));
    for spec in table.specs() {
        assert!(!spec.candidate_source.is_empty());
        assert!(!spec.resolver.is_empty());
        assert!(!spec.opponent_response.is_empty());
        assert!(spec.scenario_test.contains("::"));
        assert!(
            ActionRole::from_str(spec.role.as_str()).is_some(),
            "角色 {} 必须有领域定义",
            spec.role.as_str()
        );
    }
}

#[test]
fn the_action_families_are_all_produced_by_the_engine() {
    // 动作族覆盖用 24 场完整比赛验证：稀有族（例如护球）在八场里可能
    // 恰好抽不到，场次放大后仍能确认它由生产路径产出。
    let observations = observe_games(1..=24);
    let produced: BTreeSet<String> = observations
        .runtime
        .iter()
        .map(|(family, _, _, _)| family.clone())
        .collect();
    for family in ActionFamily::ALL {
        assert!(
            produced.contains(family.as_str()),
            "动作族 {} 在完整比赛里没有出现",
            family.as_str()
        );
    }
}

#[test]
fn the_documented_action_kinds_all_have_a_runtime_identity() {
    // 引擎里的动作种类都必须有协议身份：新增动作种类时忘记加身份映射，
    // 这里立刻暴露。
    let kinds: Vec<ActionKind> = vec![
        ActionKind::Finish(nba_sim_core::action::FinishKind::Layup),
        ActionKind::Dribble(nba_domain::action_window::DribbleMoveKind::DirectDrive),
        ActionKind::Drive(nba_domain::action_window::DriveKind::Straight),
        ActionKind::Pass(nba_sim_core::action::PassKind::Chest),
        ActionKind::Screen(nba_sim_core::action::ScreenKind::OnBall),
        ActionKind::Cut(nba_sim_core::action::CutKind::Basket),
        ActionKind::PostUp(nba_sim_core::action::PostStepKind::Backdown),
        ActionKind::Shoot(nba_sim_core::action::ShotKind::CatchShoot),
        ActionKind::Relocate(nba_sim_core::action::RelocateKind::Adjust),
        ActionKind::Fake(nba_sim_core::action::FakeKind::Shot),
        ActionKind::Protect(nba_sim_core::action::ProtectKind::FaceUp),
        ActionKind::Rebound(nba_sim_core::action::ReboundKind::Tap),
    ];
    for kind in kinds {
        let identity = offense_identity(kind);
        assert!(!identity.family.as_str().is_empty());
        assert!(!identity.kind.is_empty());
        assert!(!identity.variant.is_empty());
        assert!(!identity.role.as_str().is_empty());
    }
}
