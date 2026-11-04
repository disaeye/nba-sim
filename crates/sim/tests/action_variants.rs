//! 动作目录里新增技术的场景证据。
//!
//! 这一份覆盖本轮新写的三类技术：
//!
//! - 突破的三种变体（直接、变向、转身）：变体决定落点，因此路线不同。
//! - 背身步法的空间效果：上下步往底线错开，撤步转身往篮下切。
//! - 对位防守人的三种站位动作（滑步、贴身施压、站位吃犯规）：它们是
//!   防守人自己选的意图，各有自己的窗口与落点。

mod common;

use glam::Vec2;
use nba_sim_core::action::{ActionIntent, ActionKind, Fact};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::{PossessionKind, Phase};
use nba_sim_core::world::{World, TICK_SECONDS};
use nba_sim_core::{MatchSetup, Params, TeamSide};
use nba_domain::action_window::DriveKind;
use nba_domain::rules::GameRules;

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 一个活球有人持球的世界，持球人在弧顶附近。
fn live_world(seed: u64) -> World {
    let mut world = world(seed);
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live) && world.holder().is_some() {
            return world;
        }
    }
    panic!("the engine never reached a live ball state");
}

// ---------------------------------------------------------------- 突破变体

/// 起一次突破，返回结算之后的落点。
fn drive_with(seed: u64, kind: DriveKind) -> (World, Vec2) {
    let mut world = live_world(seed);
    let driver = world.holder().expect("live possession has a holder");
    let team = world.players[driver].team;
    let hoop = world.hoop_for(team);
    // 把持球人放到弧顶，把防守人放到他和篮筐之间。
    let from = hoop + Vec2::new(-22.0, 0.0);
    world.players[driver].pos = from;
    world.players[driver].target = from;
    world.ball.pos = from;
    world.ball.z = world.rules.ball_holder_height_ft;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    world.players[defender].pos = from + Vec2::new(6.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let to_ft = hoop + Vec2::new(-8.0, 0.0);
    let intent = ActionIntent {
        actor: driver,
        kind: ActionKind::Drive(kind),
        target: None,
        from_ft: from,
        to_ft,
        start_time: world.now(),
        duration: 0.8,
    };
    nba_sim_core::resolve::drive::resolve(&mut world, &intent, kind);
    let target = world.players[driver].target;
    (world, target)
}

#[test]
fn the_drive_variant_is_published_in_the_drive_fact() {
    for kind in [DriveKind::Straight, DriveKind::Crossover, DriveKind::Spin] {
        let (world, _) = drive_with(601, kind);
        let fact = world
            .events
            .iter()
            .rev()
            .find_map(|event| match &event.fact {
                Fact::Drive { kind, .. } => Some(*kind),
                _ => None,
            })
            .expect("the drive must be recorded");
        assert_eq!(fact, kind.as_str(), "突破事实必须带上技术变体");
    }
}

#[test]
fn the_drive_variants_keep_the_same_landing_but_change_the_beat_chance() {
    // 变体不改落点：落点一改，出手的区域分布整段跟着变，低位出手的位置
    // 是标定出来的。变体的区别在「怎么过去」：过掉防守人的机会不同。
    let (_, straight_target) = drive_with(602, DriveKind::Straight);
    let (_, crossover_target) = drive_with(602, DriveKind::Crossover);
    let (_, spin_target) = drive_with(602, DriveKind::Spin);
    assert_eq!(
        straight_target, crossover_target,
        "变体不改变突破的落点"
    );
    assert_eq!(straight_target, spin_target, "变体不改变突破的落点");

    // 过掉防守人的机会按变体走：变向从外侧绕过去，机会更大。
    let weights = nba_sim_core::params::Params::load(&nba_sim_core::params::workspace_calibration_dir())
        .expect("calibration tables load");
    let crossover = weights.policy.g("drive_variant_crossover_beat");
    let spin = weights.policy.g("drive_variant_spin_beat");
    assert!(
        crossover > 1.0,
        "变向突破过掉防守人的机会必须更大，实际 {crossover}"
    );
    assert!(
        spin < 1.0,
        "转身突破过掉防守人的机会必须更小，实际 {spin}"
    );
    // 转身突破把球护得更稳，因此丢球的风险更低。
    let crossover_turnover = weights.policy.g("drive_variant_crossover_turnover");
    let spin_turnover = weights.policy.g("drive_variant_spin_turnover");
    assert!(
        spin_turnover < crossover_turnover,
        "转身突破的丢球风险必须低于变向突破"
    );
}

#[test]
fn the_drive_variant_is_chosen_from_the_geometry() {
    // 防守人贴在持球人身上：变体按局面选，不是固定一种。
    let mut world = live_world(603);
    let driver = world.holder().expect("live possession has a holder");
    let team = world.players[driver].team;
    let hoop = world.hoop_for(team);
    let from = hoop + Vec2::new(-20.0, 0.0);
    world.players[driver].pos = from;
    world.players[driver].target = from;
    world.ball.pos = from;
    world.ball.z = world.rules.ball_holder_height_ft;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    // 防守人挡在持球人与篮筐之间：他挡在路上，只能绕。
    world.players[defender].pos = from + Vec2::new(2.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    nba_sim_core::defense::refresh_matchup(&mut world);
    world.possession.pairing_changed_at = world.now() + 1.0;
    world.clock.advance_simulation_time(1.0);
    nba_sim_core::defense::apply_defense_targets(&mut world);
    // 决策层在这次局面下必须给出变向突破：防守人挡在路上，只能绕。
    let state = world.decision_state();
    assert_eq!(
        nba_sim_core::policy::drive_kind_for(&world, &state),
        DriveKind::Crossover,
        "防守人挡在球与篮筐之间时必须选变向突破"
    );
}

// ------------------------------------------------------------ 背身步法效果

#[test]
fn the_post_footwork_moves_the_player_differently() {
    // 上下步与撤步转身的落点必须不同：步法不只是名字。
    let run = |offset_y: f32, defender_vel: f32| {
        let mut world = world(604);
        let offense = world.on_court(TeamSide::Home).as_slice()[0];
        let defender = world.on_court(TeamSide::Away).as_slice()[0];
        for (index, player) in world.players.iter_mut().enumerate() {
            player.on_court = index == offense || index == defender;
            player.vel = Vec2::ZERO;
        }
        let hoop = world.hoop_for(TeamSide::Home);
        let spot = hoop + Vec2::new(-8.0, offset_y);
        world.players[offense].pos = spot;
        world.players[offense].target = spot;
        world.players[offense].facing = (hoop - spot).normalize_or_zero();
        world.players[defender].pos = spot + Vec2::new(-2.0, 0.0);
        world.players[defender].target = world.players[defender].pos;
        world.players[defender].vel = Vec2::new(-defender_vel, 0.0);
        world.possession.team = TeamSide::Home;
        world.start_possession(TeamSide::Home, PossessionKind::Live);
        world.set_phase(Phase::Live);
        world.ball.set_held(offense);
        world.ball.pos = spot;
        world.ball.z = world.rules.ball_radius_ft;
        let intent = ActionIntent {
            actor: offense,
            kind: ActionKind::PostUp,
            target: None,
            from_ft: spot,
            to_ft: spot,
            start_time: world.now(),
            duration: 0.8,
        };
        nba_sim_core::resolve::drive::resolve_post(&mut world, &intent);
        world.players[offense].target
    };
    // 靠底线时是撤步转身，往篮下切。
    let drop_step = run(7.0, 9.0);
    // 不靠底线且防守人站定时是上下步，往底线错开。
    let up_and_under = run(0.0, 0.0);
    assert_ne!(
        drop_step, up_and_under,
        "撤步转身与上下步的落点必须不同"
    );
}

// -------------------------------------------------- 对位防守人的站位动作

#[test]
fn the_on_ball_defender_runs_a_positioning_action() {
    let mut world = live_world(605);
    let holder = world.holder().expect("live possession has a holder");
    let team = world.players[holder].team;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    world.players[defender].defense_responsibility =
        nba_sim_core::state::DefenseResponsibility::OnBall;
    // 防守人离得远：他会滑步过来。
    world.players[defender].pos = world.players[holder].pos + Vec2::new(12.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let mut slid = false;
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::OnBallPositioningChanged { kind: "slide", .. }
            )
        }) {
            slid = true;
            break;
        }
    }
    assert!(slid, "离得远的对位防守人必须滑步过来");
}

#[test]
fn the_positioning_action_has_its_own_window_and_target() {
    let mut world = live_world(606);
    let holder = world.holder().expect("live possession has a holder");
    let team = world.players[holder].team;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    world.players[defender].defense_responsibility =
        nba_sim_core::state::DefenseResponsibility::OnBall;
    world.players[defender].pos = world.players[holder].pos + Vec2::new(12.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let mut posture = None;
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if let Some(current) = world.players[defender].on_ball_positioning {
            posture = Some(current);
            break;
        }
    }
    let posture = posture.expect("对位防守人必须挑一个站位姿态");
    assert_eq!(
        posture,
        nba_sim_core::action::OnBallPositioning::Slide,
        "离得远的对位防守人必须选滑步"
    );
    // 姿态发布一次变化事实：它是持续的形态，不是带窗口的动作。
    let changes = world
        .events
        .iter()
        .filter(|event| {
            matches!(
                event.fact,
                Fact::OnBallPositioningChanged {
                    defender: event_defender,
                    ..
                } if event_defender == defender
            )
        })
        .count();
    assert_eq!(changes, 1, "一次姿态变化发布一条事实");
}

#[test]
fn a_tight_defender_contains_instead_of_sliding() {
    let mut world = live_world(607);
    let holder = world.holder().expect("live possession has a holder");
    let team = world.players[holder].team;
    let defender = world
        .on_court(team.other())
        .as_slice()
        .first()
        .copied()
        .expect("a defender is on the court");
    world.players[defender].defense_responsibility =
        nba_sim_core::state::DefenseResponsibility::OnBall;
    // 防守人已经在伸手的距离上：他压上去，不滑步。
    world.players[defender].pos = world.players[holder].pos + Vec2::new(3.0, 0.0);
    world.players[defender].target = world.players[defender].pos;
    let mut contained = false;
    for _ in 0..40 {
        world.step(TICK_SECONDS);
        if world.events.iter().any(|event| {
            matches!(
                &event.fact,
                Fact::OnBallPositioningChanged { kind: "contain", .. }
            )
        }) {
            contained = true;
            break;
        }
    }
    assert!(contained, "贴身的对位防守人必须贴身施压");
}
