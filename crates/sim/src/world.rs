//! 世界状态与主循环。
//!
//! 每 tick 的顺序固定：时钟 → 运动 → 动作到期结算 → 阶段推进 → 决策。
//! 结算发生在动作到期时，决策只读派生状态；球的位置只服务表现，不裁定
//! 任何结果。

use glam::Vec2;
use nba_domain::event::PossessionEndCause;
use nba_domain::{GameRules, TeamData};
use std::collections::BTreeMap;

use crate::action::{
    ActionIntent, ActionKind, ActionRuntime, DefensiveActionKind, DefensiveActionRuntime, Fact,
    Interaction, TeamSide, TurnoverKind,
};
use crate::ball::BallBody;
use crate::boxscore::{BoxScore, PlayerLine};
use crate::draw::{purpose, DrawKey, DrawSource, Stream};
use crate::params::RuntimeParams;
use crate::resolve;
use crate::resolve::shot::PendingShot;
pub use crate::state::Phase;
use crate::state::{
    Clock, DecisionRecord, DecisionSample, DecisionState, OnCourt, PassSample, PlayerState,
    PossessionKind, PossessionState, ScreenRead, ShotSample, Slot,
};

pub use nba_domain::rules::TICK_SECONDS;
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
    pub tick_index: u64,
    pub sequence: u64,
    pub event_id: u64,
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
    /// 全场技术犯规累计。
    pub technical: u8,
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

/// 停表跳球的准备：参跳双方、到位时刻与四个站位。
#[derive(Debug, Clone)]
pub struct PendingJumpBall {
    pub home: usize,
    pub away: usize,
    pub ready_at: f64,
    /// 参跳人与接应人的目标点：到位后才掷球。
    pub spots: Vec<(usize, Vec2)>,
    /// 已经等待的秒数：迟迟有人到不了位时不能无限停表。
    pub waited: f32,
}

#[derive(Debug, Clone)]
pub struct World {
    pub params: RuntimeParams,
    pub rules: GameRules,
    pub clock: Clock,
    phase: Phase,
    pub ball: BallBody,
    pub pass_result: Option<crate::ball::PassResult>,
    pub pass_in_flight: Option<crate::ball::PassInFlight>,
    /// 最后触球的球员。出界判定按最后触球方决定球权，所以每次持球与
    /// 碰球都要更新它。
    pub last_touch: Option<usize>,
    pub players: Vec<PlayerState>,
    pub possession: PossessionState,
    pub score: [u32; 2],
    pub box_score: BoxScore,
    /// 球员句柄到所属队伍箱体行下标的映射。构建期算一次，换人不重排行序。
    pub line_slot: Vec<u8>,
    /// 尚未结算的多人交互，按交互编号升序结算。
    pub interactions: BTreeMap<u64, Interaction>,
    pub screen_defender: Option<usize>,
    pub defensive_actions: Vec<Option<DefensiveActionRuntime>>,
    pub next_action_id: u64,
    pub next_interaction_id: u64,
    pub action_turn: Option<(u64, crate::action::ActionTurn)>,
    pub events: Vec<TimedFact>,
    pub draws: DrawSource,
    pub seed: u64,
    pub fouls: [TeamFouls; 2],
    pub rotation: RotationState,
    pub timeouts: [crate::timeouts::TimeoutBudget; 2],
    /// 最近一次得分方（队下标，0 主 1 客）与这一方的连续得分。
    pub last_scoring: Option<usize>,
    pub scoring_run: u32,
    pub possession_count: u64,
    pub period_started_with: TeamSide,
    pub alternate_possession_next: TeamSide,
    pub(crate) period_start_inbound_pending: bool,
    pub(crate) jump_ball_tapper: Option<usize>,
    pub(crate) jump_ball_receiver: Option<usize>,
    /// 跳球开始时实际站位的接应人，按球队索引记录。点球方向由被点球队
    /// 决定，接球人必须是该队已经站到落点上的那个人。
    pub(crate) jump_ball_receivers: [Option<usize>; 2],
    pub(crate) jump_ball_kind: Option<crate::action::JumpBallKind>,
    pub first_possession: Option<TeamSide>,
    pub finished: bool,
    pub home_team_name: String,
    pub away_team_name: String,
    pub last_pass_kind: Option<crate::action::PassKind>,
    pub possession_log: Vec<PossessionRecord>,
    pub pending_shot: Option<PendingShot>,
    pub pending_rebound: Option<crate::resolve::rebound::PendingRebound>,
    /// 停表跳球的准备：双方参跳人与到位时刻。到位后才掷球。
    pub pending_jump_ball: Option<PendingJumpBall>,
    /// 刚被切掉球的持球人，以及恢复之前不可再次被切的截止时间。
    pub poke_cooldown: Option<(usize, f64)>,
    /// 当前生效的护球姿态。开始其他动作或窗口到期即失效。
    pub protection: Option<crate::state::BallProtection>,
    pub possession_points: u32,
    pub inbound_passer: Option<usize>,
    /// 本次发球的地点：由停表原因写入（出界点、违例点）；无记录时用本方底线。
    pub inbound_spot: Option<Vec2>,
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
    /// 逐次实际决策的审计记录（观察摘要、执行前重校验）。
    pub decision_records: Vec<DecisionRecord>,
    /// 当前生效的决策快照下标：本次决策产生的事实都带着它。
    pub active_trace: Option<u32>,
    pub next_event_id: u64,
    pub last_event_tick: u64,
    pub next_event_sequence: u64,
    pub shot_zone_counts: [[u32; 4]; 2],
    pub shot_zone_makes: [[u32; 4]; 2],
    pub creation_counts: [u32; 7],
    pub next_shot_id: u64,
    pub next_pass_id: u64,
    pub next_foul_id: u64,
    /// 走到篮板结算的投失次数。犯规导致的投失不进入篮板，
    /// 前场篮板率的分子分母必须用这个数对账。
    pub rebound_attempts: u64,
    /// 连续得分带来的势头（0 = 无，越大越热）。按队存，随时间衰减。
    pub momentum: [f32; 2],
    /// 读球历史：防守的协防轮转看的是 `delay` 秒之前的球位。
    pub(crate) ball_history: Vec<(f64, Vec2)>,
    /// 逐 tick 运动积分的暂存缓冲，稳态不分配。
    pub(crate) body_before: Vec<Vec2>,
    pub on_court_home: OnCourt,
    pub on_court_away: OnCourt,
}

impl World {
    fn set_program_phase(
        &mut self,
        stage: crate::state::MatchStage,
        program: crate::state::MatchProgram,
        phase: Phase,
    ) {
        assert!(
            !matches!(phase, Phase::PeriodBreak { .. }),
            "period breaks use set_period_phase"
        );
        assert!(
            !matches!(phase, Phase::Timeout { .. }),
            "timeout phases use set_timeout_phase"
        );
        assert_eq!(program, phase.program(), "phase and program disagree");
        assert_eq!(
            matches!(stage, crate::state::MatchStage::Pregame),
            matches!(phase, Phase::Pregame),
            "pregame stage requires pregame phase"
        );
        assert_eq!(
            matches!(stage, crate::state::MatchStage::Finished),
            matches!(phase, Phase::Finished),
            "finished stage requires finished phase"
        );
        assert_eq!(
            phase.stage(self.clock.period(), self.rules.league.regulation_periods),
            stage,
            "phase and match stage disagree"
        );
        let current_stage = self.match_stage();
        let stage_transition_is_allowed = stage == current_stage
            || (current_stage == crate::state::MatchStage::Pregame
                && stage == crate::state::MatchStage::Regulation)
            || (current_stage == crate::state::MatchStage::Intermission
                && matches!(
                    stage,
                    crate::state::MatchStage::Intermission
                        | crate::state::MatchStage::Regulation
                        | crate::state::MatchStage::Overtime
                ))
            || (current_stage == crate::state::MatchStage::Regulation
                && matches!(
                    stage,
                    crate::state::MatchStage::Intermission
                        | crate::state::MatchStage::Overtime
                        | crate::state::MatchStage::Finished
                ))
            || (current_stage == crate::state::MatchStage::Overtime
                && matches!(
                    stage,
                    crate::state::MatchStage::Intermission
                        | crate::state::MatchStage::Regulation
                        | crate::state::MatchStage::Finished
                ));
        assert!(
            stage_transition_is_allowed,
            "match stage transition {current_stage:?} -> {stage:?} through {phase:?} is not permitted"
        );
        self.phase = phase;
    }

    pub(crate) fn set_period_phase(&mut self, phase: Phase) {
        let Phase::PeriodBreak { next_stage, .. } = phase else {
            panic!("period phase requires a period break");
        };
        let next_period = self.clock.period() + 1;
        let expected_stage = if next_period > self.rules.league.regulation_periods {
            crate::state::MatchStage::Overtime
        } else {
            crate::state::MatchStage::Intermission
        };
        assert_eq!(
            next_stage, expected_stage,
            "period break destination must match the next period"
        );
        let current_stage = self.match_stage();
        assert!(
            matches!(
                current_stage,
                crate::state::MatchStage::Regulation | crate::state::MatchStage::Overtime
            ),
            "period break cannot begin from {current_stage:?}"
        );
        let stage = phase.stage(self.clock.period(), self.rules.league.regulation_periods);
        assert!(
            matches!(
                stage,
                crate::state::MatchStage::Intermission | crate::state::MatchStage::Overtime
            ),
            "period break phase must report intermission or overtime stage"
        );
        assert_eq!(
            phase.program(),
            crate::state::MatchProgram::PeriodSettlement
        );
        self.phase = phase;
    }

    fn opening_tip_setup(
        players: &[PlayerState],
        policy: &crate::params::PolicyTable,
    ) -> (usize, usize) {
        let select_jumper = |team| {
            players
                .iter()
                .enumerate()
                .filter(|(_, player)| player.on_court && player.team == team)
                .max_by(|(_, left), (_, right)| {
                    let left_physical = crate::physical::Physical::of(left, policy);
                    let right_physical = crate::physical::Physical::of(right, policy);
                    left_physical
                        .standing_reach_ft
                        .total_cmp(&right_physical.standing_reach_ft)
                        .then_with(|| {
                            left_physical
                                .vertical_ft
                                .total_cmp(&right_physical.vertical_ft)
                        })
                        .then_with(|| right.id.cmp(&left.id))
                })
                .map(|(handle, _)| handle)
                .expect("each team must field a jump-ball participant")
        };
        (select_jumper(TeamSide::Home), select_jumper(TeamSide::Away))
    }

    fn period_jumpers(&self) -> (usize, usize) {
        let select_jumper = |team| {
            self.on_court(team)
                .into_iter()
                .max_by(|left, right| {
                    let left_physical =
                        crate::physical::Physical::of(&self.players[*left], &self.params.policy);
                    let right_physical =
                        crate::physical::Physical::of(&self.players[*right], &self.params.policy);
                    left_physical
                        .standing_reach_ft
                        .total_cmp(&right_physical.standing_reach_ft)
                        .then_with(|| {
                            left_physical
                                .vertical_ft
                                .total_cmp(&right_physical.vertical_ft)
                        })
                        .then_with(|| self.players[*right].id.cmp(&self.players[*left].id))
                })
                .expect("each team must field a jump-ball participant")
        };
        (select_jumper(TeamSide::Home), select_jumper(TeamSide::Away))
    }

    fn begin_opening_jump_ball(&mut self, home_jumper: usize, away_jumper: usize) {
        self.begin_period_jump_ball(
            home_jumper,
            away_jumper,
            crate::action::JumpBallKind::Opening,
            true,
        );
    }

    fn begin_period_jump_ball(
        &mut self,
        home_jumper: usize,
        away_jumper: usize,
        kind: crate::action::JumpBallKind,
        // 节次开局直接把站位写到位；比赛内停表跳球时球员已经走到圈边，
        // 不再写位置，只确认目标。
        place: bool,
    ) {
        self.jump_ball_kind = Some(kind);
        self.jump_ball_tapper = None;
        self.jump_ball_receiver = None;
        self.jump_ball_receivers = [None; 2];
        let center = self.rules.court.center();
        let radius = self.rules.league.center_circle_radius_ft * 0.5;
        let home_spot = center - Vec2::new(radius, 0.0);
        let away_spot = center + Vec2::new(radius, 0.0);
        if place {
            crate::body::place(self, home_jumper, home_spot);
            crate::body::place(self, away_jumper, away_spot);
        } else {
            crate::assignment::force_target_as(
                self,
                home_jumper,
                home_spot,
                crate::state::TargetKind::Station,
                away_jumper,
            );
            crate::assignment::force_target_as(
                self,
                away_jumper,
                away_spot,
                crate::state::TargetKind::Station,
                home_jumper,
            );
        }
        self.players[home_jumper].facing = Vec2::X;
        self.players[away_jumper].facing = Vec2::NEG_X;
        for team in [TeamSide::Home, TeamSide::Away] {
            let jumper = if team == TeamSide::Home {
                home_jumper
            } else {
                away_jumper
            };
            let receiver = self
                .on_court(team)
                .into_iter()
                .filter(|handle| *handle != jumper)
                .min_by(|left, right| {
                    (self.players[*left].pos - center)
                        .length_squared()
                        .total_cmp(&(self.players[*right].pos - center).length_squared())
                        .then_with(|| self.players[*left].id.cmp(&self.players[*right].id))
                })
                .expect("each team needs a jump-ball receiver");
            let direction = if team == TeamSide::Home {
                -Vec2::X
            } else {
                Vec2::X
            };
            // 接应人站到点球落点上：落点距离必须同时满足中圈外的站位要求
            // 与点球档位，否则接应人够不到球。
            let distance = self.rules.tip_off_tap_distance_ft;
            assert!(
                distance >= self.rules.league.jump_ball_receiver_distance_ft,
                "jump-ball tap distance must place the receiver outside the center circle"
            );
            assert!(
                distance >= self.rules.league.center_circle_radius_ft,
                "jump-ball receiver must stand outside the center circle"
            );
            let target = center + direction * distance;
            let radius = self.rules.player_radius_ft;
            assert!(
                target.x >= radius
                    && target.x <= self.rules.court.width_ft - radius
                    && target.y >= radius
                    && target.y <= self.rules.court.height_ft - radius,
                "jump-ball receiver spot must be inside playable court geometry"
            );
            if place {
                crate::body::place(self, receiver, target);
            } else {
                crate::assignment::force_target_as(
                    self,
                    receiver,
                    target,
                    crate::state::TargetKind::Station,
                    jumper,
                );
            }
            self.players[receiver].target_kind = crate::state::TargetKind::Station;
            self.players[receiver].target_partner = jumper;
            self.jump_ball_receivers[World::team_index(team)] = Some(receiver);
            self.emit(Fact::PlacementApplied {
                player: receiver,
                reason: "jump_ball_receiver",
            });
        }
        self.ball = BallBody::loose(center, Vec2::ZERO);
        let coefficients = crate::physical::Coefficients::from_policy(&self.params.policy);
        let home = crate::physical::Physical::with(&self.players[home_jumper], &coefficients);
        let away = crate::physical::Physical::with(&self.players[away_jumper], &coefficients);
        let peak_height = (home.standing_reach_ft + home.vertical_ft)
            .max(away.standing_reach_ft + away.vertical_ft);
        let rise = (peak_height - self.rules.chest_height_ft).max(f32::MIN_POSITIVE);
        self.ball.z = self.rules.chest_height_ft;
        self.ball.vz = (2.0 * self.rules.ball_gravity_ftps2 * rise).sqrt();
        let resolve_after = f64::from(self.ball.vz / self.rules.ball_gravity_ftps2);
        self.set_phase(Phase::JumpBall {
            home_jumper,
            away_jumper,
            resolve_at: self.now() + resolve_after,
            kind,
        });
    }

    /// 双方同时控住球：停表，参跳的四人走到中圈，到位后再掷球。
    pub(crate) fn stage_held_ball(&mut self, home_jumper: usize, away_jumper: usize) {
        let center = self.rules.court.center();
        let radius = self.rules.league.center_circle_radius_ft * 0.5;
        let distance = self.rules.tip_off_tap_distance_ft;
        let mut spots: Vec<(usize, Vec2)> = Vec::with_capacity(4);
        for jumper in [home_jumper, away_jumper] {
            let team = self.players[jumper].team;
            let opponent = if jumper == home_jumper {
                away_jumper
            } else {
                home_jumper
            };
            let spot = if team == TeamSide::Home {
                center - Vec2::new(radius, 0.0)
            } else {
                center + Vec2::new(radius, 0.0)
            };
            crate::assignment::force_target_as(
                self,
                jumper,
                spot,
                crate::state::TargetKind::Station,
                opponent,
            );
            spots.push((jumper, spot));
            let receiver = self
                .on_court(team)
                .into_iter()
                .filter(|handle| *handle != jumper)
                .min_by(|left, right| {
                    (self.players[*left].pos - center)
                        .length_squared()
                        .total_cmp(&(self.players[*right].pos - center).length_squared())
                        .then_with(|| self.players[*left].id.cmp(&self.players[*right].id))
                });
            if let Some(receiver) = receiver {
                let direction = if team == TeamSide::Home {
                    -Vec2::X
                } else {
                    Vec2::X
                };
                let target = center + direction * distance;
                crate::assignment::force_target_as(
                    self,
                    receiver,
                    target,
                    crate::state::TargetKind::Station,
                    jumper,
                );
                spots.push((receiver, target));
            }
        }
        let team = self.possession.team;
        let ready_at = self.now() + f64::from(self.params.policy.g("jump_ball_staging_seconds"));
        self.pending_jump_ball = Some(PendingJumpBall {
            home: home_jumper,
            away: away_jumper,
            ready_at,
            spots,
            waited: 0.0,
        });
        self.set_phase(Phase::DeadBall { team, ready_at });
    }

    fn resolve_opening_jump_ball(
        &mut self,
        home_jumper: usize,
        away_jumper: usize,
        kind: crate::action::JumpBallKind,
    ) {
        let coefficients = crate::physical::Coefficients::from_policy(&self.params.policy);
        let home = crate::physical::Physical::with(&self.players[home_jumper], &coefficients);
        let away = crate::physical::Physical::with(&self.players[away_jumper], &coefficients);
        let home_reach = home.standing_reach_ft + home.vertical_ft;
        let away_reach = away.standing_reach_ft + away.vertical_ft;
        let home_roll = self.draw(Stream::Reaction, home_jumper, purpose::OPENING_TIP, 0);
        let away_roll = self.draw(Stream::Reaction, away_jumper, purpose::OPENING_TIP, 1);
        let (tapper, tapped_team) =
            if home_reach > away_reach || (home_reach == away_reach && home_roll >= away_roll) {
                (home_jumper, TeamSide::Home)
            } else {
                (away_jumper, TeamSide::Away)
            };
        let direction = if tapped_team == TeamSide::Home {
            -Vec2::X
        } else {
            Vec2::X
        };
        let receiver = self.jump_ball_receivers[World::team_index(tapped_team)]
            .expect("the tapped team must have a placed jump-ball receiver");
        self.jump_ball_tapper = Some(tapper);
        self.jump_ball_receiver = Some(receiver);
        let center = self.rules.court.center();
        let target = self.rules.court.clamp_playable(
            center + direction * self.rules.tip_off_tap_distance_ft,
            self.rules.ball_radius_ft,
        );
        let control_reach = self.params.policy.w("loose_ball_catch_base_ft")
            + self.players[receiver].attribute("ball_handling")
                * self.params.policy.w("loose_ball_catch_skill_ft");
        // 停表跳球的接应人可能还在跑位：到位了就直接接，没到位就让球成为
        // 松球，由地板球争抢决定，而不是把他瞬移过去。
        let receiver_in_reach = (target - self.players[receiver].pos).length() <= control_reach;
        if !receiver_in_reach {
            // 接应人还没跑到接球点：这次战球不判归属，改由发球重新开始。
            self.jump_ball_kind = None;
            self.jump_ball_tapper = None;
            self.jump_ball_receiver = None;
            self.jump_ball_receivers = [None; 2];
            self.set_phase(Phase::DeadBall {
                team: tapped_team,
                ready_at: self.now() + 1.0,
            });
            return;
        }
        self.ball.pos = center;
        self.ball.z = self.rules.tip_off_tap_height_ft;
        let target_distance = (target - center).length();
        let vertical_rise = target_distance
            * (self.rules.tip_off_tap_vel_z_ftps / self.rules.tip_off_tap_speed_ftps);
        self.ball.vel = direction * self.rules.tip_off_tap_speed_ftps;
        self.ball.vz = (2.0 * self.rules.ball_gravity_ftps2 * vertical_rise).sqrt();
        self.ball.release(crate::state::BallState::Loose {
            pos: self.ball.pos,
            vel: self.ball.vel,
        });
        self.players[receiver].target = target;
        self.players[receiver].target_kind = crate::state::TargetKind::Window;
        self.players[receiver].target_partner = tapper;
        self.possession.team = tapped_team;
        self.period_started_with = tapped_team;
        self.emit(Fact::BallContact {
            kind: "jump_ball_tap",
            entity: Some(tapper),
            time: self.now(),
            x: center.x,
            y: center.y,
            z: self.ball.z,
            sub_tick: 0,
        });
        self.emit(Fact::JumpBall {
            kind,
            home_jumper,
            away_jumper,
            tapper,
            tapped_team,
        });
        self.set_phase(Phase::JumpBallFlight);
    }

    fn award_opening_tip_control(&mut self, tapped_team: TeamSide, receiver: usize) {
        let kind = self
            .jump_ball_kind
            .take()
            .expect("jump-ball kind must remain available until control is awarded");
        assert_eq!(
            self.players[receiver].team, tapped_team,
            "jump-ball controller must belong to the entitled team"
        );
        self.set_ball_held(receiver);
        self.start_possession(tapped_team, PossessionKind::Live);
        self.set_ball_held(receiver);
        self.players[receiver].facing = if self.attacking_right(tapped_team) {
            Vec2::X
        } else {
            Vec2::NEG_X
        };
        self.players[receiver].target = self.top_of_key(tapped_team);
        let advance_seconds = self.advance_seconds_to_front(receiver, tapped_team);
        let setup_seconds = f64::from(self.params.policy.g("possession_setup_seconds"));
        self.players[receiver].schedule.next_redecide_at =
            self.now() + advance_seconds + setup_seconds;
        self.players[receiver].schedule.pending =
            Some(crate::state::DecisionTrigger::PossessionStart);
        // 球交给接应人：球落到他手上，球员本身不位移。把球员瞬移到球的
        // 位置会让一个 tick 内的位移超过速度上限。
        self.ball.pos = self.players[receiver].pos
            + self.players[receiver].facing.normalize_or_zero() * self.rules.ball_holder_offset_ft;
        self.ball.z = self.rules.ball_holder_height_ft;
        self.ball.vel = self.players[receiver].vel;
        self.ball.vz = 0.0;
        self.emit(Fact::PlacementApplied {
            player: receiver,
            reason: "jump_ball_control",
        });
        if matches!(kind, crate::action::JumpBallKind::Opening) {
            self.first_possession = Some(tapped_team);
            if self.rules.league.use_alternate_possession_arrow {
                self.alternate_possession_next = tapped_team.other();
            }
        }
        self.set_phase(Phase::Live);
        crate::formation::set_targets(self, tapped_team);
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub(crate) fn set_ball_held(&mut self, holder: usize) {
        self.ball.set_held(holder);
        self.last_touch = Some(holder);
    }

    pub(crate) fn release_ball(&mut self, state: crate::state::BallState) {
        self.ball.release(state);
    }

    pub(crate) fn set_timeout_phase(
        &mut self,
        team: TeamSide,
        resume_at: f64,
        resume_phase: crate::state::ResumePhase,
        started_at: f64,
        previous_phase: Phase,
    ) {
        assert_ne!(
            previous_phase.program(),
            crate::state::MatchProgram::Timeout,
            "timeout cannot interrupt another timeout"
        );
        assert_eq!(
            self.phase, previous_phase,
            "timeout source phase changed before commit"
        );
        assert!(
            resume_at >= started_at,
            "timeout end cannot precede its start"
        );
        assert_eq!(
            resume_phase.phase().program(),
            previous_phase.program(),
            "timeout must preserve the interrupted program"
        );
        let phase = Phase::Timeout {
            team,
            started_at,
            resume_at,
            resume_phase,
        };
        assert_eq!(phase.program(), crate::state::MatchProgram::Timeout);
        let stage = phase.stage(self.clock.period(), self.rules.league.regulation_periods);
        assert_eq!(
            stage,
            self.match_stage(),
            "timeout must preserve the current match stage"
        );
        self.cancel_actions("timeout");
        self.phase = phase;
    }

    pub fn set_phase(&mut self, phase: Phase) {
        let phase = match phase {
            Phase::Timeout { .. } => panic!("timeout phases must be created by timeouts::request"),
            phase => phase,
        };
        if matches!(phase, Phase::PeriodBreak { .. }) {
            self.set_period_phase(phase);
            return;
        }
        let stage = phase.stage(self.clock.period(), self.rules.league.regulation_periods);
        let program = phase.program();
        self.set_program_phase(stage, program, phase);
    }

    pub fn match_stage(&self) -> crate::state::MatchStage {
        self.phase()
            .stage(self.clock.period(), self.rules.league.regulation_periods)
    }

    pub fn match_program(&self) -> crate::state::MatchProgram {
        self.phase().program()
    }

    pub fn new<P: Into<RuntimeParams>>(setup: MatchSetup, params: P, seed: u64) -> Self {
        let params = params.into();
        let rules = setup.rules.clone();
        // 读球历史按秒数上限预分配：稳态每帧只做写入与裁剪，不再扩容。
        let history_capacity =
            ((params.policy.g("ball_history_seconds") as f64 / TICK_SECONDS).ceil() as usize) + 64;
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
        crate::formation::assign_slots(&mut players, TeamSide::Home);
        crate::formation::assign_slots(&mut players, TeamSide::Away);
        // 箱体行序由队伍过滤的次序固定，换人不重排，因此下标可一次算定。
        let mut line_slot = vec![0u8; players.len()];
        let player_count = players.len();
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
        let draws = DrawSource::new(seed);
        let (home_jumper, away_jumper) = Self::opening_tip_setup(&players, &params.policy);
        let mut play_cooldowns = std::collections::BTreeMap::new();
        for index in 0..params.tactics.plays.len() {
            play_cooldowns.insert(index, 0.0);
        }
        let center = rules.court.center();
        let mut world = Self {
            params,
            rules,
            clock: Clock::new(period_seconds, shot_clock),
            phase: Phase::Pregame,
            ball: BallBody::loose(center, Vec2::ZERO),
            pass_result: None,
            pass_in_flight: None,
            last_touch: None,
            players,
            possession: PossessionState::new(
                TeamSide::Home,
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
            interactions: BTreeMap::new(),
            screen_defender: None,
            defensive_actions: vec![None; player_count],
            next_action_id: 0,
            next_interaction_id: 0,
            action_turn: None,
            events: Vec::with_capacity(4096),
            draws,
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
            period_started_with: TeamSide::Home,
            alternate_possession_next: TeamSide::Away,
            period_start_inbound_pending: false,
            jump_ball_tapper: None,
            jump_ball_receiver: None,
            jump_ball_receivers: [None; 2],
            jump_ball_kind: None,
            first_possession: None,
            finished: false,
            home_team_name: setup.home.name.clone(),
            away_team_name: setup.away.name.clone(),
            last_pass_kind: None,
            // 回合记录每回合一条；预分配容量，避免在稳态 tick 里扩容。
            possession_log: Vec::with_capacity(256),
            pending_shot: None,
            pending_rebound: None,
            pending_jump_ball: None,
            poke_cooldown: None,
            protection: None,
            possession_points: 0,
            inbound_passer: None,
            inbound_spot: None,
            violations: Vec::new(),
            last_separation: None,
            play_cooldowns,
            alternate_spacing: false,
            shot_clock_at_release: Vec::new(),
            contest_samples: Vec::new(),
            shot_samples: Vec::new(),
            pass_samples: Vec::new(),
            decision_samples: Vec::new(),
            decision_traces: Vec::new(),
            decision_records: Vec::new(),
            active_trace: None,
            next_event_id: 0,
            last_event_tick: 0,
            next_event_sequence: 0,
            shot_zone_counts: [[0; 4]; 2],
            shot_zone_makes: [[0; 4]; 2],
            creation_counts: [0; 7],
            next_shot_id: 0,
            next_pass_id: 0,
            next_foul_id: 0,
            rebound_attempts: 0,
            momentum: [f32::default(); 2],
            ball_history: Vec::with_capacity(history_capacity),
            body_before: vec![Vec2::ZERO; player_count],
            on_court_home: OnCourt::new(),
            on_court_away: OnCourt::new(),
        };
        world.sync_on_court();
        let mut bench_placements = Vec::new();
        let mut bench_counts = [0usize; 2];
        for (handle, player) in world.players.iter().enumerate() {
            if player.on_court {
                continue;
            }
            let team = player.team;
            let index = bench_counts[World::team_index(team)];
            bench_counts[World::team_index(team)] += 1;
            let spot = crate::formation::bench_spot(&world, team, index);
            bench_placements.push((handle, spot));
        }
        for (handle, spot) in bench_placements {
            crate::body::place(&mut world, handle, spot);
        }
        world.reset_positions(TeamSide::Home);
        world.begin_opening_jump_ball(home_jumper, away_jumper);
        world
    }

    pub fn run(&mut self) -> GameReport {
        while !self.finished && self.clock.tick() < MAX_TICKS {
            self.step(TICK_SECONDS);
        }
        self.report()
    }

    /// 逐步推演并逐帧交给调用方，供流式导出与实时观察使用。
    pub fn run_with<F: FnMut(&World)>(&mut self, mut sink: F) -> GameReport {
        while !self.finished && self.clock.tick() < MAX_TICKS {
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

    pub fn new_foul_id(&mut self) -> u64 {
        self.next_foul_id += 1;
        self.next_foul_id
    }

    pub fn start_action(
        &mut self,
        intent: ActionIntent,
        participants: &[(usize, crate::action::ActionIntentRole)],
    ) -> bool {
        assert!(participants.iter().any(|(player, role)| {
            *player == intent.actor && *role == crate::action::ActionIntentRole::Primary
        }));
        assert!(participants
            .iter()
            .all(|(player, _)| *player < self.players.len()));
        assert!(participants
            .iter()
            .all(|(player, _)| self.players[*player].on_court));
        assert!(
            intent
                .target
                .is_none_or(|target| target < self.players.len()),
            "action target must reference a player"
        );
        let conflict = participants
            .iter()
            .any(|(player, _)| self.players[*player].action.is_some());
        if conflict {
            self.emit(Fact::ActionIntentRejected {
                player: intent.actor,
                kind: intent.kind.as_str(),
                reason: "participant_busy",
            });
            return false;
        }
        // 护球是一种姿态，不是持续的屏障：开始其他动作就退出护球。
        if !matches!(intent.kind, crate::action::ActionKind::Protect(_)) {
            self.protection = None;
        }
        self.next_interaction_id += 1;
        let interaction_id = self.next_interaction_id;
        let (preparation, execution, follow_through) = self.action_windows(&intent);
        let configured_total = preparation + execution + follow_through;
        assert!(configured_total.is_finite() && configured_total > 0.0);
        assert!(
            intent.start_time.is_finite() && (intent.start_time - self.now()).abs() <= f64::EPSILON
        );
        assert!(intent.duration.is_finite() && intent.duration > 0.0);
        let scale = intent.duration / configured_total;
        let preparation = preparation * scale;
        let execution = execution * scale;
        let follow_through = follow_through * scale;
        let phase = if preparation > 0.0 {
            nba_domain::action_window::ActionPhase::Preparation
        } else if execution > 0.0 {
            nba_domain::action_window::ActionPhase::Execution
        } else {
            nba_domain::action_window::ActionPhase::FollowThrough
        };
        let mut ordered = participants.to_vec();
        ordered.sort_unstable_by_key(|(player, _)| *player);
        assert!(ordered.windows(2).all(|pair| pair[0].0 != pair[1].0));
        let action_id = self.activate_action_participants(
            intent.clone(),
            &ordered,
            interaction_id,
            preparation,
            execution,
            follow_through,
            phase,
        );
        self.interactions.insert(
            interaction_id,
            Interaction {
                id: interaction_id,
                action_id,
                intent,
                screen_defender: self.screen_defender,
            },
        );
        self.action_turn = None;
        true
    }

    fn action_windows(&self, intent: &ActionIntent) -> (f64, f64, f64) {
        use crate::action::{ActionKind, ShotKind};
        let rules = &self.rules;
        let (preparation, execution, follow_through) = match intent.kind {
            ActionKind::Shoot(
                ShotKind::Dunk
                | ShotKind::DrivingDunk
                | ShotKind::CuttingDunk
                | ShotKind::RunningDunk,
            ) => (
                rules.dunk_prep_seconds,
                rules.dunk_exec_seconds,
                rules.dunk_follow_seconds,
            ),
            ActionKind::Shoot(
                ShotKind::Layup
                | ShotKind::DrivingLayup
                | ShotKind::CuttingLayup
                | ShotKind::RunningLayup
                | ShotKind::ReverseLayup
                | ShotKind::Floater
                | ShotKind::AlleyOop
                | ShotKind::Putback,
            )
            | ActionKind::Finish => (
                rules.layup_prep_seconds,
                rules.layup_exec_seconds,
                rules.layup_follow_seconds,
            ),
            ActionKind::Shoot(_) => (
                rules.jump_shot_prep_seconds,
                rules.jump_shot_exec_seconds,
                rules.jump_shot_follow_seconds,
            ),
            ActionKind::Pass(_) => (
                rules.pass_prep_seconds,
                rules.pass_exec_seconds,
                rules.pass_follow_seconds,
            ),
            ActionKind::Screen(_) => (
                rules.screen_prep_seconds,
                rules.screen_exec_seconds,
                rules.screen_follow_seconds,
            ),
            ActionKind::Drive
            | ActionKind::Dribble(_)
            | ActionKind::Cut(_)
            | ActionKind::PostUp
            | ActionKind::Relocate => (
                rules.tactics.drive_min_duration_seconds,
                rules.tactics.drive_max_duration_seconds,
                rules.tactical_initiation_seconds,
            ),
            // 假动作与护球都是短技术动作：准备、执行、回收各占一份。
            ActionKind::Fake(_) => (
                rules.tactics.drive_min_duration_seconds,
                rules.tactics.drive_min_duration_seconds,
                rules.tactical_initiation_seconds,
            ),
            ActionKind::Protect(_) => (
                rules.tactics.drive_min_duration_seconds,
                rules.tactics.drive_min_duration_seconds,
                rules.tactical_initiation_seconds,
            ),
        };
        (
            f64::from(preparation),
            f64::from(execution),
            f64::from(follow_through),
        )
    }

    fn activate_action_participants(
        &mut self,
        intent: ActionIntent,
        participants: &[(usize, crate::action::ActionIntentRole)],
        interaction_id: u64,
        preparation: f64,
        execution: f64,
        follow_through: f64,
        phase: nba_domain::action_window::ActionPhase,
    ) -> u64 {
        self.next_action_id += 1;
        let action_id = self.next_action_id;
        let kind = intent.kind.as_str();
        for &(player, role) in participants {
            let participant_intent = intent.clone();
            let runtime = ActionRuntime::new(
                action_id,
                interaction_id,
                participant_intent,
                player,
                role,
                phase,
                preparation,
                execution,
                follow_through,
            );
            self.players[player].action = Some(runtime);
            self.emit(Fact::ActionStarted {
                action_id,
                interaction_id,
                player,
                kind,
                target: intent.target,
            });
        }
        action_id
    }

    pub(crate) fn cancel_actions(&mut self, reason: &'static str) {
        let resolving_action = self
            .action_turn
            .filter(|(_, turn)| *turn == crate::action::ActionTurn::Resolve)
            .map(|(action_id, _)| action_id);
        for player in 0..self.players.len() {
            let Some(runtime) = self.players[player].action.as_ref() else {
                continue;
            };
            if Some(runtime.action_id) == resolving_action {
                continue;
            }
            let runtime = self.players[player].action.take().unwrap();
            if self.players[player].target_kind == crate::state::TargetKind::Contact {
                self.players[player].target_kind = crate::state::TargetKind::Station;
                self.players[player].target_partner = usize::MAX;
            }
            self.emit(Fact::ActionCancelled {
                action_id: runtime.action_id,
                player,
                kind: runtime.intent.kind.as_str(),
                reason,
            });
        }
        self.cancel_defensive_actions(reason);
        let alive: Vec<u64> = self
            .players
            .iter()
            .filter_map(|player| player.action.as_ref().map(|runtime| runtime.action_id))
            .collect();
        self.interactions
            .retain(|_, interaction| alive.contains(&interaction.action_id));
        self.screen_defender = None;
    }

    fn cancel_defensive_actions(&mut self, reason: &'static str) {
        for defender in 0..self.defensive_actions.len() {
            self.cancel_defensive_action(defender, reason);
        }
    }

    /// 只取消指定种类的防守动作：并发动作结算时不能互相清掉。
    fn cancel_defensive_actions_of(&mut self, kind: DefensiveActionKind, reason: &'static str) {
        for defender in 0..self.defensive_actions.len() {
            if self.defensive_actions[defender]
                .as_ref()
                .is_some_and(|action| action.kind == kind)
            {
                self.cancel_defensive_action(defender, reason);
            }
        }
    }

    fn cancel_defensive_action(&mut self, defender: usize, reason: &'static str) {
        let Some(runtime) = self.defensive_actions[defender].take() else {
            return;
        };
        self.emit(Fact::DefensiveActionCancelled {
            action_id: runtime.action_id,
            defender: runtime.defender,
            shooter: runtime.shooter,
            kind: runtime.kind.as_str(),
            reason,
        });
    }

    /// 取消指定球员的当前动作；没有动作时什么也不做。
    pub(crate) fn cancel_player_action(&mut self, player: usize, reason: &'static str) {
        let Some(runtime) = self.players[player].action.take() else {
            return;
        };
        if self.players[player].target_kind == crate::state::TargetKind::Contact {
            self.players[player].target_kind = crate::state::TargetKind::Station;
            self.players[player].target_partner = usize::MAX;
        }
        self.emit(Fact::ActionCancelled {
            action_id: runtime.action_id,
            player,
            kind: runtime.intent.kind.as_str(),
            reason,
        });
        let alive: Vec<u64> = self
            .players
            .iter()
            .filter_map(|player| player.action.as_ref().map(|runtime| runtime.action_id))
            .collect();
        self.interactions
            .retain(|_, interaction| alive.contains(&interaction.action_id));
    }

    /// 流尾部尚未结算的业务实例：飞行中的出手、飞行中的传球、未执行完的罚球。
    pub fn open_business_instances(&self) -> u64 {
        let shot = u64::from(self.pending_shot.is_some());
        let pass = u64::from(self.pass_in_flight.is_some());
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
            periods: self.clock.period(),
            regulation_periods: self.rules.league.regulation_periods,
            period_duration_seconds: self.rules.league.period_duration_seconds,
            overtime_duration_seconds: self.rules.league.overtime_duration_seconds,
            tick_seconds: self.rules.tick_seconds,
            ticks: self.clock.tick(),
            redecide_interval_seconds: self.rules.redecide_interval_seconds as f64,
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
            decision_records: std::mem::take(&mut self.decision_records),
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
        self.rules.court.hoop_pos(self.attacking_right(team))
    }

    pub fn attacking_right(&self, team: TeamSide) -> bool {
        self.attacking_right_in_period(team, self.clock.period())
    }

    fn attacking_right_in_period(&self, team: TeamSide, period: u32) -> bool {
        let home_attacks_right = !period.is_multiple_of(2);
        (team == TeamSide::Home) == home_attacks_right
    }

    pub fn apply_defensive_closeout_target(&mut self, defender: usize) -> bool {
        let Some(runtime) = self.defensive_actions[defender].as_ref() else {
            return false;
        };
        if runtime.phase != nba_domain::action_window::ActionPhase::Execution {
            return false;
        }
        let shooter = runtime.shooter;
        if !self.players[defender].on_court || !self.players[shooter].on_court {
            return false;
        }
        let runtime = self.defensive_actions[defender]
            .as_ref()
            .expect("active defensive action");
        let target = runtime.target_position;
        crate::assignment::force_target_as(
            self,
            defender,
            target,
            crate::state::TargetKind::Window,
            usize::MAX,
        )
    }

    /// 各类累积缓冲的长度与容量：分配预算是「稳态 tick 不增长」，
    /// 判据要能看见所有会增长的缓冲。
    pub fn buffer_footprint(&self) -> [usize; 10] {
        [
            self.events.capacity(),
            self.decision_traces.capacity(),
            self.decision_samples.capacity(),
            self.decision_records.capacity(),
            self.shot_samples.capacity() + self.shot_samples.len(),
            self.pass_samples.len(),
            self.possession_log.len(),
            self.violations.len(),
            self.shot_clock_at_release.len() + self.contest_samples.len(),
            self.ball_history.len(),
        ]
    }

    pub fn holder(&self) -> Option<usize> {
        self.ball.holder()
    }

    /// 持球人是否被包夹：两名以上对手挤在包夹半径内。
    pub fn trapped(&self, holder: usize) -> bool {        let radius = self.params.policy.g("trap_radius_ft");
        let team = self.players[holder].team;
        let pos = self.players[holder].pos;
        let mut count = 0u8;
        for player in &self.players {
            if player.on_court && player.team != team && (player.pos - pos).length() <= radius {
                count += 1;
                if count >= 2 {
                    return true;
                }
            }
        }
        false
    }

    /// 护球姿态是否正在生效。
    pub fn protected(&self, holder: usize, now: f64) -> bool {
        self.protection
            .is_some_and(|protection| protection.holder == holder && protection.until > now)
    }

    /// 该球员正在执行的切入路线类型；没有切入动作时按直切处理。
    pub fn cut_kind_name(&self, cutter: usize) -> &'static str {
        match self.players[cutter].action.as_ref().map(|action| action.intent.kind) {
            Some(ActionKind::Cut(kind)) => kind.as_str(),
            _ => crate::action::CutKind::Basket.as_str(),
        }
    }

    pub fn now(&self) -> f64 {
        self.clock.simulation_time()
    }

    /// 按地址抽样。`entity` 是球员句柄或球队下标；`ordinal` 是语义序号。
    pub fn draw(&self, stream: Stream, entity: usize, purpose: u16, ordinal: u16) -> f32 {
        self.draws.f32(DrawKey {
            stream,
            possession: self.possession.id,
            tick: self.clock.tick(),
            entity: entity as u32,
            purpose,
            ordinal,
        })
    }

    /// 克隆重放：保留世界状态与随机源地址，清空审计日志。
    /// 反事实推演用不同的 `base_seed`，同地址在各分支取同一个值。
    pub fn fork(&self, base_seed: u64) -> Self {
        let mut fork = self.clone();
        fork.draws = DrawSource::new(base_seed);
        fork.events.clear();
        fork.next_event_id = 0;
        fork.last_event_tick = fork.clock.tick();
        fork.next_event_sequence = 0;
        fork.shot_samples.clear();
        fork.pass_samples.clear();
        fork.decision_samples.clear();
        fork.decision_traces.clear();
        fork.decision_records.clear();
        fork.possession_log.clear();
        fork.active_trace = None;
        fork.finished = false;
        fork
    }

    pub fn delayed_ball_position(&self, delay: f64) -> Vec2 {
        crate::motion::delayed_ball_position(self, delay)
    }

    pub fn emit(&mut self, fact: Fact) {
        let tick_index = self.clock.tick();
        if tick_index != self.last_event_tick {
            self.last_event_tick = tick_index;
            self.next_event_sequence = 0;
        }
        self.next_event_id += 1;
        let sequence = self.next_event_sequence;
        self.next_event_sequence += 1;
        self.events.push(TimedFact {
            time: self.now(),
            period: self.clock.period(),
            tick_index,
            sequence,
            event_id: self.next_event_id,
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
        self.scoring_run = if self.last_scoring == Some(Self::team_index(team)) {
            self.scoring_run.saturating_add(points)
        } else {
            points
        };
        self.last_scoring = Some(Self::team_index(team));
        let index = Self::team_index(team);
        self.score[index] += points;
        self.possession_points += points;
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

    pub fn sync_on_court(&mut self) {
        let mut home = OnCourt::new();
        let mut away = OnCourt::new();
        for (handle, player) in self.players.iter().enumerate() {
            if player.on_court {
                match player.team {
                    TeamSide::Home => home.push(handle),
                    TeamSide::Away => away.push(handle),
                }
            }
        }
        self.on_court_home = home;
        self.on_court_away = away;
    }

    pub fn on_court(&self, team: TeamSide) -> OnCourt {
        match team {
            TeamSide::Home => self.on_court_home,
            TeamSide::Away => self.on_court_away,
        }
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

    fn top_of_key(&self, team: TeamSide) -> Vec2 {
        crate::formation::top_of_key(self, team)
    }

    pub fn reset_positions(&mut self, offense: TeamSide) {
        crate::formation::place_opening_positions(self, offense);
    }

    fn begin_inbound(&mut self, team: TeamSide) {
        let width = self.rules.court.width_ft;
        let height = self.rules.court.height_ft;
        // 发球点在本方底线：进攻方向是右就在左底线，反之在右底线。
        let baseline_x = if self.attacking_right(team) {
            self.rules.ball_radius_ft
        } else {
            width - self.rules.ball_radius_ft
        };
        // 发球地点优先用停表时记录的位置：出界在边线上发，违例在断点发；
        // 没有记录时（命中、节次开局）回到本方底线。
        let inbound_spot = self.inbound_spot.take().unwrap_or_else(|| {
            glam::Vec2::new(baseline_x, height * 0.5)
        });
        // 发球由离发球点最近的队员执行：他就在本方底线附近，不用瞬移，也
        // 不用把进攻时钟耗在走位上。
        let handler = self
            .on_court(team)
            .into_iter()
            .min_by(|a, b| {
                let distance = |handle: usize| (self.players[handle].pos - inbound_spot).length();
                distance(*a).total_cmp(&distance(*b))
            })
            .expect("every team must have an on-court player");
        self.inbound_passer = Some(handler);
        self.cancel_actions("inbound");
        self.set_ball_held(handler);
        crate::formation::set_targets(self, team);
        // 发球队员必须自己走到发球点，不能被瞬移过去：瞬移会让球员在
        // 回合之间跳几十英尺，看起来像系统重排而不是真实移动。
        self.players[handler].target = inbound_spot;
        self.players[handler].target_kind = crate::state::TargetKind::Station;
        self.players[handler].target_partner = usize::MAX;
        self.players[handler].target_changed_at = self.now();
        self.emit(crate::action::Fact::PlacementApplied {
            player: handler,
            reason: "inbound",
        });

        // 发球时间要足够发球队员走到发球点，但不能把进攻时钟耗在走位上。
        let walk = (self.players[handler].pos - inbound_spot).length()
            / self.rules.max_player_speed_ftps.max(f32::MIN_POSITIVE);
        let ready_at = self.now() + 0.5 + f64::from(walk.clamp(0.0, 1.5));
        self.set_phase(Phase::Inbound { team, ready_at });
    }

    pub fn start_possession(&mut self, team: TeamSide, kind: PossessionKind) {
        let inbound_passer = self.inbound_passer;
        self.inbound_passer = None;
        let continuing_possession = kind == PossessionKind::Dead && self.possession.id != 0;
        if self.period_start_inbound_pending {
            assert_eq!(
                team, self.period_started_with,
                "period-start throw-in must preserve awarded team"
            );
            self.alternate_possession_next = team.other();
            self.period_start_inbound_pending = false;
        }
        crate::tactics::close_play_window(self);
        self.active_trace = None;
        self.cancel_actions("possession_start");
        if continuing_possession {
            assert_eq!(
                self.possession.team, team,
                "dead-ball restart must preserve possession team"
            );
        }
        crate::motion::reset_ball_history(self);
        self.possession.off_ball_actions = 0;
        let next_possession_id = if continuing_possession {
            self.possession.id
        } else {
            assert_eq!(
                self.possession.id, 0,
                "a new possession must close the previous possession with its typed cause"
            );
            self.possession_count + 1
        };
        self.alternate_spacing = next_possession_id.is_multiple_of(2);
        let full_shot_clock = self.rules.league.shot_clock_seconds as f64;
        let shot_clock = if continuing_possession {
            self.clock.shot_clock()
        } else {
            full_shot_clock
        };
        if continuing_possession {
            self.clock.set_shot_clock(shot_clock);
        }
        if !continuing_possession {
            self.clock.set_shot_clock(shot_clock);
            self.possession_count = next_possession_id;
            assert_eq!(
                self.possession.id, 0,
                "a new possession must close the previous possession with its typed cause"
            );
            self.possession = PossessionState::new(
                team,
                self.possession_count,
                kind,
                self.now(),
                shot_clock,
                self.clock.game_clock(),
            );
            self.possession_points = 0;
        }
        // 发球也是一次传球：接球人接到发球后直接出手是合法的进攻选择，
        // 不应该被判成「没有接球就投」。传球链统计也按官方口径把发球
        // 计入链条。回合开始或重启时持球人处于运球推进状态。
        self.possession.passes += 1;
        self.possession.ball_phase = crate::state::BallPhase::Dribble;
        self.possession.last_pass_at = Some(self.now());
        self.possession.last_pass_from = inbound_passer;
        self.possession.assist_candidate = inbound_passer.map(|handler| (handler, self.now()));
        self.pending_shot = None;
        self.pending_rebound = None;
        self.pass_result = None;
        self.pass_in_flight = None;
        crate::formation::set_targets(self, team);
        // 死球重启是唯一合法的换人窗口：防守篮板、抢断、快速反击这些
        // 活球回合开始的进攻不允许在场上换人，现实里教练也换不了。
        if kind == PossessionKind::Dead {
            self.apply_dead_ball_window();
        }
        let handler = self.primary_handler(team);
        self.set_ball_held(handler);
        self.set_phase(Phase::Live);
        if kind == PossessionKind::Live {
            self.begin_live_push(handler);
        } else {
            self.players[handler].target = self.top_of_key(team);
            self.players[handler].target_kind = crate::state::TargetKind::Station;
            self.players[handler].target_partner = usize::MAX;
            let advance_seconds = self.advance_seconds_to_front(handler, team);
            let setup_seconds = f64::from(self.params.policy.g("possession_setup_seconds"));
            self.players[handler].schedule = crate::state::DecisionSchedule {
                next_redecide_at: self.now() + advance_seconds + setup_seconds,
                pending: Some(crate::state::DecisionTrigger::PossessionStart),
            };
        }
        if !continuing_possession {
            self.emit(Fact::PossessionStarted {
                team,
                live: kind == PossessionKind::Live,
                possession_id: self.possession.id,
            });
        }
    }

    /// 持球人从当前位置推进到本方前场弧顶所需的秒数。
    ///
    /// 时长由移动速度决定：速度快则更快进入首个决策，在固定比赛时钟内产生
    /// 更多回合。总时长 = 匀速段 + 起步加速段，起步段随距离衰减。
    fn advance_seconds_to_front(&self, handler: usize, team: TeamSide) -> f64 {
        let advance_distance = (self.top_of_key(team) - self.players[handler].pos).length();
        let speed = self.players[handler]
            .effective_speed(&self.params.policy)
            .max(1.0);
        let accel = self.players[handler]
            .effective_acceleration(&self.params.policy)
            .max(1.0);
        let ramp_scale = self
            .params
            .policy
            .g("advance_ramp_distance_ft")
            .max(f32::EPSILON);
        let startup = (speed / accel) * (-advance_distance / ramp_scale).exp();
        (advance_distance / speed + startup).max(0.8) as f64
    }

    /// 活球回合的第一段：持球人自己把球推进到前场，快攻球员直奔篮下。
    ///
    /// 抢到防守篮板或抢断之后立刻决策，持球人还在本方半场，传球目标都落在
    /// 身后，结果只能是横向倒球，快攻和空切都做不出来。先推进到前场再读场面，
    /// 首个决策落在 `transition_window_seconds` 的一半处，剩下的时间留给终结。
    pub fn begin_live_push(&mut self, handler: usize) {
        let team = self.players[handler].team;
        if let Some(index) = self
            .possession
            .break_return
            .iter()
            .position(|(runner, _)| *runner == handler)
        {
            let (_, spot) = self.possession.break_return.remove(index);
            self.players[handler].home_spot = spot;
        }
        if self.possession.break_return.is_empty() {
            crate::formation::assign_break_targets(self, team, handler);
        }
        self.players[handler].target = self.top_of_key(team);
        self.players[handler].target_kind = crate::state::TargetKind::Station;
        self.players[handler].target_partner = usize::MAX;
        let advance_seconds = self.advance_seconds_to_front(handler, team);
        self.players[handler].schedule = crate::state::DecisionSchedule {
            next_redecide_at: self.now() + advance_seconds,
            pending: None,
        };
    }

    pub fn start_second_chance(&mut self, team: TeamSide, rebounder: usize, rim_contact: bool) {
        assert_eq!(
            self.possession.team, team,
            "an offensive rebound must keep the offense"
        );
        assert_ne!(
            self.possession.id,
            0,
            "an offensive rebound requires an active possession (last ended by {:?})",
            self.possession_log.last().map(|record| record.ended_by)
        );
        crate::tactics::close_play_window(self);
        self.active_trace = None;
        // 前场篮板只有球触筐才重置进攻时钟，且只补不足、不削减剩余时间。
        let shot_clock = self
            .rules
            .league
            .shot_clock_after_offensive_rebound(self.clock.shot_clock(), rim_contact);
        self.clock.set_shot_clock(shot_clock);
        self.possession.kind = PossessionKind::Live;
        self.possession.last_offensive_rebound_at = Some(self.now());
        self.possession.last_offensive_rebound_actor = Some(rebounder);
        self.possession.had_offensive_rebound = true;
        self.possession.ball_phase = crate::state::BallPhase::Putback;
        self.pending_shot = None;
        self.pending_rebound = None;
        self.pass_result = None;
        self.pass_in_flight = None;
        crate::formation::set_targets(self, team);
        self.players[rebounder].target = self.players[rebounder].pos;
        self.players[rebounder].target_kind = crate::state::TargetKind::Station;
        self.players[rebounder].target_partner = usize::MAX;
        self.set_ball_held(rebounder);
        self.set_phase(Phase::Live);
        self.cancel_actions("second_chance_start");
        self.players[rebounder].schedule = crate::state::DecisionSchedule {
            next_redecide_at: self.now() + 0.3,
            pending: Some(crate::state::DecisionTrigger::PossessionStart),
        };
        self.emit(Fact::SecondChanceStarted {
            rebounder,
            possession: self.possession.id,
        });
    }

    pub fn end_possession(
        &mut self,
        end_reason: PossessionEndCause,
        termination_detail: &'static str,
        points: u32,
    ) {
        if self.possession.id == 0 {
            return;
        }
        self.protection = None;
        crate::tactics::close_play_window(self);
        let record = PossessionRecord {
            team: self.possession.team,
            id: self.possession.id,
            kind: self.possession.kind,
            start_clock: self.possession.shot_clock_start,
            end_clock: self.clock.shot_clock(),
            duration: self.now() - self.possession.started_at,
            start_game_clock: self.possession.start_game_clock,
            end_game_clock: self.clock.game_clock(),
            passes: self.possession.passes,
            screens: self.possession.screens,
            drives: self.possession.drives,
            points,
            ended_by: termination_detail,
            first_action_seconds: self.possession.first_action_seconds,
        };
        self.emit(Fact::PossessionEnded {
            team: record.team,
            possession_id: record.id,
            points,
            end_reason,
            termination_detail,
            start_game_clock: record.start_game_clock,
            end_game_clock: record.end_game_clock,
            start_shot_clock: record.start_clock,
            end_shot_clock: record.end_clock,
            duration: record.duration,
            passes: record.passes,
            screens: record.screens,
            drives: record.drives,
            possession_kind: record.kind,
            had_shot: self.possession.had_shot,
            had_offensive_rebound: self.possession.had_offensive_rebound,
            first_action_seconds: record.first_action_seconds,
        });
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
        self.clock.advance_tick();
        let phase_before_step = self.phase;
        let momentum_phase = if matches!(phase_before_step, Phase::Timeout { .. }) {
            match phase_before_step {
                Phase::Timeout { resume_phase, .. } => resume_phase.phase(),
                _ => unreachable!(),
            }
        } else {
            phase_before_step
        };
        let game_clock_before_advance = self.clock.game_clock();
        let clock_advance = crate::clock::advance(phase_before_step, &mut self.clock, dt);
        let player_time_seconds = if clock_advance.player_time {
            dt.min(game_clock_before_advance)
        } else {
            0.0
        };
        if !matches!(
            momentum_phase,
            Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
        ) {
            crate::motion::decay_momentum(self, dt);
        }
        if player_time_seconds > 0.0 {
            for &handle in self.on_court_home.as_slice() {
                let slot = self.line_slot[handle] as usize;
                self.box_score.home[slot].seconds += player_time_seconds;
            }
            for &handle in self.on_court_away.as_slice() {
                let slot = self.line_slot[handle] as usize;
                self.box_score.away[slot].seconds += player_time_seconds;
            }
        }
        if player_time_seconds > 0.0 {
            self.update_stamina(player_time_seconds);
        }
        crate::rotation::note_intents(self);
        self.advance_action_phases();
        self.advance_defensive_actions();
        self.advance_steal_attempts();
        self.advance_block_jumps();
        if matches!(phase_before_step, Phase::JumpBallFlight) {
            self.move_jump_ball_receiver(dt);
        } else if !matches!(phase_before_step, Phase::JumpBall { .. }) {
            self.advance_movement(dt);
        }
        crate::resolve::poke::step(self, dt);
        crate::resolve::offball::step(self, dt);
        crate::ball::step(self, dt);
        self.resolve_due_interactions();
        self.advance_phase();
        self.expire_break_targets();
        if matches!(self.phase, Phase::Live) && self.holder().is_some() {
            self.maybe_decide();
        }
        self.check_closely_guarded(dt);
        self.check_backcourt();
        self.check_period_end();
    }

    /// 紧逼盯防五秒：收球后被贴身盯住且不再推进，五秒必吹违例。
    fn check_closely_guarded(&mut self, dt: f64) {
        if !self.rules.league.closely_guarded_seconds.is_finite()
            || !matches!(self.phase, Phase::Live)
        {
            self.possession.closely_guarded_seconds = 0.0;
            return;
        }
        let Some(holder) = self.holder() else {
            self.possession.closely_guarded_seconds = 0.0;
            return;
        };
        // 只有收球后算持球：还在运球时不受这条规则限制。
        if self.possession.dribble_state != crate::state::DribbleState::Gathered {
            self.possession.closely_guarded_seconds = 0.0;
            return;
        }
        let limit = self.rules.league.closely_guarded_distance_ft;
        let team = self.players[holder].team;
        let guarded = self.players.iter().enumerate().any(|(index, player)| {
            index != holder
                && player.on_court
                && player.team != team
                && (player.pos - self.players[holder].pos).length() <= limit
        });
        if !guarded {
            self.possession.closely_guarded_seconds = 0.0;
            return;
        }
        self.possession.closely_guarded_seconds += dt as f32;
        if self.possession.closely_guarded_seconds + f32::EPSILON
            < self.rules.league.closely_guarded_seconds
        {
            return;
        }
        self.possession.closely_guarded_seconds = 0.0;
        self.line_mut(holder).turnovers += 1;
        self.emit(Fact::Turnover {
            player: holder,
            kind: TurnoverKind::CloselyGuarded,
        });
        self.end_possession(PossessionEndCause::TurnoverViolation, "closely_guarded", 0);
        self.start_possession(team.other(), PossessionKind::Live);
    }

    /// 后场八秒：控球方在后场停留超过规则秒数即违例。
    fn check_backcourt(&mut self) {
        if !matches!(self.phase, Phase::Live) {
            self.possession.backcourt_deadline = None;
            return;
        }
        let Some(holder) = self.holder() else {
            return;
        };
        let team = self.possession.team;
        let attacking_right = self.attacking_right(team);
        let mid = self.rules.court.width_ft * 0.5;
        let pos = self.players[holder].pos;
        // 中线附近有一段混沌带：只有真正推进到前场深处才算建立前场控制，
        // 只有真正退回后场才算回场，避免持球人在中线上来回晃动被吹。
        let advance = self.params.policy.g("backcourt_advance_ft");
        let margin = self.params.policy.g("backcourt_margin_ft");
        let frontcourt_deep = if attacking_right {
            pos.x >= mid + advance
        } else {
            pos.x <= mid - advance
        };
        let past_mid = if attacking_right {
            pos.x > mid
        } else {
            pos.x < mid
        };
        let in_backcourt = if attacking_right {
            pos.x <= mid - margin
        } else {
            pos.x >= mid + margin
        };
        // 防守人碰过球后球回到后场是合法的：那段控制不算进攻方建立的前场控制。
        if self
            .last_touch
            .is_some_and(|touched| self.players[touched].team != team)
        {
            self.possession.frontcourt_established = false;
        }
        if past_mid {
            // 过中线就停八秒计时；只有推进到前场深处才算建立前场控制。
            self.possession.backcourt_deadline = None;
        }
        if frontcourt_deep {
            self.possession.frontcourt_established = true;
        }
        if !in_backcourt {
            return;
        }
        // 已经在前场建立控制又把球带回后场：回场违例。
        if self.possession.frontcourt_established {
            self.possession.frontcourt_established = false;
            self.possession.backcourt_deadline = None;
            self.line_mut(holder).turnovers += 1;
            self.emit(Fact::Turnover {
                player: holder,
                kind: TurnoverKind::Backcourt,
            });
            self.end_possession(PossessionEndCause::TurnoverViolation, "backcourt", 0);
            self.cancel_actions("backcourt");
            self.set_phase(Phase::DeadBall {
                team: team.other(),
                ready_at: self.now() + f64::from(self.rules.inbound_setup_seconds),
            });
            return;
        }
        let now = self.now();
        let limit = f64::from(self.rules.backcourt_seconds);
        match self.possession.backcourt_deadline {
            None => self.possession.backcourt_deadline = Some(now + limit),
            Some(deadline) if now >= deadline => {
                self.possession.backcourt_deadline = None;
                self.line_mut(holder).turnovers += 1;
                self.emit(Fact::Turnover {
                    player: holder,
                    kind: TurnoverKind::EightSecond,
                });
                self.end_possession(PossessionEndCause::TurnoverViolation, "eight_second", 0);
                self.cancel_actions("eight_second");
                self.set_phase(Phase::DeadBall {
                    team: team.other(),
                    ready_at: self.now() + f64::from(self.rules.inbound_setup_seconds),
                });
            }
            Some(_) => {}
        }
    }

    /// 快攻窗口关闭：跑向前场的球员回到各自的阵型站位。
    ///
    /// 只有被快攻跑位改过目标的球员会被改回去，持球人和其他球员的当前
    /// 目标不受影响。
    fn expire_break_targets(&mut self) {
        if self.possession.break_return.is_empty() {
            return;
        }
        let window = f64::from(self.params.policy.g("transition_window_seconds"));
        if self.now() < self.possession.started_at + window {
            return;
        }
        let returns = std::mem::take(&mut self.possession.break_return);
        for (handle, spot) in returns {
            crate::assignment::force_target(self, handle, spot);
            self.players[handle].home_spot = spot;
        }
    }

    fn update_stamina(&mut self, dt: f64) {
        crate::rotation::update_stamina(self, dt);
    }

    fn advance_movement(&mut self, dt: f64) {
        crate::motion::advance_movement(self, dt);
    }

    fn move_jump_ball_receiver(&mut self, dt: f64) {
        let Some(receiver) = self.jump_ball_receiver else {
            return;
        };
        for player in self.players.iter_mut().filter(|player| player.on_court) {
            player.target = player.pos;
            player.target_kind = crate::state::TargetKind::Station;
            player.target_partner = usize::MAX;
        }
        self.players[receiver].target_kind = crate::state::TargetKind::Window;
        self.players[receiver].target_partner = receiver;
        crate::body::integrate(self, dt);
    }

    fn advance_action_phases(&mut self) {
        for player in 0..self.players.len() {
            let Some(mut runtime) = self.players[player].action.take() else {
                continue;
            };
            if let Some(to) = runtime.phase_at(self.now()) {
                let from = runtime.phase;
                runtime.phase = to;
                self.players[player].action = Some(runtime.clone());
                self.emit(Fact::ActionPhaseChanged {
                    action_id: runtime.action_id,
                    player,
                    from,
                    to,
                });
            } else {
                self.players[player].action = Some(runtime);
            }
        }
    }

    fn advance_defensive_actions(&mut self) {
        if !matches!(self.phase(), Phase::Live | Phase::ShotFlight { .. }) {
            self.cancel_defensive_actions_of(
                DefensiveActionKind::CloseoutContest,
                "shot_action_ended",
            );
            return;
        }
        let shooting_player = self.players.iter().position(|player| {
            player.on_court
                && player.action.as_ref().is_some_and(|action| {
                    matches!(action.intent.kind, crate::action::ActionKind::Shoot(_))
                })
        });
        // 出手的那一刻投篮动作结束、球还在飞。封盖动作要继续举着手飞完
        // 整段轨迹：在出手的同一个 tick 把动作取消，等于防守人从未起跳，
        // 封盖只剩球擦过站立摸高的零星几次。
        let (defender, shooter) = if let Some(shooter) = shooting_player {
            let defense = self.players[shooter].team.other();
            let defenders = self.on_court(defense);
            let Some(defender) = defenders.as_slice().iter().copied().find(|handle| {
                self.players[*handle].defense_responsibility
                    == crate::state::DefenseResponsibility::OnBall
            }) else {
                self.cancel_defensive_actions_of(
                    DefensiveActionKind::CloseoutContest,
                    "on_ball_defender_unavailable",
                );
                return;
            };
            (defender, shooter)
        } else {
            self.cancel_defensive_actions_of(
                DefensiveActionKind::CloseoutContest,
                "shot_action_ended",
            );
            return;
        };
        for player in 0..self.defensive_actions.len() {
            if self.defensive_actions[player]
                .as_ref()
                .is_some_and(|action| {
                    action.kind == DefensiveActionKind::CloseoutContest
                        && (action.shooter != shooter || action.defender != defender)
                })
            {
                self.cancel_defensive_action(player, "matchup_changed");
            }
        }
        if self.defensive_actions[defender].is_none() {
            self.start_defensive_closeout(defender, shooter);
        }
        let now = self.now();
        let Some(mut runtime) = self.defensive_actions[defender].take() else {
            return;
        };
        if runtime.completed {
            self.defensive_actions[defender] = Some(runtime);
            return;
        }
        if !self.players[defender].on_court || !self.players[shooter].on_court {
            self.emit(Fact::DefensiveActionCancelled {
                action_id: runtime.action_id,
                defender,
                shooter,
                kind: runtime.kind.as_str(),
                reason: "participant_left_court",
            });
            return;
        }
        if let Some(to) = runtime.phase_at(now) {
            let from = runtime.phase;
            runtime.phase = to;
            self.emit(Fact::DefensiveActionPhaseChanged {
                action_id: runtime.action_id,
                defender,
                from,
                to,
            });
        }
        if now >= runtime.ends_at {
            runtime.completed = true;
            self.emit(Fact::DefensiveActionCompleted {
                action_id: runtime.action_id,
                defender,
                shooter,
                kind: runtime.kind.as_str(),
                displacement_ft: (self.players[defender].pos - runtime.start_position).length(),
            });
        }
        self.defensive_actions[defender] = Some(runtime);
    }

    /// 防守人自己的伸手切球决策：贴身且有切球倾向时赌一次。
    ///
    /// 执行窗口内切球成功率提高，收势窗口内该防守人失去干扰。两者都由
    /// 其他结算直接读取，因此这次赌博有真实的代价。
    fn advance_steal_attempts(&mut self) {
        // 活球飞行阶段（传球、出手在空中）仍然算这次赌博在继续；
        // 只有死球或球权易主才取消。
        if !matches!(
            self.phase(),
            Phase::Live | Phase::PassFlight { .. } | Phase::ShotFlight { .. }
        ) {
            self.cancel_defensive_actions_of(DefensiveActionKind::StealAttempt, "not_live");
            return;
        }
        let now = self.now();
        // 推进已有动作：球在空中也继续，等落地或有人接球再判定。
        for defender in 0..self.defensive_actions.len() {
            let Some(mut runtime) = self.defensive_actions[defender].take() else {
                continue;
            };
            if runtime.kind != DefensiveActionKind::StealAttempt {
                self.defensive_actions[defender] = Some(runtime);
                continue;
            }
            if !self.players[defender].on_court || !self.players[runtime.shooter].on_court {
                self.emit(Fact::DefensiveActionCancelled {
                    action_id: runtime.action_id,
                    defender,
                    shooter: runtime.shooter,
                    kind: runtime.kind.as_str(),
                    reason: "participant_changed",
                });
                continue;
            }
            if let Some(to) = runtime.phase_at(now) {
                let from = runtime.phase;
                runtime.phase = to;
                self.emit(Fact::DefensiveActionPhaseChanged {
                    action_id: runtime.action_id,
                    defender,
                    from,
                    to,
                });
            }
            if now >= runtime.ends_at {
                self.emit(Fact::DefensiveActionCompleted {
                    action_id: runtime.action_id,
                    defender,
                    shooter: runtime.shooter,
                    kind: runtime.kind.as_str(),
                    displacement_ft: (self.players[defender].pos - runtime.start_position).length(),
                });
                self.players[defender].schedule.next_redecide_at =
                    now + f64::from(self.rules.redecide_interval_seconds);
                continue;
            }
            self.defensive_actions[defender] = Some(runtime);
        }
        // 发起新的切球尝试：贴身的对位防守人按自己的调度决定。
        let Some(holder) = self.holder() else {
            return;
        };
        let offense = self.possession.team;
        let policy = self.rules.resolve.technique.clone();
        let security = self.rules.resolve.ball_security.clone();
        let ball = self.ball.pos;
        // 逐 tick 走这条路径，不能分配：按句柄遍历并在循环内过滤。
        for defender in 0..self.players.len() {
            if !self.players[defender].on_court
                || self.players[defender].team != offense.other()
                || self.players[defender].defense_responsibility
                    != crate::state::DefenseResponsibility::OnBall
            {
                continue;
            }
            if self.defensive_actions[defender].is_some() {
                continue;
            }
            if now < self.players[defender].schedule.next_redecide_at {
                continue;
            }
            // 评估过就后退调度：否则不满足条件的防守人每 tick 都被重新评估。
            // 主动赌博是稀有动作：每名防守人每 4 秒才考虑一次。
            self.players[defender].schedule.next_redecide_at =
                now + f64::from(self.rules.redecide_interval_seconds) * 8.0;
            let gap = self.players[defender].pos.distance(ball);
            if gap > security.poke_pressure_radius_ft {
                continue;
            }
            // 赌博倾向低的防守人很少伸手：用抢断倾向做概率筛选。
            let gamble = self.players[defender].data.tendencies.gamble_steal;
            let chance = f64::from((0.05 + 0.35 * gamble).clamp(0.0, 0.5));
            let roll = f64::from(self.draw(
                crate::draw::Stream::Decision,
                defender,
                crate::draw::purpose::STEAL_CREDIT,
                7,
            ));
            if roll >= chance {
                continue;
            }
            self.start_defensive_steal(defender, holder, &policy);
        }
    }

    /// 垂直起跳封盖：协防人跳起伸手，执行窗口内摸高增加。
    ///
    /// 跳起的人无法卡位抢篮板：起跳期间他不参与 box-out，这是这次冒险的代价。
    fn advance_block_jumps(&mut self) {
        if !matches!(self.phase(), Phase::Live | Phase::ShotFlight { .. }) {
            self.cancel_defensive_actions_of(DefensiveActionKind::BlockJump, "shot_ended");
            return;
        }
        let now = self.now();
        let policy = self.rules.resolve.technique.clone();
        for defender in 0..self.defensive_actions.len() {
            let Some(mut runtime) = self.defensive_actions[defender].take() else {
                continue;
            };
            if runtime.kind != DefensiveActionKind::BlockJump {
                self.defensive_actions[defender] = Some(runtime);
                continue;
            }
            if !self.players[defender].on_court {
                self.emit(Fact::DefensiveActionCancelled {
                    action_id: runtime.action_id,
                    defender,
                    shooter: runtime.shooter,
                    kind: runtime.kind.as_str(),
                    reason: "participant_left_court",
                });
                continue;
            }
            if let Some(to) = runtime.phase_at(now) {
                let from = runtime.phase;
                runtime.phase = to;
                self.emit(Fact::DefensiveActionPhaseChanged {
                    action_id: runtime.action_id,
                    defender,
                    from,
                    to,
                });
            }
            if now >= runtime.ends_at {
                self.emit(Fact::DefensiveActionCompleted {
                    action_id: runtime.action_id,
                    defender,
                    shooter: runtime.shooter,
                    kind: runtime.kind.as_str(),
                    displacement_ft: (self.players[defender].pos - runtime.start_position).length(),
                });
                continue;
            }
            self.defensive_actions[defender] = Some(runtime);
        }
        let Some(shooter) = self.players.iter().position(|player| {
            player.on_court
                && player.action.as_ref().is_some_and(|action| {
                    matches!(action.intent.kind, crate::action::ActionKind::Shoot(_))
                })
        }) else {
            return;
        };
        let defense = self.players[shooter].team.other();
        let spot = self.players[shooter].pos;
        let mut closest: Option<(usize, f32)> = None;
        for defender in self.on_court(defense).as_slice().iter().copied() {
            if self.defensive_actions[defender].is_some() {
                continue;
            }
            let gap = (self.players[defender].pos - spot).length();
            if gap > policy.block_jump_radius_ft {
                continue;
            }
            if closest.is_none_or(|(_, best)| gap < best) {
                closest = Some((defender, gap));
            }
        }
        let Some((defender, _)) = closest else {
            return;
        };
        self.start_defensive_block_jump(defender, shooter, &policy);
    }

    fn start_defensive_block_jump(
        &mut self,
        defender: usize,
        shooter: usize,
        policy: &nba_domain::resolve::TechniquePolicy,
    ) {
        let start_time = self.now();
        let preparation_ends_at = start_time + f64::from(policy.block_jump_prep_seconds);
        let execution_ends_at = preparation_ends_at + f64::from(policy.block_jump_exec_seconds);
        let ends_at = execution_ends_at + f64::from(policy.block_jump_follow_seconds);
        self.next_action_id += 1;
        self.next_interaction_id += 1;
        let action_id = self.next_action_id;
        let interaction_id = self.next_interaction_id;
        let start_position = self.players[defender].pos;
        self.defensive_actions[defender] = Some(DefensiveActionRuntime {
            action_id,
            interaction_id,
            kind: DefensiveActionKind::BlockJump,
            defender,
            shooter,
            phase: nba_domain::action_window::ActionPhase::Preparation,
            preparation_ends_at,
            execution_ends_at,
            ends_at,
            start_position,
            target_position: start_position,
            completed: false,
        });
        self.emit(Fact::DefensiveActionStarted {
            action_id,
            interaction_id,
            defender,
            shooter,
            kind: DefensiveActionKind::BlockJump.as_str(),
            start_position,
        });
    }

    fn start_defensive_steal(
        &mut self,
        defender: usize,
        holder: usize,
        policy: &nba_domain::resolve::TechniquePolicy,
    ) {
        let start_time = self.now();
        let preparation_ends_at = start_time + f64::from(policy.steal_attempt_prep_seconds);
        let execution_ends_at = preparation_ends_at + f64::from(policy.steal_attempt_exec_seconds);
        let ends_at = execution_ends_at + f64::from(policy.steal_attempt_follow_seconds);
        self.next_action_id += 1;
        self.next_interaction_id += 1;
        let action_id = self.next_action_id;
        let interaction_id = self.next_interaction_id;
        let start_position = self.players[defender].pos;
        self.defensive_actions[defender] = Some(DefensiveActionRuntime {
            action_id,
            interaction_id,
            kind: DefensiveActionKind::StealAttempt,
            defender,
            shooter: holder,
            phase: nba_domain::action_window::ActionPhase::Preparation,
            preparation_ends_at,
            execution_ends_at,
            ends_at,
            start_position,
            target_position: self.ball.pos,
            completed: false,
        });
        self.emit(Fact::DefensiveActionStarted {
            action_id,
            interaction_id,
            defender,
            shooter: holder,
            kind: DefensiveActionKind::StealAttempt.as_str(),
            start_position,
        });
    }

    fn start_defensive_closeout(&mut self, defender: usize, shooter: usize) {
        let rules = &self.rules;
        let window = nba_domain::action_window::ActionTimeWindow::new_closeout_contest(
            &self.players[defender].id,
            self.now() as f32,
            rules,
        );
        let start_time = self.now();
        let preparation_ends_at = start_time + f64::from(window.prep_duration);
        let execution_ends_at = preparation_ends_at + f64::from(window.exec_duration);
        let ends_at = execution_ends_at + f64::from(window.follow_duration);
        self.next_action_id += 1;
        self.next_interaction_id += 1;
        let action_id = self.next_action_id;
        let interaction_id = self.next_interaction_id;
        let phase = window.phase;
        let defender_position = self.players[defender].pos;
        let hoop = self.hoop_for(self.players[shooter].team);
        let approach = (hoop - self.players[shooter].pos).normalize_or_zero();
        let closeout_distance = self.rules.min_player_separation_ft + 0.1;
        let desired = self.players[shooter].pos + approach * closeout_distance;
        let radius = self.rules.player_radius_ft;
        let target_position = desired.clamp(
            Vec2::new(radius, radius),
            Vec2::new(
                self.rules.court.width_ft - radius,
                self.rules.court.height_ft - radius,
            ),
        );
        self.defensive_actions[defender] = Some(DefensiveActionRuntime {
            action_id,
            interaction_id,
            kind: DefensiveActionKind::CloseoutContest,
            defender,
            shooter,
            phase,
            preparation_ends_at,
            execution_ends_at,
            ends_at,
            start_position: defender_position,
            target_position,
            completed: false,
        });
        self.emit(Fact::DefensiveActionStarted {
            action_id,
            interaction_id,
            defender,
            shooter,
            kind: DefensiveActionKind::CloseoutContest.as_str(),
            start_position: defender_position,
        });
    }

    fn advance_parallel_actions(&mut self) {
        for player in 0..self.players.len() {
            let Some(runtime) = self.players[player].action.as_ref() else {
                continue;
            };
            if self.interactions.contains_key(&runtime.interaction_id)
                || self.now() < runtime.ends_at
            {
                continue;
            }
            let runtime = self.players[player].action.take().unwrap();
            self.emit(Fact::ActionCompleted {
                action_id: runtime.action_id,
                player,
                kind: runtime.intent.kind.as_str(),
            });
        }
    }

    /// 结算所有到期的交互，然后清掉没有交互承载的残余运行时。
    ///
    /// 每个交互只读自己的意图与参与者运行时；同一 tick 内多个交互按编号
    /// 升序结算，回放因此确定。
    fn resolve_due_interactions(&mut self) {
        let now = self.now();
        // 逐 tick 走这条路径，不能分配：到期交互装进固定缓冲。
        const MAX_DUE: usize = 64;
        let mut due = [0u64; MAX_DUE];
        let mut count = 0usize;
        for (id, interaction) in self.interactions.iter() {
            if now < interaction.intent.end_time() {
                continue;
            }
            assert!(count < MAX_DUE, "同一 tick 到期的交互超过 {MAX_DUE} 个");
            due[count] = *id;
            count += 1;
        }
        for interaction_id in due[..count].iter() {
            // 前一个交互结算时可能取消别的动作（球权变化、犯规、信号），
            // 被取消的交互已经从表里移除，这里直接跳过。
            let Some(interaction) = self.interactions.remove(interaction_id) else {
                continue;
            };
            let actor_running = self.players[interaction.intent.actor]
                .action
                .as_ref()
                .is_some_and(|runtime| runtime.action_id == interaction.action_id);
            if !actor_running {
                continue;
            }
            self.resolve_interaction(interaction);
        }
        self.advance_parallel_actions();
    }

    fn resolve_interaction(&mut self, interaction: Interaction) {
        let action_id = interaction.action_id;
        let screen_screener = interaction.intent.target;
        self.action_turn = Some((action_id, crate::action::ActionTurn::Resolve));
        // 掩护防守人跟着交互走：并发动作结算时不会互相覆盖。
        self.screen_defender = interaction.screen_defender;
        let outcome = resolve::resolve(self, interaction.intent);
        self.screen_defender = None;
        self.apply_outcome(outcome, screen_screener, action_id);
        self.action_turn = None;
    }

    fn apply_outcome(
        &mut self,
        outcome: crate::action::ActionOutcome,
        screen_screener: Option<usize>,
        action_id: u64,
    ) {
        let facts = outcome.facts.clone();
        for fact in facts {
            self.emit(fact);
        }
        if let Some(control) = outcome.delta.control {
            self.set_ball_held(control);
            self.players[control].schedule.next_redecide_at =
                self.now() + f64::from(self.rules.redecide_interval_seconds);
            self.players[control].schedule.pending =
                Some(crate::state::DecisionTrigger::PassReceived);
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
        for player in 0..self.players.len() {
            let Some(runtime) = self.players[player].action.take() else {
                continue;
            };
            if runtime.action_id != action_id {
                self.players[player].action = Some(runtime);
                continue;
            }
            if self.players[player].target_kind == crate::state::TargetKind::Contact {
                self.players[player].target_kind = crate::state::TargetKind::Station;
                self.players[player].target_partner = usize::MAX;
            }
            let fact = match outcome.completion {
                crate::action::Completion::Completed => Fact::ActionCompleted {
                    action_id: runtime.action_id,
                    player,
                    kind: runtime.intent.kind.as_str(),
                },
                crate::action::Completion::Failed => Fact::ActionFailed {
                    action_id: runtime.action_id,
                    player,
                    kind: runtime.intent.kind.as_str(),
                },
                crate::action::Completion::Interrupted => Fact::ActionCancelled {
                    action_id: runtime.action_id,
                    player,
                    kind: runtime.intent.kind.as_str(),
                    reason: "interrupted",
                },
            };
            self.emit(fact);
        }
        self.players[outcome.actor].schedule.next_redecide_at =
            self.now() + f64::from(self.rules.redecide_interval_seconds);
        self.players[outcome.actor].schedule.pending =
            Some(crate::state::DecisionTrigger::ActionComplete);
    }

    /// 当前是否存在满足条件的交互，供职责分配与计时判定读派生状态。
    pub fn any_interaction(&self, predicate: impl Fn(&ActionIntent) -> bool) -> bool {
        self.interactions
            .values()
            .any(|interaction| predicate(&interaction.intent))
    }

    fn advance_phase(&mut self) {
        let now = self.now();
        match self.phase {
            Phase::DeadBall { team, ready_at } => {
                if now >= ready_at {
                    if let Some(pending) = self.pending_jump_ball.clone() {
                        // 参跳四人到位才掷球：停表期间不等同于瞬移。
                        let arrived = pending.spots.iter().all(|(handle, spot)| {
                            (self.players[*handle].pos - *spot).length() <= 1.5
                        });
                        if arrived || pending.waited >= 5.0 {
                            self.pending_jump_ball = None;
                            self.begin_period_jump_ball(
                                pending.home,
                                pending.away,
                                crate::action::JumpBallKind::HeldBall,
                                false,
                            );
                        } else {
                            let next = now + 0.5;
                            if let Some(entry) = self.pending_jump_ball.as_mut() {
                                entry.ready_at = next;
                                entry.waited += 0.5;
                            }
                            self.set_phase(Phase::DeadBall {
                                team,
                                ready_at: next,
                            });
                        }
                    } else {
                        self.begin_inbound(team);
                    }
                }
            }
            Phase::Timeout {
                team: _,
                started_at,
                resume_at,
                resume_phase,
            } => {
                if now >= resume_at {
                    let paused = now - started_at;
                    let resumed = resume_phase.phase().shift_deadlines(paused);
                    self.set_phase(resumed);
                }
            }
            Phase::Inbound { team, ready_at } => {
                if now >= ready_at {
                    self.start_possession(team, PossessionKind::Dead);
                }
            }
            Phase::JumpBall {
                home_jumper,
                away_jumper,
                resolve_at,
                kind,
            } => {
                if now >= resolve_at {
                    self.resolve_opening_jump_ball(home_jumper, away_jumper, kind);
                }
            }
            Phase::JumpBallFlight => {
                if let Some(tapper) = self.jump_ball_tapper {
                    let tapped_team = self.players[tapper].team;
                    let controller = self
                        .jump_ball_receiver
                        .expect("jump-ball tap must select a receiver");
                    let reach = self.params.policy.w("loose_ball_catch_base_ft")
                        + self.players[controller].attribute("ball_handling")
                            * self.params.policy.w("loose_ball_catch_skill_ft");
                    let physical = crate::physical::Physical::of(
                        &self.players[controller],
                        &self.params.policy,
                    );
                    let hand_height = physical.standing_reach_ft + physical.vertical_ft;
                    if self.ball.z <= hand_height + reach
                        && (self.players[controller].pos - self.ball.pos).length() <= reach
                    {
                        self.jump_ball_tapper = None;
                        self.jump_ball_receiver = None;
                        self.award_opening_tip_control(tapped_team, controller);
                    }
                }
            }
            Phase::PassFlight { resolve_at } => {
                if self.pass_result.is_some() || now >= resolve_at {
                    self.finish_pass();
                }
            }
            Phase::ShotFlight { resolve_at } | Phase::FreeThrowFlight { resolve_at } => {
                if self
                    .pending_shot
                    .as_ref()
                    .is_some_and(|pending| pending.result.is_some())
                    || now >= resolve_at
                {
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
                foul_id,
                technical_retained_team: _,
                after_technical: _,
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
                        foul_id,
                    );
                }
            }
            Phase::PeriodBreak {
                resume_at,
                next_stage,
            } => {
                if now >= resume_at {
                    let stage_after_break =
                        if self.clock.period() >= self.rules.league.regulation_periods {
                            crate::state::MatchStage::Overtime
                        } else {
                            crate::state::MatchStage::Intermission
                        };
                    assert_eq!(
                        next_stage, stage_after_break,
                        "period break destination must match the next period"
                    );
                    let next_period_duration =
                        if self.clock.period() >= self.rules.league.regulation_periods {
                            f64::from(self.rules.league.overtime_duration_seconds)
                        } else {
                            f64::from(self.rules.league.period_duration_seconds)
                        };
                    self.clock.start_next_period(
                        next_period_duration,
                        self.rules.league.shot_clock_seconds as f64,
                    );
                    self.fouls[0].period = 0;
                    self.fouls[1].period = 0;
                    // 换节后最后时段的暂停配额重新计算。
                    for ledger in self.timeouts.iter_mut() {
                        ledger.period = self.clock.period();
                        ledger.used_in_final_window = 0;
                    }
                    let next_period = self.clock.period();
                    if self.rules.league.period_starts_with_jump_ball(next_period) {
                        let (home_jumper, away_jumper) = self.period_jumpers();
                        self.begin_period_jump_ball(
                            home_jumper,
                            away_jumper,
                            crate::action::JumpBallKind::Overtime,
                            true,
                        );
                    } else {
                        self.alternate_possession_next = self.period_started_with.other();
                        let team = self.alternate_possession_next;
                        self.period_started_with = team;
                        self.period_start_inbound_pending = true;
                        self.begin_inbound(team);
                    }
                }
            }
            Phase::Live => {
                // 球在空中时时钟走完不是违例：那一球会自己解出进与不进，
                // 违例只在计时归零时球还在某个球员手里成立。
                let shooting =
                    self.any_interaction(|intent| matches!(&intent.kind, ActionKind::Shoot(_)));
                if self.clock.shot_clock() <= 0.0 && self.pending_rebound.is_none() && !shooting {
                    if std::env::var("NBA_TRACE_CLOCK").is_ok() {
                        eprintln!(
                            "CLOCK VIOLATION now={:.2} started={:.2} elapsed={:.2} phase={:?} holder={:?} next_decide={:.2} pending={:?} active={:?} play={:?}",
                            self.now(),
                            self.possession.started_at,
                            self.now() - self.possession.started_at,
                            self.possession.ball_phase,
                            self.holder(),
                            self.holder()
                                .map(|handle| self.players[handle].schedule.next_redecide_at)
                                .unwrap_or(f64::NAN),
                            self.holder()
                                .map(|handle| self.players[handle].schedule.pending)
                                .unwrap_or(None),
                            self.interactions.keys().copied().collect::<Vec<_>>(),
                            self.possession.play,
                        );
                    }
                    let team = self.possession.team;
                    let actor = self.holder().unwrap_or_else(|| self.primary_handler(team));
                    self.line_mut(actor).turnovers += 1;
                    self.emit(Fact::Turnover {
                        player: actor,
                        kind: crate::action::TurnoverKind::ShotClock,
                    });
                    self.end_possession(PossessionEndCause::TurnoverViolation, "shot_clock", 0);
                    self.start_possession(team.other(), PossessionKind::Live);
                }
            }
            Phase::Pregame | Phase::Finished => {}
        }
    }

    /// 高吊球到点时防守人能不能把球打掉：距离越近越容易。
    fn contest_lob_catch(&mut self, receiver: usize) -> Option<usize> {
        let radius = self.params.policy.w("lob_contest_radius_ft");
        let target = self.players[receiver].pos;
        let team = self.players[receiver].team;
        let mut nearest: Option<(usize, f32)> = None;
        for (handle, player) in self.players.iter().enumerate() {
            if !player.on_court || player.team == team {
                continue;
            }
            let distance = (player.pos - target).length();
            if distance <= radius && nearest.is_none_or(|(_, best)| distance < best) {
                nearest = Some((handle, distance));
            }
        }
        let (by, distance) = nearest?;
        let closeness = 1.0 - distance / radius;
        let chance = self.params.policy.w("lob_contest_base")
            + self.params.policy.w("lob_contest_steal") * self.players[by].attribute("steal")
            + self.params.policy.w("lob_contest_close") * closeness;
        let roll = self.draw(
            crate::draw::Stream::Pass,
            by,
            crate::draw::purpose::CUT_COMPLETION,
            0,
        );
        (roll < chance.clamp(0.0, 0.9)).then_some(by)
    }

    fn finish_pass(&mut self) {
        let Some(info) = self.pass_in_flight.take() else {
            return;
        };
        let pass_id = info.pass_id;
        let from = info.from;
        let kind = info.kind;
        // 高吊球到篮下时是贴身争抢：防守人就在接球人身边，手够到球就能把它
        // 打掉。空接本身不是稳的两分。
        let result = match self.pass_result.take() {
            Some(crate::ball::PassResult::Caught { to })
                if kind == crate::action::PassKind::Lob =>
            {
                match self.contest_lob_catch(to) {
                    Some(by) => Some(crate::ball::PassResult::Intercepted { by }),
                    None => Some(crate::ball::PassResult::Caught { to }),
                }
            }
            other => other,
        };
        match result {
            Some(crate::ball::PassResult::Caught { to: caught }) => {
                self.set_ball_held(caught);
                self.set_phase(Phase::Live);
                self.cancel_actions("pass_caught");
                self.possession.last_pass_at = Some(self.now());
                self.possession.ball_phase = crate::state::BallPhase::Catch;
                self.emit(Fact::Pass {
                    pass_id,
                    from,
                    to: caught,
                    kind,
                    completed: true,
                });
                if kind == crate::action::PassKind::Lob {
                    let cutter = caught;
                    let defender_distance = self
                        .players
                        .iter()
                        .filter(|player| {
                            player.on_court && player.team != self.players[cutter].team
                        })
                        .map(|player| (player.pos - self.players[cutter].pos).length())
                        .fold(f32::INFINITY, f32::min);
                    self.possession.last_cut_at = Some(self.now());
                    self.possession.last_cut_actor = Some(cutter);
                    self.possession.ball_phase = crate::state::BallPhase::Cut;
                    self.emit(Fact::Cut {
                        cutter,
                        arrived: true,
                        defender_distance_ft: defender_distance,
                        kind: self.cut_kind_name(cutter),
                    });
                }
                self.players[caught].schedule.next_redecide_at =
                    self.now() + f64::from(self.rules.redecide_interval_seconds);
                self.players[caught].schedule.pending =
                    Some(crate::state::DecisionTrigger::PassReceived);
            }
            Some(crate::ball::PassResult::Intercepted { by }) => {
                let recovery_pos = self.ball.pos;
                // 手碰到球和把球拿住是两件事：碰到只说明站位对了，能不能
                // 抓住看他的抢断与球感。拿不住就是打掉的活球，谁抢到算谁的。
                let catch = self.params.policy.w("pass_catch_base")
                    + self.players[by].attribute("steal")
                        * self.params.policy.w("pass_catch_steal")
                    + self.players[by].attribute("ball_handling")
                        * self.params.policy.w("pass_catch_handling");
                if self.draw(
                    crate::draw::Stream::Pass,
                    by,
                    crate::draw::purpose::PASS_INTERCEPT,
                    1,
                ) >= catch
                {
                    // 手只碰得到球、拿不住：球变活球，谁先抢到谁算抢断。
                    self.emit(Fact::BallContact {
                        kind: "deflection",
                        entity: Some(by),
                        time: self.now(),
                        x: recovery_pos.x,
                        y: recovery_pos.y,
                        z: self.rules.ball_radius_ft,
                        sub_tick: 0,
                    });
                    self.ball.pos = recovery_pos;
                    self.ball.z = self.rules.ball_radius_ft;
                    self.ball.vel = Vec2::ZERO;
                    self.ball.vz = 0.0;
                    self.release_ball(crate::state::BallState::Loose {
                        pos: recovery_pos,
                        vel: Vec2::ZERO,
                    });
                    crate::resolve::rebound::resolve_turnover(self);
                    if let Some(pending) = self.pending_rebound.as_mut() {
                        pending.poke_by = Some((by, from));
                    }
                    self.cancel_actions("turnover");
                    return;
                }
                self.emit(Fact::BallContact {
                    kind: "interception",
                    entity: Some(by),
                    time: self.now(),
                    x: recovery_pos.x,
                    y: recovery_pos.y,
                    z: self.rules.ball_radius_ft,
                    sub_tick: 0,
                });
                self.line_mut(from).turnovers += 1;
                self.line_mut(by).steals += 1;
                self.emit(Fact::Steal {
                    player: by,
                    victim: from,
                });
                self.emit(Fact::Turnover {
                    player: from,
                    kind: crate::action::TurnoverKind::BadPass,
                });
                let team = self.players[from].team;
                self.end_possession(PossessionEndCause::TurnoverSteal, "pass_intercepted", 0);
                self.start_possession(team.other(), PossessionKind::Live);
                self.set_ball_held(by);
                self.players[by].target = self.top_of_key(team.other());
                self.players[by].target_kind = crate::state::TargetKind::Window;
                self.players[by].target_partner = by;
                self.players[by].schedule.next_redecide_at =
                    self.now() + f64::from(self.rules.redecide_interval_seconds);
                self.players[by].schedule.pending =
                    Some(crate::state::DecisionTrigger::PossessionStart);
            }
            Some(crate::ball::PassResult::Incomplete) => {
                if std::env::var("NBA_TRACE_MISS").is_ok() {
                    let d = (self.ball.pos - self.players[info.to].pos).length();
                    eprintln!("MISS dice to={} dist={d:.1} z={:.2}", info.to, self.ball.z);
                }
                self.finish_bad_pass(from);
            }
            None => {
                if std::env::var("NBA_TRACE_MISS").is_ok() {
                    let d = (self.ball.pos - self.players[info.to].pos).length();
                    let z = self.ball.z;
                    let reach = crate::ball::pass_catch_radius(&self.rules, &self.players[info.to]);
                    let kept = self.players[info.to].target;
                    eprintln!(
                        "MISS geom to={} dist={d:.1} z={z:.2} catch={reach:.1} gap_target={:.1}",
                        info.to,
                        (self.players[info.to].pos - kept).length()
                    );
                }
                self.finish_bad_pass(from);
            }
        }
    }

    fn finish_bad_pass(&mut self, passer: usize) {
        // 没接住的传球和被打掉的传球一样只是活球：双方都要抢，谁控制谁
        // 决定这次进攻的结局。球回到自己人手里，进攻继续，不记失误；
        // 落到对方手里才记失误与抢断。
        crate::resolve::rebound::resolve_turnover(self);
        if let Some(pending) = self.pending_rebound.as_mut() {
            pending.pass_by = Some(passer);
        }
    }

    fn maybe_decide(&mut self) {
        crate::policy::tick(self);
        // 无球球员各自决策：持球人的决策不覆盖他们的动作。
        crate::policy::off_ball_tick(self);
    }

    fn check_period_end(&mut self) {
        if self.clock.game_clock() > 0.0 || self.finished {
            return;
        }
        if matches!(
            self.phase(),
            Phase::FreeThrows { .. } | Phase::FreeThrowFlight { .. }
        ) {
            return;
        }
        if self
            .pending_shot
            .as_ref()
            .is_some_and(|shot| shot.result.is_none() && shot.free_throw.is_none())
        {
            return;
        }
        crate::tactics::close_play_window(self);
        if matches!(self.phase(), Phase::Finished | Phase::PeriodBreak { .. }) {
            return;
        }
        let tied = self.score[0] == self.score[1];
        let regulation_over = self.clock.period() >= 4;
        self.pending_rebound = None;
        if regulation_over && tied {
            self.emit(Fact::PeriodEnd {
                period: self.clock.period(),
            });
            self.end_possession(PossessionEndCause::PeriodEnd, "period_end", 0);
            self.cancel_actions("period_end");
            self.clock.set_period_duration(300.0);
            self.set_period_phase(Phase::PeriodBreak {
                resume_at: self.now() + 1.0,
                next_stage: crate::state::MatchStage::Overtime,
            });
            return;
        }
        if !regulation_over {
            self.emit(Fact::PeriodEnd {
                period: self.clock.period(),
            });
            self.end_possession(PossessionEndCause::PeriodEnd, "period_end", 0);
            self.cancel_actions("period_end");
            self.set_period_phase(Phase::PeriodBreak {
                resume_at: self.now() + 1.0,
                next_stage: if self.clock.period() >= 4 {
                    crate::state::MatchStage::Overtime
                } else {
                    crate::state::MatchStage::Intermission
                },
            });
            return;
        }
        self.end_possession(PossessionEndCause::PeriodEnd, "period_end", 0);
        self.cancel_actions("game_end");
        self.finished = true;
        self.set_program_phase(
            crate::state::MatchStage::Finished,
            crate::state::MatchProgram::Complete,
            Phase::Finished,
        );
        self.emit(Fact::GameEnd);
    }
}

pub use crate::formation::{defense_spots, offense_spots};
pub use crate::report::GameReport;
