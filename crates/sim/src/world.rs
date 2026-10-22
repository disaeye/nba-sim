//! 世界状态与主循环。
//!
//! 每 tick 的顺序固定：时钟 → 运动 → 动作到期结算 → 阶段推进 → 决策。
//! 结算发生在动作到期时，决策只读派生状态；球的位置只服务表现，不裁定
//! 任何结果。

use glam::Vec2;
use nba_domain::{GameRules, TeamData};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;

use crate::action::{ActionIntent, Fact, TeamSide};
use crate::boxscore::{BoxScore, PlayerLine};
use crate::params::Params;
use crate::resolve;
use crate::resolve::shot::PendingShot;
pub use crate::state::Phase;
use crate::state::{
    BallState, Clock, DecisionSample, DecisionState, OnCourt, PassSample, PlayerState,
    PossessionKind, PossessionState, ScreenRead, ShotSample, Slot,
};

pub const TICK_SECONDS: f64 = 0.05;
pub const DECISION_INTERVAL: f64 = 0.55;
/// 推进到前场之后的落位时间：持球人到达顶弧还需要给队友落位的时间。
pub const POSSESSION_SETUP_SECONDS: f64 = 3.25;
const MAX_TICKS: u64 = 400_000;

#[derive(Debug, Clone)]
pub struct MatchSetup {
    pub home: TeamData,
    pub away: TeamData,
    pub home_lineup: [usize; 5],
    pub away_lineup: [usize; 5],
    pub rules: GameRules,
}

impl MatchSetup {
    pub fn builtin(rules: GameRules) -> Self {
        let home = TeamData::builtin_home_with_geometry(rules.court);
        let away = TeamData::builtin_away_with_geometry(rules.court);
        Self {
            home_lineup: [0, 1, 2, 3, 4],
            away_lineup: [0, 1, 2, 3, 4],
            home,
            away,
            rules,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TimedFact {
    pub time: f64,
    pub period: u32,
    pub fact: Fact,
    /// 直接产生这条事实的那次决策（指向 `World::decision_traces`）。
    pub trace: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct TeamFouls {
    /// 单节球队犯规（bonus 计数）：进攻犯规与技术犯规不计入。
    pub period: u8,
    /// 全场球队犯规累计（含进攻犯规）。
    pub total: u8,
}

/// 轮换状态：换人意图与停表窗口内的换人配额。
#[derive(Debug, Clone, Default)]
pub struct RotationState {
    /// 每队的待执行换人意图。活球期间只登记，不停表就不动阵容。
    pub pending: [bool; 2],
    /// 当前停表窗口内已执行的换人数（按队）。
    pub applied_in_window: [u32; 2],
}

#[derive(Debug, Clone)]
pub struct PossessionRecord {
    pub team: TeamSide,
    pub id: u64,
    pub kind: PossessionKind,
    pub start_clock: f64,
    pub end_clock: f64,
    pub duration: f64,
    /// 回合开始时的比赛钟读数（本节剩余秒数）。
    ///
    /// `duration` 是墙钟，含死球停表；回合的节奏必须用比赛钟量。
    pub start_game_clock: f64,
    /// 回合结束时的比赛钟读数。
    pub end_game_clock: f64,
    pub passes: u32,
    pub screens: u32,
    pub drives: u32,
    pub points: u32,
    pub ended_by: &'static str,
    /// 回合开始到首个动作起手的秒数：推进速度的直接度量。
    pub first_action_seconds: Option<f64>,
}

pub struct World {
    pub params: Params,
    pub rules: GameRules,
    pub clock: Clock,
    pub phase: Phase,
    pub ball: BallState,
    pub players: Vec<PlayerState>,
    pub possession: PossessionState,
    pub score: [u32; 2],
    pub box_score: BoxScore,
    /// 球员句柄到所属队伍箱体行下标的映射。构建期算一次，换人不重排行序。
    pub line_slot: Vec<u8>,
    pub active: Option<ActionIntent>,
    pub next_decision_at: f64,
    pub events: Vec<TimedFact>,
    pub rng: ChaCha8Rng,
    pub seed: u64,
    pub fouls: [TeamFouls; 2],
    pub rotation: RotationState,
    pub timeouts: [crate::timeouts::TimeoutBudget; 2],
    /// 最近一次得分方（队下标，0 主 1 客）与这一方的连续得分。
    pub last_scoring: Option<usize>,
    pub scoring_run: u32,
    pub possession_count: u64,
    pub period_started_with: TeamSide,
    pub finished: bool,
    pub home_team_name: String,
    pub away_team_name: String,
    pub last_pass_kind: Option<crate::action::PassKind>,
    pub possession_log: Vec<PossessionRecord>,
    pub pending_shot: Option<PendingShot>,
    pub possession_points: u32,
    pub wall: f64,
    pub violations: Vec<String>,
    pub last_separation: Option<(usize, f32, f64)>,
    /// 选板冷却表：剧本下标 → 可再次激活的时刻。
    pub play_cooldowns: std::collections::BTreeMap<usize, f64>,
    pub alternate_spacing: bool,
    pub shot_clock_at_release: Vec<f64>,
    pub contest_samples: Vec<f32>,
    /// 逐次出手、传球与决策的过程观测。
    pub shot_samples: Vec<ShotSample>,
    pub pass_samples: Vec<PassSample>,
    pub decision_samples: Vec<DecisionSample>,
    /// 逐次决策的完整因果快照：候选集、效用分解与玻尔兹曼概率。
    pub decision_traces: Vec<crate::state::DecisionTraceSample>,
    /// 当前生效的决策快照下标：本次决策产生的事实都带着它。
    pub active_trace: Option<u32>,
    pub shot_zone_counts: [[u32; 4]; 2],
    pub shot_zone_makes: [[u32; 4]; 2],
    pub creation_counts: [u32; 7],
    pub next_shot_id: u64,
    pub next_pass_id: u64,
    /// 走到篮板结算的投失次数。犯规导致的投失不进入篮板，
    /// 前场篮板率的分子分母必须用这个数对账。
    pub rebound_attempts: u64,
    /// 连续得分带来的势头（0 = 无，越大越热）。按队存，随时间衰减。
    pub momentum: [f32; 2],
    /// 读球历史：防守的协防轮转看的是 `delay` 秒之前的球位。
    pub(crate) ball_history: Vec<(f64, Vec2)>,
}

impl World {
    pub fn new(setup: MatchSetup, params: Params, seed: u64) -> Self {
        let rules = setup.rules.clone();
        // 读球历史按秒数上限预分配：稳态每帧只做写入与裁剪，不再扩容。
        let history_capacity = ((params.policy.g("ball_history_seconds") as f64 / TICK_SECONDS)
            .ceil() as usize)
            + 8;
        let mut players = Vec::new();
        for (index, data) in setup.home.players.iter().enumerate() {
            let on_court = setup.home_lineup.contains(&index);
            let slot = Slot::Handler;
            let mut player = PlayerState::new(data.clone(), TeamSide::Home, slot, Vec2::ZERO);
            player.on_court = on_court;
            players.push(player);
        }
        for (index, data) in setup.away.players.iter().enumerate() {
            let on_court = setup.away_lineup.contains(&index);
            let slot = Slot::Handler;
            let mut player = PlayerState::new(data.clone(), TeamSide::Away, slot, Vec2::ZERO);
            player.on_court = on_court;
            players.push(player);
        }
        assign_slots(&mut players, TeamSide::Home);
        assign_slots(&mut players, TeamSide::Away);
        // 箱体行序由队伍过滤的次序固定，换人不重排，因此下标可一次算定。
        let mut line_slot = vec![0u8; players.len()];
        let mut home_slot = 0u8;
        let mut away_slot = 0u8;
        for (handle, player) in players.iter().enumerate() {
            match player.team {
                TeamSide::Home => {
                    line_slot[handle] = home_slot;
                    home_slot += 1;
                }
                TeamSide::Away => {
                    line_slot[handle] = away_slot;
                    away_slot += 1;
                }
            }
        }
        let home_lines = players
            .iter()
            .filter(|p| p.team == TeamSide::Home)
            .map(|p| {
                PlayerLine::new(
                    &p.id,
                    &p.name,
                    &p.jersey,
                    TeamSide::Home,
                    p.data.tendencies.three_point_preference,
                )
            })
            .collect();
        let away_lines = players
            .iter()
            .filter(|p| p.team == TeamSide::Away)
            .map(|p| {
                PlayerLine::new(
                    &p.id,
                    &p.name,
                    &p.jersey,
                    TeamSide::Away,
                    p.data.tendencies.three_point_preference,
                )
            })
            .collect();
        let period_seconds = rules.league.period_duration_seconds as f64;
        let shot_clock = rules.league.shot_clock_seconds as f64;
        let rules_capture = rules.clone();
        let mut rng = ChaCha8Rng::seed_from_u64(seed);
        let first = if rng.gen::<bool>() {
            TeamSide::Home
        } else {
            TeamSide::Away
        };
        let mut world = Self {
            params,
            rules,
            clock: Clock::new(period_seconds, shot_clock),
            phase: Phase::DeadBall {
                team: first,
                ready_at: 0.6,
            },
            ball: BallState::Loose {
                pos: Vec2::new(47.0, 25.0),
                vel: Vec2::ZERO,
            },
            players,
            possession: PossessionState::new(
                first,
                0,
                PossessionKind::Dead,
                0.0,
                shot_clock,
                period_seconds,
            ),
            score: [0, 0],
            box_score: BoxScore {
                home: home_lines,
                away: away_lines,
            },
            line_slot,
            active: None,
            next_decision_at: 0.0,
            events: Vec::new(),
            rng,
            seed,
            fouls: [TeamFouls::default(), TeamFouls::default()],
            rotation: RotationState::default(),
            timeouts: [
                crate::timeouts::TimeoutBudget::new(&rules_capture),
                crate::timeouts::TimeoutBudget::new(&rules_capture),
            ],
            last_scoring: None,
            scoring_run: 0,
            possession_count: 0,
            period_started_with: first,
            finished: false,
            home_team_name: setup.home.name.clone(),
            away_team_name: setup.away.name.clone(),
            last_pass_kind: None,
            possession_log: Vec::new(),
            pending_shot: None,
            possession_points: 0,
            wall: 0.0,
            violations: Vec::new(),
            last_separation: None,
            play_cooldowns: std::collections::BTreeMap::new(),
            alternate_spacing: false,
            shot_clock_at_release: Vec::new(),
            contest_samples: Vec::new(),
            shot_samples: Vec::new(),
            pass_samples: Vec::new(),
            decision_samples: Vec::new(),
            decision_traces: Vec::new(),
            active_trace: None,
            shot_zone_counts: [[0; 4]; 2],
            shot_zone_makes: [[0; 4]; 2],
            creation_counts: [0; 7],
            next_shot_id: 0,
            next_pass_id: 0,
            rebound_attempts: 0,
            momentum: [f32::default(); 2],
            ball_history: Vec::with_capacity(history_capacity),
        };
        world.reset_positions(first);
        world
    }

    pub fn run(&mut self) -> GameReport {
        while !self.finished && self.clock.tick < MAX_TICKS {
            self.step(TICK_SECONDS);
        }
        self.report()
    }

    /// 逐步推演并逐帧交给调用方，供流式导出与实时观察使用。
    pub fn run_with<F: FnMut(&World)>(&mut self, mut sink: F) -> GameReport {
        while !self.finished && self.clock.tick < MAX_TICKS {
            self.step(TICK_SECONDS);
            sink(self);
        }
        self.report()
    }

    pub fn new_shot_id(&mut self) -> u64 {
        self.next_shot_id += 1;
        self.next_shot_id
    }

    pub fn new_pass_id(&mut self) -> u64 {
        self.next_pass_id += 1;
        self.next_pass_id
    }

    /// 流尾部尚未结算的业务实例：飞行中的出手、飞行中的传球、未执行完的罚球。
    pub fn open_business_instances(&self) -> u64 {
        let shot = u64::from(self.pending_shot.is_some());
        let pass = u64::from(matches!(self.ball, BallState::Pass { .. }));
        let free_throws = match self.phase {
            Phase::FreeThrows { remaining, .. } => u64::from(remaining),
            _ => 0,
        };
        shot + pass + free_throws
    }

    pub fn report(&mut self) -> GameReport {
        GameReport {
            seed: self.seed,
            home_team: self.home_team_name.clone(),
            away_team: self.away_team_name.clone(),
            home_score: self.score[0],
            away_score: self.score[1],
            periods: self.clock.period,
            ticks: self.clock.tick,
            open_business_instances: self.open_business_instances(),
            rebound_attempts: self.rebound_attempts,
            possessions: self.possession_log.clone(),
            box_score: self.box_score.clone(),
            events: std::mem::take(&mut self.events),
            shot_clock_at_release: std::mem::take(&mut self.shot_clock_at_release),
            contest_samples: std::mem::take(&mut self.contest_samples),
            shot_samples: std::mem::take(&mut self.shot_samples),
            pass_samples: std::mem::take(&mut self.pass_samples),
            decision_samples: std::mem::take(&mut self.decision_samples),
            decision_traces: std::mem::take(&mut self.decision_traces),
            shot_zone_counts: self.shot_zone_counts,
            shot_zone_makes: self.shot_zone_makes,
            creation_counts: self.creation_counts,
            violations: std::mem::take(&mut self.violations),
            games: 1,
        }
    }

    pub fn team_index(team: TeamSide) -> usize {
        match team {
            TeamSide::Home => 0,
            TeamSide::Away => 1,
        }
    }

    pub fn hoop_for(&self, team: TeamSide) -> Vec2 {
        self.rules.court.hoop_pos(team == TeamSide::Home)
    }

    pub fn attacking_right(&self, team: TeamSide) -> bool {
        team == TeamSide::Home
    }

    pub fn holder(&self) -> Option<usize> {
        match self.ball {
            BallState::Held(index) => Some(index),
            _ => None,
        }
    }

    pub fn now(&self) -> f64 {
        self.wall
    }

    pub fn delayed_ball_position(&self, delay: f64) -> Vec2 {
        crate::motion::delayed_ball_position(self, delay)
    }

    pub fn emit(&mut self, fact: Fact) {
        self.events.push(TimedFact {
            time: self.now(),
            period: self.clock.period,
            fact,
            trace: self.active_trace,
        });
    }

    pub fn line(&self, player: usize) -> &PlayerLine {
        let team = self.players[player].team;
        let slot = self.line_slot[player] as usize;
        let lines = match team {
            TeamSide::Home => &self.box_score.home,
            TeamSide::Away => &self.box_score.away,
        };
        &lines[slot]
    }

    pub fn line_mut(&mut self, player: usize) -> &mut PlayerLine {
        let team = self.players[player].team;
        let slot = self.line_slot[player] as usize;
        let lines = match team {
            TeamSide::Home => &mut self.box_score.home,
            TeamSide::Away => &mut self.box_score.away,
        };
        &mut lines[slot]
    }

    pub fn add_points(&mut self, team: TeamSide, points: u32) {
        let index = Self::team_index(team);
        self.score[index] += points;
        self.possession_points += points;
        if self.last_scoring == Some(index) {
            self.scoring_run += points;
        } else {
            self.last_scoring = Some(index);
            self.scoring_run = points;
        }
        // 势头：得分方累积，被得分方清零。它按秒衰减，不是一次性标记。
        let gain = self.params.policy.w("momentum_gain_per_point");
        let cap = self.params.policy.w("momentum_cap");
        self.momentum[index] = (self.momentum[index] + gain * points as f32).min(cap);
        self.momentum[1 - index] = f32::default();
        for player in 0..self.players.len() {
            if !self.players[player].on_court {
                continue;
            }
            let delta = if self.players[player].team == team {
                points as i32
            } else {
                -(points as i32)
            };
            self.line_mut(player).plus_minus += delta;
        }
    }

    /// 谁该叫暂停：对手刚打出一波超过阀值的连续得分时，返回被得分方的队下标。
    pub fn run_against(&self) -> Option<usize> {
        let scorer = self.last_scoring?;
        if self.scoring_run < self.rules.rotation.timeout_run_threshold {
            return None;
        }
        Some(1 - scorer)
    }

    pub fn apply_dead_ball_window(&mut self) {
        crate::rotation::apply_dead_ball_window(self);
    }

    pub fn on_court(&self, team: TeamSide) -> OnCourt {
        let mut set = OnCourt::new();
        for (handle, player) in self.players.iter().enumerate() {
            if player.on_court && player.team == team {
                set.push(handle);
            }
        }
        set
    }

    pub fn primary_handler(&self, team: TeamSide) -> usize {
        self.on_court(team)
            .into_iter()
            .max_by(|a, b| {
                let score = |handle: usize| {
                    self.players[handle].data.attributes.ball_handling * 2.0
                        + self.players[handle].data.attributes.passing
                        + self.players[handle].data.tendencies.pass_frequency
                };
                score(*a).partial_cmp(&score(*b)).expect("finite scores")
            })
            .expect("every team must have an on-court handler")
    }

    fn entry_point(&self, team: TeamSide) -> Vec2 {
        crate::formation::entry_point(self, team)
    }

    fn top_of_key(&self, team: TeamSide) -> Vec2 {
        crate::formation::top_of_key(self, team)
    }

    pub fn reset_positions(&mut self, offense: TeamSide) {
        crate::formation::reset_positions(self, offense);
    }

    pub fn start_possession(&mut self, team: TeamSide, kind: PossessionKind) {
        crate::tactics::close_play_window(self);
        self.active_trace = None;
        // 新回合的读球历史从头开始：上一个回合的球位不是这次协防的依据。
        crate::motion::reset_ball_history(self);
        self.possession_count += 1;
        self.alternate_spacing = self.possession_count.is_multiple_of(2);
        let shot_clock = self.rules.league.shot_clock_seconds as f64;
        self.clock.shot_clock = shot_clock;
        self.possession = PossessionState::new(
            team,
            self.possession_count,
            kind,
            self.now(),
            shot_clock,
            self.clock.game_clock,
        );
        self.possession_points = 0;
        self.pending_shot = None;
        self.reset_positions(team);
        // 死球重启是唯一合法的换人窗口：防守篮板、抢断、快速反击这些
        // 活球回合开始的进攻不允许在场上换人，现实里教练也换不了。
        if kind == PossessionKind::Dead {
            self.apply_dead_ball_window();
        }
        let handler = self.primary_handler(team);
        let entry = self.entry_point(team);
        self.players[handler].pos = entry;
        self.players[handler].target = self.top_of_key(team);
        self.ball = BallState::Held(handler);
        self.phase = Phase::Live;
        self.active = None;
        // 推进到前场的时长由移动速度决定：速度快则更快进入首个决策，
        // 在固定比赛时钟内产生更多回合。
        let advance_distance = (self.top_of_key(team) - entry).length();
        let speed = self.players[handler]
            .effective_speed(&self.params.policy)
            .max(1.0);
        // 推进时长 = 匀速段 + 起步加速段：加速度决定多快到达目标速度。
        let accel = self.players[handler]
            .effective_acceleration(&self.params.policy)
            .max(1.0);
        // 起步段随推进距离衰减：接球时多半已在移动，长推进接近匀速。
        let ramp_scale = self
            .params
            .policy
            .g("advance_ramp_distance_ft")
            .max(f32::EPSILON);
        let startup = (speed / accel) * (-advance_distance / ramp_scale).exp();
        let advance_seconds = (advance_distance / speed + startup).max(0.8) as f64;
        self.next_decision_at = self.now() + advance_seconds + POSSESSION_SETUP_SECONDS;
        self.emit(Fact::PossessionStarted {
            team,
            live: kind == PossessionKind::Live,
        });
    }

    pub fn start_second_chance(&mut self, team: TeamSide, rebounder: usize, rim_contact: bool) {
        crate::tactics::close_play_window(self);
        self.active_trace = None;
        self.possession_count += 1;
        // 前场篮板只有球触筐才重置进攻时钟，且只补不足、不削减剩余时间。
        let shot_clock = self
            .rules
            .league
            .shot_clock_after_offensive_rebound(self.clock.shot_clock, rim_contact);
        self.clock.shot_clock = shot_clock;
        self.possession = PossessionState::new(
            team,
            self.possession_count,
            PossessionKind::Live,
            self.now(),
            shot_clock,
            self.clock.game_clock,
        );
        self.possession.last_offensive_rebound_at = Some(self.now());
        self.possession.had_offensive_rebound = true;
        self.possession_points = 0;
        self.pending_shot = None;
        self.reset_positions(team);
        self.players[rebounder].target = self.players[rebounder].pos;
        self.ball = BallState::Held(rebounder);
        self.phase = Phase::Live;
        self.active = None;
        self.next_decision_at = self.now() + 0.3;
        self.emit(Fact::PossessionStarted { team, live: true });
    }

    pub fn end_possession(&mut self, cause: &'static str, points: u32) {
        if self.possession.id == 0 {
            return;
        }
        crate::tactics::close_play_window(self);
        let record = PossessionRecord {
            team: self.possession.team,
            id: self.possession.id,
            kind: self.possession.kind,
            start_clock: self.possession.shot_clock_start,
            end_clock: self.clock.shot_clock,
            duration: self.now() - self.possession.started_at,
            start_game_clock: self.possession.start_game_clock,
            end_game_clock: self.clock.game_clock,
            passes: self.possession.passes,
            screens: self.possession.screens,
            drives: self.possession.drives,
            points,
            ended_by: cause,
            first_action_seconds: self.possession.first_action_seconds,
        };
        self.possession_log.push(record);
        self.possession.id = 0;
    }

    pub fn decision_state(&self) -> DecisionState {
        crate::views::decision_state(self)
    }

    pub fn step(&mut self, dt: f64) {
        if self.finished {
            return;
        }
        self.clock.tick += 1;
        self.wall += dt;
        let live = matches!(
            self.phase,
            Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
        );
        let clock_running = matches!(
            self.phase,
            Phase::Live
                | Phase::PassFlight { .. }
                | Phase::ShotFlight { .. }
                | Phase::DeadBall { .. }
        );
        if clock_running {
            self.clock.game_clock = (self.clock.game_clock - dt).max(0.0);
        }
        if live {
            self.clock.elapsed += dt;
            if matches!(self.phase, Phase::Live) {
                self.clock.shot_clock = (self.clock.shot_clock - dt).max(0.0);
            }
            for handle in 0..self.players.len() {
                if self.players[handle].on_court {
                    self.line_mut(handle).seconds += dt;
                }
            }
            self.update_stamina(dt);
        }
        crate::rotation::note_intents(self);
        self.advance_movement(dt);
        self.advance_active();
        self.advance_phase();
        self.maybe_decide();
        self.check_period_end();
    }

    fn update_stamina(&mut self, dt: f64) {
        crate::rotation::update_stamina(self, dt);
    }

    fn advance_movement(&mut self, dt: f64) {
        crate::motion::advance_movement(self, dt);
    }

    fn advance_active(&mut self) {
        let Some(active) = &self.active else {
            return;
        };
        if self.now() < active.end_time() {
            return;
        }
        let screen_screener = active.target;
        let outcome = resolve::resolve(self);
        let facts = outcome.facts.clone();
        for fact in facts {
            self.emit(fact);
        }
        if let Some(control) = outcome.delta.control {
            self.ball = BallState::Held(control);
        }
        if let Some(separation) = outcome.delta.separation_ft {
            self.last_separation = Some((outcome.actor, separation, self.now()));
        }
        if let Some(route) = outcome.delta.defender_route {
            let screener = screen_screener.unwrap_or(outcome.actor);
            self.possession.last_screen = Some(ScreenRead {
                route,
                at: self.now(),
                screener,
                separation_ft: outcome.delta.separation_ft.unwrap_or_default(),
            });
            // 换防路线意味着两名防守人交换对位对象。
            if matches!(route, crate::action::DefenderRoute::Switch) {
                let carrier = self.holder().unwrap_or(outcome.actor);
                crate::defense::switch_matchup(self, outcome.actor, carrier);
            }
        }
        self.active = None;
        self.next_decision_at = self.now() + DECISION_INTERVAL;
    }

    fn advance_phase(&mut self) {
        let now = self.now();
        match self.phase.clone() {
            Phase::DeadBall { team, ready_at } => {
                if now >= ready_at {
                    self.start_possession(team, PossessionKind::Dead);
                }
            }
            Phase::Timeout { team, resume_at } => {
                if now >= resume_at {
                    self.start_possession(team, PossessionKind::Dead);
                }
            }
            Phase::Inbound { team, ready_at } => {
                if now >= ready_at {
                    self.start_possession(team, PossessionKind::Dead);
                }
            }
            Phase::PassFlight { resolve_at } => {
                if now >= resolve_at {
                    self.finish_pass();
                }
            }
            Phase::ShotFlight { resolve_at } => {
                if now >= resolve_at {
                    crate::resolve::shot::finish(self);
                }
            }
            Phase::FreeThrows {
                shooter,
                total,
                remaining,
                made,
                next_at,
                and_one,
                retain_possession,
            } => {
                if now >= next_at {
                    crate::resolve::freethrow::shoot(
                        self,
                        shooter,
                        total,
                        remaining,
                        made,
                        and_one,
                        retain_possession,
                    );
                }
            }
            Phase::PeriodBreak { resume_at } => {
                if now >= resume_at {
                    self.clock.period += 1;
                    self.clock.game_clock = self.clock.period_seconds;
                    self.clock.shot_clock = self.rules.league.shot_clock_seconds as f64;
                    self.fouls[0].period = 0;
                    self.fouls[1].period = 0;
                    // 换节后最后时段的暂停配额重新计算。
                    for ledger in self.timeouts.iter_mut() {
                        ledger.period = self.clock.period;
                        ledger.used_in_final_window = 0;
                    }
                    let team = self.period_started_with.other();
                    self.period_started_with = team;
                    self.phase = Phase::DeadBall {
                        team,
                        ready_at: self.now() + 0.5,
                    };
                }
            }
            Phase::Live => {
                if self.clock.shot_clock <= 0.0 {
                    let actor = self.holder().unwrap_or(0);
                    self.line_mut(actor).turnovers += 1;
                    self.emit(Fact::Turnover {
                        player: actor,
                        kind: crate::action::TurnoverKind::ShotClock,
                    });
                    let team = self.players[actor].team;
                    self.end_possession("shot_clock", 0);
                    self.start_possession(team.other(), PossessionKind::Live);
                }
            }
            Phase::Finished => {}
        }
    }

    fn finish_pass(&mut self) {
        let BallState::Pass { to, .. } = self.ball.clone() else {
            return;
        };
        self.ball = BallState::Held(to);
        self.phase = Phase::Live;
        self.active = None;
        self.next_decision_at = self.now() + DECISION_INTERVAL;
    }

    fn maybe_decide(&mut self) {
        if !matches!(self.phase, Phase::Live) {
            return;
        }
        if self.active.is_some() {
            return;
        }
        if self.now() < self.next_decision_at {
            return;
        }
        // 选板与故意犯规都在决策时刻推进：两者都会发事实，
        // 而事实写入会分配内存，稳态 tick 必须保持零分配。
        crate::tactics::update_play_window(self);
        if crate::timeouts::coach_stop_run(self) {
            return;
        }
        if let Some(plan) = crate::endgame::intentional_foul(self) {
            crate::resolve::foul::apply_non_shooting_foul(self, plan.fouler, plan.target);
            return;
        }
        let decision = crate::policy::choose_action(self);
        let intent = decision.intent.clone();
        let since_screen = self
            .possession
            .last_screen
            .map(|read| self.now() - read.at)
            .unwrap_or(f64::INFINITY);
        let shooter_three = self.players[intent.actor].data.attributes.shooting_three;
        self.decision_samples.push(DecisionSample {
            route: crate::views::screen_read_route(self),
            family: intent.kind.family(),
            shot_clock: self.clock.shot_clock,
            since_screen,
            shooter_three,
            utility: decision.utility,
            beta: decision.beta,
            gamma: decision.gamma,
            candidates: decision.candidate_count.min(u8::MAX as usize) as u8,
            trace: Some(self.decision_traces.len() as u32),
        });
        self.active_trace = Some(self.decision_traces.len() as u32);
        self.decision_traces.push(decision.trace);
        self.next_decision_at = self.now() + DECISION_INTERVAL;
        if self.possession.first_action_seconds.is_none() {
            self.possession.first_action_seconds = Some(self.now() - self.possession.started_at);
        }
        match intent.kind {
            crate::action::ActionKind::Drive
            | crate::action::ActionKind::Finish
            | crate::action::ActionKind::Dribble
            | crate::action::ActionKind::PostUp
            | crate::action::ActionKind::Relocate => {
                self.players[intent.actor].target = intent.to_ft;
            }
            crate::action::ActionKind::Cut => {
                if let Some(cutter) = intent.target {
                    self.players[cutter].target = self.hoop_for(self.players[cutter].team);
                }
            }
            _ => {}
        }
        if let crate::action::ActionKind::Pass(_) = intent.kind {
            let target = intent.target.expect("pass requires a target");
            self.ball = BallState::Pass {
                from: intent.actor,
                to: target,
                from_pos: intent.from_ft,
                to_pos: intent.to_ft,
                start: intent.start_time,
                duration: intent.duration,
                completed: true,
            };
            self.phase = Phase::PassFlight {
                resolve_at: intent.end_time(),
            };
        }
        self.active = Some(intent);
    }

    fn check_period_end(&mut self) {
        if self.clock.game_clock > 0.0 || self.finished {
            return;
        }
        crate::tactics::close_play_window(self);
        if matches!(self.phase, Phase::Finished | Phase::PeriodBreak { .. }) {
            return;
        }
        let tied = self.score[0] == self.score[1];
        let regulation_over = self.clock.period >= 4;
        if regulation_over && tied {
            self.emit(Fact::PeriodEnd {
                period: self.clock.period,
            });
            self.end_possession("period_end", 0);
            self.clock.period_seconds = 300.0;
            self.phase = Phase::PeriodBreak {
                resume_at: self.now() + 1.0,
            };
            return;
        }
        if !regulation_over {
            self.emit(Fact::PeriodEnd {
                period: self.clock.period,
            });
            self.end_possession("period_end", 0);
            self.phase = Phase::PeriodBreak {
                resume_at: self.now() + 1.0,
            };
            return;
        }
        self.end_possession("period_end", 0);
        self.finished = true;
        self.phase = Phase::Finished;
        self.emit(Fact::GameEnd);
    }
}

pub struct GameReport {
    pub seed: u64,
    pub home_team: String,
    pub away_team: String,
    pub home_score: u32,
    pub away_score: u32,
    pub periods: u32,
    pub ticks: u64,
    pub open_business_instances: u64,
    pub rebound_attempts: u64,
    pub possessions: Vec<PossessionRecord>,
    pub box_score: BoxScore,
    pub events: Vec<TimedFact>,
    pub shot_clock_at_release: Vec<f64>,
    pub contest_samples: Vec<f32>,
    pub shot_samples: Vec<ShotSample>,
    pub pass_samples: Vec<PassSample>,
    pub decision_samples: Vec<DecisionSample>,
    /// 逐次决策的因果快照，供决定审计与回归比对。
    pub decision_traces: Vec<crate::state::DecisionTraceSample>,
    pub shot_zone_counts: [[u32; 4]; 2],
    pub shot_zone_makes: [[u32; 4]; 2],
    pub creation_counts: [u32; 7],
    pub violations: Vec<String>,
    pub games: u32,
}

impl GameReport {
    pub fn empty() -> Self {
        Self {
            seed: 0,
            home_team: String::new(),
            away_team: String::new(),
            home_score: 0,
            away_score: 0,
            periods: 0,
            ticks: 0,
            open_business_instances: 0,
            rebound_attempts: 0,
            possessions: Vec::new(),
            box_score: BoxScore::default(),
            events: Vec::new(),
            shot_clock_at_release: Vec::new(),
            contest_samples: Vec::new(),
            shot_samples: Vec::new(),
            pass_samples: Vec::new(),
            decision_samples: Vec::new(),
            decision_traces: Vec::new(),
            shot_zone_counts: [[0; 4]; 2],
            shot_zone_makes: [[0; 4]; 2],
            creation_counts: [0; 7],
            violations: Vec::new(),
            games: 0,
        }
    }

    pub fn merge(&mut self, other: &GameReport) {
        self.games += other.games;
        self.ticks += other.ticks;
        self.rebound_attempts += other.rebound_attempts;
        // 未结算实例是单场比赛的流尾性质，聚合后没有对应物，不累加。
        self.open_business_instances = 0;
        self.home_score += other.home_score;
        self.away_score += other.away_score;
        self.possessions.extend_from_slice(&other.possessions);
        for team in [0usize, 1] {
            for zone in 0..4 {
                self.shot_zone_counts[team][zone] += other.shot_zone_counts[team][zone];
                self.shot_zone_makes[team][zone] += other.shot_zone_makes[team][zone];
            }
        }
        for (mine, theirs) in self
            .creation_counts
            .iter_mut()
            .zip(other.creation_counts.iter())
        {
            *mine += theirs;
        }
        self.contest_samples
            .extend_from_slice(&other.contest_samples);
        self.shot_clock_at_release
            .extend_from_slice(&other.shot_clock_at_release);
        self.shot_samples.extend_from_slice(&other.shot_samples);
        self.pass_samples.extend_from_slice(&other.pass_samples);
        self.decision_samples
            .extend_from_slice(&other.decision_samples);
        self.violations.extend_from_slice(&other.violations);
        // 过程判据从事实流聚合，因此聚合报告必须保留事件。
        // 事件里的 `trace` 是局内下标，并入后要按下标偏移重映射，
        // 否则聚合报告的决策快照会指向另一场比赛的另一手动作。
        let offset = self.decision_traces.len() as u32;
        let start = self.events.len();
        self.events.extend_from_slice(&other.events);
        if offset != 0 {
            for event in &mut self.events[start..] {
                if let Some(index) = event.trace {
                    event.trace = Some(index + offset);
                }
            }
        }
        self.decision_traces
            .extend(other.decision_traces.iter().cloned());
        merge_lines(&mut self.box_score.home, &other.box_score.home);
        merge_lines(&mut self.box_score.away, &other.box_score.away);
    }
}

fn merge_lines(mine: &mut Vec<PlayerLine>, theirs: &[PlayerLine]) {
    for line in theirs {
        match mine.iter_mut().find(|existing| existing.id == line.id) {
            Some(existing) => {
                existing.seconds += line.seconds;
                existing.points += line.points;
                existing.fgm += line.fgm;
                existing.fga += line.fga;
                existing.tpm += line.tpm;
                existing.tpa += line.tpa;
                existing.ftm += line.ftm;
                existing.fta += line.fta;
                existing.oreb += line.oreb;
                existing.dreb += line.dreb;
                existing.assists += line.assists;
                existing.steals += line.steals;
                existing.blocks += line.blocks;
                existing.turnovers += line.turnovers;
                existing.fouls += line.fouls;
                existing.shooting_fouls += line.shooting_fouls;
                existing.and_one += line.and_one;
                existing.plus_minus += line.plus_minus;
            }
            None => mine.push(line.clone()),
        }
    }
}

fn assign_slots(players: &mut [PlayerState], team: TeamSide) {
    let mut used = [false; 5];
    for player in players.iter_mut().filter(|p| p.on_court && p.team == team) {
        player.slot = slot_for_position(player.data.position, &mut used);
    }
}

fn slot_for_position(position: nba_domain::PlayerPosition, used: &mut [bool; 5]) -> Slot {
    let order: [Slot; 5] = match position {
        nba_domain::PlayerPosition::Point | nba_domain::PlayerPosition::Combo => [
            Slot::Handler,
            Slot::Wing,
            Slot::Corner,
            Slot::Post,
            Slot::DunkerSpot,
        ],
        nba_domain::PlayerPosition::Wing => [
            Slot::Wing,
            Slot::Corner,
            Slot::Handler,
            Slot::Post,
            Slot::DunkerSpot,
        ],
        nba_domain::PlayerPosition::Forward => [
            Slot::Corner,
            Slot::Post,
            Slot::Wing,
            Slot::DunkerSpot,
            Slot::Handler,
        ],
        nba_domain::PlayerPosition::Big => [
            Slot::Post,
            Slot::DunkerSpot,
            Slot::Corner,
            Slot::Wing,
            Slot::Handler,
        ],
        nba_domain::PlayerPosition::Center => [
            Slot::DunkerSpot,
            Slot::Post,
            Slot::Corner,
            Slot::Wing,
            Slot::Handler,
        ],
    };
    for slot in order {
        let index = slot_index(slot);
        if !used[index] {
            used[index] = true;
            return slot;
        }
    }
    Slot::DunkerSpot
}

fn slot_index(slot: Slot) -> usize {
    match slot {
        Slot::Handler => 0,
        Slot::Wing => 1,
        Slot::Corner => 2,
        Slot::Post => 3,
        Slot::DunkerSpot => 4,
        Slot::Top => 0,
    }
}

pub use crate::formation::{defense_spots, offense_spots};
