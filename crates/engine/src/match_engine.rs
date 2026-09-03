//! 比赛模拟编排层：阶段驱动 + 约束过滤 + 效用决策 + 事件化输出。
//!
//! 这是引擎的主循环（架构文档 §11），取代旧的散落 if 决策：
//! - 阶段状态机推进（Inbound → Transition/SetPlay → Resolution → Rebound/DeadBallReset）
//! - 世界级 Runtime 约束检查（24 秒等）
//! - 决策系统按约束管线产出动作意图
//! - 执行系统更新球弹道与物理
//! - 裁决系统处理抢断/得分/篮板与阶段转换

use glam::Vec2;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::setup::MatchSetup;
use nba_decision::constraint::{
    CandidateAction, ConstraintContext, EnforcementAction, PhaseType, ViolationKind,
};
use nba_decision::modulation::{CoachStrategy, PlayerModulationState};
use nba_decision::pipeline::{DecisionOutput, DecisionSystem};
use nba_decision::tactics::{DefensiveTactic, TacticalPlanner, TacticalSet};
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, GameRules, Possession, SubPhase, TeamData};
use nba_invariants::{InvariantChecker, Violation, ViolationTaxonomy};
use nba_officiating::resolution::{DriveResolution, ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_physics::movement::{LocomotionState, PhysicsWorld, PlayerPhysicsState};
use nba_protocol::*;
use nba_semantics::{SemanticContact, SemanticEvaluator, SpacingEvaluation};

#[derive(Debug, Clone, Copy)]
enum ScopeBoundary {
    Possessions,
    Period { last_period: u32 },
    Game,
}

/// 主模拟状态。每个 possession 是阶段事件流的最小产出单位。
pub struct MatchEngine {
    /// Monotonic fixed-step index used by semantic facts and replay consumers.
    pub tick_index: u64,
    pub physics: PhysicsWorld,
    pub possession: Possession,
    pub possession_id: u32,
    pub sub_phase: SubPhase,
    pub sub_phase_timer: f32,
    pub tactical_set: TacticalSet,
    pub carrier_idx: usize,
    pub shot_clock: f32,
    pub game_clock: f32,
    pub current_time: f32,
    pub period: u32,
    pub home_score: u32,
    pub away_score: u32,
    pub ball_pos_3d: (Vec2, f32),
    pub ball_state: BallTrajectoryKind,
    pub last_passer_id: Option<String>,
    pub active_windows: HashMap<String, ActionTimeWindow>,
    pub current_event: Option<String>,
    pub current_callout: Option<String>,
    pub current_intensity: Option<String>,
    pub rng: ChaCha8Rng,
    pub target_possessions: usize,
    pub completed_possessions: usize,
    /// Requested-scope completion is separate from the real game's lifecycle.
    pub simulation_complete: bool,
    /// Raw `step()` runs continuously; scoped exports enable this boundary.
    pub scope_active: bool,
    scope_boundary: ScopeBoundary,

    pub rules: GameRules,
    pub decision: DecisionSystem,
    pub modulation: HashMap<String, PlayerModulationState>,
    pub coach: CoachStrategy,
    pub home_team: TeamData,
    pub away_team: TeamData,
    pub team_traits: HashMap<String, nba_domain::TeamTraits>,
    pub home_roster_order: Vec<String>,
    pub away_roster_order: Vec<String>,
    pub home_offense_tactic: TacticalSet,
    pub away_offense_tactic: TacticalSet,
    pub home_defensive_tactic: DefensiveTactic,
    pub away_defensive_tactic: DefensiveTactic,
    pub last_decision_trace: Option<Box<DecisionDebug>>,
    pub latest_spacing: Option<SpacingEvaluation>,
    pub latest_contacts: Vec<SemanticContact>,
    pub pending_events: Vec<GameEvent>,
    pub current_event_types: Vec<String>,
    pub current_enforcements: Vec<String>,
    pub event_sequence: u64,
    pub current_event_log: Vec<FrameEvent>,
    pub team_fouls_home: u32,
    pub team_fouls_away: u32,
    pub free_throws_remaining: u8,
    pub free_throw_attempt: u8,
    pub free_throw_shooter: Option<String>,
    pub inbound_baseline: Vec2,
    pub game_flow: GameFlowState,
    pub inbound_elapsed: f32,
    pub backcourt_elapsed: f32,
    pub period_break_elapsed: f32,
    pub last_decision_time: f32,
    /// Per-tick invariant checker; validates every emitted frame against the
    /// physical/basketball rules that must always hold, regardless of tactics.
    pub invariant_checker: InvariantChecker,
    /// Violations produced by the most recent `step()`; exported for callers
    /// that want a single aggregated report rather than per-tick stderr.
    pub last_tick_violations: Vec<Violation>,
    /// 投篮与比赛统计分解（2P/3P/FT 命中率与出手数、失误、犯规）。
    pub box_score: MatchBoxScore,
    /// 当前回合开始时的游戏时钟（用于计算回合时长）。
    /// 最近一次回合总结的 index（complete_possession 兜底发射的判据，
    /// M8 验收"回合零遗漏"：任何结束路径都必须有总结）。
    pub last_possession_summary_index: Option<u64>,
    pub current_possession_start_clock: f32,
    /// 当前回合内的连续传球次数。
    pub current_possession_passes: u32,
    /// 当前回合内的出手球员 ID。
    pub current_possession_shooter: Option<String>,
    /// 当前回合内的出手干扰度。
    pub current_possession_contest: Option<f32>,
}

/// 比赛投篮分解与核心统计。
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MatchBoxScore {
    pub fg2_attempts: u32,
    pub fg2_made: u32,
    pub fg3_attempts: u32,
    pub fg3_made: u32,
    pub ft_attempts: u32,
    pub ft_made: u32,
    pub turnovers: u32,
    pub fouls: u32,
}

impl MatchBoxScore {
    pub fn fg2_pct(&self) -> f32 {
        if self.fg2_attempts == 0 { 0.0 } else { self.fg2_made as f32 / self.fg2_attempts as f32 }
    }
    pub fn fg3_pct(&self) -> f32 {
        if self.fg3_attempts == 0 { 0.0 } else { self.fg3_made as f32 / self.fg3_attempts as f32 }
    }
    pub fn ft_pct(&self) -> f32 {
        if self.ft_attempts == 0 { 0.0 } else { self.ft_made as f32 / self.ft_attempts as f32 }
    }
}

/// Summary of a scoped simulation export.
#[derive(Debug)]
pub struct ExportSummary {
    /// Number of ticks written to the stream.
    pub ticks: usize,
    /// Human-readable scope description (e.g. "1 Quarter").
    pub scope_desc: String,
    /// All invariant violations detected across the run; empty means the
    /// simulation satisfied every physical/basketball rule that was checked.
    pub violations: Vec<Violation>,
    /// 比赛统计数据分解。
    pub box_score: MatchBoxScore,
    /// 违例多维分类账分析结果。
    pub taxonomy: ViolationTaxonomy,
}

pub type Simulation = MatchEngine;

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        Self::with_setup(MatchSetup::builtin(GameRules::default()), seed)
    }

    pub fn with_rules(seed: u64, rules: GameRules) -> Self {
        Self::with_setup(MatchSetup::builtin(rules), seed)
    }

    pub fn with_setup(setup: MatchSetup, seed: u64) -> Self {
        setup
            .validate()
            .expect("invalid match setup: validate rules, teams, lineups, and tactics first");
        let rules = setup.rules.clone();
        let mut physics = PhysicsWorld::with_backend(&rules, setup.physics_backend);
        for (team, lineup, side) in [
            (&setup.home_team, &setup.home_lineup, "home"),
            (&setup.away_team, &setup.away_lineup, "away"),
        ] {
            let starter_ids = lineup
                .starters
                .iter()
                .collect::<std::collections::HashSet<_>>();
            let active_ids = lineup
                .starters
                .iter()
                .chain(lineup.bench.iter())
                .collect::<std::collections::HashSet<_>>();
            for player in &team.players {
                if !active_ids.contains(&&player.id) {
                    continue;
                }
                let (x, y) = player.initial_position_ft;
                let is_home = side == "home";
                physics.register_player(PlayerPhysicsState {
                    id: player.id.clone(),
                    jersey: player.jersey.clone(),
                    team: side.to_string(),
                    pos_ft: Vec2::new(x, y),
                    vel_ft: Vec2::ZERO,
                    accel_ft: Vec2::ZERO,
                    target_pos_ft: Vec2::new(x, y),
                    target_speed_ftps: 0.0,
                    // M9/attributes T2：属性→物理量经 capability 映射层（曲线下限走规则通道）。
                    max_speed_ftps: nba_domain::effective_max_speed(&rules, &player.attributes),
                    max_accel_ftps2: nba_domain::effective_max_accel(&rules, &player.attributes),
                    has_ball: is_home && player.id == lineup.starters[0],
                    on_court: starter_ids.contains(&&player.id),
                    action: if is_home && player.id == lineup.starters[0] {
                        "Initiate".to_string()
                    } else if starter_ids.contains(&&player.id) {
                        "SetPosition".to_string()
                    } else {
                        "Bench".to_string()
                    },
                    slot: player
                        .roles
                        .first()
                        .map(|role| format!("{:?}", role))
                        .unwrap_or_else(|| "Player".to_string()),
                    morale: "Normal".to_string(),
                    stamina: 100.0 * player.attributes.stamina.max(0.1),
                    max_stamina: 100.0 * player.attributes.stamina.max(0.1),
                    foul_count: 0,

                    locomotion: LocomotionState::Idle,
                    facing_dir: if is_home { Vec2::X } else { -Vec2::X },
                    turn_decel_timer: 0.0,
                    is_locked_kinematics: false,
                    attributes: player.attributes.clone(),
                    roles: player.roles.clone(),
                    tendencies: player.tendencies.clone(),
                });
            }
        }
        let initial_carrier_id = setup.home_lineup.starters[0].clone();
        let initial_pos = physics
            .get_player(&initial_carrier_id)
            .map(|p| p.pos_ft)
            .unwrap_or(Vec2::new(rules.court.width_ft * 0.3, rules.court.hoop_y_ft));
        let mut player_ids: Vec<String> = physics.get_players().keys().cloned().collect();
        player_ids.sort();
        let modulation = player_ids
            .into_iter()
            .filter_map(|id| {
                physics.get_player(&id).map(|player| {
                    (
                        player.id.clone(),
                        PlayerModulationState {
                            stamina: (player.stamina / player.max_stamina.max(f32::EPSILON))
                                .clamp(0.0, 1.0),
                            ..PlayerModulationState::default()
                        },
                    )
                })
            })
            .collect();
        let home_tactic = TacticalSet::from_id(&setup.home_lineup.offense_tactic)
            .expect("validated home offense tactic");
        let away_tactic = TacticalSet::from_id(&setup.away_lineup.offense_tactic)
            .expect("validated away offense tactic");
        let home_defense = DefensiveTactic::from_id(&setup.home_lineup.defense_tactic)
            .expect("validated home defense tactic");
        let away_defense = DefensiveTactic::from_id(&setup.away_lineup.defense_tactic)
            .expect("validated away defense tactic");
        let team_traits = [
            ("home".to_string(), setup.home_team.team_traits.clone()),
            ("away".to_string(), setup.away_team.team_traits.clone()),
        ]
        .into_iter()
        .collect();
        Self {
            tick_index: 0,
            physics,
            possession: Possession::Home,
            possession_id: 1,
            sub_phase: SubPhase::Initiation,
            sub_phase_timer: 0.0,
            tactical_set: home_tactic,
            carrier_idx: 0,
            shot_clock: rules.league.shot_clock_seconds,
            game_clock: rules.period_duration(1),
            current_time: 0.0,
            period: 1,
            home_score: 0,
            away_score: 0,
            ball_pos_3d: (initial_pos, rules.ball_bounce_base_ft),
            ball_state: BallTrajectoryKind::Held {
                carrier_id: initial_carrier_id,
            },
            last_passer_id: None,
            active_windows: HashMap::new(),
            current_event: Some("TIPOFF".to_string()),
            current_callout: Some("比赛开始，跳球后主队获得第一攻球权。".to_string()),

            rng: ChaCha8Rng::seed_from_u64(seed),
            current_intensity: Some("BuildUp".to_string()),
            target_possessions: 10,
            rules: rules.clone(),
            completed_possessions: 0,
            simulation_complete: false,
            scope_active: false,
            scope_boundary: ScopeBoundary::Possessions,
            decision: DecisionSystem::with_weights(rules.decision.clone()),
            modulation,
            coach: CoachStrategy::default(),
            home_team: setup.home_team,
            away_team: setup.away_team,
            team_traits,
            home_roster_order: setup.home_lineup.starters.to_vec(),
            away_roster_order: setup.away_lineup.starters.to_vec(),
            home_offense_tactic: home_tactic,
            away_offense_tactic: away_tactic,
            home_defensive_tactic: home_defense,
            away_defensive_tactic: away_defense,
            last_decision_trace: None,
            latest_spacing: None,
            latest_contacts: Vec::new(),
            pending_events: Vec::new(),
            current_event_types: Vec::new(),
            current_enforcements: Vec::new(),
            event_sequence: 0,
            current_event_log: Vec::new(),
            team_fouls_home: 0,
            team_fouls_away: 0,
            free_throws_remaining: 0,
            free_throw_attempt: 0,
            free_throw_shooter: None,
            inbound_baseline: Vec2::new(0.0, rules.court.hoop_y_ft),
            game_flow: GameFlowState::TipOff,
            inbound_elapsed: 0.0,

            backcourt_elapsed: 0.0,
            period_break_elapsed: 0.0,
            last_decision_time: -rules.decision_interval_seconds,
            invariant_checker: InvariantChecker::new(),
            last_tick_violations: Vec::new(),
            box_score: MatchBoxScore::default(),
            last_possession_summary_index: None,
            current_possession_start_clock: rules.league.period_duration_seconds,
            current_possession_passes: 0,
            current_possession_shooter: None,
            current_possession_contest: None,
        }
    }

    /// 在回合转换边界发射 L2 回合语义总结事件（docs/design.md §8.2）。
    pub fn emit_possession_summary(
        &mut self,
        terminal_event: &str,
        rebounder_id: Option<String>,
        turnover_player_id: Option<String>,
        rebound_distance_ft: Option<f32>,
    ) {
        let start_clock = self.current_possession_start_clock;
        let end_clock = self.game_clock;
        let duration_seconds = (start_clock - end_clock).abs().max(0.1);
        let offense_team = match self.possession {
            Possession::Home => "home".to_string(),
            Possession::Away => "away".to_string(),
        };
        let summary = nba_domain::PossessionSummary {
            possession_index: self.completed_possessions as u64,
            offense_team,
            start_clock,
            end_clock,
            duration_seconds,
            passes_count: self.current_possession_passes,
            terminal_event: terminal_event.to_string(),
            shooter_id: self.current_possession_shooter.clone(),
            rebounder_id,
            turnover_player_id,
            shot_contest_intensity: self.current_possession_contest,
            rebound_distance_ft,
        };
        self.last_possession_summary_index = Some(summary.possession_index);
        self.pending_events.push(nba_domain::GameEvent::PossessionSummary(summary));
        // 重置下一个回合的上下文
        self.current_possession_start_clock = self.game_clock;
        self.current_possession_passes = 0;
        self.current_possession_shooter = None;
        self.current_possession_contest = None;
    }
    pub fn is_finished(&self) -> bool {
        self.game_flow == GameFlowState::GameEnd || (self.scope_active && self.simulation_complete)
    }

    fn break_finished(&self) -> bool {
        self.period_break_elapsed + f32::EPSILON >= self.rules.period_break_seconds
    }

    /// Parses a scope string into a possession budget and human description.
    /// Malformed scopes are rejected instead of silently changing the request.
    pub fn parse_scope(scope: &str) -> Result<(usize, String), String> {
        Self::parse_scope_with_rules(scope, &GameRules::default())
    }

    pub fn parse_scope_with_rules(
        scope: &str,
        rules: &GameRules,
    ) -> Result<(usize, String), String> {
        let normalized = scope.trim().to_ascii_lowercase();
        let possessions = match normalized.as_str() {
            "1p" => 1usize,
            "5p" => 5usize,
            "10p" => 10usize,
            "1q" => rules.estimated_possessions_per_period as usize,
            "full" => (rules.estimated_possessions_per_period as usize)
                .checked_mul(rules.league.regulation_periods as usize)
                .ok_or_else(|| "scope possession budget overflowed".to_string())?,
            value if value.ends_with('p') => value[..value.len() - 1]
                .parse::<usize>()
                .map_err(|_| format!("invalid scope: {scope}"))?,
            _ => return Err(format!("invalid scope: {scope}")),
        };
        if possessions == 0 {
            return Err("scope must request at least one possession".to_string());
        }
        let description = if normalized == "1q" {
            format!("1 Quarter (approx {possessions} possessions)")
        } else if normalized == "full" {
            format!("Full Game (approx {possessions} possessions)")
        } else {
            format!("{possessions} Possessions")
        };
        Ok((possessions, description))
    }
    pub fn set_scope(&mut self, scope: &str) -> Result<String, String> {
        let (target_possessions, description) = Self::parse_scope_with_rules(scope, &self.rules)?;
        let normalized = scope.trim().to_ascii_lowercase();
        self.target_possessions = target_possessions;
        self.scope_active = true;
        self.scope_boundary = match normalized.as_str() {
            "1q" => ScopeBoundary::Period {
                last_period: self.period,
            },
            "full" => ScopeBoundary::Game,
            _ => ScopeBoundary::Possessions,
        };
        self.simulation_complete = match self.scope_boundary {
            ScopeBoundary::Possessions => self.completed_possessions >= target_possessions,
            ScopeBoundary::Period { .. } | ScopeBoundary::Game => false,
        };
        if self.simulation_complete {
            self.settle_scope_ball(self.ball_pos_3d);
        }
        Ok(description)
    }

    /// Result of a scoped export: tick count, scope label, and any invariant
    /// violations detected across the run. Callers should treat a non-empty
    /// violation list as a failed simulation, not merely a warning.
    pub fn simulate_scope_and_export(
        &mut self,
        scope: &str,
        out_path: &str,
    ) -> std::io::Result<ExportSummary> {
        let scope_desc = self
            .set_scope(scope)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;

        let file = File::create(out_path)?;
        let mut writer = BufWriter::new(file);
        let mut ticks_count = 0;
        let mut all_violations: Vec<Violation> = Vec::new();
        while !self.is_finished() {
            let tick = self.step();
            all_violations.append(&mut self.last_tick_violations);
            let json = serde_json::to_string(&tick)?;
            writer.write_all(json.as_bytes())?;
            writer.write_all(b"\n")?;
            ticks_count += 1;
        }
        if ticks_count == 0 {
            let tick = self.build_tick();
            let json = serde_json::to_string(&tick)?;
            writer.write_all(json.as_bytes())?;
            writer.write_all(b"\n")?;
            ticks_count = 1;
        }
        writer.flush()?;
        if !all_violations.is_empty() {
            eprintln!("=== INVARIANT VIOLATIONS ({} total) ===", all_violations.len());
            for v in &all_violations {
                eprintln!("  {}", v);
            }
        }
        let taxonomy = ViolationTaxonomy::from_violations(&all_violations);
        Ok(ExportSummary {
            ticks: ticks_count,
            scope_desc,
            violations: all_violations,
            box_score: self.box_score.clone(),
            taxonomy,
        })
    }
    /// 当前阶段映射到约束阶段类型；宏观生命周期决定发球、推进和罚球语义。
    fn phase_type(&self) -> PhaseType {
        if self.game_flow == GameFlowState::FreeThrow {
            return PhaseType::FreeThrow;
        }
        if self.game_flow == GameFlowState::TipOff {
            return PhaseType::TipOff;
        }
        match self.sub_phase {
            SubPhase::Initiation if self.game_flow == GameFlowState::DeadBall => PhaseType::Inbound,
            SubPhase::Initiation => PhaseType::Transition,
            SubPhase::ActionExecution => PhaseType::SetPlay,
            SubPhase::ShotAttempt => PhaseType::Resolution,
            SubPhase::FlightAndRebound => PhaseType::Rebound,
            SubPhase::DeadBallReset => PhaseType::DeadBallReset,
        }
    }
    fn constraint_ctx<'a>(&'a self) -> ConstraintContext<'a> {
        let possession_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        ConstraintContext {
            physics: &self.physics,
            ball_pos: self.ball_pos_3d.0,
            possession_team,
            shot_clock: self.shot_clock,
            game_clock: self.game_clock,
            phase: self.phase_type(),
            game_flow: self.game_flow,
            ball_phase: self.ball_phase(),
            inbound_elapsed: self.inbound_elapsed,
            backcourt_elapsed: self.backcourt_elapsed,
            rules: &self.rules,
            team_traits: &self.team_traits,
        }
    }
    /// 球的宏观相位（由领域层 BallState 派生，M2：标签不再是独立状态）。
    fn ball_phase(&self) -> nba_domain::BallPhase {
        if self.free_throws_remaining > 0 {
            return nba_domain::BallPhase::Dead;
        }
        self.ball_state.phase()
    }

    /// 持球人 id。
    fn carrier_id(&self) -> String {
        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::ControlTransfer { carrier_id, .. } => carrier_id.clone(),
            _ => self
                .player_id_for_team_index(
                    if self.possession == Possession::Home {
                        "home"
                    } else {
                        "away"
                    },
                    self.carrier_idx,
                )
                .unwrap_or_default(),
        }
    }
    /// Keeps the active offensive scheme derived from the configured team
    /// owning the ball; transitions must not replace setup with random data.
    fn sync_team_tactics(&mut self) {
        self.tactical_set = match self.possession {
            Possession::Home => self.home_offense_tactic,
            Possession::Away => self.away_offense_tactic,
        };
    }

    /// Re-evaluate coach strategy at every possession boundary.
    fn update_coach_strategy(&mut self) {
        let score_diff = self.home_score as i32 - self.away_score as i32;
        self.coach = CoachStrategy::evaluate(score_diff, self.period, self.game_clock, &self.rules);
    }
    /// 球弹道状态的唯一写入口（BallState Consolidation 的第一步）。
    ///
    /// 所有 `ball_state` 变更都必须经过此方法，保证：
    /// - 归属派生（physics 层的 `has_ball`）随状态同步，不出现"两人持球"；
    /// - 未来可在此插入状态转换合法性校验与领域事件，无需改各调用点。
    ///
    /// `current_t` 用于同步持有/受控状态的持球人参考；非持有状态传当前时间即可。
    fn transition_ball_state(&mut self, next: BallTrajectoryKind) {
        // M2：经领域层纯函数转换表校验，非法边被拒绝并记入强制项
        // （architecture.md §3.2 唯一写入口 + §3.3 转换表穷举）。
        match nba_domain::transition_ball_state(&self.ball_state, next) {
            Ok(next) => {
                self.ball_state = next;
                self.sync_ball_holder();
            }
            Err(reason) => {
                self.current_enforcements
                    .push(format!("ILLEGAL_BALL_TRANSITION:{}", reason));
            }
        }
    }

    /// 将 physics 层的逐球员 `has_ball` 与 `ball_state` 的归属对齐。
    /// Held / Drive / ControlTransfer 视为"有明确持球人"，其余状态清空持球标志。
    fn sync_ball_holder(&mut self) {
        let holder: Option<&str> = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::ControlTransfer { carrier_id, .. } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.physics.set_ball_holder(holder);
    }

    pub fn set_game_flow(&mut self, flow: GameFlowState) {
        if self.game_flow == flow {
            return;
        }
        if !self.game_flow.can_transition_to(flow) {
            self.current_enforcements.push(format!(
                "ILLEGAL_FLOW_TRANSITION:{:?}->{:?}",
                self.game_flow, flow
            ));
            return;
        }
        self.game_flow = flow;
    }

    fn sync_game_flow(&mut self) {
        if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) && self.break_finished()
        {
            self.period += 1;
            self.game_clock = self.rules.period_duration(self.period);
            self.shot_clock = self.rules.league.shot_clock_seconds;
            self.team_fouls_home = 0;
            self.team_fouls_away = 0;
            self.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::LiveBall);
            self.transition_phase(SubPhase::Initiation);
            self.last_decision_time = -self.rules.decision_interval_seconds;
            self.current_event = Some("PERIOD_START".to_string());
            self.current_callout = Some(format!("第{}节开始", self.period));
        }
    }
    fn transition_phase(&mut self, next: SubPhase) {
        let previous = self.phase_type();
        self.sub_phase = next;
        self.sub_phase_timer = 0.0;
        if next == SubPhase::Initiation {
            self.inbound_elapsed = 0.0;
            self.backcourt_elapsed = 0.0;
        }
        let next_phase = self.phase_type();
        if previous != next_phase {
            self.pending_events.push(GameEvent::PhaseTransition {
                from: previous,
                to: next_phase,
            });
        }
    }
    /// Advance one fixed step and validate the emitted frame.
    ///
    /// The invariant checker runs after every tick (including every early
    /// return inside `step_inner`) so that a physics/basketball violation is
    /// reported with the exact tick that produced it, instead of being
    /// discovered only by post-hoc log analysis.
    pub fn step(&mut self) -> StreamTick {
        let tick = self.step_inner();
        self.last_tick_violations = self.invariant_checker.check_tick(&tick);
        for v in &self.last_tick_violations {
            eprintln!("[INVARIANT] {}", v);
        }
        tick
    }

    fn step_inner(&mut self) -> StreamTick {
        if self.scope_active && self.simulation_complete {
            self.current_event = None;
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.current_event_log.clear();
            self.last_decision_trace = None;
            return self.build_tick();
        }
        self.tick_index = self.tick_index.saturating_add(1);

        let dt = self.rules.tick_seconds;
        let was_tip_off = self.game_flow == GameFlowState::TipOff;
        let was_period_break = matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        );
        self.sync_game_flow();
        self.current_event_log.clear();
        self.sync_team_tactics();

        // Tip-off is a configurable dead-ball presentation phase. A zero
        // duration transitions in this same fixed step so the default policy
        // retains the historical first-tick behavior.
        if was_tip_off {
            self.current_time += dt;
            self.sub_phase_timer += dt;
            if self.sub_phase_timer + f32::EPSILON < self.rules.tip_off_duration_seconds {
                self.physics.reset_motion();
                self.current_event = Some("TIPOFF".to_string());
                self.current_callout = Some("裁判抛球，双方中锋起跳争夺跳球！".to_string());
                self.current_enforcements.clear();
                self.last_decision_trace = None;
                return self.build_tick();
            }
            self.set_game_flow(GameFlowState::LiveBall);
            self.transition_phase(SubPhase::Initiation);
        }

        if was_period_break
            && !matches!(
                self.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            )
        {
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            self.publish_events();
            return self.build_tick();
        }
        if matches!(
            self.game_flow,
            GameFlowState::Timeout | GameFlowState::GameEnd
        ) {
            self.current_event = None;
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            return self.build_tick();
        }
        if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) {
            self.period_break_elapsed += dt;
            self.sync_game_flow();
            if matches!(
                self.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            ) {
                self.current_event = None;
                self.current_event_types.clear();
                self.current_enforcements.clear();
                self.last_decision_trace = None;
                return self.build_tick();
            }
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            self.publish_events();
            return self.build_tick();
        }

        if !was_tip_off {
            self.current_time += dt;
            self.sub_phase_timer += dt;
        }
        if self.sub_phase == SubPhase::Initiation
            && self.game_flow == GameFlowState::DeadBall
            && matches!(self.ball_state, BallTrajectoryKind::InboundReady { .. })
        {
            self.inbound_elapsed += dt;
        } else if self.sub_phase != SubPhase::DeadBallReset {
            self.inbound_elapsed = 0.0;
        }
        if self.game_flow.allows_live_ball_actions() {
            self.backcourt_elapsed += dt;
            if self.game_clock > 0.0 {
                self.game_clock = (self.game_clock - dt).max(0.0);
            }
            if !matches!(
                self.ball_state,
                BallTrajectoryKind::Shot { .. } | BallTrajectoryKind::RimRebound { .. }
            ) && self.shot_clock > 0.0
            {
                self.shot_clock = (self.shot_clock - dt).max(0.0);
            }
        }
        self.period_break_elapsed = 0.0;
        let current_t = self.current_time;
        let is_home = self.possession == Possession::Home;
        self.current_event = None;
        self.current_event_types.clear();
        self.current_enforcements.clear();
        self.last_decision_trace = None;

        // Runtime constraints observe the current snapshot before execution.
        let ctx = self.constraint_ctx();
        let runtime_findings = self.decision.registry.evaluate_runtime(&ctx);
        let runtime_violation = if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime | GameFlowState::GameEnd
        ) {
            None
        } else {
            runtime_findings.iter().find_map(|finding| {
                if let EnforcementAction::Violation { kind } = finding.enforcement {
                    Some((finding.constraint.id, kind))
                } else {
                    None
                }
            })
        };
        if let Some((constraint_id, kind)) = runtime_violation {
            self.current_enforcements
                .push(format!("{}:{}", constraint_id, kind.as_str()));
            self.pending_events.push(GameEvent::RuleViolation {
                constraint_id: constraint_id.to_string(),
                reason: kind.as_str().to_string(),
            });
            self.current_event = Some("VIOLATION".to_string());
            let violation_callout = format!(
                "{}！{} 失去球权",
                constraint_id,
                team_name_zh(is_home)
            );
            self.start_violation_turnover(kind);
            // 确保违例判定帧忠实呈现哨响违例事实，不被随后的发球准备覆写
            self.current_callout = Some(violation_callout);
            self.current_event = Some("VIOLATION".to_string());
            self.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return self.build_tick();
        }
        // Advance action windows in stable player-id order.
        if !self.active_windows.is_empty() {
            let mut window_ids: Vec<String> = self.active_windows.keys().cloned().collect();
            window_ids.sort();
            let mut window_events = Vec::new();
            for window_id in window_ids {
                let Some(window) = self.active_windows.get_mut(&window_id) else {
                    continue;
                };
                let previous = window.phase;
                let current = window.update(current_t);
                self.physics.set_player_locked(
                    &window.player_id,
                    window.lock_kinematics && !window.is_finished(current_t),
                    None,
                );
                if current != previous {
                    window_events.push(GameEvent::WindowTransition {
                        player_id: window.player_id.clone(),
                        action_type: window.action_type,
                        new_phase: current,
                    });
                }
            }
            self.active_windows
                .retain(|_, window| !window.is_finished(current_t));
            self.pending_events.extend(window_events);
        }
        let facts = self.physics.drain_facts();
        if !facts.is_empty() {
            self.pending_events.extend(facts.into_iter().map(physics_fact_to_event));
        }
        if self.free_throws_remaining > 0
            && self.game_flow == GameFlowState::FreeThrow
            && self.sub_phase_timer >= self.rules.free_throw_interval_seconds
        {
            self.resolve_free_throw();
            self.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return self.build_tick();
        }
        if self.rules.period_expired(self.game_clock)
            && matches!(
                self.game_flow,
                GameFlowState::LiveBall | GameFlowState::Overtime | GameFlowState::TipOff
            )
        {
            let ball_live = matches!(
                self.ball_state,
                BallTrajectoryKind::Shot { .. } | BallTrajectoryKind::RimRebound { .. }
            );
            if !ball_live {
                self.finish_period();
                self.physics.step(nba_domain::FixedDt(dt));
                self.publish_events();
                return self.build_tick();
            }
        }
        if self.game_flow == GameFlowState::GameEnd {
            self.publish_events();
            return self.build_tick();
        }

        // Phase state machine: inbound decisions are evaluated only during the
        // inbound phase; live-ball decisions use the same registry pipeline.
        let mut decision_output: Option<DecisionOutput> = None;
        match self.sub_phase {
            SubPhase::Initiation => {
                if self.game_flow == GameFlowState::DeadBall {
                    if matches!(self.ball_state, BallTrajectoryKind::InboundReady { .. })
                        && current_t - self.last_decision_time
                            >= self.rules.decision_interval_seconds
                    {
                        let carrier = self.carrier_id();
                        let stamina = self
                            .physics
                            .get_player(&carrier)
                            .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                            .unwrap_or(1.0);
                        let morale_bias = self.morale_bias_for(&carrier);
                        let mut rng =
                            std::mem::replace(&mut self.rng, ChaCha8Rng::seed_from_u64(0));
                        let ctx = self.constraint_ctx();
                        decision_output = self.decision.decide_on_ball(
                            &ctx,
                            &carrier,
                            stamina,
                            morale_bias,
                            &self.coach,
                            &mut rng,
                        );
                        self.rng = rng;
                    }
                } else if self.sub_phase_timer >= self.rules.tactical_initiation_seconds {
                    self.transition_phase(SubPhase::ActionExecution);
                    self.set_game_flow(GameFlowState::LiveBall);
                    self.current_event = Some("TACTICAL_EXECUTION".to_string());
                    self.current_callout =
                        Some(format!("战术发起：{}", self.tactical_set.name_zh()));
                }
            }
            SubPhase::ActionExecution => {
                if current_t - self.last_decision_time >= self.rules.decision_interval_seconds {
                    let carrier = self.carrier_id();
                    let stamina = self
                        .physics
                        .get_player(&carrier)
                        .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                        .unwrap_or(1.0);
                    let morale_bias = self.morale_bias_for(&carrier);
                    let mut rng = std::mem::replace(&mut self.rng, ChaCha8Rng::seed_from_u64(0));
                    let ctx = self.constraint_ctx();
                    decision_output = self.decision.decide_on_ball(
                        &ctx,
                        &carrier,
                        stamina,
                        morale_bias,
                        &self.coach,
                        &mut rng,
                    );
                    self.rng = rng;
                }
            }
            SubPhase::ShotAttempt | SubPhase::FlightAndRebound | SubPhase::DeadBallReset => {}
        }

        // ============================================================
        // 2. 执行决策输出（意图执行重校验，architecture.md §5.2）
        if let Some(out) = decision_output {
            let ctx = self.constraint_ctx();
            if let Err(blocked_reason) = self.decision.registry.revalidate_intent(&ctx, &out.action) {
                self.current_enforcements.push(format!("INTENT_REVALIDATION_BLOCKED:{}", blocked_reason));
                // 硬约束校验失败，拒绝执行该动作，保持 Dwell 保护；
                // trace 仍必须完整发布（architecture.md §5.3：禁止"决策了但没有
                // trace"的路径），供评判器统计"落地改变"率。
                let mut debug = convert_trace(&out.trace);
                debug
                    .enforcement
                    .extend(self.current_enforcements.iter().cloned());
                self.last_decision_trace = Some(Box::new(debug));
            } else {
                let trace = out.trace.clone();
            match out.action {
                CandidateAction::Shoot {
                    shooter_id,
                    from_pos,
                    is_three,
                } => {
                    self.execute_shot(&shooter_id, from_pos, is_three, current_t);
                }
                CandidateAction::Drive {
                    driver_id,
                    from_pos,
                    target_pos,
                } => {
                    self.execute_drive(&driver_id, from_pos, target_pos, current_t);
                }
                CandidateAction::Pass {
                    passer_id,
                    receiver_id,
                    from_pos,
                    to_pos,
                } => {
                    self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t, false);
                }
                CandidateAction::InboundPass {
                    passer_id,
                    receiver_id,
                    from_pos,
                    to_pos,
                } => {
                    self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t, true);
                }
                CandidateAction::Dwell { .. } => {
                    // 观察等待：无操作。
                }
            }
                self.last_decision_time = current_t;
                let mut debug = convert_trace(&trace);
                debug
                    .enforcement
                    .extend(self.current_enforcements.iter().cloned());
                self.last_decision_trace = Some(Box::new(debug));
            }
        }
        // Step the physical world before sampling the ball at this tick.
        self.physics.step(nba_domain::FixedDt(dt));
        let sample_3d = BallisticsEngine::sample_ball_position(
            &self.ball_state,
            current_t,
            self.physics.get_players(),
            &self.rules,
        );
        self.ball_pos_3d = sample_3d;

        // ============================================================
        // 3. 球弹道状态更新与拦截检查
        let mut new_ball_state = None;
        let mut steal_triggered_defender = None;
        let mut loose_ball_secured_player = None;

        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => {
                let cid = carrier_id.clone();
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
                if self.physics.get_player(&cid).is_none() {
                    self.ball_pos_3d = (
                        Vec2::new(
                            self.rules.court.width_ft / 2.0,
                            self.rules.court.height_ft / 2.0,
                        ),
                        self.rules.ball_holder_height_ft,
                    );
                }
            }
            BallTrajectoryKind::ControlTransfer {
                carrier_id,
                start_time,
                duration,
                ..
            } => {
                let cid = carrier_id.clone();
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
                if current_t - *start_time >= *duration {
                    new_ball_state = Some(BallTrajectoryKind::Held { carrier_id: cid });
                }
            }
            BallTrajectoryKind::InboundTransfer {
                baseline_pos,
                inbounder_id,
                start_time,
                duration,
                ..
            } => {
                if current_t - start_time >= *duration {
                    self.ball_pos_3d = (*baseline_pos, self.rules.chest_height_ft);
                    new_ball_state = Some(BallTrajectoryKind::InboundReady {
                        baseline_pos: *baseline_pos,
                        inbounder_id: inbounder_id.clone(),
                    });
                }
            }
            BallTrajectoryKind::InboundReady { .. } => {}

            BallTrajectoryKind::Pass {
                target_id,
                start_time,
                duration,
                from_pos,
                to_pos,
                inbound,
                receive_success,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let def_team = if is_home { "away" } else { "home" };
                let mut defender_ids: Vec<String> = self
                    .physics
                    .get_players()
                    .values()
                    .filter(|player| player.on_court && player.team == def_team)
                    .map(|player| player.id.clone())
                    .collect();
                defender_ids.sort();

                let mut intercept: Option<(String, bool)> = None;
                let segment = *to_pos - *from_pos;
                let segment_length_sq = segment.length_squared();
                if segment_length_sq > 0.001 {
                    for defender_id in defender_ids {
                        let Some(defender) = self.physics.get_player(&defender_id).cloned() else {
                            continue;
                        };
                        let projection_t = ((defender.pos_ft - *from_pos).dot(segment)
                            / segment_length_sq)
                            .clamp(0.0, 1.0);
                        let closest = *from_pos + segment * projection_t;
                        let lane_clearance = (defender.pos_ft - closest).length();
                        let distance_to_ball = (defender.pos_ft - self.ball_pos_3d.0).length();
                        if distance_to_ball >= self.rules.player_radius_ft
                            || self.ball_pos_3d.1
                                >= self.rules.pass_peak_ft + self.rules.defender_reach_ft
                        {
                            continue;
                        }
                        let intersection = GameEvent::PathIntersection {
                            ball_pos: (
                                self.ball_pos_3d.0.x,
                                self.ball_pos_3d.0.y,
                                self.ball_pos_3d.1,
                            ),
                            defender_id: defender.id.clone(),
                            clearance_dist: lane_clearance,
                            flight_time: current_t - start_time,
                        };
                        let outcome = ResolutionLayer::resolve_pass_intersection_with_policy(
                            &intersection,
                            self.physics.get_players(),
                            &self.rules.resolve.base_rates,
                            &mut self.rng,
                        );
                        match outcome {
                            ResolutionOutcome::PassIntercepted { defender_id } => {
                                intercept = Some((defender_id, true));
                                break;
                            }
                            ResolutionOutcome::PassTipped { defender_id } => {
                                intercept = Some((defender_id, false));
                                break;
                            }
                            _ => {}
                        }
                    }
                }

                let passer_id = self.last_passer_id.clone().unwrap_or_default();
                if let Some((defender_id, secured)) = intercept {
                    let position = self.ball_pos_3d.0;
                    if secured {
                        self.pending_events.push(GameEvent::PassIntercepted {
                            passer_id,
                            receiver_id: target_id.clone(),
                            defender_id: defender_id.clone(),
                            position: (position.x, position.y),
                        });
                        steal_triggered_defender = Some(defender_id);
                    } else {
                        self.pending_events.push(GameEvent::PassTipped {
                            passer_id,
                            receiver_id: target_id.clone(),
                            defender_id,
                            position: (position.x, position.y),
                        });
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
                            pos: position,
                            vel: segment / duration.max(f32::EPSILON),
                            z: self.ball_pos_3d.1,
                            vel_z: 0.0,
                            last_touch_team: self.possession,
                        });
                    }
                } else if tau >= 1.0 {
                    let receiver_id = target_id.clone();
                    if *receive_success {
                        self.current_possession_passes += 1;
                        self.pending_events.push(GameEvent::PassReceived {
                            receiver_id: receiver_id.clone(),
                        });
                        if *inbound {
                            self.transition_phase(SubPhase::Initiation);
                            self.set_game_flow(GameFlowState::LiveBall);
                            self.last_decision_time = -self.rules.decision_interval_seconds;
                        }
                        new_ball_state = Some(BallTrajectoryKind::Held {
                            carrier_id: receiver_id,
                        });
                        self.last_passer_id = None;
                    } else {
                        self.pending_events.push(GameEvent::PassDropped {
                            passer_id,
                            receiver_id,
                            position: (self.ball_pos_3d.0.x, self.ball_pos_3d.0.y),
                        });
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
                            pos: self.ball_pos_3d.0,
                            vel: segment / duration.max(f32::EPSILON),
                            z: self.ball_pos_3d.1,
                            vel_z: 0.0,
                            last_touch_team: self.possession,
                        });
                    }
                }
            }
            BallTrajectoryKind::Drive {
                driver_id,
                target_pos: _,
                start_time,
                duration,
                successful,
                finish_made,
                fouler_id,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
                if tau >= 1.0 {
                    let driver_id = driver_id.clone();
                    let successful = *successful;
                    let finish_made = *finish_made;
                    let fouler_id = fouler_id.clone();
                    let driver_pos = self
                        .physics
                        .get_player(&driver_id)
                        .map(|player| player.pos_ft)
                        .unwrap_or(self.ball_pos_3d.0);
                    let holder_height = self.rules.ball_holder_height_ft;
                    self.pending_events.push(GameEvent::DriveOutcome {
                        driver_id: driver_id.clone(),
                        successful,
                        finish_made,
                    });

                    if let Some(fouler_id) = fouler_id {
                        self.pending_events.push(GameEvent::Foul {
                            fouled_player_id: driver_id.clone(),
                            fouler_id,
                            is_shooting: true,
                        });
                        self.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(BallTrajectoryKind::Dead {
                            pos: driver_pos,
                            z: holder_height,
                            last_touch_team: self.possession,
                        });
                        self.transition_phase(SubPhase::DeadBallReset);
                        self.current_event = Some("DRIVE_FOUL".to_string());
                        self.current_callout =
                            Some(format!("{} 突破造成投篮犯规，获得罚球机会", driver_id));
                    } else if successful {
                        let hoop_pos = self.rules.court.hoop_pos(is_home);
                        let dist_to_hoop = (driver_pos - hoop_pos).length();
                        // Spatial gate: if driver is still outside the paint / perimeter,
                        // this drive was stalled before reaching finishing position.
                        let finish_range = 16.0_f32;
                        if dist_to_hoop > finish_range {
                            self.ball_pos_3d = (driver_pos, holder_height);
                            new_ball_state = Some(BallTrajectoryKind::Held {
                                carrier_id: driver_id.clone(),
                            });
                            self.transition_phase(SubPhase::Initiation);
                            self.current_event = Some("DRIVE_STOPPED".to_string());
                            self.current_callout =
                                Some(format!("{} 突破被防守延误于外线，重新组织", driver_id));
                        } else {
                            let shot_dur = BallisticsEngine::shot_duration(
                                dist_to_hoop,
                                self.rules.rim_height_ft,
                                &self.rules,
                            ).max(0.4);
                            self.transition_phase(SubPhase::ShotAttempt);
                            self.pending_events.push(GameEvent::ShotRelease {
                                shooter_id: driver_id.clone(),
                                pos: (driver_pos.x, driver_pos.y),
                                is_three: false,
                                contest_level: 0.2,
                                make_probability: if finish_made { 1.0 } else { 0.0 },
                            });
                            self.current_possession_shooter = Some(driver_id.clone());
                            let contest_val = if finish_made { 0.35 } else { 0.65 };
                            self.current_possession_contest = Some(contest_val);
                            self.current_event = Some("SHOT_RELEASE".to_string());
                            let driver_display = self
                                .physics
                                .get_player(&driver_id)
                                .map(|p| format!("{}号", p.jersey))
                                .unwrap_or_else(|| driver_id.clone());
                            self.current_callout = Some(format!("{} 起跳突破上篮！", driver_display));
                            new_ball_state = Some(BallTrajectoryKind::Shot {
                                shooter_id: driver_id.clone(),
                                from_pos: driver_pos,
                                hoop_pos,
                                start_time: current_t,
                                duration: shot_dur,
                                is_made: finish_made,
                                is_three: false,
                                peak_z: self.rules.rim_height_ft + 1.5,
                            });
                        }
                    } else {
                        self.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(BallTrajectoryKind::Held {
                            carrier_id: driver_id.clone(),
                        });
                        self.transition_phase(SubPhase::Initiation);
                        self.current_event = Some("DRIVE_STOPPED".to_string());
                        self.current_callout =
                            Some(format!("{} 突破被防守延误，重新组织", driver_id));
                    }
                    self.last_decision_time = current_t;
                }
            }
            BallTrajectoryKind::Shot {
                shooter_id,
                hoop_pos,
                start_time,
                duration,
                is_made,
                is_three,
                from_pos,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                if tau >= 1.0 {
                    let sid = shooter_id.clone();
                    let h_pos = *hoop_pos;
                    let s_pos = *from_pos;
                    let made = *is_made;
                    let three = *is_three;
                    self.pending_events.push(GameEvent::HoopArrival {
                        shooter_id: sid.clone(),
                        shot_origin: (s_pos.x, s_pos.y),
                        is_made: made,
                        is_three: three,
                        contest_intensity: 0.0,
                    });
                    if three {
                        self.box_score.fg3_attempts += 1;
                        if made {
                            self.box_score.fg3_made += 1;
                        }
                    } else {
                        self.box_score.fg2_attempts += 1;
                        if made {
                            self.box_score.fg2_made += 1;
                        }
                    }
                    match ResolutionLayer::resolve_shot_arrival(made, three, &sid) {
                        ResolutionOutcome::Score { points, .. } => {
                            if is_home {
                                self.home_score += points;
                            } else {
                                self.away_score += points;
                            }
                            let baseline =
                                Court::nearest_boundary_with_geometry(h_pos, self.rules.court);
                            self.ball_pos_3d = (h_pos, self.rules.rim_height_ft);
                            self.emit_possession_summary("SCORE", None, None, None);
                            self.transition_phase(SubPhase::DeadBallReset);
                            self.start_inbound_transition(baseline, (h_pos, self.rules.rim_height_ft));
                        }
                        ResolutionOutcome::Miss { .. } => {
                            let shooter_name = self
                                .physics
                                .get_player(&sid)
                                .map(|p| p.jersey.clone())
                                .unwrap_or_else(|| sid.clone());
                            self.current_event = Some("SHOT_MISSED".to_string());
                            self.current_callout =
                                Some(format!("砸框而出！{} 投篮不中，争抢篮板！", shooter_name));
                            let rebound_from = (h_pos, self.rules.rim_height_ft);
                            self.transition_phase(SubPhase::FlightAndRebound);
                            let landing_spot = BallisticsEngine::compute_rebound_landing(
                                s_pos,
                                h_pos,
                                &mut self.rng,
                                &self.rules,
                            );
                            new_ball_state = Some(BallTrajectoryKind::RimRebound {
                                from_pos: rebound_from.0,
                                from_z: rebound_from.1,
                                hoop_pos: h_pos,
                                target_landing: landing_spot.landing_pos,
                                start_time: current_t,
                                duration: landing_spot.flight_duration,
                                peak_z: self.rules.rebound_peak_ft,
                                last_touch_team: self.possession,
                            });
                        }
                        _ => {}
                    }
                }
            }
            BallTrajectoryKind::RimRebound {
                target_landing,
                start_time,
                duration,
                ..
            } => {
                let tau = if *duration <= f32::EPSILON {
                    1.0
                } else {
                    ((current_t - start_time) / duration).clamp(0.0, 1.0)
                };
                if tau >= 1.0 {
                    let reb_pos = *target_landing;
                    let reb_id = self.resolve_rebounder(reb_pos);
                    let reb_name = self
                        .physics
                        .get_player(&reb_id)
                        .map(|p| p.jersey.clone())
                        .unwrap_or(reb_id.clone());
                    let original_offense = if is_home { "home" } else { "away" };
                    let is_offensive = self
                        .physics
                        .get_player(&reb_id)
                        .map(|p| p.team == original_offense)
                        .unwrap_or(false);
                    self.pending_events.push(GameEvent::ReboundContest {
                        rebounder_id: reb_id.clone(),
                        landing_pos: (reb_pos.x, reb_pos.y),
                        is_offensive,
                    });
                    self.current_event = Some("REBOUND".to_string());
                    self.current_callout = Some(format!(
                        "{} 抢到{}篮板，重新组织进攻！",
                        reb_name,
                        if is_offensive { "前场" } else { "防守" }
                    ));
                    if !is_offensive {
                        let p_pos = self.physics.get_player(&reb_id).map(|p| p.pos_ft).unwrap_or(reb_pos);
                        let dist = (p_pos - reb_pos).length();
                        self.emit_possession_summary("DEFENSIVE_REBOUND", Some(reb_id.clone()), None, Some(dist));
                    }
                    self.start_rebound_outlet(reb_id, reb_pos, is_offensive);
                }
            }
            BallTrajectoryKind::LooseBall { pos, vel, z, vel_z, .. } => {
                let next_pos = self
                    .rules
                    .court
                    .clamp_playable(*pos + *vel * dt, self.rules.player_radius_ft);
                let next_z = (*z + *vel_z * dt).max(0.0);
                self.ball_pos_3d = (next_pos, next_z);
                let candidates = self.physics.query_nearby(
                    next_pos,
                    self.rules.player_radius_ft + self.rules.defender_reach_ft,
                    &nba_physics::EntityFilter::Any,
                );
                if let Some(player_id) = candidates.into_iter().next() {
                    self.pending_events.push(GameEvent::LooseBallSecured {
                        player_id: player_id.clone(),
                        position: (next_pos.x, next_pos.y),
                    });
                    loose_ball_secured_player = Some(player_id);
                } else {
                    new_ball_state = Some(BallTrajectoryKind::LooseBall {
                        pos: next_pos,
                        vel: *vel * self.rules.ball_velocity_retention,
                        z: next_z,
                        vel_z: *vel_z,
                        last_touch_team: self.possession,
                    });
                }
            }
            BallTrajectoryKind::Dead { .. } => {}
        }
        if let Some(def_id) = steal_triggered_defender {
            self.current_event = Some("STEAL".to_string());
            let def_display = self
                .physics
                .get_player(&def_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| def_id.clone());
            self.current_callout = Some(format!("传球路线被识破！{} 飞身抢断！", def_display));
            let ball_intercept_pos = self.ball_pos_3d.0;
            self.start_steal_transition(def_id, ball_intercept_pos);
        } else if let Some(player_id) = loose_ball_secured_player {
            let from_pos = self.ball_pos_3d.0;
            let from_z = self.ball_pos_3d.1;
            self.start_loose_ball_transition(player_id.clone(), from_pos);
            let duration = self.rules.min_pass_duration_seconds.max(0.04);
            self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                from_pos,
                from_z,
                carrier_id: player_id,
                start_time: current_t,
                duration,
            });
            self.ball_pos_3d = (from_pos, from_z);
        } else if let Some(nbs) = new_ball_state {
            self.transition_ball_state(nbs);
            if !matches!(self.ball_state, BallTrajectoryKind::Held { .. }) {
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
            }
        }

        // ============================================================
        // ============================================================
        // 4. 战术目标生成 & 移动导航（每 tick）
        // ============================================================
        let off_roster = match self.possession {
            Possession::Home => &self.home_roster_order,
            Possession::Away => &self.away_roster_order,
        };
        let live_off_positions: Vec<Vec2> = off_roster
            .iter()
            .filter_map(|pid| self.physics.get_player(pid).map(|p| p.pos_ft))
            .collect();
        let (mut home_targets, mut away_targets) =
            TacticalPlanner::plan_possession_targets_with_rules(
                self.tactical_set,
                self.sub_phase,
                self.possession,
                self.ball_pos_3d.0,
                self.carrier_idx,
                self.sub_phase_timer,
                &mut self.rng,
                &self.rules,
                Some(&live_off_positions),
            );
        let home_roster = self.home_roster_order.clone();
        let away_roster = self.away_roster_order.clone();
        TacticalPlanner::bind_targets(&mut home_targets, &home_roster);
        TacticalPlanner::bind_targets(&mut away_targets, &away_roster);
        let active_driver_id = match &self.ball_state {
            BallTrajectoryKind::Drive { driver_id, .. } => Some(driver_id.as_str()),
            _ => None,
        };
        for target in home_targets.into_iter().chain(away_targets) {
            if let Some(player_id) = target.player_id {
                if active_driver_id == Some(player_id.as_str()) {
                    continue;
                }
                self.physics.set_player_target(
                    &player_id,
                    target.target_pos,
                    target.speed,
                    &target.action,
                    &target.slot,
                    &target.morale,
                );
            }
        }
        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.physics.set_ball_holder(active_carrier);

        let player_ids: Vec<String> = {
            let mut ids: Vec<String> = self.physics.get_players().keys().cloned().collect();
            ids.sort();
            ids
        };
        let modulation = &mut self.modulation;
        for pid in &player_ids {
            let speed = self
                .physics
                .get_player(pid)
                .map(|p| p.vel_ft.length())
                .unwrap_or(0.0);
            modulation
                .entry(pid.clone())
                .or_default()
                .update_stamina_with_rules(speed, dt, &self.rules);
        }
        // modulation 状态在物理推进后保留为决策反馈，渲染读取当前值。

        let raw_contacts = self.physics.drain_contacts();
        self.latest_contacts = SemanticEvaluator::event_facts(
            raw_contacts.clone(),
            &self.physics,
            self.possession,
            self.sub_phase,
            self.tick_index,
            &self.rules,
        );
        self.latest_spacing = Some(SemanticEvaluator::spacing(
            self.possession,
            self.ball_pos_3d.0,
            &self.physics,
            &self.rules,
        ));
        self.pending_events
            .extend(self.latest_contacts.iter().map(semantic_contact_to_event));
        self.pending_events.extend(
            self.physics
                .drain_facts()
                .into_iter()
                .map(physics_fact_to_event),
        );
        for player_id in &player_ids {
            if let Some(state) = self.modulation.get(player_id) {
                if let Some(player) = self.physics.get_player_mut(player_id) {
                    player.stamina = state.stamina * player.max_stamina;
                    player.morale = format!("{:?}", state.morale);
                }
            }
        }

        self.publish_events();

        self.build_tick()
    }

    fn morale_bias_for(&self, player_id: &str) -> f32 {
        let policy = &self.rules.modulation;
        match self.modulation.get(player_id).map(|m| m.morale) {
            Some(nba_decision::modulation::MoraleState::HotHand) => policy.hot_hand_bias,
            Some(nba_decision::modulation::MoraleState::Clutch) => policy.clutch_bias,
            Some(nba_decision::modulation::MoraleState::Normal) => 0.0,
            Some(nba_decision::modulation::MoraleState::Frustrated) => policy.frustrated_bias,
            Some(nba_decision::modulation::MoraleState::Exhausted) => policy.exhausted_bias,
            None => 0.0,
        }
    }

    fn publish_events(&mut self) {
        while !self.pending_events.is_empty() {
            let events = std::mem::take(&mut self.pending_events);
        let mut adjudicated_events = Vec::with_capacity(events.len() + 2);
        for event in events {
            if let GameEvent::Contact {
                player_a, player_b, ..
            } = &event
            {
                if let Some(contact) = self.latest_contacts.iter().find(|contact| {
                    contact.raw.entity_a == *player_a && contact.raw.entity_b == *player_b
                }) {
                    if let ResolutionOutcome::Foul {
                        fouled_player_id,
                        fouler_id,
                        is_shooting,
                    } = ResolutionLayer::resolve_semantic_contact(
                        contact,
                        self.physics.get_players(),
                        &self.rules.resolve.contact,
                        &mut self.rng,
                    ) {
                        adjudicated_events.push(GameEvent::Foul {
                            fouled_player_id,
                            fouler_id,
                            is_shooting,
                        });
                    }
                }
            }
            adjudicated_events.push(event);
        }
        let ctx = self.constraint_ctx();
        let findings = self
            .decision
            .registry
            .evaluate_events(&ctx, &adjudicated_events);

        for event in &adjudicated_events {
            if let GameEvent::Foul {
                fouled_player_id,
                fouler_id,
                is_shooting,
            } = event
            {
                if let Some(state) = self.modulation.get_mut(fouled_player_id) {
                    state.catch_equilibrium = (state.catch_equilibrium
                        - self.rules.semantics.contact_minor_speed_ratio * 0.2)
                        .clamp(0.4, 1.0);
                }
                if let Some(state) = self.modulation.get_mut(fouler_id) {
                    state.turnover_count = state.turnover_count.saturating_add(1);
                }
                let fouler_is_home = self
                    .physics
                    .get_player(fouler_id)
                    .map(|p| p.team == "home")
                    .unwrap_or(false);
                let team_fouls = if fouler_is_home {
                    self.team_fouls_home = self.team_fouls_home.saturating_add(1);
                    self.team_fouls_home
                } else {
                    self.team_fouls_away = self.team_fouls_away.saturating_add(1);
                    self.team_fouls_away
                };
                if let Some(player) = self.physics.get_player_mut(fouler_id) {
                    player.foul_count = player.foul_count.saturating_add(1);
                }
                // 犯满离场（charter 7：个人犯满上限为联赛档案参数）。
                if self
                    .physics
                    .get_player(fouler_id)
                    .is_some_and(|p| p.foul_count >= self.rules.league.max_personal_fouls)
                {
                    self.forced_substitution(fouler_id);
                }
                self.current_event = Some(
                    if *is_shooting {
                        "SHOOTING_FOUL"
                    } else {
                        "FOUL"
                    }
                    .to_string(),
                );
                if *is_shooting || team_fouls >= self.rules.league.bonus_fouls_per_period {
                    self.free_throws_remaining = if *is_shooting {
                        self.rules.league.shooting_foul_free_throws
                    } else {
                        self.rules.league.bonus_free_throws
                    };
                    // The fouled team keeps possession and shoots.
                    self.free_throw_shooter = Some(fouled_player_id.clone());
                    self.set_game_flow(GameFlowState::FreeThrow);
                    self.transition_phase(SubPhase::DeadBallReset);
                }
            }
        }

        let mut applied_keys = std::collections::HashSet::new();
        for finding in &findings {
            let key = finding.summary();
            // 只有 Violate 才触发强制项：Flagged 是咨询性信号（如非持球人的
            // BOUNDARY_CROSSING），不能生成 RuleViolation 事件——否则每个
            // 边界事实都会附带一条伪 VIOLATION/ENFORCEMENT_APPLIED。
            let is_violate = matches!(
                finding.result.status,
                nba_decision::constraint::ConstraintStatus::Violate { .. }
            );
            if is_violate
                && finding.enforcement != EnforcementAction::None
                && applied_keys.insert(key)
            {
                self.apply_enforcement(finding);
            }
        }

        self.current_event_types.extend(
            adjudicated_events
                .iter()
                .map(|event| event.event_type_str().to_string()),
        );
        for event in adjudicated_events {
            self.event_sequence = self.event_sequence.saturating_add(1);
            let data = serde_json::to_value(&event).ok();
            self.current_event_log.push(FrameEvent {
                sequence: self.event_sequence,
                time: (self.current_time * 100.0).round() / 100.0,
                kind: event.event_type_str().to_string(),
                data,
            });
        }
        if let Some(primary) = self.current_event_types.last() {
            self.current_event = Some(primary.clone());
        }
    }
}

    fn apply_enforcement(&mut self, finding: &nba_decision::constraint::ConstraintFinding) {
        match &finding.enforcement {
            EnforcementAction::None
            | EnforcementAction::BlockAction
            | EnforcementAction::ModifyAction { .. }
            | EnforcementAction::DelayAction { .. }
            | EnforcementAction::TriggerFoul { .. }
            | EnforcementAction::ResetPosition => {}
            EnforcementAction::Violation { kind } => {
                let action = kind.as_str().to_string();
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.pending_events.push(GameEvent::RuleViolation {
                    constraint_id: finding.constraint.id.to_string(),
                    reason: action.clone(),
                });
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action,
                });
            }
            EnforcementAction::ChangePossession { reason } => {
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, reason));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action: format!("CHANGE_POSSESSION:{}", reason),
                });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
            EnforcementAction::EndPhase { next } => {
                let action = format!("END_PHASE:{}", next.as_str());
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action,
                });
                if *next == PhaseType::DeadBallReset {
                    self.set_game_flow(GameFlowState::DeadBall);
                    self.transition_phase(SubPhase::DeadBallReset);
                }
            }
            EnforcementAction::Turnover { reason } => {
                self.current_enforcements
                    .push(format!("{}:TURNOVER:{}", finding.constraint.id, reason));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action: format!("TURNOVER:{}", reason),
                });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
        }
    }

    /// Resolve a drive once, then expose its presentation through the physics body.
    fn execute_drive(&mut self, driver_id: &str, from_pos: Vec2, target_pos: Vec2, current_t: f32) {
        let Some(driver) = self.physics.get_player(driver_id).cloned() else {
            return;
        };
        let defender = self.physics.openness(driver_id);
        let lane_density =
            SemanticEvaluator::spacing(self.possession, target_pos, &self.physics, &self.rules)
                .paint_crowding;
        let stamina = (driver.stamina / driver.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0);
        let defender_id = defender.closest_defender_id.clone();
        let foul_rate = defender_id
            .as_ref()
            .map(|_| self.rules.resolve.base_rates.foul_on_drive_rate)
            .unwrap_or(0.0);
        let resolution = DriveResolution::resolve(
            self.rules.resolve.base_rates.drive_success,
            self.rules.resolve.shot_type_rates.drive_finish_2pt,
            foul_rate,
            driver.attributes.finishing,
            stamina,
            lane_density,
            defender.contest_intensity,
            self.rules.resolve.shot_type_block_bias.drive_finish,
            self.rules.resolve.player_skill.finishing_weight,
            &self.rules.resolve.drive,
            &mut self.rng,
        );
        let fouler_id = resolution.shooting_foul.then_some(defender_id).flatten();
        let target_pos = self
            .rules
            .court
            .clamp_playable(target_pos, self.rules.player_radius_ft);
        let drive_speed = driver.max_speed_ftps;
        self.physics.set_player_target(
            driver_id,
            target_pos,
            drive_speed,
            "DriveToBasket",
            "BallHandler",
            &driver.morale,
        );
        self.transition_ball_state(BallTrajectoryKind::Drive {
            driver_id: driver_id.to_string(),
            from_pos,
            target_pos,
            start_time: current_t,
            duration: self.rules.tactics.action_duration_seconds,
            successful: resolution.successful,
            finish_made: resolution.finish_made,
            fouler_id,
        });
        self.ball_pos_3d = (from_pos, self.rules.ball_holder_height_ft);
        self.transition_phase(SubPhase::ActionExecution);
        self.pending_events.push(GameEvent::DriveInitiated {
            driver_id: driver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            target_pos: (target_pos.x, target_pos.y),
        });
        self.current_event = Some("DRIVE_INITIATED".to_string());
        self.current_callout = Some(format!("{} 持球突破，冲击篮筐！", driver.jersey));
        self.current_intensity = Some("Climax".to_string());
    }

    /// Resolve shot quality at release; the resulting outcome is then replayable
    /// independently of the later presentation trajectory.
    fn execute_shot(
        &mut self,
        shooter_id: &str,
        from_pos: Vec2,
        is_three_hint: bool,
        current_t: f32,
    ) {
        let is_home = self.possession == Possession::Home;
        let hoop = self.rules.court.hoop_pos(is_home);
        let shooter = self.physics.get_player(shooter_id);
        let shooter_pos = shooter.map(|player| player.pos_ft).unwrap_or(from_pos);
        let dist_to_hoop = (shooter_pos - hoop).length();
        let is_three_by_distance = dist_to_hoop >= self.rules.league.three_point_distance_ft;
        let is_three = is_three_by_distance || is_three_hint;
        let openness = self.physics.openness(shooter_id);
        let spacing_bonus = self
            .latest_spacing
            .map(|spacing| spacing.shot_quality_bonus)
            .unwrap_or(0.0);
        let skill = shooter
            .map(|player| {
                if dist_to_hoop < self.rules.rim_shot_distance_ft {
                    player.attributes.finishing
                } else if is_three {
                    player.attributes.shooting_three
                } else {
                    player.attributes.shooting_mid
                }
            })
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let stamina = shooter
            .map(|player| (player.stamina / player.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let skill_adjustment =
            (skill - 0.5) * self.rules.resolve.player_skill.shooting_weight * 2.0;
        let stamina_adjustment = (stamina - 1.0) * self.rules.resolve.player_skill.shooting_weight;
        let contest_penalty = openness.contest_intensity * self.rules.shot_contest_sensitivity;
        let base_fg = if dist_to_hoop < self.rules.rim_shot_distance_ft {
            self.rules.resolve.base_rates.shot_make_2pt
        } else if is_three {
            self.rules.resolve.base_rates.shot_make_3pt
        } else {
            self.rules.resolve.base_rates.shot_make_2pt
        };
        let final_fg_pct = (base_fg + skill_adjustment + stamina_adjustment + spacing_bonus
            - contest_penalty)
            .clamp(self.rules.shot_pct_floor, self.rules.shot_pct_ceiling);
        let is_made = self.rng.gen_bool(final_fg_pct as f64);

        let peak_z =
            self.rules.shot_peak_base_ft + dist_to_hoop * self.rules.shot_peak_distance_factor;
        let flight_time = BallisticsEngine::shot_duration(dist_to_hoop, peak_z, &self.rules);
        self.active_windows.insert(
            shooter_id.to_string(),
            ActionTimeWindow::new_jump_shot(shooter_id, current_t, &self.rules),
        );
        self.current_possession_shooter = Some(shooter_id.to_string());
        self.current_possession_contest = Some(openness.contest_intensity);

        let release_pos = self.ball_pos_3d.0;
        self.transition_ball_state(BallTrajectoryKind::Shot {
            shooter_id: shooter_id.to_string(),
            from_pos: release_pos,
            hoop_pos: hoop,
            start_time: current_t,
            duration: flight_time,
            is_made,
            is_three,
            peak_z,
        });
        self.transition_phase(SubPhase::ShotAttempt);
        self.pending_events.push(GameEvent::ShotRelease {
            shooter_id: shooter_id.to_string(),
            pos: (release_pos.x, release_pos.y),
            is_three,
            contest_level: openness.contest_intensity,
            make_probability: final_fg_pct,
        });
        self.current_event = Some("SHOT_RELEASE".to_string());
        let shooter_name = self
            .physics
            .get_player(shooter_id)
            .map(|p| p.jersey.clone())
            .unwrap_or_else(|| shooter_id.to_string());
        self.current_callout = Some(format!(
            "{} 迎着防守果断出手{}！",
            shooter_name,
            if is_three { "三分球" } else { "跳投" }
        ));
        self.current_intensity = Some("Climax".to_string());
    }

    fn resolve_free_throw(&mut self) {
        if self.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_attributes = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.attributes.clone())
            .unwrap_or_default();
        let made = self.rng.gen_bool(
            nba_domain::free_throw_probability(&self.rules, &shooter_attributes) as f64,
        );
        let shooter_is_home = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.possession == Possession::Home);
        let attempt = self.free_throw_attempt.saturating_add(1);
        let ft_pos = Court::free_throw_pos(shooter_is_home, &self.rules);
        self.pending_events.push(GameEvent::FreeThrowAttempt {
            shooter_id: shooter_id.clone(),
            attempt,
            made,
        });
        self.box_score.ft_attempts += 1;
        if made {
            self.box_score.ft_made += 1;
            if shooter_is_home {
                self.home_score += 1;
            } else {
                self.away_score += 1;
            }
        }
        self.free_throw_attempt = attempt;
        self.free_throws_remaining = self.free_throws_remaining.saturating_sub(1);
        self.sub_phase_timer = 0.0;
        if self.free_throws_remaining == 0 {
            self.free_throw_shooter = None;
            self.free_throw_attempt = 0;
            if made {
                let hoop = self.rules.court.hoop_pos(shooter_is_home);
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(hoop, self.ball_pos_3d);
            } else {
                self.start_free_throw_rebound(shooter_is_home, ft_pos);
            }
        }
    }

    fn start_free_throw_rebound(&mut self, shooter_is_home: bool, ft_pos: Vec2) {
        let hoop = self.rules.court.hoop_pos(shooter_is_home);
        let rebound_from = self.ball_pos_3d;
        let landing_spot =
            BallisticsEngine::compute_rebound_landing(ft_pos, hoop, &mut self.rng, &self.rules);
        self.shot_clock = self.rules.league.offensive_rebound_shot_clock_seconds;
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::FlightAndRebound);
        self.transition_ball_state(BallTrajectoryKind::RimRebound {
            from_pos: rebound_from.0,
            from_z: rebound_from.1,
            hoop_pos: hoop,
            target_landing: landing_spot.landing_pos,
            start_time: self.current_time,
            duration: landing_spot.flight_duration,
            peak_z: self.rules.rebound_peak_ft,
            last_touch_team: self.possession,
        });
    }

    /// Test/diagnostic hook: resolve the current free throw with a forced outcome.
    pub fn resolve_forced_free_throw(&mut self, made: bool) {
        if self.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_is_home = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.possession == Possession::Home);
        let attempt = self.free_throw_attempt.saturating_add(1);
        self.pending_events.push(GameEvent::FreeThrowAttempt {
            shooter_id: shooter_id.clone(),
            attempt,
            made,
        });
        if made {
            if shooter_is_home {
                self.home_score += 1;
            } else {
                self.away_score += 1;
            }
        }
        self.free_throw_attempt = attempt;
        self.free_throws_remaining = self.free_throws_remaining.saturating_sub(1);
        self.sub_phase_timer = 0.0;
        if self.free_throws_remaining == 0 {
            self.free_throw_shooter = None;
            self.free_throw_attempt = 0;
            if made {
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(Court::hoop_pos(shooter_is_home), self.ball_pos_3d);
            } else {
                self.start_free_throw_rebound(
                    shooter_is_home,
                    Court::free_throw_pos(shooter_is_home, &self.rules),
                );
            }
        }
    }

    fn execute_pass(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        _from_pos: Vec2,
        to_pos: Vec2,
        current_t: f32,
        inbound: bool,
    ) {
        let from_pos = self.ball_pos_3d.0;
        let initial_dist = self.physics.get_player(receiver_id).map(|p| (p.pos_ft - from_pos).length()).unwrap_or(20.0);
        let initial_dur = self.rules.pass_duration(initial_dist, inbound);
        let target_lead_pos = self
            .physics
            .get_player(receiver_id)
            .map(|p| {
                BallisticsEngine::extrapolate_receiver_pos(
                    p,
                    initial_dur.clamp(0.0, 0.4),
                    &self.rules,
                )
            })
            .unwrap_or(to_pos);

        self.active_windows.insert(
            passer_id.to_string(),
            ActionTimeWindow::new_pass(passer_id, current_t, &self.rules),
        );

        let pass_dist = (target_lead_pos - from_pos).length();
        let duration = self.rules.pass_duration(pass_dist, inbound);
        let receive_success =
            self.resolve_pass_success(passer_id, receiver_id, from_pos, target_lead_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos,
            to_pos: target_lead_pos,
            target_id: receiver_id.to_string(),
            start_time: current_t,
            duration,
            peak_z: self.rules.pass_peak_ft,
            inbound,
            receive_success,
        });
        self.last_passer_id = Some(passer_id.to_string());
        self.transition_phase(SubPhase::ActionExecution);
        self.pending_events.push(GameEvent::PassRelease {
            passer_id: passer_id.to_string(),
            receiver_id: receiver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            to_pos: (target_lead_pos.x, target_lead_pos.y),
        });
        if let Some(index) = self.player_index_for_id(receiver_id) {
            self.carrier_idx = index;
        }
        self.current_event = Some(if inbound { "INBOUND_PASS" } else { "PASS" }.to_string());
        self.current_callout = Some(if inbound {
            "界外发球进入飞行，接应点开始读取防守".to_string()
        } else {
            "突分策应！外线转移球创造空位机会".to_string()
        });
    }

    /// Resolve a pass once at release from spatial facts, player capabilities,
    /// and the configured officiating policy. Arrival only replays this fact.
    fn resolve_pass_success(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        from_pos: Vec2,
        to_pos: Vec2,
    ) -> bool {
        let (Some(passer), Some(receiver)) = (
            self.physics.get_player(passer_id).cloned(),
            self.physics.get_player(receiver_id).cloned(),
        ) else {
            return false;
        };
        let evaluation = SemanticEvaluator::pass(
            from_pos,
            to_pos,
            passer_id,
            receiver_id,
            &self.physics,
            &self.rules,
        );
        let catch_equilibrium = self
            .modulation
            .get(receiver_id)
            .map(|state| state.catch_equilibrium)
            .unwrap_or(1.0);
        matches!(
            ResolutionLayer::resolve_pass_arrival(
                &passer,
                &receiver,
                evaluation.lane_risk,
                evaluation.target_openness,
                catch_equilibrium,
                &self.rules.resolve.pass,
                self.rules.resolve.base_rates.pass_success,
                &mut self.rng,
            ),
            ResolutionOutcome::PassReceived { .. }
        )
    }

    /// Resolves the rebound winner from the landing window, then delegates
    /// the contest probability to the officiating layer.
    fn resolve_rebounder(&mut self, landing: Vec2) -> String {
        let defensive_team = match self.possession {
            Possession::Home => "away",
            Possession::Away => "home",
        };
        let offensive_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let search_radius = self.rules.court.width_ft.hypot(self.rules.court.height_ft);
        let mut offensive_candidates =
            self.rebound_candidates(landing, offensive_team, search_radius);
        let mut defensive_candidates =
            self.rebound_candidates(landing, defensive_team, search_radius);
        offensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        defensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        let Some((offensive_id, offensive_distance)) = offensive_candidates.first() else {
            return defensive_candidates
                .first()
                .map(|(id, _)| id.clone())
                .unwrap_or_else(|| self.new_possession_pg());
        };
        let Some((defensive_id, defensive_distance)) = defensive_candidates.first() else {
            return offensive_id.clone();
        };
        let Some(offensive_player) = self.physics.get_player(offensive_id).cloned() else {
            return defensive_id.clone();
        };
        let Some(defensive_player) = self.physics.get_player(defensive_id).cloned() else {
            return offensive_id.clone();
        };

        match ResolutionLayer::resolve_rebound(
            &offensive_player,
            &defensive_player,
            *offensive_distance,
            *defensive_distance,
            &self.rules.resolve.rebound,
            &mut self.rng,
        ) {
            ResolutionOutcome::ReboundSecured { rebounder_id, .. } => rebounder_id,
            _ => defensive_id.clone(),
        }
    }

    fn rebound_candidates(
        &self,
        landing: Vec2,
        team: &str,
        search_radius: f32,
    ) -> Vec<(String, f32)> {
        self.physics
            .query_nearby(
                landing,
                search_radius,
                &nba_physics::EntityFilter::Team(team.to_string()),
            )
            .into_iter()
            .filter_map(|id| {
                self.physics
                    .get_player(&id)
                    .map(|player| (id, (player.pos_ft - landing).length()))
            })
            .collect()
    }

    fn update_scope_completion(&mut self) {
        if !self.scope_active || self.simulation_complete {
            return;
        }
        let reached = match self.scope_boundary {
            ScopeBoundary::Possessions => self.completed_possessions >= self.target_possessions,
            ScopeBoundary::Period { last_period } => {
                self.period > last_period
                    || (self.period == last_period
                        && matches!(
                            self.game_flow,
                            GameFlowState::QuarterEnd | GameFlowState::Halftime
                        ))
            }
            ScopeBoundary::Game => self.game_flow == GameFlowState::GameEnd,
        };
        if reached {
            self.simulation_complete = true;
            self.settle_scope_ball(self.ball_pos_3d);
        }
    }

    // ========================================================================
    // Possession boundary
    fn complete_possession(&mut self) {
        // M8"回合零遗漏"：若本回合结束路径此前没有产出总结（如松球易主、
        // 死球转换），先补一条兜底总结再推进计数。
        let count = self.completed_possessions as u64;
        if self.last_possession_summary_index != Some(count) {
            self.emit_possession_summary("UNATTRIBUTED_END", None, None, None);
        }
        self.completed_possessions = self.completed_possessions.saturating_add(1);
        self.update_scope_completion();
    }

    /// 犯满离场的强制换人（tactics.md 2.3.3 优先级 1：强制换人）。
    ///
    /// 替补实体已在物理层注册（on_court=false，不参与运动学）；换人 =
    /// 翻转在场标志并交接场上位置。候选按 id 字典序取最小者（确定性）。
    /// 若替补全部不可用，则保留原球员（保持 5v5 不变量优先）。
    pub fn forced_substitution(&mut self, out_player_id: &str) {
        let team = match self.physics.get_player(out_player_id) {
            Some(p) => p.team.clone(),
            None => return,
        };
        // 犯规方不可能持球；若命中持球者（异常调用），拒绝换人以保球权一致。
        if self.physics.get_player(out_player_id).is_some_and(|p| p.has_ball) {
            return;
        }
        let max_fouls = self.rules.league.max_personal_fouls;
        let mut candidates: Vec<String> = self
            .physics
            .get_players()
            .values()
            .filter(|p| {
                p.team == team
                    && !p.on_court
                    && p.foul_count < max_fouls
            })
            .map(|p| p.id.clone())
            .collect();
        candidates.sort();
        let Some(in_player_id) = candidates.first().cloned() else {
            return;
        };
        let (out_pos, out_action_slot) = match self.physics.get_player(out_player_id) {
            Some(p) => (p.pos_ft, p.slot.clone()),
            None => return,
        };
        // 离场者退至替补席区（界外，运动学冻结）。
        let bench_spot = Vec2::new(
            self.rules.court.width_ft / 2.0
                + if team == "home" { -6.0 } else { 6.0 },
            self.rules.court.height_ft + 4.0,
        );
        if let Some(p) = self.physics.get_player_mut(out_player_id) {
            p.on_court = false;
            p.has_ball = false;
            p.action = "Bench".to_string();
            p.pos_ft = bench_spot;
            p.target_pos_ft = bench_spot;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
        }
        if let Some(p) = self.physics.get_player_mut(&in_player_id) {
            p.on_court = true;
            p.action = "SetPosition".to_string();
            p.pos_ft = out_pos;
            p.target_pos_ft = out_pos;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
            p.slot = out_action_slot;
        }
        // 战术绑定名册同步（slot 绑定按名册顺序索引）。
        for roster in [&mut self.home_roster_order, &mut self.away_roster_order] {
            if let Some(slot_ref) = roster.iter_mut().find(|id| *id == out_player_id) {
                *slot_ref = in_player_id.clone();
            }
        }
        self.pending_events.push(GameEvent::Substitution {
            team,
            out_player: out_player_id.to_string(),
            in_player: in_player_id,
        });
    }

    fn settle_scope_ball(&mut self, position: (Vec2, f32)) {
        self.ball_pos_3d = position;
        self.transition_ball_state(BallTrajectoryKind::Dead {
            pos: position.0,
            z: position.1,
            last_touch_team: self.possession,
        });
        self.set_game_flow(GameFlowState::DeadBall);
        self.transition_phase(SubPhase::DeadBallReset);
    }

    // 阶段转换器
    fn start_violation_turnover(&mut self, _kind: ViolationKind) {
        self.box_score.turnovers += 1;
        self.emit_possession_summary("TURNOVER_VIOLATION", None, None, None);
        let current_ball_3d = self.ball_pos_3d;
        let baseline = Court::nearest_boundary_with_geometry(current_ball_3d.0, self.rules.court);
        self.start_inbound_transition(baseline, current_ball_3d);
    }

    fn start_steal_transition(&mut self, stealer_id: String, intercept_pos: Vec2) {
        self.box_score.turnovers += 1;
        self.emit_possession_summary("TURNOVER_STEAL", None, Some(stealer_id.clone()), None);
        self.complete_possession();
        if self.simulation_complete {
            self.settle_scope_ball((intercept_pos, self.rules.ball_holder_height_ft));
            return;
        }
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.shot_clock = self.rules.league.shot_clock_seconds;
        self.carrier_idx = self.player_index_for_id(&stealer_id).unwrap_or(0);
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::Initiation);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.ball_pos_3d = (intercept_pos, self.rules.ball_holder_height_ft);
        self.transition_ball_state(BallTrajectoryKind::Held {
            carrier_id: stealer_id,
        });
    }

    fn start_rebound_outlet(&mut self, rebounder_id: String, reb_pos: Vec2, is_offensive: bool) {
        if !is_offensive {
            self.complete_possession();
            if self.simulation_complete {
                self.settle_scope_ball((reb_pos, self.rules.chest_height_ft));
                return;
            }
        }
        self.possession = if is_offensive {
            self.possession
        } else {
            opposite(self.possession)
        };
        if !is_offensive {
            self.possession_id += 1;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.shot_clock = if is_offensive {
            self.rules.league.offensive_rebound_shot_clock_seconds
        } else {
            self.rules.league.shot_clock_seconds
        };
        self.carrier_idx = self.player_index_for_id(&rebounder_id).unwrap_or(0);
        let target_id = if is_offensive {
            rebounder_id.clone()
        } else {
            let candidate = self.new_possession_pg();
            if candidate == rebounder_id {
                let team = match self.possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                self.team_roster_ids(team)
                    .into_iter()
                    .find(|id| id != &rebounder_id)
                    .unwrap_or_else(|| rebounder_id.clone())
            } else {
                candidate
            }
        };
        if target_id == rebounder_id {
            self.transition_phase(SubPhase::Initiation);
            self.last_decision_time = -self.rules.decision_interval_seconds;
            let actual_pos = self.physics.get_player(&rebounder_id).map(|p| p.pos_ft).unwrap_or(reb_pos);
            let dist = (actual_pos - reb_pos).length();
            let max_speed = self.rules.ball_max_speed_ftps * 0.45;
            let transfer_dur = (dist / max_speed).max(0.18);
            self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                from_pos: reb_pos,
                from_z: self.rules.ball_holder_height_ft,
                carrier_id: rebounder_id,
                start_time: self.current_time,
                duration: transfer_dur,
            });
            return;
        }
        let target_pos = self
            .physics
            .get_player(&target_id)
            .map(|p| p.pos_ft)
            .unwrap_or_else(|| {
                reb_pos + Vec2::new(self.rules.rebound_outlet_fallback_distance_ft, 0.0)
            });
        let pass_dist = (target_pos - reb_pos).length();
        let receive_success =
            self.resolve_pass_success(&rebounder_id, &target_id, reb_pos, target_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos: reb_pos,
            to_pos: target_pos,
            target_id: target_id.clone(),
            start_time: self.current_time,
            duration: self.rules.pass_duration(pass_dist, false),
            peak_z: self.rules.pass_peak_ft,
            inbound: false,
            receive_success,
        });
        self.last_passer_id = Some(rebounder_id);
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::LiveBall);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.current_event = Some("OUTLET_PASS".to_string());
        self.current_callout = Some(format!(
            "篮板球转入{}，发动战术：{}",
            if is_offensive {
                "二次进攻"
            } else {
                "转换进攻"
            },
            self.tactical_set.name_zh()
        ));
    }

    fn start_loose_ball_transition(&mut self, player_id: String, position: Vec2) {
        let player_team = self
            .physics
            .get_player(&player_id)
            .map(|player| player.team.as_str());
        let secured_possession = match player_team {
            Some("home") => Possession::Home,
            Some("away") => Possession::Away,
            _ => self.possession,
        };
        let possession_changed = secured_possession != self.possession;
        if possession_changed {
            self.complete_possession();
            if self.simulation_complete {
                self.settle_scope_ball((position, self.rules.ball_holder_height_ft));
                return;
            }
            self.possession = secured_possession;
            self.possession_id += 1;
            self.shot_clock = self.rules.league.shot_clock_seconds;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.carrier_idx = self.player_index_for_id(&player_id).unwrap_or(0);
        self.ball_pos_3d = (position, self.rules.ball_holder_height_ft);
        self.transition_ball_state(BallTrajectoryKind::Held {
            carrier_id: player_id.clone(),
        });
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::Initiation);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.last_passer_id = None;
    }

    fn start_inbound_transition(&mut self, baseline_pos: Vec2, current_ball_3d: (Vec2, f32)) {
        self.complete_possession();
        if self.simulation_complete {
            self.settle_scope_ball(current_ball_3d);
            return;
        }
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = self.rules.league.shot_clock_seconds;
        self.sync_team_tactics();
        self.carrier_idx = 0;
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::DeadBall);
        self.inbound_baseline = baseline_pos;
        let inbounder_id = self.new_possession_pg();
        let release_pos = Court::inbound_release_pos_with_geometry(
            baseline_pos,
            self.rules.inbound_release_depth_ft,
            self.rules.court,
        );
        let duration = self
            .rules
            .pass_duration((release_pos - current_ball_3d.0).length(), true);
        self.transition_ball_state(BallTrajectoryKind::InboundTransfer {
            from_pos: current_ball_3d.0,
            from_z: current_ball_3d.1,
            baseline_pos: release_pos,
            inbounder_id,
            start_time: self.current_time,
            duration,
        });
        self.inbound_elapsed = 0.0;
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.current_event = Some("INBOUND_SETUP".to_string());
        self.current_callout = Some(format!(
            "界外发球准备，呼叫战术：{}",
            self.tactical_set.name_zh()
        ));
    }
    fn finish_period(&mut self) {
        let previous_period = self.period;
        if self.period < self.rules.league.regulation_periods {
            self.team_fouls_home = 0;
            self.team_fouls_away = 0;
            let is_halftime = self.rules.league.regulation_periods > 1
                && self.period == self.rules.league.regulation_periods / 2;
            self.set_game_flow(if is_halftime {
                GameFlowState::Halftime
            } else {
                GameFlowState::QuarterEnd
            });
            self.period_break_elapsed = 0.0;
            self.transition_phase(SubPhase::DeadBallReset);
            self.pending_events.push(GameEvent::PhaseTransition {
                from: PhaseType::SetPlay,
                to: PhaseType::DeadBallReset,
            });
            self.current_event = Some("PERIOD_END".to_string());
            self.current_callout = Some(format!(
                "第{}节结束，进入{}休息",
                previous_period,
                if previous_period == self.rules.league.regulation_periods / 2 {
                    "中场"
                } else {
                    "节间"
                }
            ));
        } else if self.home_score == self.away_score {
            self.period += 1;
            self.game_clock = self.rules.league.overtime_duration_seconds;
            self.shot_clock = self.rules.league.shot_clock_seconds;
            self.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::Overtime);
            self.transition_phase(SubPhase::Initiation);
            self.current_event = Some("OVERTIME_START".to_string());
        } else {
            self.set_game_flow(GameFlowState::GameEnd);
            self.transition_phase(SubPhase::DeadBallReset);
            self.pending_events.push(GameEvent::PhaseTransition {
                from: self.phase_type(),
                to: PhaseType::DeadBallReset,
            });
            self.current_event = Some("GAME_END".to_string());
            self.current_callout = Some(format!(
                "比赛结束，{}获胜",
                if self.home_score > self.away_score {
                    "主队"
                } else {
                    "客队"
                }
            ));
        }
        self.update_scope_completion();
    }

    /// Returns the first configured starter for the team currently in control.
    fn new_possession_pg(&self) -> String {
        let roster = match self.possession {
            Possession::Home => &self.home_roster_order,
            Possession::Away => &self.away_roster_order,
        };
        roster.first().cloned().unwrap_or_default()
    }
    fn team_roster_ids(&self, team: &str) -> Vec<String> {
        match team {
            "home" => self.home_roster_order.clone(),
            "away" => self.away_roster_order.clone(),
            _ => Vec::new(),
        }
    }

    pub fn new_possession_pg_for_test(&self) -> String {
        self.new_possession_pg()
    }

    #[doc(hidden)]
    pub fn execute_shot_for_test(&mut self, shooter_id: &str, from_pos: Vec2, is_three: bool) {
        self.execute_shot(shooter_id, from_pos, is_three, self.current_time);
    }
    pub fn possession(&self) -> nba_domain::Possession {
        self.possession
    }

    pub fn force_possession_for_test(&mut self, possession: nba_domain::Possession) {
        self.possession = possession;
    }

    fn player_id_for_team_index(&self, team: &str, index: usize) -> Option<String> {
        self.team_roster_ids(team).into_iter().nth(index)
    }

    fn player_index_for_id(&self, player_id: &str) -> Option<usize> {
        let team = self.physics.get_player(player_id)?.team.as_str();
        self.team_roster_ids(team)
            .into_iter()
            .position(|id| id == player_id)
    }

    // ========================================================================
    // Tick 协议输出
    // ========================================================================

    /// Returns a protocol snapshot without advancing the simulation.
    pub fn snapshot(&self) -> StreamTick {
        self.build_tick()
    }

    fn build_tick(&self) -> StreamTick {
        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::ControlTransfer {
                carrier_id,
                ..
            } => Some(carrier_id.clone()),
            _ => None,
        };
        let mut render_players = self
            .physics
            .get_players()
            .values()
            .map(|p| {
                let norm = Court::ft_to_norm_with_geometry(p.pos_ft, self.rules.court);
                RenderPlayer {
                    id: p.id.clone(),
                    jersey: p.jersey.clone(),
                    team: p.team.clone(),
                    x: (norm.x * 1_000_000.0).round() / 1_000_000.0,
                    y: (norm.y * 1_000_000.0).round() / 1_000_000.0,
                    zone: format!(
                        "{:?}",
                        self.rules.court.region(
                            p.pos_ft,
                            p.team == "home",
                            self.rules.league.three_point_distance_ft,
                        )
                    ),
                    has_ball: active_carrier.as_deref() == Some(p.id.as_str()),
                    on_court: p.on_court,
                    action: p.action.clone(),
                    slot: p.slot.clone(),
                    morale: p.morale.clone(),
                    stm: (p.stamina * 10.0).round() / 10.0,
                    stm_max: p.max_stamina,
                    foul_count: p.foul_count,
                }
            })
            .collect::<Vec<_>>();
        render_players.sort_by(|a, b| a.id.cmp(&b.id));
        let ball_norm = Court::ft_to_norm_with_geometry(self.ball_pos_3d.0, self.rules.court);

        let ball_status = match &self.ball_state {
            BallTrajectoryKind::Held { .. } => "HELD",
            BallTrajectoryKind::ControlTransfer { .. } => "CONTROL_TRANSFER",
            BallTrajectoryKind::InboundTransfer { .. } => "INBOUND_TRANSFER",
            BallTrajectoryKind::InboundReady { .. } => "INBOUND_READY",
            BallTrajectoryKind::Pass { .. } => "PASS",
            BallTrajectoryKind::Drive { .. } => "DRIVE",
            BallTrajectoryKind::Shot { .. } => "SHOT",
            BallTrajectoryKind::RimRebound { .. } => "REBOUND",
            BallTrajectoryKind::LooseBall { .. } => "LOOSE_BALL",
            BallTrajectoryKind::Dead { .. } => "DEAD",
        };
        let home_team = RenderTeam {
            id: self.home_team.id.clone(),
            name: self.home_team.name.clone(),
            short_name: self.home_team.short_name.clone(),
        };
        let away_team = RenderTeam {
            id: self.away_team.id.clone(),
            name: self.away_team.name.clone(),
            short_name: self.away_team.short_name.clone(),
        };
        let possession_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let frame = RenderFrame {
            t: (self.current_time * 100.0).round() / 100.0,
            t_game: (self.game_clock * 10.0).round() / 10.0,
            shot_clock: (self.shot_clock * 10.0).round() / 10.0,
            period: self.period,
            phase: format!("{:?}", self.sub_phase),
            possession_id: self.possession_id,
            possession_team: possession_team.to_string(),
            home_team,
            away_team,
            score: RenderScore {
                home: self.home_score,
                away: self.away_score,
            },
            players: render_players,
            ball: RenderBall {
                x: ball_norm.x,
                y: ball_norm.y,
                z: (self.ball_pos_3d.1 * 100.0).round() / 100.0,
                status: ball_status.to_string(),
                holder_id: active_carrier.clone(),
            },
            event_type: self.current_event.clone(),
            events: self.current_event_types.clone(),
            game_flow: format!("{:?}", self.game_flow),
            team_fouls_home: self.team_fouls_home,
            team_fouls_away: self.team_fouls_away,
            free_throws_remaining: self.free_throws_remaining,
            defensive_tactic: Some(self.defensive_tactic_name()),
            event_sequence: self.event_sequence,
            event_log: self.current_event_log.clone(),
            simulation_complete: self.simulation_complete,
            completed_possessions: self.completed_possessions,
            target_possessions: self.target_possessions,
            callout: self.current_callout.clone(),
            intensity: self.current_intensity.clone(),
            rules: FrameRules {
                tick_seconds: self.rules.tick_seconds,
                court_width_ft: self.rules.court.width_ft,
                court_height_ft: self.rules.court.height_ft,
                hoop_left_x_ft: self.rules.court.hoop_left_x_ft,
                hoop_right_x_ft: self.rules.court.hoop_right_x_ft,
                hoop_y_ft: self.rules.court.hoop_y_ft,
                player_radius_ft: self.rules.player_radius_ft,
                min_player_separation_ft: self.rules.min_player_separation_ft,
                separation_safety_margin_ft: self.rules.separation_safety_margin_ft,
                max_player_speed_ftps: self.rules.max_player_speed_ftps,
                max_player_accel_ftps2: self.rules.max_player_accel_ftps2,
                ball_max_speed_ftps: self.rules.ball_max_speed_ftps,
                three_point_distance_ft: self.rules.league.three_point_distance_ft,
                shot_clock_seconds: self.rules.league.shot_clock_seconds,
                holder_leash_ft: self.rules.invariant_holder_leash_ft,
                speed_tolerance_ftps: self.rules.invariant_speed_tolerance_ftps,
                ball_z_max_ft: self.rules.ball_z_max_ft,
            },
            debug: self.last_decision_trace.clone(),
        };

        StreamTick {
            frame,
            tactical_set: self.tactical_set.name_zh().to_string(),
            game_clock: (self.game_clock * 10.0).round() / 10.0,
            keyframe_index: None,
        }
    }
}

fn physics_fact_to_event(fact: nba_physics::PhysicsFact) -> GameEvent {
    match fact {
        nba_physics::PhysicsFact::BoundaryCross {
            entity_id,
            attempted_pos,
            boundary_name,
        } => GameEvent::BoundaryCross {
            player_id: entity_id,
            pos: (attempted_pos.x, attempted_pos.y),
            boundary_name,
        },
    }
}

fn semantic_contact_to_event(contact: &SemanticContact) -> GameEvent {
    let relative_velocity = contact.raw.relative_velocity.unwrap_or(Vec2::ZERO);
    GameEvent::Contact {
        player_a: contact.raw.entity_a.clone(),
        player_b: contact.raw.entity_b.clone(),
        impact_speed: relative_velocity.length(),
        contact_normal: (contact.raw.normal.x, contact.raw.normal.y),
        is_screen: matches!(
            contact.kind,
            nba_semantics::ContactKind::LegalScreen
                | nba_semantics::ContactKind::IllegalScreenCandidate
        ),
        semantic_kind: format!("{:?}", contact.kind),
        semantic_severity: format!("{:?}", contact.severity),
        possessor_id: contact.context.possessor.clone(),
        legal_position: contact.context.legal_position,
    }
}

fn opposite(p: Possession) -> Possession {
    match p {
        Possession::Home => Possession::Away,
        Possession::Away => Possession::Home,
    }
}
impl MatchEngine {
    fn defensive_tactic_name(&self) -> String {
        match self.possession {
            Possession::Home => self.away_defensive_tactic.name_zh().to_string(),
            Possession::Away => self.home_defensive_tactic.name_zh().to_string(),
        }
    }
}

fn team_name_zh(is_home: bool) -> &'static str {
    if is_home {
        "主队"
    } else {
        "客队"
    }
}

/// 转换决策追踪到协议调试层。
fn convert_trace(trace: &nba_decision::pipeline::DecisionTrace) -> DecisionDebug {
    DecisionDebug {
        player: trace.player_id.clone(),
        chosen: trace.chosen_label.clone(),
        utilities: trace
            .utilities
            .iter()
            .map(|(k, u)| DebugUtility {
                kind: k.clone(),
                utility: *u,
            })
            .collect(),
        probabilities: trace
            .probabilities
            .iter()
            .map(|(k, p)| DebugProb {
                kind: k.clone(),
                prob: *p,
            })
            .collect(),
        flags: trace
            .flags_full
            .iter()
            .map(|(id, reason, penalty)| DebugFlag {
                constraint: id.to_string(),
                reason: reason.clone(),
                penalty: *penalty,
            })
            .collect(),
        blocked: trace
            .blocked
            .iter()
            .map(|(cand, why)| format!("{} ✗ {}", cand, why))
            .collect(),
        active_constraints: trace
            .active_constraints
            .iter()
            .map(|s| s.to_string())
            .collect(),
        enforcement: trace.enforcement.clone(),
    }
}
