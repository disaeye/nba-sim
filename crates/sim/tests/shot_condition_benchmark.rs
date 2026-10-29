//! 常规比赛条件下的几何基准。
//!
//! 固定位置、无干扰的外部基准（`shot_geometry_benchmark`）只检验理想条件下
//! 的模型正确性；这份基准检验比赛里真正会出现的条件：
//!
//! - 三分线上的接球投；
//! - 延长三分（30 ft 与 40 ft）；
//! - 移动中的接球投（出手时球手仍在移动）；
//! - 运球急停跳投；
//! - 四档防守干扰（松、一般、紧、封盖）。
//!
//! 每个条件下，模型预测的命中率与实际飞行结果必须落在统计误差内。

use glam::Vec2;
use nba_domain::rules::GameRules;
use nba_sim_core::action::{ActionIntent, ActionKind, ShotKind};
use nba_sim_core::params::{workspace_calibration_dir, Params};
use nba_sim_core::state::{BallPhase, BallState, DribbleState, Phase};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, TeamSide, World};

/// 防守干扰的四档：球手在防守人身上的相对位置。
///
/// 干扰由三维手部几何决定，所以这里用「防守人站在球手与篮筐之间的哪个
/// 侧、离多远」来构造，而不是直接写一个干扰数值。距离与横向偏移取真实
/// 防守位置：真���防守人总是斜着站位封盖，不会站在投篮线正后方把每一次
/// 出手都挡下来。
fn defender_spot(shooter: Vec2, hoop: Vec2, level: u8) -> Vec2 {
    let to_hoop = (hoop - shooter).normalize_or_zero();
    let side = Vec2::new(-to_hoop.y, to_hoop.x);
    match level {
        0 => shooter + to_hoop * 8.0 + side * 5.0,
        1 => shooter + to_hoop * 5.0 + side * 3.5,
        2 => shooter + to_hoop * 3.5 + side * 3.0,
        _ => shooter + to_hoop * 3.0 + side * 1.5,
    }
}

struct Condition {
    name: &'static str,
    distance: f32,
    kind: ShotKind,
    phase: BallPhase,
    /// 出手时球手的移动速度（ft/s）。
    speed: f32,
    /// 防守干扰档位。
    contest_level: u8,
}

fn fixture(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 摆好一次出手：只有持球人与一名防守人在场上，其余球员离场。
fn setup(world: &mut World, condition: &Condition) -> (usize, usize) {
    let shooter = world.on_court(TeamSide::Home).as_slice()[0];
    let defender = world.on_court(TeamSide::Away).as_slice()[0];
    for (index, player) in world.players.iter_mut().enumerate() {
        player.on_court = index == shooter || index == defender;
        player.vel = Vec2::ZERO;
    }
    let hoop = world.hoop_for(TeamSide::Home);
    let spot = hoop + Vec2::new(-condition.distance, 0.0);
    world.players[shooter].pos = spot;
    world.players[shooter].target = spot;
    world.players[shooter].facing = (hoop - spot).normalize_or_zero();
    world.players[shooter].stamina = 1.0;
    world.players[shooter].data.attributes.shooting_close = 0.5;
    world.players[shooter].data.attributes.shooting_near = 0.5;
    world.players[shooter].data.attributes.shooting_mid = 0.5;
    world.players[shooter].data.attributes.shooting_three = 0.5;
    world.players[shooter].data.attributes.free_throw = 0.5;
    let defender_spot = defender_spot(spot, hoop, condition.contest_level);
    world.players[defender].pos = defender_spot;
    world.players[defender].target = defender_spot;
    world.players[defender].facing = (spot - defender_spot).normalize_or_zero();
    world.possession.team = TeamSide::Home;
    world.phase = Phase::Live;
    world.pending_shot = None;
    (shooter, defender)
}

/// 出手一次，返回（是否命中，模型预测的命中率，封盖是否发生，模型预测的封盖率）。
fn shoot_once(
    world: &mut World,
    condition: &Condition,
    shooter: usize,
) -> (bool, f32, bool, f32) {
    let hoop = world.hoop_for(TeamSide::Home);
    // 移动中的出手：球手有速度，瞄准误差因此变大。
    let direction = (hoop - world.players[shooter].pos).normalize_or_zero();
    world.players[shooter].vel = direction * condition.speed;
    // 出手结算会把持球阶段改写，因此每次出手前必须恢复本次条件的阶段。
    world.possession.ball_phase = condition.phase;
    world.possession.dribble_state = DribbleState::Dribbling;
    world.possession.gather_pos = None;
    world.ball.set_held(shooter);
    let model = nba_sim_core::shot_quality::evaluate(
        world,
        shooter,
        condition.kind,
        condition.phase,
        world.players[shooter].pos,
    );
    let predicted = model.p_make;
    let predicted_block = model.block_risk;
    let intent = ActionIntent {
        actor: shooter,
        kind: ActionKind::Shoot(condition.kind),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.2,
    };
    nba_sim_core::resolve::shot::resolve(world, &intent, condition.kind);
    for _ in 0..1200 {
        nba_sim_core::ball::step(world, TICK_SECONDS);
        world.wall += TICK_SECONDS;
        if let Some(pending) = world.pending_shot.as_ref() {
            if let Some(result) = pending.result {
                let made = result == nba_sim_core::resolve::shot::ShotResult::Made;
                let blocked = matches!(
                    result,
                    nba_sim_core::resolve::shot::ShotResult::Blocked { .. }
                );
                return (made, predicted, blocked, predicted_block);
            }
        }
    }
    panic!("shot did not resolve");
}

fn measure(condition: &Condition, attempts: u32, seed: u64) {
    let mut world = fixture(seed);
    let (shooter, _defender) = setup(&mut world, condition);
    let mut made = 0u32;
    let mut blocked = 0u32;
    let mut predicted = 0.0f64;
    let mut predicted_block = 0.0f64;
    for _ in 0..attempts {
        // 每次出手后球权与球位要复位：上一手的球仍是飞行状态。
        world.ball.release(BallState::Loose {
            pos: world.hoop_for(TeamSide::Home),
            vel: Vec2::ZERO,
        });
        world.pending_shot = None;
        world.clock.tick += 1;
        let (hit, p, was_blocked, p_block) = shoot_once(&mut world, condition, shooter);
        made += u32::from(hit);
        blocked += u32::from(was_blocked);
        predicted += f64::from(p);
        predicted_block += f64::from(p_block);
    }
    let observed = made as f64 / f64::from(attempts);
    let predicted = predicted / f64::from(attempts);
    let block_rate = blocked as f64 / f64::from(attempts);
    let predicted_block = predicted_block / f64::from(attempts);
    // 封盖是球飞到手上才发生的事件：它从命中里扣掉，因此预测的命中概率
    // 要按实际封盖率折算后才与实测可比。
    let effective = predicted * (1.0 - block_rate);
    // 二项抽样误差加一个小的模型容差。
    let error = (effective * (1.0 - effective) / f64::from(attempts)).sqrt();
    let band = 3.0 * error + 0.02;
    println!(
        "{}: observed {observed:.4} predicted {predicted:.4} block {block_rate:.4} (model {predicted_block:.4}) effective {effective:.4} band ±{band:.4}",
        condition.name
    );
    assert!(
        (observed - effective).abs() <= band,
        "{}: observed {observed:.4} vs effective {effective:.4} exceeds ±{band:.4}",
        condition.name
    );
}

#[test]
fn game_condition_shots_match_the_model() {
    let conditions = [
        Condition {
            name: "三分线接球投",
            distance: 23.75,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 0.0,
            contest_level: 1,
        },
        Condition {
            name: "延长三分 30 ft",
            distance: 30.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 0.0,
            contest_level: 1,
        },
        Condition {
            name: "延长三分 40 ft",
            distance: 40.0,
            kind: ShotKind::PullUp,
            phase: BallPhase::Dribble,
            speed: 0.0,
            contest_level: 1,
        },
        Condition {
            name: "移动接球投",
            distance: 24.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 12.0,
            contest_level: 1,
        },
        Condition {
            name: "运球急停跳投",
            distance: 22.0,
            kind: ShotKind::PullUp,
            phase: BallPhase::Dribble,
            speed: 0.0,
            contest_level: 1,
        },
        Condition {
            name: "松对抗接球投",
            distance: 24.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 0.0,
            contest_level: 0,
        },
        Condition {
            name: "紧对抗接球投",
            distance: 24.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 0.0,
            contest_level: 2,
        },
        Condition {
            name: "封盖干扰接球投",
            distance: 24.0,
            kind: ShotKind::CatchShoot,
            phase: BallPhase::Catch,
            speed: 0.0,
            contest_level: 3,
        },
        Condition {
            name: "紧干扰两分急停",
            distance: 8.0,
            kind: ShotKind::PullUp,
            phase: BallPhase::Dribble,
            speed: 0.0,
            contest_level: 2,
        },
    ];
    for (index, condition) in conditions.iter().enumerate() {
        measure(condition, 3000, 900 + index as u64);
    }
}
