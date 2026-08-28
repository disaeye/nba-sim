//! 比赛模拟编排层：阶段驱动 + 约束过滤 + 效用决策 + 事件化输出。
//!
//! 这是引擎的主循环（架构文档 §11），取代旧的散落 if 决策：
//! - 阶段状态机推进（Inbound → Transition/SetPlay → Resolution → Rebound/DeadBallReset）
//! - 世界级 Runtime 约束检查（24 秒等）
//! - 决策系统按约束管线产出动作意图
//! - 执行系统更新球弹道与物理
//! - 裁决系统处理抢断/得分/篮板与阶段转换

use std::fs::File;
use std::io::{BufWriter, Write};
use glam::Vec2;
use rand::Rng;
use rand::SeedableRng;
use rand::rngs::StdRng;
use std::collections::HashMap;

use nba_officiating::config::ResolveConfig;
use nba_physics::ballistics::{BallisticsEngine, BallTrajectoryKind};
use nba_decision::constraint::{CandidateAction, ConstraintContext, EnforcementAction, PhaseType, ViolationKind};
use nba_domain::court::Court;
use nba_decision::pipeline::{DecisionSystem, DecisionOutput};
use nba_domain::event::PhysicsEvent;
use nba_physics::movement::{PhysicsWorld, PlayerPhysicsState, LocomotionState};
use nba_domain::action_window::ActionTimeWindow;
use nba_decision::modulation::{CoachStrategy, PlayerModulationState};
use nba_protocol::*;
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_decision::tactics::{TacticalPlanner, TacticalSet, SubPhase, Possession};
use nba_physics::spatial::SpatialGeometry;

/// 主模拟状态。每个 possession 是阶段事件流的最小产出单位。
pub struct MatchEngine {
    pub physics: PhysicsWorld,
    pub possession: Possession,
    pub possession_id: u32,
    pub sub_phase: SubPhase,
    pub sub_phase_timer: f32,
    pub tactical_set: TacticalSet,
    pub carrier_idx: usize, // 0..5
    pub shot_clock: f32,
    pub game_clock: f32,
    pub current_time: f32,
    pub period: u32,
    pub home_score: u32,
    pub away_score: u32,
    pub ball_pos_3d: (Vec2, f32), // (x, y), z in feet
    pub ball_state: BallTrajectoryKind,
    pub active_windows: HashMap<String, ActionTimeWindow>,
    pub current_event: Option<String>,
    pub current_callout: Option<String>,
    pub current_intensity: Option<String>,
    pub rng: StdRng,
    pub target_possessions: usize,
    pub completed_possessions: usize,

    // --- 约束驱动架构新增子系统 ---
    /// 决策系统（候选生成 + 约束管线 + 效用采样）
    pub decision: DecisionSystem,
    /// 裁决概率配置（adjudication 模块接线）
    pub resolve_config: ResolveConfig,
    /// 球员心理/体力调制状态（modulation 模块接线，10 人）
    pub modulation: Vec<PlayerModulationState>,
    /// 教练宏观策略（modulation 模块接线）
    pub coach: CoachStrategy,
    /// 本 tick 的决策追踪（调试层）
    pub last_decision_trace: Option<Box<DecisionDebug>>,
    /// 死球标志（违例/得分后未发球期间）
    pub is_dead_ball: bool,
    /// 每回合决策节流：上次决策时间
    pub last_decision_time: f32,
    /// 24 秒已触发标志（防重复裁决）
    pub shot_clock_violated: bool,
}

pub type Simulation = MatchEngine;

const HOME_JERSEYS: [&str; 5] = ["0", "7", "4", "8", "9"];
const AWAY_JERSEYS: [&str; 5] = ["23", "3", "15", "1", "28"];

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        let rng = StdRng::seed_from_u64(seed);
        let mut physics = PhysicsWorld::new();

        let home_jerseys = [
            ("H_1", "0", 28.0, 25.0),
            ("H_2", "7", 35.0, 10.0),
            ("H_3", "4", 35.0, 40.0),
            ("H_4", "8", 22.0, 16.0),
            ("H_5", "9", 15.0, 25.0)
        ];
        for (id, jersey, x, y) in home_jerseys {
            physics.register_player(PlayerPhysicsState {
                id: id.to_string(),
                jersey: jersey.to_string(),
                team: "home".to_string(),
                pos_ft: Vec2::new(x, y),
                vel_ft: Vec2::ZERO,
                accel_ft: Vec2::ZERO,
                target_pos_ft: Vec2::new(x, y),
                target_speed_ftps: 0.0,
                has_ball: id == "H_1",
                action: "Initiate".to_string(),
                slot: "PG".to_string(),
                morale: "Normal".to_string(),
                stamina: 100.0,
                max_stamina: 100.0,
                locomotion: LocomotionState::Idle,
                facing_dir: Vec2::new(1.0, 0.0),
                turn_decel_timer: 0.0,
                is_locked_kinematics: false,
            });
        }

        let away_jerseys = [
            ("A_1", "23", 22.0, 25.0),
            ("A_2", "3", 28.0, 12.0),
            ("A_3", "15", 28.0, 38.0),
            ("A_4", "1", 16.0, 18.0),
            ("A_5", "28", 10.0, 25.0)
        ];
        for (id, jersey, x, y) in away_jerseys {
            physics.register_player(PlayerPhysicsState {
                id: id.to_string(),
                jersey: jersey.to_string(),
                team: "away".to_string(),
                pos_ft: Vec2::new(x, y),
                vel_ft: Vec2::ZERO,
                accel_ft: Vec2::ZERO,
                target_pos_ft: Vec2::new(x, y),
                target_speed_ftps: 0.0,
                has_ball: false,
                action: "DropDefend".to_string(),
                slot: "PG".to_string(),
                morale: "Normal".to_string(),
                stamina: 100.0,
                max_stamina: 100.0,
                locomotion: LocomotionState::Idle,
                facing_dir: Vec2::new(-1.0, 0.0),
                turn_decel_timer: 0.0,
                is_locked_kinematics: false,
            });
        }

        let carrier_idx = 0;
        let initial_carrier_id = "H_1".to_string();
        let initial_pos = physics.get_player(&initial_carrier_id).map(|p| p.pos_ft).unwrap_or(Vec2::new(28.0, 25.0));

        Self {
            physics,
            possession: Possession::Home,
            possession_id: 1,
            sub_phase: SubPhase::Initiation,
            sub_phase_timer: 0.0,
            tactical_set: TacticalSet::HighPickAndRoll,
            carrier_idx,
            shot_clock: 24.0,
            game_clock: 720.0,
            current_time: 0.0,
            period: 1,
            home_score: 0,
            away_score: 0,

            ball_pos_3d: (initial_pos, 3.5),
            ball_state: BallTrajectoryKind::Held { carrier_id: initial_carrier_id },
            active_windows: HashMap::new(),
            current_event: Some("TIPOFF".to_string()),
            current_callout: Some("比赛开始！凯尔特人获得第一攻球权，组织高位挡拆战术！".to_string()),
            current_intensity: Some("BuildUp".to_string()),
            rng,
            target_possessions: 10,
            completed_possessions: 0,
            decision: DecisionSystem::new(),
            resolve_config: ResolveConfig::default(),
            modulation: vec![PlayerModulationState::default(); 10],
            coach: CoachStrategy::default(),
            last_decision_trace: None,
            is_dead_ball: false,
            last_decision_time: -10.0,
            shot_clock_violated: false,
        }
    }

    pub fn is_finished(&self) -> bool {
        self.completed_possessions >= self.target_possessions || self.game_clock <= 0.0
    }

    pub fn simulate_scope_and_export(&mut self, scope: &str, out_path: &str) -> std::io::Result<(usize, String)> {
        let (target_poss, scope_desc) = match scope {
            "1p" => (1, "1 Possession".to_string()),
            "5p" => (5, "5 Possessions".to_string()),
            "10p" => (10, "10 Possessions".to_string()),
            "1q" => (35, "1 Quarter (approx 35 possessions)".to_string()),
            "full" => (140, "Full Game (approx 140 possessions)".to_string()),
            other => {
                if let Some(stripped) = other.strip_suffix('p') {
                    let p = stripped.parse::<usize>().unwrap_or(10);
                    (p, format!("{} Possessions", p))
                } else {
                    (10, "10 Possessions".to_string())
                }
            }
        };

        self.target_possessions = target_poss;

        let file = File::create(out_path)?;
        let mut writer = BufWriter::new(file);

        let mut ticks_count = 0;
        while !self.is_finished() {
            let tick = self.step();
            let json = serde_json::to_string(&tick)?;
            writer.write_all(json.as_bytes())?;
            writer.write_all(b"\n")?;
            ticks_count += 1;
        }

        writer.flush()?;
        Ok((ticks_count, scope_desc))
    }

    /// 当前阶段映射到约束阶段类型（文档 §5 阶段约束绑定）。
    fn phase_type(&self) -> PhaseType {
        match self.sub_phase {
            SubPhase::Initiation => PhaseType::Inbound,
            SubPhase::ActionExecution => PhaseType::SetPlay,
            SubPhase::ShotAttempt => PhaseType::Resolution,
            SubPhase::FlightAndRebound => PhaseType::Rebound,
            SubPhase::DeadBallReset => PhaseType::DeadBallReset,
        }
    }

    /// 构建约束求值上下文。
    fn constraint_ctx<'a>(&'a self) -> ConstraintContext<'a> {
        let possession_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        ConstraintContext {
            players: self.physics.get_players(),
            ball_pos: self.ball_pos_3d.0,
            possession_team,
            shot_clock: self.shot_clock,
            game_clock: self.game_clock,
            phase: self.phase_type(),
            ball_in_flight: matches!(
                self.ball_state,
                BallTrajectoryKind::Pass { .. } | BallTrajectoryKind::Shot { .. }
            ),
            is_dead_ball: self.is_dead_ball,
        }
    }

    /// 持球人 id。
    fn carrier_id(&self) -> String {
        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => carrier_id.clone(),
            _ => format!("{}_{}", if self.possession == Possession::Home { "H" } else { "A" }, self.carrier_idx + 1),
        }
    }

    /// 教练策略重评估（每回合开始时调用）。
    fn update_coach_strategy(&mut self) {
        let score_diff = self.home_score as i32 - self.away_score as i32;
        self.coach = CoachStrategy::evaluate(score_diff, self.period, self.game_clock);
    }

    pub fn step(&mut self) -> StreamTick {
        let dt = 0.04; // 25 Hz
        self.current_time += dt;
        self.sub_phase_timer += dt;
        self.shot_clock = (self.shot_clock - dt).max(0.0);
        self.game_clock = (self.game_clock - dt).max(0.0);
        let current_t = self.current_time;

        self.current_event = None;
        self.current_intensity = None;
        self.last_decision_trace = None;

        let is_home = self.possession == Possession::Home;
        let _hoop = Court::hoop_pos(is_home);

        // Advance & Clean Action Time Windows
        self.active_windows.retain(|_, window| {
            window.update(current_t);
            !window.is_finished(current_t)
        });

        // ============================================================
        // 0. 世界级 Runtime 约束检查（24 秒违例等，文档 §4.2.1）
        // ============================================================
        let ctx = self.constraint_ctx();
        if let Some((constraint, enforcement)) = self.decision.registry.evaluate_world(&ctx) {
            match enforcement {
                EnforcementAction::Violation { kind } => {
                    self.current_event = Some("VIOLATION".to_string());
                    self.current_callout = Some(format!("{}！{} 失去球权", constraint.id, team_name_zh(is_home)));
                    self.start_violation_turnover(kind);
                    return self.build_tick();
                }
                _ => {}
            }
        }

        // ============================================================
        // 1. 阶段状态机推进（文档 §5 阶段约束绑定）
        // ============================================================
        let mut decision_output: Option<DecisionOutput> = None;
        match self.sub_phase {
            SubPhase::Initiation => {
                if self.sub_phase_timer >= 2.2 {
                    self.sub_phase = SubPhase::ActionExecution;
                    self.sub_phase_timer = 0.0;
                    self.current_event = Some("TACTICAL_EXECUTION".to_string());
                    self.current_callout = Some(format!("战术发起：{}", self.tactical_set.name_zh()));
                }
            }
            SubPhase::ActionExecution => {
                // 决策节流：每 0.8s 决策一次
                if current_t - self.last_decision_time >= 0.8 {
                    self.last_decision_time = current_t;
                    let carrier = self.carrier_id();
                    let stamina = self.physics.get_player(&carrier).map(|p| p.stamina).unwrap_or(100.0);
                    let morale_bias = self.morale_bias_for(&carrier);
                    let possession_team = if is_home { "home" } else { "away" };
                    let ctx = ConstraintContext {
                        players: self.physics.get_players(),
                        ball_pos: self.ball_pos_3d.0,
                        possession_team,
                        shot_clock: self.shot_clock,
                        game_clock: self.game_clock,
                        phase: self.phase_type(),
                        ball_in_flight: matches!(
                            self.ball_state,
                            BallTrajectoryKind::Pass { .. } | BallTrajectoryKind::Shot { .. }
                        ),
                        is_dead_ball: self.is_dead_ball,
                    };
                    let decision = &self.decision;
                    let mut rng = std::mem::replace(&mut self.rng, StdRng::seed_from_u64(0));
                    decision_output = decision.decide_on_ball(&ctx, &carrier, stamina, morale_bias, &mut rng);
                    self.rng = rng;
                }
            }
            SubPhase::ShotAttempt | SubPhase::FlightAndRebound | SubPhase::DeadBallReset => {}
        }

        // ============================================================
        // 2. 执行决策输出（约束已过滤，直接执行，文档 §6 流水线末端）
        // ============================================================
        if let Some(out) = decision_output {
            match out.action {
                CandidateAction::Shoot { shooter_id, from_pos, is_three } => {
                    self.execute_shot(&shooter_id, from_pos, is_three, current_t);
                }
                CandidateAction::Pass { passer_id, receiver_id, from_pos, to_pos } => {
                    self.execute_pass(&passer_id, &receiver_id, from_pos, to_pos, current_t);
                }
                CandidateAction::Dwell { .. } => {
                    // 观察等待：无操作
                }
            }
            self.last_decision_trace = Some(Box::new(convert_trace(&out.trace)));
        }

        // Step Physics
        self.physics.step(dt);

        let sample_3d = BallisticsEngine::sample_ball_position(&self.ball_state, current_t, self.physics.get_players());
        self.ball_pos_3d = sample_3d;

        // ============================================================
        // 3. 球弹道状态更新与拦截检查
        // ============================================================
        let mut new_ball_state = None;
        let mut steal_triggered_defender = None;

        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => {
                let cid = carrier_id.clone();
                if let Some(p) = self.physics.get_player(&cid) {
                    let bounce = 3.25 + 0.35 * (current_t * 2.2 * std::f32::consts::TAU).sin();
                    let offset = if p.vel_ft.length() > 0.5 {
                        p.vel_ft.normalize() * 0.8
                    } else {
                        Vec2::new(0.5, 0.0)
                    };
                    self.ball_pos_3d = (p.pos_ft + offset, bounce);
                }
            }
            BallTrajectoryKind::Pass { target_id, start_time, duration, from_pos, to_pos, .. } => {
                let tau = ((current_t - start_time) / duration).clamp(0.0, 1.0);

                // 事中约束：传球走廊拦截裁决
                let def_team = if is_home { "away" } else { "home" };
                for def in self.physics.get_players().values() {
                    if def.team != def_team {
                        continue;
                    }
                    let seg = *to_pos - *from_pos;
                    let seg_len_sq = seg.length_squared();
                    if seg_len_sq > 0.001 {
                        let t = ((def.pos_ft - *from_pos).dot(seg) / seg_len_sq).clamp(0.0, 1.0);
                        let _closest = *from_pos + seg * t;
                        let ball_curr_xy = self.ball_pos_3d.0;
                        let dist_to_ball = (def.pos_ft - ball_curr_xy).length();
                        if dist_to_ball < 1.8 && self.ball_pos_3d.1 < 6.5 {
                            let intercept_event = PhysicsEvent::PathIntersection {
                                ball_pos: (self.ball_pos_3d.0.x, self.ball_pos_3d.0.y, self.ball_pos_3d.1),
                                defender_id: def.id.clone(),
                                clearance_dist: dist_to_ball,
                                flight_time: current_t - start_time,
                            };
                            let outcome = ResolutionLayer::resolve_pass_intersection(&intercept_event, self.physics.get_players(), &mut self.rng);
                            if let ResolutionOutcome::PassIntercepted { defender_id } = outcome {
                                steal_triggered_defender = Some(defender_id);
                                break;
                            }
                        }
                    }
                }

                if steal_triggered_defender.is_none() && tau >= 1.0 {
                    if let Some(target_p) = self.physics.get_player(target_id) {
                        self.ball_pos_3d = (target_p.pos_ft, 4.0);
                    }
                    // 发球/一传落地即活球：死球阶段转回回合发起（修复 DeadBallReset 卡死）
                    if self.sub_phase == SubPhase::DeadBallReset {
                        self.sub_phase = SubPhase::Initiation;
                        self.sub_phase_timer = 0.0;
                        self.is_dead_ball = false;
                        self.last_decision_time = -10.0;
                    }
                    new_ball_state = Some(BallTrajectoryKind::Held { carrier_id: target_id.clone() });
                }
            }
            BallTrajectoryKind::Shot { shooter_id, hoop_pos, start_time, duration, is_made, is_three, from_pos, .. } => {
                let tau = ((current_t - start_time) / duration).clamp(0.0, 1.0);
                if tau >= 1.0 {
                    let made = *is_made;
                    let three = *is_three;
                    let sid = shooter_id.clone();
                    let h_pos = *hoop_pos;
                    let s_pos = *from_pos;
                    let shooter_name = self.physics.get_player(&sid).map(|p| p.jersey.clone()).unwrap_or(sid);

                    if made {
                        let pts = if three { 3 } else { 2 };
                        if is_home {
                            self.home_score += pts;
                        } else {
                            self.away_score += pts;
                        }
                        let baseline_z = 10.0;
                        self.ball_pos_3d = (h_pos, baseline_z);
                        self.sub_phase = SubPhase::DeadBallReset;
                        self.sub_phase_timer = 0.0;
                        self.start_inbound_transition(h_pos, (h_pos, baseline_z));
                    } else {
                        self.current_event = Some("SHOT_MISSED".to_string());
                        self.current_callout = Some(format!("砸框而出！{} 投篮不中，争抢篮板！", shooter_name));
                        self.sub_phase = SubPhase::FlightAndRebound;
                        self.sub_phase_timer = 0.0;

                        let landing_spot = BallisticsEngine::compute_rebound_landing(s_pos, h_pos, &mut self.rng);

                        new_ball_state = Some(BallTrajectoryKind::RimRebound {
                            hoop_pos: h_pos,
                            target_landing: landing_spot.landing_pos,
                            start_time: current_t,
                            duration: landing_spot.flight_duration,
                            peak_z: 11.5,
                        });
                    }
                }
            }
            BallTrajectoryKind::RimRebound { target_landing, start_time, duration, .. } => {
                let tau = ((current_t - start_time) / duration).clamp(0.0, 1.0);
                if tau >= 1.0 {
                    // 篮板归属：adjudication 概率裁决 + 距离加权
                    let reb_pos = *target_landing;
                    let reb_id = self.resolve_rebounder(reb_pos);
                    let reb_name = self.physics.get_player(&reb_id).map(|p| p.jersey.clone()).unwrap_or(reb_id.clone());
                    self.current_event = Some("REBOUND".to_string());
                    self.current_callout = Some(format!("{} 保护好防守篮板！一传发动反击！", reb_name));

                    self.start_rebound_outlet(reb_id, reb_pos);
                }
            }
            BallTrajectoryKind::LooseBall { .. } => {}
        }

        if let Some(def_id) = steal_triggered_defender {
            self.current_event = Some("STEAL".to_string());
            self.current_callout = Some(format!("传球路线被识破！{} 飞身抢断！", def_id));
            let ball_intercept_pos = self.ball_pos_3d.0;
            self.start_steal_transition(def_id, ball_intercept_pos);
        } else if let Some(nbs) = new_ball_state {
            self.ball_state = nbs;
        }

        // ============================================================
        // 4. 战术目标生成 & 移动导航（每 tick）
        // ============================================================
        let (home_targets, away_targets) = TacticalPlanner::plan_possession_targets(
            self.tactical_set,
            self.sub_phase,
            self.possession,
            self.ball_pos_3d.0,
            self.carrier_idx,
            self.sub_phase_timer,
            &mut self.rng,
        );

        for (i, t) in home_targets.into_iter().enumerate() {
            let pid = format!("H_{}", i + 1);
            self.physics.set_player_target(&pid, t.target_pos, t.speed, &t.action, &t.slot, &t.morale);
        }

        for (i, t) in away_targets.into_iter().enumerate() {
            let pid = format!("A_{}", i + 1);
            self.physics.set_player_target(&pid, t.target_pos, t.speed, &t.action, &t.slot, &t.morale);
        }

        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.physics.set_ball_holder(active_carrier);

        // ============================================================
        // 5. 调制状态推进（体力/士气反馈环，modulation 接线）
        // ============================================================
        for (i, p) in self.physics.get_players().iter().enumerate() {
            let ordered: Vec<&PlayerPhysicsState> = {
                let mut v: Vec<&PlayerPhysicsState> = self.physics.get_players().values().collect();
                v.sort_by(|a, b| a.id.cmp(&b.id));
                v
            };
            let idx = ordered.iter().position(|p| p.id == p.id).unwrap_or(i);
            let _ = idx;
            let speed = p.1.vel_ft.length();
            self.modulation[i.min(9)].update_stamina(speed, dt);
        }

        // 同步调制状态回物理体（体力/士气渲染 + 决策输入）
        Self::sync_modulation_to_physics(&mut self.physics, &self.modulation);

        self.build_tick()
    }

    /// 将 modulation 状态同步到物理体渲染字段。
    fn sync_modulation_to_physics(physics: &mut PhysicsWorld, modulation: &[PlayerModulationState]) {
        let mut ordered: Vec<String> = physics.get_players().keys().cloned().collect();
        ordered.sort();
        for (i, pid) in ordered.iter().enumerate() {
            if let Some(m) = modulation.get(i) {
                if let Some(p) = physics.get_player_mut(pid) {
                    p.stamina = m.stamina * 100.0;
                    p.morale = format!("{:?}", m.morale);
                }
            }
        }
    }

    /// 士气偏置：HotHand 提升出手欲望，Exhausted/Frustrated 抑制。
    fn morale_bias_for(&self, player_id: &str) -> f32 {
        let idx = player_index(player_id);
        match self.modulation.get(idx) {
            Some(m) => match m.morale {
                nba_decision::modulation::MoraleState::HotHand => 0.10,
                nba_decision::modulation::MoraleState::Clutch => 0.08,
                nba_decision::modulation::MoraleState::Normal => 0.0,
                nba_decision::modulation::MoraleState::Frustrated => -0.05,
                nba_decision::modulation::MoraleState::Exhausted => -0.12,
            },
            None => 0.0,
        }
    }

    // ==========================================================================
    // 动作执行器
    // ==========================================================================

    /// 执行投篮（adjudication 接线：命中率基于 resolve_config）。
    fn execute_shot(&mut self, shooter_id: &str, _from_pos: Vec2, _is_three_hint: bool, current_t: f32) {
        let is_home = self.possession == Possession::Home;
        let hoop = Court::hoop_pos(is_home);
        let shooter_pos = self.physics.get_player(shooter_id).map(|p| p.pos_ft).unwrap_or(Vec2::new(30.0, 25.0));
        let dist_to_hoop = (shooter_pos - hoop).length();
        let is_three = dist_to_hoop >= 23.75;

        let openness = SpatialGeometry::get_openness(shooter_id, self.physics.get_players());
        let base_fg = if dist_to_hoop < 8.0 {
            self.resolve_config.base_rates.shot_make_2pt
        } else if is_three {
            self.resolve_config.base_rates.shot_make_3pt
        } else {
            self.resolve_config.base_rates.shot_make_2pt
        };
        let contest_penalty = openness.contest_intensity * 0.22;
        let final_fg_pct = (base_fg - contest_penalty).clamp(0.10, 0.85);
        let is_made = self.rng.gen_bool(final_fg_pct as f64);

        let flight_time = (dist_to_hoop / 26.0).clamp(0.95, 1.45);
        let peak_z = 14.0 + (dist_to_hoop * 0.25).clamp(0.0, 6.0);

        self.active_windows.insert(shooter_id.to_string(), ActionTimeWindow::new_jump_shot(shooter_id, current_t));

        let release_pos = self.ball_pos_3d.0;
        self.ball_state = BallTrajectoryKind::Shot {
            shooter_id: shooter_id.to_string(),
            from_pos: release_pos,
            hoop_pos: hoop,
            start_time: current_t,
            duration: flight_time,
            is_made,
            is_three,
            peak_z,
        };
        self.sub_phase = SubPhase::ShotAttempt;
        self.sub_phase_timer = 0.0;
        self.current_event = Some("SHOT_RELEASE".to_string());
        let shooter_name = self.physics.get_player(shooter_id).map(|p| p.jersey.clone()).unwrap_or_else(|| shooter_id.to_string());
        self.current_callout = Some(format!("{} 迎着防守果断出手{}！", shooter_name, if is_three { "三分球" } else { "跳投" }));
        self.current_intensity = Some("Climax".to_string());
    }

    /// 执行传球。
    fn execute_pass(&mut self, passer_id: &str, receiver_id: &str, _from_pos: Vec2, _to_pos: Vec2, current_t: f32) {
        let from_pos = self.ball_pos_3d.0;
        let target_lead_pos = self
            .physics
            .get_player(receiver_id)
            .map(|p| BallisticsEngine::extrapolate_receiver_pos(p, 0.65))
            .unwrap_or(_to_pos);

        self.active_windows.insert(passer_id.to_string(), ActionTimeWindow::new_pass(passer_id, current_t));

        let pass_dist = (target_lead_pos - from_pos).length();
        let duration = (pass_dist / 32.0).clamp(0.45, 1.4);
        self.ball_state = BallTrajectoryKind::Pass {
            from_pos,
            to_pos: target_lead_pos,
            target_id: receiver_id.to_string(),
            start_time: current_t,
            duration,
            peak_z: 4.0,
        };
        // 更新 carrier_idx 以驱动战术规划
        if let Some(idx) = receiver_id.rsplit_once('_').and_then(|(_, n)| n.parse::<usize>().ok()) {
            self.carrier_idx = idx.saturating_sub(1).min(4);
        }
        self.current_event = Some("PASS".to_string());
        self.current_callout = Some("突分策应！外线转移球创造空位机会".to_string());
    }

    /// 篮板归属裁决：距离落点最近的 2 名候选 + 防守篮板概率偏置。
    fn resolve_rebounder(&mut self, landing: Vec2) -> String {
        let def_team = if self.possession == Possession::Home { "away" } else { "home" };
        let off_team = if self.possession == Possession::Home { "home" } else { "away" };

        // 距离加权候选
        let mut best_def: Option<(String, f32)> = None;
        let mut best_off: Option<(String, f32)> = None;
        for p in self.physics.get_players().values() {
            let d = (p.pos_ft - landing).length();
            if p.team == def_team && best_def.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
                best_def = Some((p.id.clone(), d));
            }
            if p.team == off_team && best_off.as_ref().map(|(_, bd)| d < *bd).unwrap_or(true) {
                best_off = Some((p.id.clone(), d));
            }
        }

        // adjudication: offensive_rebound_rate
        let off_reb_rate = self.resolve_config.base_rates.offensive_rebound_rate;
        let (def_id, def_d) = best_def.unwrap_or(("H_4".into(), 10.0));
        let (off_id, off_d) = best_off.unwrap_or(("A_4".into(), 12.0));

        // 距离归一补偿：进攻方冲抢通常离落点更远
        let off_chance = off_reb_rate * (def_d / (off_d + def_d)).clamp(0.3, 1.4);
        if self.rng.gen_bool(off_chance.clamp(0.05, 0.5) as f64) {
            off_id
        } else {
            def_id
        }
    }

    // ==========================================================================
    // 阶段转换器
    // ==========================================================================

    fn start_violation_turnover(&mut self, _kind: ViolationKind) {
        self.completed_possessions += 1;
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = 24.0;
        self.shot_clock_violated = false;
        self.tactical_set = TacticalSet::HighPickAndRoll;
        self.carrier_idx = 0;
        self.is_dead_ball = false;
        self.last_decision_time = -10.0;

        // 违例后发球：从违例点发球给新球权方 PG（球飞行而非瞬移）
        let inbound_pos = self.ball_pos_3d.0;
        let receiver_id = self.new_possession_pg();
        let target_pos = self
            .physics
            .get_player(&receiver_id)
            .map(|p| p.pos_ft)
            .unwrap_or(inbound_pos + Vec2::new(10.0, 0.0));
        let pass_dist = (target_pos - inbound_pos).length();
        self.ball_state = BallTrajectoryKind::Pass {
            from_pos: inbound_pos,
            to_pos: target_pos,
            target_id: receiver_id,
            start_time: self.current_time,
            duration: (pass_dist / 32.0).clamp(0.5, 1.6),
            peak_z: 5.0,
        };
        self.sub_phase = SubPhase::Initiation;
        self.sub_phase_timer = 0.0;
    }

    fn start_steal_transition(&mut self, stealer_id: String, intercept_pos: Vec2) {
        self.completed_possessions += 1;
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = 24.0;
        self.tactical_set = TacticalSet::FastBreakTransition;
        self.carrier_idx = 0;
        self.sub_phase = SubPhase::Initiation;
        self.sub_phase_timer = 0.0;
        self.last_decision_time = -10.0;

        self.ball_pos_3d = (intercept_pos, 4.0);
        self.ball_state = BallTrajectoryKind::Held { carrier_id: stealer_id };
    }

    fn start_rebound_outlet(&mut self, _rebounder_id: String, reb_pos: Vec2) {
        self.completed_possessions += 1;
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = 24.0;

        let sets = [
            TacticalSet::FastBreakTransition,
            TacticalSet::HighPickAndRoll,
            TacticalSet::FiveOutMotion,
            TacticalSet::IsolationDrive,
            TacticalSet::DriveAndKick,
        ];
        self.tactical_set = sets[self.rng.gen_range(0..sets.len())];
        self.carrier_idx = 0;

        let target_pg_id = self.new_possession_pg();

        let target_pos = self.physics.get_player(&target_pg_id).map(|p| p.pos_ft).unwrap_or(reb_pos + Vec2::new(10.0, 0.0));
        let pass_dist = (target_pos - reb_pos).length();
        let duration = (pass_dist / 32.0).clamp(0.65, 1.8);

        let current_t = self.current_time;
        self.ball_state = BallTrajectoryKind::Pass {
            from_pos: reb_pos,
            to_pos: target_pos,
            target_id: target_pg_id,
            start_time: current_t,
            duration,
            peak_z: 6.0,
        };

        self.sub_phase = SubPhase::Initiation;
        self.sub_phase_timer = 0.0;
        self.last_decision_time = -10.0;

        self.current_event = Some("OUTLET_PASS".to_string());
        self.current_callout = Some(format!(
            "{} 摘板后长传推进，发动战术：{}",
            if self.possession == Possession::Home { "凯尔特人" } else { "湖人队" },
            self.tactical_set.name_zh()
        ));
    }

    fn start_inbound_transition(&mut self, baseline_pos: Vec2, current_ball_3d: (Vec2, f32)) {
        self.completed_possessions += 1;
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = 24.0;

        let sets = [
            TacticalSet::HighPickAndRoll,
            TacticalSet::FiveOutMotion,
            TacticalSet::IsolationDrive,
            TacticalSet::DriveAndKick,
            TacticalSet::PostUp,
        ];
        self.tactical_set = sets[self.rng.gen_range(0..sets.len())];
        self.carrier_idx = 0;

        let receiver_id = self.new_possession_pg();

        let target_pos = self.physics.get_player(&receiver_id).map(|p| p.pos_ft).unwrap_or(baseline_pos + Vec2::new(15.0, 0.0));
        let current_ball_pos = current_ball_3d.0;
        let pass_dist = (target_pos - current_ball_pos).length();
        let duration = (pass_dist / 30.0).clamp(0.75, 2.0);

        let current_t = self.current_time;
        let peak_z = current_ball_3d.1;
        self.ball_state = BallTrajectoryKind::Pass {
            from_pos: current_ball_pos,
            to_pos: target_pos,
            target_id: receiver_id,
            start_time: current_t,
            duration,
            peak_z,
        };

        self.current_event = Some("INBOUND_PASS".to_string());
        self.current_callout = Some(format!(
            "{} 底线发球推进，呼叫战术：{}",
            if self.possession == Possession::Home { "凯尔特人" } else { "湖人队" },
            self.tactical_set.name_zh()
        ));
    }

    /// 新球权方 PG 的球员 id。
    fn new_possession_pg(&self) -> String {
        if self.possession == Possession::Home {
            "H_1".to_string()
        } else {
            "A_1".to_string()
        }
    }

    // ==========================================================================
    // Tick 协议输出
    // ==========================================================================

    fn build_tick(&mut self) -> StreamTick {
        let _is_home = self.possession == Possession::Home;
        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => Some(carrier_id.clone()),
            _ => None,
        };

        let mut render_players = Vec::new();
        for (i, jersey) in HOME_JERSEYS.iter().enumerate() {
            let pid = format!("H_{}", i + 1);
            if let Some(p) = self.physics.get_player(&pid) {
                let norm = Court::ft_to_norm(p.pos_ft);
                render_players.push(RenderPlayer {
                    team: "home".to_string(),
                    jersey: jersey.to_string(),
                    x: (norm.x * 1000.0).round() / 1000.0,
                    y: (norm.y * 1000.0).round() / 1000.0,
                    has_ball: active_carrier.as_deref() == Some(pid.as_str()),
                    action: p.action.clone(),
                    slot: p.slot.clone(),
                    morale: p.morale.clone(),
                    zone: "perimeter".to_string(),
                    stm: (p.stamina * 10.0).round() / 10.0,
                    stm_max: p.max_stamina,
                });
            }
        }

        for (i, jersey) in AWAY_JERSEYS.iter().enumerate() {
            let pid = format!("A_{}", i + 1);
            if let Some(p) = self.physics.get_player(&pid) {
                let norm = Court::ft_to_norm(p.pos_ft);
                render_players.push(RenderPlayer {
                    team: "away".to_string(),
                    jersey: jersey.to_string(),
                    x: (norm.x * 1000.0).round() / 1000.0,
                    y: (norm.y * 1000.0).round() / 1000.0,
                    has_ball: active_carrier.as_deref() == Some(pid.as_str()),
                    action: p.action.clone(),
                    slot: p.slot.clone(),
                    morale: p.morale.clone(),
                    zone: "perimeter".to_string(),
                    stm: (p.stamina * 10.0).round() / 10.0,
                    stm_max: p.max_stamina,
                });
            }
        }

        let ball_norm = Court::ft_to_norm(self.ball_pos_3d.0);
        render_players.sort_by(|a, b| a.jersey.cmp(&b.jersey));

        let ball_status = match &self.ball_state {
            BallTrajectoryKind::Held { .. } => "HELD",
            BallTrajectoryKind::Pass { .. } => "PASS",
            BallTrajectoryKind::Shot { .. } => "SHOT",
            BallTrajectoryKind::RimRebound { .. } => "REBOUND",
            BallTrajectoryKind::LooseBall { .. } => "LOOSE_BALL",
        };

        let frame = RenderFrame {
            t: (self.current_time * 100.0).round() / 100.0,
            t_game: (self.game_clock * 10.0).round() / 10.0,
            shot_clock: (self.shot_clock * 10.0).round() / 10.0,
            period: self.period,
            phase: format!("{:?}", self.sub_phase),
            possession_id: self.possession_id,
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
            callout: self.current_callout.clone(),
            intensity: self.current_intensity.clone(),
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

fn opposite(p: Possession) -> Possession {
    match p {
        Possession::Home => Possession::Away,
        Possession::Away => Possession::Home,
    }
}

fn team_name_zh(is_home: bool) -> &'static str {
    if is_home { "凯尔特人" } else { "湖人队" }
}

/// H_1..H_5 → 0..4, A_1..A_5 → 5..9
fn player_index(id: &str) -> usize {
    match id.split_once('_') {
        Some(("H", n)) | Some(("A", n)) => {
            let team_off = if id.starts_with('A') { 5 } else { 0 };
            n.parse::<usize>().map(|n| team_off + n - 1).unwrap_or(0)
        }
        _ => 0,
    }
}

/// 转换决策追踪到协议调试层。
fn convert_trace(trace: &nba_decision::pipeline::DecisionTrace) -> DecisionDebug {
    DecisionDebug {
        player: trace.player_id.clone(),
        chosen: trace.chosen_kind.to_string(),
        utilities: trace
            .utilities
            .iter()
            .map(|(k, u)| DebugUtility { kind: k.to_string(), utility: *u })
            .collect(),
        probabilities: trace
            .probabilities
            .iter()
            .map(|(k, p)| DebugProb { kind: k.to_string(), prob: *p })
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
        blocked: trace.blocked.iter().map(|s| s.to_string()).collect(),
    }
}
