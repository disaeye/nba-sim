//! 末段行为的连续性守卫：出手选择与故意犯规都由连续的处境量决定。
//!
//! 这里不再断言「第 4 节剩几秒必须出现某个枚举指令」，那些指令已经删除。
//! 断言的内容是「处境变了，选择跟着变，而且方向是对的」。量化的涌现区间
//! 由独立的涌现测试盯（`situational_emergence`）。

use nba_domain::rules::GameRules;
use nba_sim_core::action::ActionKind;
use nba_sim_core::endgame::{intentional_foul, leverage, remaining_seconds, win_probability};
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::resolve::shooter_zone;
use nba_sim_core::state::{Phase, PossessionKind};
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, TeamSide, World};

fn world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(MatchSetup::builtin(GameRules::default()), params, seed)
}

/// 推进到末节的一次活球进攻。
fn fourth_quarter_live(world: &mut World) -> usize {
    for _ in 0..400_000 {
        world.step(TICK_SECONDS);
        if world.clock.period >= 4
            && matches!(world.phase, Phase::Live)
            && world.possession.kind == PossessionKind::Live
        {
            if let Some(handle) = world.holder() {
                return handle;
            }
        }
    }
    panic!("no fourth-quarter live possession");
}

fn set_score(world: &mut World, team: TeamSide, diff: i32) {
    let base = 100;
    let mine = World::team_index(team);
    let theirs = World::team_index(team.other());
    if diff >= 0 {
        world.score[mine] = base + diff as u32;
        world.score[theirs] = base;
    } else {
        world.score[mine] = base;
        world.score[theirs] = base + (-diff) as u32;
    }
}

fn place_at_arc(world: &mut World, carrier: usize, team: TeamSide) {
    let hoop = world.hoop_for(team);
    let spot = hoop + (hoop - world.players[carrier].pos).normalize_or_zero() * 25.0;
    world.players[carrier].pos = spot;
    world.players[carrier].target = spot;
}

/// 清掉刚刚发生的掩护读球窗口。
///
/// 读球窗口是「掩护后第一次决策」的应答集约束；取样一个静止局面时，
/// 该窗口不应继续存在，否则每次抽样都被同一张应答集卡住。
fn clear_screen_read(world: &mut World) {
    world.possession.last_screen = None;
}

#[test]
fn leverage_is_continuous_across_the_period_boundary() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, -6);
    // 第三节末与第四节初的同一种处境：τ 与 γ 必须给出同一个量级。
    world.clock.period = 3;
    world.clock.game_clock = 1.0;
    let third = leverage(&world, -6, remaining_seconds(&world));
    world.clock.period = 4;
    world.clock.game_clock = 720.0;
    let fourth = leverage(&world, -6, remaining_seconds(&world));
    assert!(
        (third.variance_appetite - fourth.variance_appetite).abs() < 0.2,
        "the variance appetite jumped across the period boundary: {:?} vs {:?}",
        third,
        fourth
    );
    // 差距越大、时间越少，方差偏好越强。
    let small = leverage(&world, -2, 600.0);
    let large = leverage(&world, -14, 600.0);
    assert!(
        large.variance_appetite > small.variance_appetite,
        "a larger deficit must sharpen the variance appetite"
    );
    assert!(
        large.score_pressure < small.score_pressure,
        "τ must follow the deficit: {:?} vs {:?}",
        large,
        small
    );
    let late = leverage(&world, -6, 20.0);
    let early = leverage(&world, -6, 1200.0);
    assert!(
        late.urgency > early.urgency,
        "urgency must grow as the clock runs down"
    );
    // 两侧都应落在 (0, 1] 里：它们是概率尺度，不是无穷大的杠杆。
    for value in [late, early, small, large] {
        assert!(value.urgency > 0.0 && value.urgency <= 1.0);
    }
}

#[test]
fn a_leading_team_is_credited_a_higher_win_probability_than_a_trailing_one() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 5);
    let leading = win_probability(&world, team, true);
    set_score(&mut world, team, -5);
    let trailing = win_probability(&world, team, true);
    assert!(
        leading > trailing,
        "win probability did not follow the score: {leading:.3} vs {trailing:.3}"
    );
    assert!((0.0..=1.0).contains(&leading) && (0.0..=1.0).contains(&trailing));
}

#[test]
fn a_trailing_team_late_prefers_three_point_attempts() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, -3);
    world.clock.game_clock = 7.0;
    world.clock.shot_clock = 6.0;
    place_at_arc(&mut world, carrier, team);
    clear_screen_read(&mut world);
    let mut threes = 0;
    let mut twos = 0;
    for _ in 0..400 {
        let decision = nba_sim_core::policy::choose_action(&mut world);
        let intent = decision.intent;
        if let ActionKind::Shoot(_) = intent.kind {
            let zone = shooter_zone(&world, world.players[intent.actor].pos, team);
            if zone == "three" {
                threes += 1;
            } else {
                twos += 1;
            }
        }
    }
    assert!(threes > 0, "the trailing team never attempted a three");    assert!(
        threes > twos,
        "down three late the shot mix did not lean to threes: {threes} threes vs {twos} twos"
    );
}

#[test]
fn a_leading_team_late_shoots_less_than_a_trailing_one() {
    let sample_shots = |diff: i32| {
        let mut world = world(13);
        let carrier = fourth_quarter_live(&mut world);
        let team = world.players[carrier].team;
        set_score(&mut world, team, diff);
        world.clock.game_clock = 26.0;
        world.clock.shot_clock = 20.0;
        clear_screen_read(&mut world);
        let mut shots = 0;
        for _ in 0..400 {
            let decision = nba_sim_core::policy::choose_action(&mut world);
            if matches!(decision.intent.kind, ActionKind::Shoot(_)) {
                shots += 1;
            }
        }
        shots
    };
    let leading = sample_shots(4);
    let trailing = sample_shots(-4);
    assert!(
        leading < trailing,
        "variance appetite did not suppress the leading team's shots: {leading} vs {trailing}"
    );
}

#[test]
fn trailing_team_commits_an_intentional_foul_when_down_four_late() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 4);
    world.clock.game_clock = 26.0;
    let plan = intentional_foul(&world);
    assert!(
        plan.is_some(),
        "the trailing defense did not commit an intentional foul"
    );
    let plan = plan.expect("foul plan");
    assert_eq!(world.players[plan.fouler].team, team.other());
    assert_eq!(plan.target, carrier, "the holder is the only eligible target here");
}

#[test]
fn a_normal_possession_never_triggers_an_intentional_foul() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let team = world.players[carrier].team;
    set_score(&mut world, team, 0);
    world.clock.period = 2;
    world.clock.game_clock = 400.0;
    assert!(
        intentional_foul(&world).is_none(),
        "a mid-game possession was intentionally fouled"
    );
}

#[test]
fn a_trailing_defense_helps_harder_than_a_leading_one() {
    let mut world = world(13);
    let carrier = fourth_quarter_live(&mut world);
    let offense = world.players[carrier].team;
    let hoop = world.hoop_for(offense);
    // 持球人压到篮下，制造真实的协防需求。
    let rim = hoop + (world.players[carrier].pos - hoop).normalize_or_zero() * 3.0;
    world.players[carrier].pos = rim;
    world.players[carrier].target = rim;
    world.clock.game_clock = 120.0;

    // 最靠内的协防目标点：越靠近篮筐，说明协防缩得越很。
    let nearest_help = |world: &World| {
        nba_sim_core::defense::solve_defense_targets(world)
            .iter()
            .map(|target| (*target - hoop).length())
            .fold(f32::INFINITY, f32::min)
    };

    // 进攻方领先 8 分 → 防守方落后 → 该赌得更凶。
    set_score(&mut world, offense, 8);
    let trailing = nearest_help(&world);
    // 进攻方落后 8 分 → 防守方领先 → 该收得更紧。
    set_score(&mut world, offense, -8);
    let leading = nearest_help(&world);
    assert!(
        trailing <= leading + 1e-3,
        "a trailing defense must collapse harder: {trailing:.2} ft vs {leading:.2} ft"
    );
}

#[test]
fn a_poor_free_throw_shooter_is_fouled_more_often_than_a_good_one() {
    let foul_with = |free_throw: f32| {
        let mut world = world(13);
        let carrier = fourth_quarter_live(&mut world);
        let team = world.players[carrier].team;
        // 进攻方领先两分、剩一分钟：防守方正在考虑要不要赌一把。
        set_score(&mut world, team, 2);
        world.clock.game_clock = 60.0;
        world.players[carrier].data.attributes.free_throw = free_throw;
        // 砍人只能在 bonus 内：没进 bonus 的犯规不给罚球，砍了也换不回球权。
        let defense = World::team_index(team.other());
        world.fouls[defense].period = world.rules.league.bonus_fouls_per_period as u8;
        intentional_foul(&world).is_some()
    };
    assert!(
        foul_with(0.30),
        "a 30% free-throw shooter was not worth fouling while down two with a minute left"
    );
    assert!(
        !foul_with(0.90),
        "a 90% free-throw shooter was fouled while down two with a minute left"
    );
}
