//! 犯规与罚则：投篮犯规、进攻犯规、bonus、罚球程序入口。
//!
//! 犯规总是停表。差别只在停表之后是谁的球：投篮犯规与 bonus 犯规进入
//! 罚球，其余犯规由被犯规方发边线球。这条「一律停表」是死球换人窗口的
//! 来源——主力身背 4 犯之所以能下场，是因为对手的每一次犯规都把比赛
//! 停下来一次。

use crate::action::{Fact, FoulKind};
use crate::state::{Phase, TeamSide};
use crate::world::World;

/// 犯规的公共计账：个人累计、球队单节犯规、bonus 判定、事实发布，
/// 返回实际判给的罚球次数。
///
/// 个人累计与球队单节犯规是两套账：进攻犯规计入个人累计（满 6 犯仍离场），
/// 但不计入球队单节犯规，因此不会把对手送进 bonus。`award` 是该犯规类型
/// 自带的罚球数（投篮犯规 2/3 罚），为 0 时按球队单节犯规判定 bonus。
fn book(
    world: &mut World,
    fouler: usize,
    fouled: usize,
    kind: FoulKind,
    award: u8,
    and_one: bool,
) -> u8 {
    world.line_mut(fouler).fouls += 1;
    let disqualified = crate::rotation::record_foul(world, fouler);
    let index = World::team_index(world.players[fouler].team);
    let counts = nba_domain::rules::counts_toward_team_period_fouls(kind);
    if counts {
        world.fouls[index].period += 1;
    }
    world.fouls[index].total += 1;
    if disqualified {
        world.rotation.pending[index] = true;
    }
    let free_throws = if award > 0 {
        award
    } else if counts {
        bonus_free_throws(world, world.players[fouler].team)
    } else {
        0
    };
    world.emit(Fact::Foul {
        fouler,
        fouled,
        kind,
        free_throws,
        and_one,
    });
    free_throws
}

/// 球队是否已经进入 bonus。
///
/// 必须在球队单节犯规已经加上本次犯规之后调用：第 5 犯本身就在 bonus 内。
pub fn bonus_free_throws(world: &World, fouling_team: TeamSide) -> u8 {
    let index = World::team_index(fouling_team);
    world.rules.league.bonus_free_throws_for(
        world.fouls[index].period as u32,
        world.clock.period,
        world.clock.game_clock,
    )
}

/// 最后时段的无球犯规（一罚一掷）是否适用。
///
/// 这条规则本身就是「不许用犯规阻断无球跑动」：如果罚球后不保留球权，
/// 落后方就能靠抓无球人无限换回球权。
fn is_away_from_play(world: &World, fouled: usize) -> bool {
    world
        .rules
        .league
        .in_final_window(world.clock.period, world.clock.game_clock)
        && world.holder() != Some(fouled)
}

pub fn apply_shooting_foul(
    world: &mut World,
    fouler: usize,
    shooter: usize,
    free_throws: u8,
    made: bool,
) {
    world.line_mut(fouler).shooting_fouls += 1;
    if made {
        world.line_mut(shooter).and_one += 1;
    }
    book(
        world,
        fouler,
        shooter,
        FoulKind::Shooting,
        free_throws,
        made,
    );
    world.phase = Phase::FreeThrows {
        shooter,
        total: free_throws,
        remaining: free_throws,
        made: 0,
        next_at: world.now() + world.rules.free_throw_interval_seconds as f64,
        and_one: made,
        retain_possession: false,
    };
    world.apply_dead_ball_window();
}

/// 非投篮普通犯规：bonus 之内两罚，最后时段对无球人犯规，
/// 其余情况由被犯规方发边线球。
pub fn apply_non_shooting_foul(world: &mut World, fouler: usize, fouled: usize) {
    let away_from_play = is_away_from_play(world, fouled);
    let award = if away_from_play {
        world.rules.league.away_from_play_free_throws
    } else {
        0
    };
    let free_throws = book(world, fouler, fouled, FoulKind::Personal, award, false);
    if free_throws > 0 {
        world.phase = Phase::FreeThrows {
            shooter: fouled,
            total: free_throws,
            remaining: free_throws,
            made: 0,
            next_at: world.now() + world.rules.free_throw_interval_seconds as f64,
            and_one: false,
            retain_possession: away_from_play,
        };
    } else {
        world.end_possession("foul", 0);
        world.phase = Phase::DeadBall {
            team: world.players[fouled].team,
            ready_at: world.now() + world.rules.inbound_setup_seconds as f64,
        };
    }
    world.apply_dead_ball_window();
}

/// 进攻犯规：计入个人累计与球队全场犯规，不计入球队单节犯规，
/// 不判给对手罚球，只把球权交给对方。
pub fn apply_offensive_foul(world: &mut World, fouler: usize, fouled: usize) {
    let team = world.players[fouler].team;
    book(world, fouler, fouled, FoulKind::Offensive, 0, false);
    world.end_possession("offensive_foul", 0);
    world.phase = Phase::DeadBall {
        team: team.other(),
        ready_at: world.now() + world.rules.inbound_setup_seconds as f64,
    };
    world.apply_dead_ball_window();
}
