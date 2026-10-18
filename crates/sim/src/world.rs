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
    BallState, Clock, DecisionState, DefenderView, HelpView, PlayerState, PossessionKind,
    PossessionState, Slot, TeammateView,
};

pub const TICK_SECONDS: f64 = 0.05;
pub const DECISION_INTERVAL: f64 = 0.55;
/// 推进到前场之后的落位时间：持球人到达顶弧还需要给队友落位的时间。
pub const POSSESSION_SETUP_SECONDS: f64 = 3.0;
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
}

#[derive(Debug, Clone, Default)]
pub struct TeamFouls {
    pub period: u8,
    pub total: u8,
}

#[derive(Debug, Clone)]
pub struct PossessionRecord {
    pub team: TeamSide,
    pub id: u64,
    pub kind: PossessionKind,
    pub start_clock: f64,
    pub end_clock: f64,
    pub duration: f64,
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
    pub active: Option<ActionIntent>,
    pub next_decision_at: f64,
    pub events: Vec<TimedFact>,
    pub rng: ChaCha8Rng,
    pub seed: u64,
    pub fouls: [TeamFouls; 2],
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
    pub alternate_spacing: bool,
    pub shot_clock_at_release: Vec<f64>,
    pub contest_samples: Vec<f32>,
    pub shot_zone_counts: [[u32; 4]; 2],
    pub shot_zone_makes: [[u32; 4]; 2],
    pub creation_counts: [u32; 7],
    pub next_shot_id: u64,
    pub next_pass_id: u64,
    /// 走到篮板结算的投失次数。犯规导致的投失不进入篮板，
    /// 前场篮板率的分子分母必须用这个数对账。
    pub rebound_attempts: u64,
}

impl World {
    pub fn new(setup: MatchSetup, params: Params, seed: u64) -> Self {
        let rules = setup.rules.clone();
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
        let home_lines = players
            .iter()
            .filter(|p| p.team == TeamSide::Home)
            .map(|p| PlayerLine::new(&p.id, &p.name, &p.jersey, TeamSide::Home))
            .collect();
        let away_lines = players
            .iter()
            .filter(|p| p.team == TeamSide::Away)
            .map(|p| PlayerLine::new(&p.id, &p.name, &p.jersey, TeamSide::Away))
            .collect();
        let period_seconds = rules.league.period_duration_seconds as f64;
        let shot_clock = rules.league.shot_clock_seconds as f64;
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
            possession: PossessionState::new(first, 0, PossessionKind::Dead, 0.0, shot_clock),
            score: [0, 0],
            box_score: BoxScore {
                home: home_lines,
                away: away_lines,
            },
            active: None,
            next_decision_at: 0.0,
            events: Vec::new(),
            rng,
            seed,
            fouls: [TeamFouls::default(), TeamFouls::default()],
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
            alternate_spacing: false,
            shot_clock_at_release: Vec::new(),
            contest_samples: Vec::new(),
            shot_zone_counts: [[0; 4]; 2],
            shot_zone_makes: [[0; 4]; 2],
            creation_counts: [0; 7],
            next_shot_id: 0,
            next_pass_id: 0,
            rebound_attempts: 0,
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

    pub fn emit(&mut self, fact: Fact) {
        self.events.push(TimedFact {
            time: self.now(),
            period: self.clock.period,
            fact,
        });
    }

    pub fn line_mut(&mut self, player: usize) -> &mut PlayerLine {
        let team = self.players[player].team;
        let id = self.players[player].id.clone();
        let lines = match team {
            TeamSide::Home => &mut self.box_score.home,
            TeamSide::Away => &mut self.box_score.away,
        };
        lines
            .iter_mut()
            .find(|line| line.id == id)
            .expect("box score line must exist for every player")
    }

    pub fn add_points(&mut self, team: TeamSide, points: u32) {
        let index = Self::team_index(team);
        self.score[index] += points;
        self.possession_points += points;
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

    pub fn substitute_if_needed(&mut self) {
        for team in [TeamSide::Home, TeamSide::Away] {
            for handle in self.on_court(team) {
                let needs_rest =
                    self.players[handle].fouls >= 6 || self.players[handle].stamina < 0.55;
                if !needs_rest {
                    continue;
                }
                let bench = (0..self.players.len())
                    .filter(|candidate| {
                        self.players[*candidate].team == team
                            && !self.players[*candidate].on_court
                            && self.players[*candidate].fouls < 6
                    })
                    .max_by(|a, b| {
                        self.players[*a]
                            .stamina
                            .partial_cmp(&self.players[*b].stamina)
                            .expect("finite stamina")
                    });
                let Some(bench) = bench else {
                    continue;
                };
                let position = self.players[handle].pos;
                let target = self.players[handle].target;
                let slot = self.players[handle].slot;
                self.players[bench].on_court = true;
                self.players[bench].pos = position;
                self.players[bench].target = target;
                self.players[bench].slot = slot;
                self.players[handle].on_court = false;
                if self.holder() == Some(handle) {
                    self.ball = BallState::Held(bench);
                }
                self.emit(Fact::Substitution {
                    team,
                    incoming: bench,
                    outgoing: handle,
                });
            }
        }
    }

    pub fn on_court(&self, team: TeamSide) -> Vec<usize> {
        (0..self.players.len())
            .filter(|handle| self.players[*handle].on_court && self.players[*handle].team == team)
            .collect()
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
        if self.attacking_right(team) {
            Vec2::new(28.0, 25.0)
        } else {
            Vec2::new(66.0, 25.0)
        }
    }

    fn top_of_key(&self, team: TeamSide) -> Vec2 {
        if self.attacking_right(team) {
            Vec2::new(64.0, 25.0)
        } else {
            Vec2::new(30.0, 25.0)
        }
    }

    pub fn reset_positions(&mut self, offense: TeamSide) {
        let spots = offense_spots(
            &self.params.formations,
            self.attacking_right(offense),
            self.alternate_spacing,
        );
        let defense = defense_spots(&self.params.formations, self.attacking_right(offense));
        let mut offense_index = 0;
        let mut defense_index = 0;
        for handle in 0..self.players.len() {
            if !self.players[handle].on_court {
                continue;
            }
            let is_offense = self.players[handle].team == offense;
            let target = if is_offense {
                let spot = spots[offense_index.min(spots.len() - 1)];
                offense_index += 1;
                spot
            } else {
                let spot = defense[defense_index.min(defense.len() - 1)];
                defense_index += 1;
                spot
            };
            self.players[handle].pos = target;
            self.players[handle].vel = Vec2::ZERO;
            self.players[handle].target = target;
            self.players[handle].home_spot = target;
        }
    }

    pub fn start_possession(&mut self, team: TeamSide, kind: PossessionKind) {
        self.substitute_if_needed();
        self.possession_count += 1;
        self.alternate_spacing = self.possession_count % 2 == 0;
        let shot_clock = self.rules.league.shot_clock_seconds as f64;
        self.clock.shot_clock = shot_clock;
        self.possession =
            PossessionState::new(team, self.possession_count, kind, self.now(), shot_clock);
        self.possession_points = 0;
        self.pending_shot = None;
        self.reset_positions(team);
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
        let speed = self.players[handler].effective_speed().max(1.0);
        // 推进时长 = 匀速段 + 起步加速段：加速度决定多快到达目标速度。
        let accel = self.players[handler].effective_acceleration().max(1.0);
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

    pub fn start_second_chance(&mut self, team: TeamSide, rebounder: usize) {
        self.possession_count += 1;
        let shot_clock = self.rules.league.offensive_rebound_shot_clock_seconds as f64;
        self.clock.shot_clock = shot_clock;
        self.possession = PossessionState::new(
            team,
            self.possession_count,
            PossessionKind::Live,
            self.now(),
            shot_clock,
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
        let record = PossessionRecord {
            team: self.possession.team,
            id: self.possession.id,
            kind: self.possession.kind,
            start_clock: self.possession.shot_clock_start,
            end_clock: self.clock.shot_clock,
            duration: self.now() - self.possession.started_at,
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
        let actor = self.holder().expect("decision requires a ball holder");
        let team = self.players[actor].team;
        let hoop = self.hoop_for(team);
        let pos = self.players[actor].pos;
        let mut defenders: Vec<(usize, f32)> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.on_court && p.team != team)
            .map(|(index, p)| (index, (p.pos - pos).length()))
            .collect();
        defenders.sort_by(|a, b| a.1.partial_cmp(&b.1).expect("finite distances"));
        let primary = defenders.first().map(|(index, distance)| {
            let defender = &self.players[*index];
            let closing = (defender.vel - self.players[actor].vel)
                .dot((pos - defender.pos).normalize_or_zero());
            DefenderView {
                index: *index,
                distance_ft: *distance,
                closing_speed_ftps: closing,
                between_ball_and_hoop: crate::state::is_between(defender.pos, pos, hoop),
                beaten: *distance > 5.0,
                route: None,
            }
        });
        let help = defenders.get(1).map(|(index, distance)| {
            let defender = &self.players[*index];
            HelpView {
                distance_ft: *distance,
                arrival_seconds: (*distance / defender.effective_speed()).max(0.05),
            }
        });
        let teammates = self
            .on_court(team)
            .into_iter()
            .filter(|handle| *handle != actor)
            .map(|handle| {
                let teammate = &self.players[handle];
                let nearest = self
                    .players
                    .iter()
                    .filter(|p| p.on_court && p.team != team)
                    .map(|p| (p.pos - teammate.pos).length())
                    .fold(f32::INFINITY, f32::min);
                TeammateView {
                    index: handle,
                    pos: teammate.pos,
                    openness: crate::state::contest_from_defender(nearest, false),
                    defender_distance_ft: nearest,
                    lane_clearance: crate::state::lane_clearance(
                        &self.players,
                        pos,
                        teammate.pos,
                        team,
                    ),
                    distance_to_hoop: (teammate.pos - hoop).length(),
                }
            })
            .collect();
        let lane = crate::state::lane_clearance(&self.players, pos, hoop, team);
        let mismatch = self.mismatch(actor, team);
        DecisionState {
            actor,
            actor_pos: pos,
            hoop,
            attacking_right: self.attacking_right(team),
            clock: self.clock.game_clock,
            shot_clock: self.clock.shot_clock,
            margin: self.score[Self::team_index(team)] as i32
                - self.score[Self::team_index(team.other())] as i32,
            period: self.clock.period,
            primary_defender: primary,
            help,
            teammates,
            lane_clearance: lane,
            mismatch,
            stamina: self.players[actor].stamina,
            possession_kind: self.possession.kind,
            elapsed_in_possession: self.now() - self.possession.started_at,
        }
    }

    fn mismatch(&self, actor: usize, team: TeamSide) -> f32 {
        let nearest = self
            .players
            .iter()
            .filter(|p| p.on_court && p.team != team)
            .min_by(|a, b| {
                (a.pos - self.players[actor].pos)
                    .length_squared()
                    .partial_cmp(&(b.pos - self.players[actor].pos).length_squared())
                    .expect("finite distances")
            });
        let Some(defender) = nearest else {
            return 0.0;
        };
        let offense = self.players[actor].data.attributes.ball_handling
            + self.players[actor].data.attributes.agility * 0.5;
        let defense =
            defender.data.attributes.defense_perimeter + defender.data.attributes.agility * 0.5;
        (offense - defense).clamp(-1.0, 1.0) * 0.5
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
        self.advance_movement(dt);
        self.advance_active();
        self.advance_phase();
        self.maybe_decide();
        self.check_period_end();
    }

    fn update_stamina(&mut self, dt: f64) {
        for player in self.players.iter_mut() {
            if !player.on_court {
                continue;
            }
            let drain = 0.00060 * (1.0 - player.data.attributes.stamina);
            player.stamina = (player.stamina - drain * dt as f32).clamp(0.45, 1.0);
        }
    }

    fn advance_movement(&mut self, dt: f64) {
        crate::policy::move_toward_targets(self);
        let targets: Vec<(usize, Vec2)> = self
            .players
            .iter()
            .enumerate()
            .filter(|(_, p)| p.on_court)
            .map(|(handle, p)| (handle, p.target))
            .collect();
        for (handle, target) in targets {
            let mut player = self.players[handle].clone();
            resolve::move_player(&mut player, target, dt as f32);
            self.players[handle] = player;
        }
    }

    fn advance_active(&mut self) {
        let Some(active) = &self.active else {
            return;
        };
        if self.now() < active.end_time() {
            return;
        }
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
            } => {
                if now >= next_at {
                    crate::resolve::freethrow::shoot(
                        self, shooter, total, remaining, made, and_one,
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
        let intent = crate::policy::choose_action(self);
        self.next_decision_at = self.now() + DECISION_INTERVAL;
        if self.possession.first_action_seconds.is_none() {
            self.possession.first_action_seconds = Some(self.now() - self.possession.started_at);
        }
        match intent.kind {
            crate::action::ActionKind::Drive
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
        self.violations.extend_from_slice(&other.violations);
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

pub fn offense_spots(
    formations: &crate::params::Formations,
    attacking_right: bool,
    alternate: bool,
) -> [Vec2; 5] {
    let source = if alternate && !formations.offense_alt.is_empty() {
        &formations.offense_alt
    } else {
        &formations.offense
    };
    let spots = [
        Vec2::from(source[0]),
        Vec2::from(source[1]),
        Vec2::from(source[2]),
        Vec2::from(source[3]),
        Vec2::from(source[4]),
    ];
    mirror(spots, attacking_right)
}

pub fn defense_spots(formations: &crate::params::Formations, attacking_right: bool) -> [Vec2; 5] {
    let spots = [
        Vec2::from(formations.defense[0]),
        Vec2::from(formations.defense[1]),
        Vec2::from(formations.defense[2]),
        Vec2::from(formations.defense[3]),
        Vec2::from(formations.defense[4]),
    ];
    mirror(spots, attacking_right)
}

fn mirror(spots: [Vec2; 5], attacking_right: bool) -> [Vec2; 5] {
    if attacking_right {
        spots
    } else {
        spots.map(|point| Vec2::new(94.0 - point.x, 50.0 - point.y))
    }
}
