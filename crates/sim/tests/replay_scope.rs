//! 联赛档案里的即时回放范围：节末压哨出手的自动复核。
//!
//! 复核结论有实质后果：`game_clock_at_period_end` 在档案里时，节钟归零后
//! 结算的出手由裁判核对「球离手那一刻钟还有没有走完」。钟响后离手的得分
//! 被取消；钟响前离手的得分保留。`shot_clock_at_period_end` 核对进攻钟，
//! `clock_malfunction` 确认被更正的时钟读数。范围之外的档案不产生复核事实。

mod common;

use nba_sim_core::action::{Fact, ShotKind};
use nba_sim_core::resolve::shot::{PendingShot, ShotResult};
use nba_sim_core::world::World;
use nba_sim_core::TeamSide;

/// 建一笔已经飞完、球进筐的待结算出手。
fn made_shot(world: &mut World, released_game_clock: f64) {
    let shooter = world
        .holder()
        .or_else(|| {
            world
                .players
                .iter()
                .position(|player| player.on_court && player.team == world.possession.team)
        })
        .expect("the roster has an offensive player");
    let team = world.players[shooter].team;
    let hoop = world.hoop_for(team);
    world.ball.pos = hoop;
    world.ball.z = world.rules.rim_height_ft;
    world.pending_shot = Some(PendingShot {
        shooter,
        kind: ShotKind::CatchShoot,
        zone: "rim",
        distance_ft: 3.0,
        contest: 0.0,
        fouled: false,
        and_one: false,
        free_throws: 0,
        points: 2,
        is_three: false,
        assisted_by: None,
        shot_id: 9001,
        free_throw: None,
        block_by: None,
        result: Some(ShotResult::Made),
        touched_backboard: false,
        released_game_clock,
        released_clock_checkpoint: None,
        released_event_id: 0,
        released_shot_clock: 5.0,
    });
}

fn reviews(world: &World) -> Vec<(bool, &'static str)> {
    world
        .events
        .iter()
        .filter_map(|event| match &event.fact {
            Fact::Review {
                overturned, call, ..
            } => Some((*overturned, *call)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_shot_released_after_the_horn_is_waved_off() {
    let mut world = common::short_world(901);
    world.clock.set_game_clock(0.0);
    let home = TeamSide::Home;
    let score_before = world.score[World::team_index(world.players[0].team)];
    world.possession.team = world.players[0].team;
    let _ = home;
    made_shot(&mut world, 0.0);
    nba_sim_core::resolve::shot::finish(&mut world);
    let score_after = world.score[World::team_index(world.players[0].team)];
    assert_eq!(score_after, score_before, "钟响后离手的进球必须被取消");
    assert_eq!(
        reviews(&world),
        vec![
            (true, "game_clock_at_period_end"),
            (false, "shot_clock_at_period_end"),
        ],
        "自动复核必须发出一条推翻原判的事实"
    );
}

#[test]
fn a_shot_released_before_the_horn_survives_replay() {
    let mut world = common::short_world(902);
    world.clock.set_game_clock(0.0);
    let team = world.players[0].team;
    world.possession.team = team;
    let score_before = world.score[World::team_index(team)];
    made_shot(&mut world, 0.4);
    nba_sim_core::resolve::shot::finish(&mut world);
    assert_eq!(
        world.score[World::team_index(team)],
        score_before + 2,
        "钟响前离手的进球必须保留"
    );
    assert_eq!(
        reviews(&world),
        vec![
            (false, "game_clock_at_period_end"),
            (false, "shot_clock_at_period_end"),
        ],
        "自动复核必须发出一条维持原判的事实"
    );
}

#[test]
fn a_profile_without_period_end_replay_does_not_review() {
    let mut world = common::short_world(903);
    world
        .rules
        .league
        .review
        .review_scope
        .retain(|scope| scope != "game_clock_at_period_end" && scope != "shot_clock_at_period_end");
    world.clock.set_game_clock(0.0);
    let team = world.players[0].team;
    world.possession.team = team;
    let score_before = world.score[World::team_index(team)];
    made_shot(&mut world, 0.0);
    nba_sim_core::resolve::shot::finish(&mut world);
    assert!(reviews(&world).is_empty(), "范围之外不得凭空产生复核事实");
    assert_eq!(
        world.score[World::team_index(team)],
        score_before + 2,
        "没有复核程序时按原出手机制结算"
    );
}

#[test]
fn the_overturned_period_end_ruling_restores_the_game_clock() {
    // 复核否定了「钟响后离手」这个认定，本节剩余时间因此必须恢复：
    // 比赛钟从出手那一刻的检查点恢复，而不是停在零。
    let mut world = common::short_world(904);
    let team = world.players[0].team;
    world.possession.team = team;
    let shooter = world
        .holder()
        .or_else(|| {
            world
                .players
                .iter()
                .position(|player| player.on_court && player.team == team)
        })
        .expect("the roster has an offensive player");
    world.set_ball_held(shooter);
    // 出手那一刻的比赛钟是 0.4 秒。
    world.clock.set_game_clock(0.4);
    // 走生产出手路径：检查点由出手结算自己建立。
    let intent = nba_sim_core::action::ActionIntent {
        actor: shooter,
        kind: nba_sim_core::action::ActionKind::Shoot(ShotKind::CatchShoot),
        target: None,
        from_ft: world.players[shooter].pos,
        to_ft: world.players[shooter].pos,
        start_time: world.now(),
        duration: 0.4,
    };
    nba_sim_core::resolve::shot::resolve(
        &mut world,
        &intent,
        ShotKind::CatchShoot,
    );
    let checkpoint = world
        .pending_shot
        .as_ref()
        .and_then(|pending| pending.released_clock_checkpoint);
    assert!(checkpoint.is_some(), "出手必须留下比赛钟检查点");
    // 球飞到篮筐时比赛钟已经归零。
    world.clock.set_game_clock(0.0);
    let mut pending = world
        .pending_shot
        .take()
        .expect("the shot is pending settlement");
    pending.result = Some(ShotResult::Made);
    world.pending_shot = Some(pending);
    nba_sim_core::resolve::shot::finish(&mut world);
    assert_eq!(
        reviews(&world),
        vec![
            (false, "game_clock_at_period_end"),
            (false, "shot_clock_at_period_end"),
            (false, "clock_malfunction"),
        ],
        "钟响前离手的进球必须经过复核并维持原判"
    );
    assert!(
        world.clock.game_clock() > 0.0,
        "改判成立时比赛钟必须恢复：实际 {:.3}",
        world.clock.game_clock()
    );
}
