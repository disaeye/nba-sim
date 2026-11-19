//! 成对机制实验：同一检查点只改一项登记的初始条件，比较中间事实。
//!
//! 两条对照从同一个世界快照出发，随机抽样地址因此相同：只有被改动的
//! 那一项不同，其余条件一致。生产候选、执行与裁决照常运行，不预读结果。
//!
//! 三条臂：
//!
//! - 基准：不改任何条件。
//! - 几何：把对位防守人沿持球人方向推远。防守人的真实位置因此改变，
//!   持球人看得见这个改变，他的到达时间估值随之改变。
//! - 感知：几何与基准完全相同，只把对位防守人挪到持球人视野之外。
//!   持球人因此看不到他，仍然按上一次观察的位置估他的到达时间。
//!
//! 每条臂给出两个数：持球人相信的对位防守人到达时间与真实到达时间。
//! 两者的差就是感知的影响：看得见时两者一致，看不见时估值停在旧位置。

use nba_sim_core::action::ActionKind;
use nba_sim_core::state::DecisionRecord;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, RuntimeParams, World};
use serde::Serialize;

/// 一条实验臂：只有这一项与其它臂不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Arm {
    /// 基准。
    Baseline,
    /// 对位防守人被推远：几何改变。
    DefenderFarther,
    /// 对位防守人被挪出视野：感知改变，几何与基准一致。
    DefenderHidden,
}

impl Arm {
    pub const ALL: [Self; 3] = [Self::Baseline, Self::DefenderFarther, Self::DefenderHidden];

    pub const fn name(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::DefenderFarther => "defender_farther",
            Self::DefenderHidden => "defender_hidden",
        }
    }
}

/// 单条臂的中间事实。
#[derive(Debug, Serialize)]
pub struct ArmReport {
    pub arm: Arm,
    pub tick: u64,
    /// 对位防守人的球员身份。
    pub defender_id: String,
    /// 对位防守人此刻是否在持球人的视野里。
    pub defender_observed: bool,
    /// 持球人读到的对位防守人位置。
    pub defender_read_ft: Option<(f32, f32)>,
    /// 对位防守人的真实位置。
    pub defender_actual_ft: (f32, f32),
    /// 持球人读到的位置与真实位置差多少英尺：正数表示读到的是旧位置。
    pub defender_read_error_ft: Option<f32>,
    /// 按持球人读到的位置算出的对位防守人到达持球人处的秒数。
    pub defender_read_arrival_seconds: Option<f32>,
    /// 按真实位置算出的对位防守人到达持球人处的秒数。
    pub defender_actual_arrival_seconds: Option<f32>,
    /// 按持球人读到的位置算出的最近防守人到达持球人处的秒数。
    pub nearest_read_arrival_seconds: Option<f32>,
    /// 按真实位置算出的最近防守人到达持球人处的秒数。
    pub nearest_actual_arrival_seconds: Option<f32>,
    /// 出手候选是否存在。
    pub shoot_candidate: bool,
    /// 出手窗口的余量：防守人最早到达时刻减去出手释放时刻。
    pub shoot_window_margin_seconds: Option<f32>,
    /// 出手候选的能力项估值。
    pub shoot_epv: Option<f32>,
    /// 掩护候选是否存在。
    pub screen_candidate: bool,
    /// 实际选中的动作。
    pub chosen_action: Option<String>,
    pub chosen_probability: Option<f32>,
}

/// 成对实验的结果。
#[derive(Debug, Serialize)]
pub struct PairedReport {
    pub seed: u64,
    pub start_tick: u64,
    pub arms: Vec<ArmReport>,
    /// 本次实验的对照是否成立：对照不成立时不算缺陷。
    pub comparable: bool,
    /// 与判据冲突的发现。
    pub findings: Vec<String>,
}

impl PairedReport {
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }

    pub fn arm(&self, arm: Arm) -> Option<&ArmReport> {
        self.arms.iter().find(|report| report.arm == arm)
    }
}

/// 一次实验的快照：持球人、对位防守人与世界状态。
struct Snapshot {
    world: World,
    holder: usize,
    defender: usize,
    facing: glam::Vec2,
}

/// 找到持球人将要决策的一刻并记录现场。
///
/// 现场必须满足四个条件：持球人在进攻方向的半场（半场之外没有掩护与
/// 协防）、持球人此刻没有身体动作（因此他可以在下一步重新决策）、持球人
/// 已经有过观察记忆（因此“看见”与“没看见”有可比的基础）、持球人此刻
/// 看得见对位防守人。
fn capture(setup: MatchSetup, params: &RuntimeParams, seed: u64) -> Result<Snapshot, String> {
    let mut world = World::new(setup, params.clone(), seed);
    let mut holder = None;
    for _ in 0..40_000 {
        world.step(TICK_SECONDS);
        let Some(current) = world.holder() else {
            continue;
        };
        if !world.live_play() || world.players[current].action.is_some() {
            continue;
        }
        if world.players[current].reads.is_empty() {
            continue;
        }
        let team = world.players[current].team;
        let hoop = world.hoop_for(team);
        if (world.players[current].pos - hoop).length() > 45.0 {
            continue;
        }
        let Some(defender) = nearest_defender(&world, current) else {
            continue;
        };
        if !observed(&world, current, defender) {
            continue;
        }
        holder = Some(current);
        break;
    }
    let holder = holder.ok_or("没有找到持球人即将决策的一刻")?;
    let defender = nearest_defender(&world, holder).ok_or("场上没有对位防守人")?;
    let facing = world.players[holder].facing;
    Ok(Snapshot {
        world,
        holder,
        defender,
        facing,
    })
}

fn observed(world: &World, holder: usize, target: usize) -> bool {
    nba_sim_core::views::observe(world, holder)
        .players
        .iter()
        .any(|player| player.index == target)
}

fn nearest_defender(world: &World, holder: usize) -> Option<usize> {
    let team = world.players[holder].team;
    let holder_pos = world.players[holder].pos;
    world
        .players
        .iter()
        .enumerate()
        .filter(|(_, player)| player.on_court && player.team != team)
        .min_by(|(_, left), (_, right)| {
            (left.pos - holder_pos)
                .length_squared()
                .total_cmp(&(right.pos - holder_pos).length_squared())
        })
        .map(|(index, _)| index)
}

/// 把对位防守人挪到持球人身后：超出视野扇区，持球人看不到他。
fn hide_defender(world: &mut World, holder: usize, defender: usize) {
    let facing = world.players[holder].facing.normalize_or_zero();
    let behind = if facing == glam::Vec2::ZERO {
        glam::Vec2::X
    } else {
        -facing
    };
    let spot = world.players[holder].pos + behind * 9.0;
    let radius = world.rules.player_radius_ft;
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    world.players[defender].pos = spot.clamp(
        glam::Vec2::new(radius, radius),
        glam::Vec2::new(width - radius, height - radius),
    );
    world.players[defender].vel = glam::Vec2::ZERO;
    world.players[defender].target = world.players[defender].pos;
}

/// 把对位防守人沿持球人方向推远：几何改变，防守人仍在视野内。
fn push_defender_away(world: &mut World, holder: usize, defender: usize, feet: f32) {
    let holder_pos = world.players[holder].pos;
    let defender_pos = world.players[defender].pos;
    let away = (defender_pos - holder_pos).normalize_or_zero();
    let radius = world.rules.player_radius_ft;
    let width = world.rules.court.width_ft;
    let height = world.rules.court.height_ft;
    world.players[defender].pos = (defender_pos + away * feet).clamp(
        glam::Vec2::new(radius, radius),
        glam::Vec2::new(width - radius, height - radius),
    );
    world.players[defender].vel = glam::Vec2::ZERO;
    world.players[defender].target = world.players[defender].pos;
}

/// 跑一条臂：改完现场后推进到持球人下一次决策，读出决策记录。
fn run_arm(snapshot: &Snapshot, arm: Arm, params: &RuntimeParams) -> Result<ArmReport, String> {
    let mut world = snapshot.world.clone();
    // 三条臂用同一个朝向：视野扇区因此在臂之间保持一致，臂与臂之间只差
    // 被登记的那一项。
    world.players[snapshot.holder].facing = snapshot.facing;
    match arm {
        Arm::Baseline => {}
        Arm::DefenderFarther => {
            push_defender_away(&mut world, snapshot.holder, snapshot.defender, 6.0)
        }
        Arm::DefenderHidden => hide_defender(&mut world, snapshot.holder, snapshot.defender),
    }
    // 决策时刻由实验固定：持球人此刻没有身体动作，下一步就重新决策。
    world.players[snapshot.holder].schedule.next_redecide_at = world.now();
    world.players[snapshot.holder].schedule.pending = None;
    let tick = world.clock.tick();
    let before = world.decision_records.len();
    world.step(TICK_SECONDS);
    let decision = world
        .decision_records
        .iter()
        .skip(before)
        .filter(|record| record.actor == snapshot.holder)
        .next_back()
        .ok_or("持球人没有产出决策记录")?;
    Ok(arm_report(arm, tick, snapshot, decision, params, &world))
}

fn arm_report(
    arm: Arm,
    tick: u64,
    snapshot: &Snapshot,
    decision: &DecisionRecord,
    params: &RuntimeParams,
    world: &World,
) -> ArmReport {
    let shoot = decision
        .candidates
        .iter()
        .enumerate()
        .find(|(_, candidate)| matches!(candidate.kind, ActionKind::Shoot(_)));
    let screen = decision
        .candidates
        .iter()
        .any(|candidate| matches!(candidate.kind, ActionKind::Screen(_)));
    let shoot_window_margin_seconds = shoot.and_then(|(index, _)| {
        decision
            .windows
            .iter()
            .find(|window| window.candidate == index)
            .and_then(|window| window.window.margin())
    });
    let defender_observed = decision
        .observation
        .players
        .iter()
        .any(|player| player.index == snapshot.defender);
    let defender_read = decision
        .player_reads
        .iter()
        .find(|known| known.player == snapshot.defender)
        .map(|known| known.sample.position);
    let defender_actual = world.players[snapshot.defender].pos;
    let defender_read_error_ft = defender_read.map(|read| (read - defender_actual).length());
    let holder_pos = world.players[snapshot.holder].pos;
    let defender_read_arrival_seconds =
        arrival_of(world, snapshot.holder, snapshot.defender, holder_pos, params, true);
    let defender_actual_arrival_seconds =
        arrival_of(world, snapshot.holder, snapshot.defender, holder_pos, params, false);
    let nearest_read_arrival_seconds = nearest_arrival(world, snapshot.holder, holder_pos, params, true);
    let nearest_actual_arrival_seconds =
        nearest_arrival(world, snapshot.holder, holder_pos, params, false);
    let chosen = decision.candidates.get(decision.chosen);
    ArmReport {
        arm,
        tick,
        defender_id: world.players[snapshot.defender].id.clone(),
        defender_observed,
        defender_read_ft: defender_read.map(|read| (read.x, read.y)),
        defender_actual_ft: (defender_actual.x, defender_actual.y),
        defender_read_error_ft,
        defender_read_arrival_seconds,
        defender_actual_arrival_seconds,
        nearest_read_arrival_seconds,
        nearest_actual_arrival_seconds,
        shoot_candidate: shoot.is_some(),
        shoot_window_margin_seconds,
        shoot_epv: shoot.map(|(_, candidate)| candidate.utility.epv.ability),
        screen_candidate: screen,
        chosen_action: chosen.map(|candidate| candidate.kind.as_str().to_string()),
        chosen_probability: chosen.map(|candidate| candidate.probability),
    }
}

/// 一名防守人到达持球人处的秒数。
///
/// `belief` 为真时按持球人记忆里的位置算，为假时按真实位置算。
fn arrival_of(
    world: &World,
    holder: usize,
    defender: usize,
    holder_pos: glam::Vec2,
    params: &RuntimeParams,
    belief: bool,
) -> Option<f32> {
    let player = world.players.get(defender)?;
    if !player.on_court {
        return None;
    }
    let sample = if belief {
        nba_sim_core::opportunity::read(world, holder, defender)?
    } else {
        nba_sim_core::opportunity::PlayerRead {
            tick: world.clock.tick(),
            position: player.pos,
            velocity: player.vel,
        }
    };
    let body = nba_sim_core::physical::Physical::of(player, &params.policy);
    Some(nba_sim_core::opportunity::arrival_seconds(
        sample,
        holder_pos,
        params.policy.g("contest_proximity_ft"),
        body.speed_ftps,
        body.accel_ftps2,
    ))
}

/// 最近防守人到达持球人处的秒数。
///
/// 按信念计算时，记忆里没有的防守人不参与：持球人不知道他，就不算他
/// 什么时候能到。
fn nearest_arrival(
    world: &World,
    holder: usize,
    holder_pos: glam::Vec2,
    params: &RuntimeParams,
    belief: bool,
) -> Option<f32> {
    let mut best: Option<f32> = None;
    for (handle, player) in world.players.iter().enumerate() {
        if !player.on_court || player.team == world.players[holder].team {
            continue;
        }
        let Some(seconds) = arrival_of(world, holder, handle, holder_pos, params, belief) else {
            continue;
        };
        best = Some(best.map_or(seconds, |current: f32| current.min(seconds)));
    }
    best
}

/// 跑完整的成对实验并核对判据。
pub fn paired_screen_experiment(
    setup: MatchSetup,
    params: &RuntimeParams,
    seed: u64,
) -> Result<PairedReport, String> {
    let snapshot = capture(setup, params, seed)?;
    let mut arms = Vec::new();
    for arm in Arm::ALL {
        arms.push(run_arm(&snapshot, arm, params)?);
    }
    let mut findings = Vec::new();
    let comparable = check_directions(&arms, &mut findings);
    Ok(PairedReport {
        seed,
        start_tick: snapshot.world.clock.tick(),
        arms,
        comparable,
        findings,
    })
}

/// 判据：看得见的几何改变带来一致的估值改变；看不见的几何改变不改变估值。
///
/// 对照不成立时返回假：持球人在决策时刻看不见对位防守人（被遮挡或不在
/// 视野扇区内），几何与感知两条对照都无法比较。这不是缺陷。
fn check_directions(arms: &[ArmReport], findings: &mut Vec<String>) -> bool {
    let baseline = arms.iter().find(|arm| arm.arm == Arm::Baseline);
    let farther = arms.iter().find(|arm| arm.arm == Arm::DefenderFarther);
    let hidden = arms.iter().find(|arm| arm.arm == Arm::DefenderHidden);
    let (Some(baseline), Some(farther), Some(hidden)) = (baseline, farther, hidden) else {
        return false;
    };
    if !baseline.defender_observed {
        return false;
    }
    let geometry_comparable = farther.defender_observed
        && baseline.defender_read_error_ft.is_some_and(|error| error < 0.05)
        && farther.defender_read_error_ft.is_some_and(|error| error < 0.05);
    if geometry_comparable {
        // 同一次几何改动必须同时改变真实到达时刻与持球人的估值，而且两者
        // 始终一致。改动方向取决于防守人原本站在哪一侧，因此只核对变化量。
        let actual_change = match (
            baseline.defender_actual_arrival_seconds,
            farther.defender_actual_arrival_seconds,
        ) {
            (Some(before), Some(after)) => (after - before).abs(),
            _ => f32::default(),
        };
        let read_change = match (
            baseline.defender_read_arrival_seconds,
            farther.defender_read_arrival_seconds,
        ) {
            (Some(before), Some(after)) => (after - before).abs(),
            _ => f32::default(),
        };
        if actual_change <= 0.05 || read_change <= 0.05 {
            findings.push(format!(
                "防守人推远后到达时间没有改变：真实变化 {actual_change:.3}s，估值变化 {read_change:.3}s"
            ));
        }
        for arm in [baseline, farther] {
            let read = arm.defender_read_arrival_seconds;
            let actual = arm.defender_actual_arrival_seconds;
            if !matches!((read, actual), (Some(belief), Some(truth)) if (belief - truth).abs() < 0.05) {
                findings.push(format!(
                    "观察得到防守人时到达估值与真实值不一致：{:?}，{:?}", read, actual
                ));
            }
        }
    }
    let perception_comparable = !hidden.defender_observed;
    if perception_comparable {
        let geometry_moved = match (
            baseline.defender_actual_arrival_seconds,
            hidden.defender_actual_arrival_seconds,
        ) {
            (Some(before), Some(after)) => (before - after).abs(),
            _ => 0.0,
        };
        if geometry_moved <= 0.05 {
            findings.push("被挪出视野的防守人没有改变真实几何，感知对照不可比".to_string());
        } else {
            // 持球人的估值必须停在旧位置：读到的位置与真实位置的差要大过
            // 真实几何的变化量，或持球人根本读不到这个人。
            let read = hidden.defender_read_ft;
            let moved = match read {
                Some((x, y)) => {
                    let actual = hidden.defender_actual_ft;
                    ((x - actual.0).powi(2) + (y - actual.1).powi(2)).sqrt()
                }
                None => f32::INFINITY,
            };
            if moved < HIDDEN_POSITION_TOLERANCE_FT {
                findings.push(format!(
                    "被挪出视野的防守人仍然出现在持球人的估值里：位置误差 {moved:.2} 英尺",
                ));
            }
        }
    }
    geometry_comparable || perception_comparable
}

/// 持球人读到旧位置与真实位置的差，多于这个值才算“读到的确实是旧位置”。
const HIDDEN_POSITION_TOLERANCE_FT: f32 = 3.0;
