use nba_domain::rules::GameRules;
use nba_sim_core::draw::{purpose, DrawKey, DrawSource, Stream};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::state::Phase;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

fn live_holder(world: &mut World) {
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        if matches!(world.phase(), Phase::Live)
            && world.holder().is_some()
            && world.interactions.is_empty()
        {
            return;
        }
    }
    panic!("no live possession with a holder");
}

/// 一次快照的各部分：每名球员的位置与速度、比赛钟、比分、tick 与持球人。
type Snapshot = (Vec<(f32, f32, f32, f32)>, f64, [u32; 2], u32, String);

fn snapshot(world: &World) -> Snapshot {
    let players = world
        .players
        .iter()
        .map(|player| (player.pos.x, player.pos.y, player.vel.x, player.vel.y))
        .collect();
    (
        players,
        world.clock.shot_clock(),
        world.score,
        world.clock.tick() as u32,
        format!("{:?}", world.phase()),
    )
}

/// 同地址抽样与调用顺序无关。
#[test]
fn draw_addresses_are_order_independent() {
    let source = DrawSource::new(7);
    let aim = DrawKey {
        stream: Stream::Aim,
        possession: 3,
        tick: 41,
        entity: 2,
        purpose: purpose::SHOT_MAKE,
        ordinal: 0,
    };
    let pass = DrawKey {
        stream: Stream::Pass,
        possession: 3,
        tick: 41,
        entity: 4,
        purpose: purpose::PASS_INTERCEPT,
        ordinal: 0,
    };
    let first_aim = source.f32(aim);
    let first_pass = source.f32(pass);
    let second_pass = source.f32(pass);
    let second_aim = source.f32(aim);
    assert_eq!(first_aim, second_aim);
    assert_eq!(first_pass, second_pass);
    assert_ne!(first_aim, first_pass);
}

/// 克隆状态与同一基础种子从同一时刻重放，逐 tick 状态相同。
#[test]
fn fork_replays_identically() {
    let mut world = world(17);
    live_holder(&mut world);
    let mut left = world.fork(world.draws.base_seed);
    let mut right = world.fork(world.draws.base_seed);
    for _ in 0..200 {
        left.step(TICK_SECONDS);
        right.step(TICK_SECONDS);
        assert_eq!(snapshot(&left), snapshot(&right));
    }
}
