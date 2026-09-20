//! 世界状态的直接等价性守卫。
//!
//! ## 为什么需要这个文件
//!
//! 黄金哈希（`golden_hash.rs`）与统计基线（`stats_baseline.rs`）都是**间接**
//! 观察：一个看的是 2000 tick 的哈希，一个看的是 8 场聚合。当一次改动的
//! 目标是「删掉一块重复实现」时，间接观察无法排除「被删的路径其实在某处
//! 生效」——它在 2000 tick 内可能一次都没被执行。
//!
//! 本文件用直接断言说明：引擎在任意 tick 的**全部权威状态**（时钟、比分、
//! 犯规、球态、球位、每个球员的位置/速度/体力/犯规数）都能由公开的只读投影
//! 读出，且读数与引擎自身的访问器一致。这些读数正是「平行世界副本」应当
//! 载有的内容；副本被删除后，等价性由这里持续守住。
//!
//! 纪律：本文件只读公开访问器，不触碰任何内部字段。

use nba_engine::MatchEngine;
use nba_protocol::StreamTick;

/// 从一帧与快照中抽取权威状态的可比较摘要。
#[derive(Debug, PartialEq)]
struct WorldDigest {
    tick_index: u64,
    period: u32,
    game_clock: f32,
    shot_clock: f32,
    current_time: f32,
    home_score: u32,
    away_score: u32,
    possession_id: u32,
    team_fouls_home: u32,
    team_fouls_away: u32,
    ball_state: String,
    ball_x: f32,
    ball_y: f32,
    ball_z: f32,
    players: Vec<PlayerDigest>,
}

#[derive(Debug, PartialEq)]
struct PlayerDigest {
    id: String,
    x: f32,
    y: f32,
    velocity: f32,
    stamina: f32,
    foul_count: u8,
    on_court: bool,
}

fn digest(engine: &MatchEngine, tick: &StreamTick) -> WorldDigest {
    let snap = engine.engine_snapshot();
    let players = {
        let mut ids: Vec<String> = snap
            .lineups
            .home_roster_order
            .iter()
            .chain(snap.lineups.away_roster_order.iter())
            .cloned()
            .collect();
        ids.sort();
        ids.into_iter()
            .filter_map(|id| {
                snap.get_player(&id).map(|p| PlayerDigest {
                    id: id.clone(),
                    x: p.pos_ft.x,
                    y: p.pos_ft.y,
                    velocity: p.vel_ft.length(),
                    stamina: p.stamina,
                    foul_count: p.foul_count,
                    on_court: p.on_court,
                })
            })
            .collect()
    };
    WorldDigest {
        tick_index: engine.tick_index(),
        period: tick.frame.period,
        game_clock: tick.frame.t_game,
        shot_clock: tick.frame.shot_clock,
        current_time: tick.frame.t,
        home_score: snap.game.home_score,
        away_score: snap.game.away_score,
        possession_id: snap.game.possession_id,
        team_fouls_home: tick.frame.team_fouls_home,
        team_fouls_away: tick.frame.team_fouls_away,
        ball_state: tick.frame.ball.status.clone(),
        ball_x: tick.frame.ball.x,
        ball_y: tick.frame.ball.y,
        ball_z: tick.frame.ball.z,
        players,
    }
}

/// 一帧的权威状态必须与引擎自身的访问器读数一致。
///
/// 两者来自不同的投影路径（`StreamTick` 由 `build_tick` 组装，访问器读
/// 状态组），任何一条路径读到了过期的副本都会在这里暴露。
///
/// `t` / `t_game` / `shot_clock` 三个字段在帧里带渲染舍入
/// （`projection.rs` 分别乘 100/10/10 后取整），因此这里在同一精度上比较。
#[test]
fn tick_projection_matches_engine_accessors() {
    let mut engine = MatchEngine::new(42);
    for step in 0..3000 {
        let tick = engine.step();
        assert_eq!(
            tick.frame.t_game,
            (engine.game_clock() * 10.0).round() / 10.0,
            "step {step}: frame game clock disagrees with accessor"
        );
        assert_eq!(
            tick.frame.shot_clock,
            (engine.shot_clock() * 10.0).round() / 10.0,
            "step {step}: frame shot clock disagrees with accessor"
        );
        assert_eq!(
            tick.frame.t,
            (engine.current_time() * 100.0).round() / 100.0,
            "step {step}: frame elapsed time disagrees with accessor"
        );
        assert_eq!(
            tick.frame.period,
            engine.period(),
            "step {step}: frame period disagrees with accessor"
        );
        assert_eq!(
            tick.frame.score.home,
            engine.home_score(),
            "step {step}: frame home score disagrees with accessor"
        );
        assert_eq!(
            tick.frame.score.away,
            engine.away_score(),
            "step {step}: frame away score disagrees with accessor"
        );
        assert_eq!(
            tick.frame.possession_id,
            engine.possession_id(),
            "step {step}: frame possession id disagrees with accessor"
        );
    }
}

/// 同一场比赛的两次运行必须产生逐字段相同的世界状态序列。
///
/// 这不是黄金哈希的重复：哈希把整段压成一个数字，失败时不指出差异；本测试
/// 指出第一个分歧的 tick 与字段。
#[test]
fn world_state_sequence_is_reproducible() {
    let mut a = MatchEngine::new(7);
    let mut b = MatchEngine::new(7);
    for step in 0..3000 {
        let ta = a.step();
        let tb = b.step();
        assert_eq!(
            digest(&a, &ta),
            digest(&b, &tb),
            "same seed diverged at step {step}"
        );
    }
}

/// 球员的权威运动状态必须能由只读投影读出，且在比赛过程中确实被更新。
///
/// 这条断言的对象是「世界状态的载体」本身：删掉平行副本之前，副本载有的
/// 就是这些量；副本删除后，它们仍必须从引擎自身读出并随比赛变化。
#[test]
fn player_world_state_is_readable_and_mutates_over_a_game() {
    let mut engine = MatchEngine::new(42);
    engine.set_scope("full").expect("full scope is valid");
    let mut first: Option<WorldDigest> = None;
    let mut positions_changed = false;
    let mut stamina_changed = false;
    let mut ball_moved = false;
    let mut ticks = 0usize;
    while !engine.is_finished() && ticks < 200_000 {
        let tick = engine.step();
        ticks += 1;
        if !ticks.is_multiple_of(500) {
            continue;
        }
        let current = digest(&engine, &tick);
        match &first {
            None => first = Some(current),
            Some(initial) => {
                for (before, now) in initial.players.iter().zip(current.players.iter()) {
                    positions_changed |= before.id == now.id
                        && ((before.x - now.x).abs() > 1e-4 || (before.y - now.y).abs() > 1e-4);
                    stamina_changed |=
                        before.id == now.id && (before.stamina - now.stamina).abs() > 1e-4;
                }
                ball_moved |= (initial.ball_x - current.ball_x).abs() > 1e-4
                    || (initial.ball_y - current.ball_y).abs() > 1e-4;
            }
        }
    }
    assert!(engine.is_finished(), "full-scope game must finish");
    let initial = first.expect("game ran long enough to sample world state");
    assert_eq!(
        initial.players.len(),
        10,
        "world state must expose all ten on-court players"
    );
    assert!(
        positions_changed,
        "player positions must change across a game"
    );
    assert!(stamina_changed, "player stamina must change across a game");
    assert!(ball_moved, "ball position must change across a game");
}
