//! 传球结算：拦截判定、接球质量、接球人控制权。

use glam::Vec2;

use crate::action::{ActionIntent, ActionKind, Completion, Fact, PassKind, StateDelta};
use crate::world::World;

pub fn resolve(
    world: &mut World,
    intent: &ActionIntent,
    kind: PassKind,
) -> crate::action::ActionOutcome {
    let from = intent.actor;
    let to = intent.target.expect("pass requires a receiver");
    let pass_id = world.new_pass_id();
    // 传球终结了持球状态：收球标志在出手时清掉。
    world.possession.dribble_state = crate::state::DribbleState::Dribbling;
    world.possession.gather_pos = None;
    world.possession.ball_phase = crate::state::BallPhase::Dribble;
    world.possession.passes += 1;
    world.possession.last_pass_at = Some(world.now());
    world.possession.last_pass_from = Some(from);
    world.possession.assist_candidate = Some((from, world.now()));
    world.last_pass_kind = Some(kind);
    let now = world.now();
    let rules = world.rules.clone();
    let from_player = world.players[from].clone();
    let mut to_player = world.players[to].clone();
    // 高吊球在空中等切入人跑到接球点：飞行时间由切入人的跑动时间决定，
    // 落点是切入人将要到达的接球点。
    let (landing, flight) = if kind == crate::action::PassKind::Lob {
        let (spot, flight) = crate::resolve::pass::cut_plan(world, to);
        (Some(spot), Some(flight))
    } else {
        // 普通传球瞄向接球人按当前速度在飞行时间内跑到的位置：提前量
        // 与飞行时间互相决定，落点由 `catch_point` 迭代求解。
        let point = crate::value::catch_point(world, from, to);
        to_player.pos = point;
        (None, None)
    };
    let distance = (to_player.pos - from_player.pos).length();
    // 传球技术改变飞行时长：手递手几乎没有飞行段，头顶传球弧线更高更慢，
    // 背后传球出手快但轨迹难控。
    let kind_flight = match kind {
        crate::action::PassKind::Handoff => 0.5,
        crate::action::PassKind::Overhead => 1.25,
        crate::action::PassKind::BehindBack => 0.9,
        _ => 1.0,
    };
    let duration = flight
        .unwrap_or_else(|| f64::from(rules.pass_duration(distance, false)) * kind_flight);
    world.ball.release_pass(crate::ball::PassRelease {
        rules: &rules,
        from_player: &from_player,
        to_player: &to_player,
        now,
        pass_id,
        from,
        to,
        kind,
        landing,
        flight,
    });
    world.pass_in_flight = Some(crate::ball::PassInFlight {
        pass_id,
        from,
        to,
        kind,
        bounced: false,
    });
    if let crate::state::BallState::Pass { to_pos, .. } = world.ball.state() {
        world.players[to].target = to_pos;
        world.players[to].target_kind = crate::state::TargetKind::Window;
        world.players[to].target_partner = to;
    }
    let passing = world.players[from].data.attributes.passing;
    let base = world.params.policy.w("bad_pass_rate");
    let skill_base = world.params.policy.w("bad_pass_skill_base");
    let defense_gamble = world
        .players
        .iter()
        .filter(|p| p.on_court && p.team != world.players[from].team)
        .map(|p| p.data.tendencies.gamble_steal)
        .fold(0.0f32, f32::max);
    let gamble_effect = 1.0 + (defense_gamble - 0.5) * 0.1;
    let rate = (base * (1.0 + (skill_base - passing) * 1.0) * gamble_effect).clamp(0.005, 0.045);
    let drawn = world.draw(
        crate::draw::Stream::Pass,
        from,
        crate::draw::purpose::BAD_PASS,
        0,
    );
    if std::env::var("NBA_TRACE_MISS").is_ok() {
        eprintln!("BADPASS from={from} drawn={drawn:.4} rate={rate:.4} passing={passing:.2}");
    }
    if drawn < rate {
        world.pass_result = Some(crate::ball::PassResult::Incomplete);
    }
    world.set_phase(crate::world::Phase::PassFlight {
        resolve_at: now + duration + 0.5,
    });
    world.emit(Fact::Pass {
        pass_id,
        from,
        to,
        kind,
        completed: false,
    });
    world.pass_samples.push(crate::state::PassSample {
        possession: world.possession.id,
        from,
        to,
        from_pos: intent.from_ft,
        to_pos: intent.to_ft,
        at: intent.start_time,
    });
    crate::action::ActionOutcome {
        actor: from,
        kind: ActionKind::Pass(kind),
        completion: Completion::Completed,
        delta: StateDelta {
            control: None,
            ..Default::default()
        },
        facts: Vec::new(),
    }
}

/// 高吊球的飞行时间：球按传球速度到篮下。
/// 高吊球的飞行时间：等切入人跑到篮下。
///
/// 高吊球是「球在空中等切入人到位」：球飞到篮下的时间必须等于切入人
/// 跑到篮下的时间，否则球要么提前落地（切入人还没到），要么球到时切入人
/// 已经被防守人挡住。
/// 空切计划：接球点与飞行时间。
///
/// 高吊球是「球在空中等切入人到位」：球飞到接球点的飞行时间必须等于
/// 切入人跑到接���点的���间，接球点因此是切入人在这段时间里真能跑到的
/// 位置——而不是他无论如何也跑不到的篮下。
pub fn cut_plan(world: &World, cutter: usize) -> (Vec2, f64) {
    let basket = cut_target(world, cutter);
    let to_basket = basket - world.players[cutter].pos;
    let distance = to_basket.length();
    let speed = world.players[cutter]
        .effective_speed(&world.params.policy)
        .max(1.0);
    let accel = world.players[cutter]
        .effective_acceleration(&world.params.policy)
        .max(1.0);
    // 切入人从静止起步：先花一段加速时间，然后匀速跑。球在空中飞的时间
    // 必须与他真的能跑到的时间一致，球才落在他手里。
    //
    // 跑动不是直线：绕开防守人要多走一点路，因此这里按七成效率估计实际
    // 能跑到的距离，接球点就是他那时真在的位置。
    let startup = speed / accel;
    let mut flight = startup;
    let mut covered = 0.5 * accel * flight * flight;
    while covered < distance && flight < 2.0 {
        flight += 0.05;
        covered = 0.5 * accel * startup * startup + speed * (flight - startup).max(0.0);
    }
    // 空切的接球点就是篮下那个点：切入人先跑到那里站住，球再落到他手里。
    // 按跑动距离把落点往前缩会让球落在切入人还没到的地方，而切入人的
    // 实际跑动受防守人阻挡，与自由奔跑的估算对不上，球就落地判成不完整。
    let flight = f64::from((flight * 1.45).clamp(0.6, 2.6));
    (basket, flight)
}

/// 高吊球的飞行时间：等切入人跑到接球点。
pub fn lob_flight_time(world: &World, cutter: usize) -> f64 {
    cut_plan(world, cutter).1
}

/// 空切的接球点：切入人跑到的篮下位置。
pub fn cut_target(world: &World, cutter: usize) -> Vec2 {
    let team = world.players[cutter].team;
    world.hoop_for(team)
        + Vec2::new(
            -if world.attacking_right(team) {
                1.0
            } else {
                -1.0
            },
            0.0,
        ) * 2.5
}

pub fn resolve_cut(world: &mut World, intent: &ActionIntent) -> crate::action::ActionOutcome {
    let cutter = intent.target.expect("cut requires a cutter");
    let mut pass_intent = intent.clone();
    pass_intent.kind = ActionKind::Pass(PassKind::Lob);
    pass_intent.target = Some(cutter);
    // 高吊球瞄向切入人跑到的接球点，而不是他此刻的位置。
    pass_intent.to_ft = crate::resolve::pass::cut_plan(world, cutter).0;
    let outcome = resolve(world, &pass_intent, PassKind::Lob);
    // 空切是一次「球在空中等切入人到位」的传球：发球之后切入人必须真的
    // 跑到接球点接球。发球完成不等于空切完成。
    world.players[cutter].target = crate::resolve::pass::cut_plan(world, cutter).0;
    world.players[cutter].target_kind = crate::state::TargetKind::Window;
    world.players[cutter].target_partner = cutter;
    outcome
}
