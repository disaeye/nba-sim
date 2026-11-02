//! 外部基准：固定能力、位置、无干扰条件下的命中率。
//!
//! 两层检查：
//! 1. 模型预测的命中率落在实测曲线的参考区间内（±0.05）：模型必须复现
//!    联盟在这个出手类型与距离上的真实命中率。
//! 2. 实际飞行结果落在模型预测的统计误差内：价值层与执行层描述同一件事。
//!
//! 参考值取自运行时表 `shot_distance.json`（逐出手类型、逐距离档的实测
//! 命中率）。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, ShotKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{BallPhase, BallState, DribbleState, Phase};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, TeamSide, World};

struct Condition {
    name: &'static str,
    distance: f32,
    kind: ShotKind,
    phase: BallPhase,
    /// 该出手类型在这个距离上的实测命中率。
    reference: f32,
}

fn fixture(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn setup(world: &mut World, _condition: &Condition) -> usize {
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    for player in world.players.iter_mut() {
        player.on_court = false;
        player.vel = Vec2::ZERO;
    }
    world.players[shooter].on_court = true;
    world.players[shooter].stamina = 1.0;
    world.players[shooter].data.attributes.shooting_close = 0.5;
    world.players[shooter].data.attributes.shooting_near = 0.5;
    world.players[shooter].data.attributes.shooting_mid = 0.5;
    world.players[shooter].data.attributes.shooting_three = 0.5;
    world.players[shooter].data.attributes.free_throw = 0.5;
    world.possession.team = TeamSide::Home;
    world.set_phase(Phase::Live);
    world.pending_shot = None;
    shooter
}

fn shoot_once(world: &mut World, condition: &Condition, shooter: usize) -> (bool, f32) {
    let hoop = world.hoop_for(TeamSide::Home);
    // 参考值的距离轴是出手点到篮筐的距离，也就是释放点的距离；站位点
    // 需要再退一个持球偏移，释放点才会落在参考的距离上。
    let spot = hoop
        + Vec2::new(
            -(condition.distance + world.rules.ball_holder_offset_ft),
            0.0,
        );
    world.players[shooter].pos = spot;
    world.players[shooter].facing = (hoop - spot).normalize_or_zero();
    world.possession.ball_phase = condition.phase;
    world.possession.dribble_state = DribbleState::Dribbling;
    world.possession.gather_pos = None;
    world.ball.set_held(shooter);
    let model =
        nba_sim_core::shot_quality::evaluate(world, shooter, condition.kind, condition.phase, spot);
    let intent = ActionIntent {
        actor: shooter,
        kind: ActionKind::Shoot(condition.kind),
        target: None,
        from_ft: spot,
        to_ft: spot,
        start_time: world.now(),
        duration: 0.2,
    };
    nba_sim_core::resolve::shot::resolve(world, &intent, condition.kind);
    for _ in 0..1200 {
        world.clock.advance_simulation_time(TICK_SECONDS);
        nba_sim_core::ball::step(world, TICK_SECONDS);
        if let Some(pending) = world.pending_shot.as_ref() {
            if let Some(result) = pending.result {
                return (
                    result == nba_sim_core::resolve::shot::ShotResult::Made,
                    model.p_make,
                );
            }
        }
    }
    panic!("shot did not resolve");
}

fn measure(condition: &Condition, attempts: u32, seed: u64) {
    let mut world = fixture(seed);
    let shooter = setup(&mut world, condition);
    let mut made = 0u32;
    let mut predicted = 0.0f64;
    for _ in 0..attempts {
        world.ball.release(BallState::Loose {
            pos: world.hoop_for(TeamSide::Home),
            vel: Vec2::ZERO,
        });
        world.pending_shot = None;
        world.clock.advance_tick();
        let (hit, p) = shoot_once(&mut world, condition, shooter);
        made += u32::from(hit);
        predicted += f64::from(p);
    }
    let observed = made as f64 / f64::from(attempts);
    let predicted = predicted / f64::from(attempts);
    // 第一层：模型必须复现实测命中率。
    assert!(
        (predicted - f64::from(condition.reference)).abs() <= 0.05,
        "{}: model {predicted:.4} misses the measured reference {:.4}",
        condition.name,
        condition.reference
    );
    // 第二层：实际飞行必须与模型一致。
    let error = (predicted * (1.0 - predicted) / f64::from(attempts)).sqrt();
    let band = 3.0 * error + 0.02;
    println!(
        "{}: observed {observed:.4} predicted {predicted:.4} reference {:.4} band ±{band:.4}",
        condition.name, condition.reference
    );
    assert!(
        (observed - predicted).abs() <= band,
        "{}: observed {observed:.4} vs predicted {predicted:.4} exceeds ±{band:.4}",
        condition.name
    );
}

#[test]
fn fixed_open_shots_match_the_measured_reference_rates() {
    let conditions = [
        Condition {
            name: "篮下上篮 2 ft",
            distance: 2.0,
            kind: ShotKind::Layup,
            phase: BallPhase::Catch,
            reference: 0.703782,
        },
        Condition {
            name: "近筐抛投 8 ft",
            distance: 8.0,
            kind: ShotKind::Floater,
            phase: BallPhase::Catch,
            reference: 0.439215,
        },
        Condition {
            name: "中距离跳投 18 ft",
            distance: 18.0,
            kind: ShotKind::PullUp,
            phase: BallPhase::Dribble,
            reference: 0.405816,
        },
        Condition {
            name: "三分线接球投 25 ft",
            distance: 25.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            reference: 0.383408,
        },
    ];
    for (index, condition) in conditions.iter().enumerate() {
        measure(condition, 3000, 700 + index as u64);
    }
}

#[test]
fn free_throw_matches_the_measured_reference() {
    let mut world = fixture(750);
    let shooter = setup(
        &mut world,
        &Condition {
            name: "罚球",
            distance: 13.75,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            reference: 0.75,
        },
    );
    // 罚球站在罚球线上。
    let hoop = world.hoop_for(TeamSide::Home);
    world.players[shooter].pos = hoop + Vec2::new(-13.75, 0.0);
    world.players[shooter].facing = (hoop - world.players[shooter].pos).normalize_or_zero();
    let mut made = 0u32;
    let attempts = 3000u32;
    for _ in 0..attempts {
        world.clock.advance_tick();
        world.pending_shot = None;
        world.possession.ball_phase = BallPhase::Catch;
        world.ball.set_held(shooter);
        nba_sim_core::resolve::freethrow::shoot(&mut world, shooter, 1, 1, 0, false, false, 1);
        for _ in 0..1200 {
            world.clock.advance_simulation_time(TICK_SECONDS);
            nba_sim_core::ball::step(&mut world, TICK_SECONDS);
            if let Some(pending) = world.pending_shot.as_ref() {
                if let Some(result) = pending.result {
                    made += u32::from(result == nba_sim_core::resolve::shot::ShotResult::Made);
                    break;
                }
            }
        }
    }
    let observed = made as f64 / f64::from(attempts);
    println!("罚球: observed {observed:.4} reference 0.7500");
    assert!(
        (observed - 0.75).abs() <= 0.05,
        "free throw rate {observed:.4} misses the reference 0.75"
    );
}
