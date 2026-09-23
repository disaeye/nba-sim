//! 集成测试共用的场景构造器。
//!
//! 这里的函数只做两件事：按给定位置造一个球员、按给定球权造一个约束上下文。
//! 放在 `tests/support/` 而不是各测试文件里复制，避免同一份夹具在多处漂移
//! （历史上 `make_player` 的三份副本字段不一致过）。
//!
//! Rust 集成测试的 `support/` 子目录不会被当作独立测试目标编译，
//! 各测试文件用 `mod support;` 引入。

#![allow(dead_code)]

use glam::Vec2;
use std::collections::HashMap;
use std::sync::LazyLock;

use nba_decision::constraint::{ConstraintContext, PhaseType};
use nba_physics::movement::PlayerPhysicsState;

pub static DEFAULT_RULES: LazyLock<nba_domain::GameRules> =
    LazyLock::new(nba_domain::GameRules::default);

pub fn make_player(id: &str, team: &str, x: f32, y: f32) -> PlayerPhysicsState {
    PlayerPhysicsState {
        id: id.to_string(),
        jersey: id.to_string(),
        team: team.to_string(),
        pos_ft: Vec2::new(x, y),
        vel_ft: Vec2::ZERO,
        accel_ft: Vec2::ZERO,
        target_pos_ft: Vec2::new(x, y),
        max_speed_ftps: 22.0,
        max_accel_ftps2: 35.0,
        target_speed_ftps: 0.0,
        has_ball: false,
        on_court: true,
        action: "Idle".to_string(),
        slot: "PG".to_string(),
        morale: "Normal".to_string(),
        stamina: 100.0,
        max_stamina: 100.0,
        foul_count: 0,
        locomotion: nba_physics::movement::LocomotionState::Idle,
        facing_dir: Vec2::X,
        turn_decel_timer: 0.0,
        is_locked_kinematics: false,
        out_of_bounds_placement: false,
        is_receiving_pass: false,
        is_driving_to_rim: false,
        boundary_cross_latched: false,
        attributes: Default::default(),
        tendencies: Default::default(),
    }
}

pub fn make_physics(players: &HashMap<String, PlayerPhysicsState>) -> nba_physics::PhysicsWorld {
    let rules = &*DEFAULT_RULES;
    let mut physics =
        nba_physics::PhysicsWorld::with_backend(rules, nba_physics::PhysicsBackend::SimpleCircle);
    let mut ids: Vec<_> = players.values().cloned().collect();
    ids.sort_by(|left, right| left.id.cmp(&right.id));
    for player in ids {
        physics.register_player(player);
    }
    physics
}

/// 造一个活球、阵地进攻的约束上下文。
///
/// 默认：主队球权、进攻钟 12s、比赛钟 300s、`SetPlay` 阶段。
/// 调用方按需覆盖字段。
pub fn base_ctx<'a>(physics: &'a nba_physics::PhysicsWorld) -> ConstraintContext<'a> {
    let rules = &*DEFAULT_RULES;
    static TEAM_TRAITS: LazyLock<HashMap<String, nba_domain::TeamTraits>> = LazyLock::new(|| {
        HashMap::from([
            ("home".to_string(), nba_domain::TeamTraits::default()),
            ("away".to_string(), nba_domain::TeamTraits::default()),
        ])
    });
    ConstraintContext {
        physics,
        ball_pos: Vec2::new(47.0, 25.0),
        possession_team: "home",
        shot_clock: 12.0,
        game_clock: 300.0,
        phase: PhaseType::SetPlay,
        game_flow: nba_domain::GameFlowState::LiveBall,
        ball_phase: nba_domain::BallPhase::Held,
        inbound_elapsed: 0.0,
        backcourt_elapsed: 0.0,
        rules,
        team_traits: &TEAM_TRAITS,
        possession_had_shot: false,
    }
}

/// 关闭接球人的预估噪声（层 A 对照开关）。
///
/// `receive_estimate_noise_ft == 0` 时接球人精确知道落点 —— 这是
/// round-10 之前的行为，仅用于「与位置无关」的机制测试。
pub fn setup_noise_off(engine: &mut nba_engine::MatchEngine) {
    engine.rules_mut_for_test().receive_estimate_noise_ft = 0.0;
}

/// 构建一场单节的引擎，用于需要跑完整节的场景。
pub fn scoped_engine(seed: u64, scope: &str) -> nba_engine::MatchEngine {
    let mut engine = nba_engine::MatchEngine::new(seed);
    engine.set_scope(scope).expect("scope must be valid");
    engine
}

// ============================================================================
// 行为指纹（接线类测试的共同口径）
// ============================================================================
//
// 指纹刻意取「会被下游消费的事实」而非内部字段：比分、投篮出手数、罚球数、
// 失误、球位置、以及事件种类序列。若某条接线真实生效，扰动该输入必然改变其中
// 至少一项。三个接线测试文件（`wiring_proof`、`rules_complete_wiring`、
// `decision_wiring`）共用这一份实现，避免同一口径出现三份可能漂移的副本。

pub const PROOF_SEEDS: [u64; 4] = [42, 1, 7, 100];

/// 接线类扰动测试的窗口长度。
///
/// 概率类参数在这个尺度上仍可能掷出相同结果，因此接线测试的判据是
/// 「至少 N/4 个 seed 可见差异」，而不是「全部 seed 必须差异」。
pub const PROOF_TICKS: usize = 3000;

/// 需要更长窗口的接线测试用的长度。
///
/// 用于「改变一个参数后，差异要通过一条**低频**路径才能体现」的系数：
/// 例如 `drive_finish_range_ft` 只影响突破停在多远算作终结，而突破在上述
/// 3000 tick 窗口内只发生几次。实测该系数在 3000 tick 上只 1/4 个 seed
/// 可见、6000 tick 上 4/4 可见（两种窗口都是合法扰动，取后者使断言
/// 真的在测那个系数，而不是在测采样运气）。
pub const PROOF_TICKS_MEDIUM: usize = 6000;

#[derive(Debug)]
pub struct BehaviorFingerprint {
    pub home_score: u32,
    pub away_score: u32,
    pub fg2_attempts: u32,
    pub fg3_attempts: u32,
    pub ft_attempts: u32,
    pub turnovers: u32,
    pub ball_x: f32,
    pub ball_y: f32,
    pub events: Vec<String>,
}

impl BehaviorFingerprint {
    /// 两个指纹是否可观察到差异（用于「接线必须改变行为」的判据）。
    pub fn differs_from(&self, other: &Self) -> bool {
        self.home_score != other.home_score
            || self.away_score != other.away_score
            || self.fg2_attempts != other.fg2_attempts
            || self.fg3_attempts != other.fg3_attempts
            || self.ft_attempts != other.ft_attempts
            || self.turnovers != other.turnovers
            || (self.ball_x - other.ball_x).abs() > 1e-4
            || (self.ball_y - other.ball_y).abs() > 1e-4
            || self.events != other.events
    }
}

/// 跑一段固定 tick 数的模拟并返回可分发的行为指纹。
pub fn fingerprint(engine: &mut nba_engine::MatchEngine, ticks: usize) -> BehaviorFingerprint {
    let mut events: Vec<String> = Vec::new();
    for _ in 0..ticks {
        let tick = engine.step();
        for e in &tick.frame.event_log {
            events.push(e.kind.clone());
        }
    }
    let snap = engine.engine_snapshot();
    BehaviorFingerprint {
        home_score: snap.game.home_score,
        away_score: snap.game.away_score,
        fg2_attempts: snap.box_score.fg2_attempts,
        fg3_attempts: snap.box_score.fg3_attempts,
        ft_attempts: snap.box_score.ft_attempts,
        turnovers: snap.box_score.turnovers,
        ball_x: snap.ball.pos_3d.0.x,
        ball_y: snap.ball.pos_3d.0.y,
        events,
    }
}

/// 多 seed 上跑同一组规则，返回指纹集合（降低单 seed 采样偶然性）。
pub fn fingerprint_for_setup(
    rules: nba_domain::GameRules,
    seeds: &[u64],
    ticks: usize,
) -> Vec<BehaviorFingerprint> {
    // 种子间并行（rayon）：接线测试的成本主体就是多种子长窗口模拟，
    // 种子互不共享状态，并行只改变 wall time，不改变任一 seed 的指纹。
    // 返回顺序仍与 seeds 一致（par_iter 的 collect 保序）。
    use rayon::prelude::*;
    seeds
        .par_iter()
        .map(|&seed| {
            let mut engine = nba_engine::MatchEngine::with_rules(seed, rules.clone());
            fingerprint(&mut engine, ticks)
        })
        .collect()
}

/// 用 `MatchSetup`（而非只有规则）构造引擎，供需要改写名册的场景使用。
pub fn fingerprint_for_match_setup(
    setup: nba_engine::MatchSetup,
    seeds: &[u64],
    ticks: usize,
) -> Vec<BehaviorFingerprint> {
    seeds
        .iter()
        .map(|&seed| {
            let mut engine = nba_engine::MatchEngine::with_setup(setup.clone(), seed);
            fingerprint(&mut engine, ticks)
        })
        .collect()
}
