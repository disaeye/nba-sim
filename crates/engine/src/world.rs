//! D22 纯数据实体世界 (ECS Dataflow World)
//!
//! 依据 ADR-011 与 docs/dev/current/plan.md §3：
//! MatchWorld 是纯数据组件容器，无任何隐藏可变引用或阶段耦合，
//! 作为四大执行管线（Perception, Decision, Physics, Officiating）的单一状态中枢。

use glam::Vec2;
use nba_decision::tactics::TacticalSet;
use nba_domain::{GameFlowState, GameRules, PlayerAttributes, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::LocomotionState;

/// 球员运动学与动力学组件 (Transform & Dynamics Component)
#[derive(Debug, Clone)]
pub struct TransformComponent {
    pub pos_ft: Vec2,
    pub vel_ft: Vec2,
    pub accel_ft: Vec2,
    pub facing: Vec2,
}

/// 球员身体与运动极限约束 (Physical Limit Component)
#[derive(Debug, Clone)]
pub struct PhysicalLimitComponent {
    pub max_speed_ftps: f32,
    pub max_accel_ftps2: f32,
    pub traction_limit: f32,
}

/// 球员单场状态组件 (Player Runtime State Component)
#[derive(Debug, Clone)]
pub struct PlayerRuntimeComponent {
    pub id: String,
    pub team: String,
    pub jersey: String,
    pub on_court: bool,
    pub stamina: f32,
    pub max_stamina: f32,
    pub foul_count: u8,
    pub morale: String,
    pub action: String,
    pub slot: String,
    pub locomotion: LocomotionState,
}

/// 动作学细分阶段 (Action Phase & Micro-Kinematics)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ActionPhase {
    Idle,
    Gather { elapsed: f32, duration: f32 },
    Elevate { elapsed: f32, duration: f32 },
    Release { released: bool },
    Landing { elapsed: f32, duration: f32 },
}

impl ActionPhase {
    /// 是否处于合法盖帽窗口 (只能在起跳上升期与出手瞬间)
    pub fn is_blockable_window(&self) -> bool {
        matches!(self, Self::Elevate { .. } | Self::Release { .. })
    }

    /// 是否处于投篮犯规侵犯圆柱体保护期 (起跳到落地全周期)
    pub fn is_shooting_foul_protected_window(&self) -> bool {
        matches!(self, Self::Elevate { .. } | Self::Release { .. } | Self::Landing { .. })
    }

    /// 动作阶段步进
    pub fn step(&mut self, dt: f32) -> ActionPhaseTransition {
        match self {
            Self::Gather { elapsed, duration } => {
                *elapsed += dt;
                if *elapsed >= *duration {
                    *self = Self::Elevate { elapsed: 0.0, duration: 0.25 };
                    ActionPhaseTransition::Elevating
                } else {
                    ActionPhaseTransition::Continuing
                }
            }
            Self::Elevate { elapsed, duration } => {
                *elapsed += dt;
                if *elapsed >= *duration {
                    *self = Self::Release { released: false };
                    ActionPhaseTransition::ReadyToRelease
                } else {
                    ActionPhaseTransition::Continuing
                }
            }
            Self::Release { released } => {
                if !*released {
                    *released = true;
                    *self = Self::Landing { elapsed: 0.0, duration: 0.20 };
                    ActionPhaseTransition::Released
                } else {
                    ActionPhaseTransition::Continuing
                }
            }
            Self::Landing { elapsed, duration } => {
                *elapsed += dt;
                if *elapsed >= *duration {
                    *self = Self::Idle;
                    ActionPhaseTransition::Completed
                } else {
                    ActionPhaseTransition::Continuing
                }
            }
            Self::Idle => ActionPhaseTransition::Continuing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionPhaseTransition {
    Continuing,
    Elevating,
    ReadyToRelease,
    Released,
    Completed,
}

/// 篮球动力学与所有权组件 (Ball Component)
#[derive(Debug, Clone)]
pub struct BallComponent {
    pub pos_3d: (Vec2, f32),
    pub vel_3d: (Vec2, f32),
    pub state: BallTrajectoryKind,
    pub associated_player_id: Option<String>,
    pub last_touch_team: Possession,
}

/// 比赛时钟与节次组件 (Clock Component)
#[derive(Debug, Clone)]
pub struct ClockComponent {
    pub period: u32,
    pub game_clock: f32,
    pub shot_clock: f32,
    pub current_time: f32,
    pub sub_phase: SubPhase,
    pub sub_phase_timer: f32,
}

/// 比赛账本与积分判则组件 (Ledger Component)
#[derive(Debug, Clone)]
pub struct LedgerComponent {
    pub home_score: u32,
    pub away_score: u32,
    pub possession: Possession,
    pub possession_id: u32,
    pub home_fouls_in_period: u8,
    pub away_fouls_in_period: u8,
    pub possession_arrow: Option<Possession>,
}

/// 纯数据驱动的比赛实体世界 (MatchWorld)
#[derive(Debug, Clone)]
pub struct MatchWorld {
    pub transforms: Vec<TransformComponent>,
    pub limits: Vec<PhysicalLimitComponent>,
    pub states: Vec<PlayerRuntimeComponent>,
    pub action_phases: Vec<ActionPhase>,
    pub attributes: Vec<PlayerAttributes>,
    pub ball: BallComponent,
    pub clock: ClockComponent,
    pub ledger: LedgerComponent,
    pub tactical_set: TacticalSet,
    pub game_flow: GameFlowState,
    pub rules: GameRules,
}

impl MatchWorld {
    /// 创建默认初始化的比赛实体世界
    pub fn new_initial(rules: GameRules) -> Self {
        Self {
            transforms: Vec::with_capacity(10),
            limits: Vec::with_capacity(10),
            states: Vec::with_capacity(10),
            action_phases: Vec::with_capacity(10),
            attributes: Vec::with_capacity(10),
            ball: BallComponent {
                pos_3d: (Vec2::new(47.0, 25.0), 10.0),
                vel_3d: (Vec2::ZERO, 0.0),
                state: BallTrajectoryKind::Dead {
                    pos: Vec2::new(47.0, 25.0),
                    z: 0.0,
                    last_touch_team: Possession::Home,
                    last_touch_player: None,
                },
                associated_player_id: None,
                last_touch_team: Possession::Home,
            },
            clock: ClockComponent {
                period: 1,
                game_clock: 720.0,
                shot_clock: 24.0,
                current_time: 0.0,
                sub_phase: SubPhase::Initiation,
                sub_phase_timer: 0.0,
            },
            ledger: LedgerComponent {
                home_score: 0,
                away_score: 0,
                possession: Possession::Home,
                possession_id: 1,
                home_fouls_in_period: 0,
                away_fouls_in_period: 0,
                possession_arrow: None,
            },
            tactical_set: TacticalSet::HighPickAndRoll,
            game_flow: GameFlowState::Pregame,
            rules,
        }
    }

    /// 当前场上球员数量
    pub fn player_count(&self) -> usize {
        self.states.len()
    }

    /// 获取所有在场活球球员的只读视图
    pub fn on_court_players(
        &self,
    ) -> impl Iterator<Item = (&PlayerRuntimeComponent, &TransformComponent)> {
        self.states
            .iter()
            .zip(self.transforms.iter())
            .filter(|(s, _)| s.on_court)
    }

    /// 执行一次基于 ECS 数据流的纯管线步进
    pub fn step_pipeline(&mut self, dt: f32) -> Vec<String> {
        let perception = PerceptionSystem::evaluate(self);
        let intentions = DecisionSystem::evaluate(self, &perception);
        PhysicsSystem::step(self, &intentions, dt);
        OfficiatingSystem::resolve(self)
    }
}

/// 空间拓扑与压迫感知结果 (Spatial Perception Output)
#[derive(Debug, Clone)]
pub struct SpatialPerception {
    pub defensive_contest_density: Vec<f32>,
    pub nearest_defender_dist: Vec<f32>,
    pub voronoi_openness: Vec<f32>,
    pub open_passing_lanes: Vec<bool>,
    pub weak_side_open_target: Option<usize>,
}

/// 阶段一：空间拓扑感知系统 (PerceptionSystem)
pub struct PerceptionSystem;

impl PerceptionSystem {
    pub fn evaluate(world: &MatchWorld) -> SpatialPerception {
        let n = world.transforms.len();
        let mut density = vec![0.0_f32; n];
        let mut nearest = vec![999.0_f32; n];
        let mut openness = vec![0.0_f32; n];
        let mut open_lanes = vec![true; n];
        let mut max_weak_side_openness = 0.0_f32;
        let mut best_weak_target = None;

        for i in 0..n {
            let p_i = world.transforms[i].pos_ft;
            let team_i = &world.states[i].team;
            let mut min_d = 999.0_f32;
            let mut press = 0.0_f32;

            for j in 0..n {
                if i == j {
                    continue;
                }
                let team_j = &world.states[j].team;
                if team_i != team_j {
                    let p_j = world.transforms[j].pos_ft;
                    let d = p_i.distance(p_j);
                    if d < min_d {
                        min_d = d;
                    }
                    if d < 16.0 {
                        // 考虑防守人朝向衰减 (Facing vector dot product)
                        let to_offense = (p_i - p_j).normalize_or_zero();
                        let facing = world.transforms[j].facing;
                        let alignment = (facing.dot(to_offense)).max(0.1);
                        let kernel = alignment / ((d / 4.0).powi(2) + 1.0);
                        press += kernel;
                    }
                }
            }
            nearest[i] = min_d;
            density[i] = press;
            open_lanes[i] = min_d > 4.0;

            // 局部 Voronoi 空间开阔度近似: 以 min_d 为半径，受防守压迫核折扣
            let safe_radius = min_d.min(25.0);
            let open_area = std::f32::consts::PI * safe_radius.powi(2) * (1.0 / (press + 1.0));
            openness[i] = open_area;

            // 弱侧大空位探测 (底角与侧翼空切窗口)
            if open_area > 150.0 && press < 0.25 && open_area > max_weak_side_openness {
                max_weak_side_openness = open_area;
                best_weak_target = Some(i);
            }
        }

        SpatialPerception {
            defensive_contest_density: density,
            nearest_defender_dist: nearest,
            voronoi_openness: openness,
            open_passing_lanes: open_lanes,
            weak_side_open_target: best_weak_target,
        }
    }
}

/// 球员单步意图动作
#[derive(Debug, Clone)]
pub enum PlayerIntentAction {
    Hold,
    MoveTo { target: Vec2 },
    PassTo { target_id: String },
    Shoot,
}

/// 阶段二：连续博弈意图系统 (DecisionSystem)
#[derive(Debug, Clone)]
pub struct MatchIntentions {
    pub actions: Vec<PlayerIntentAction>,
}

pub struct DecisionSystem;

impl DecisionSystem {
    pub fn evaluate(world: &MatchWorld, perception: &SpatialPerception) -> MatchIntentions {
        let mut actions = Vec::with_capacity(world.states.len());
        for (idx, state) in world.states.iter().enumerate() {
            if !state.on_court {
                actions.push(PlayerIntentAction::Hold);
                continue;
            }
            // 若为持球人且受到高压迫，倾向于传球或突破；若空位则倾向于终结
            let contest = perception.defensive_contest_density.get(idx).copied().unwrap_or(0.0);
            if contest > 2.0 {
                actions.push(PlayerIntentAction::MoveTo {
                    target: world.transforms[idx].pos_ft + world.transforms[idx].facing * 2.0,
                });
            } else {
                actions.push(PlayerIntentAction::Hold);
            }
        }
        MatchIntentions { actions }
    }
}

/// 阶段三：连续受限势能动力学系统 (PhysicsSystem)
pub struct PhysicsSystem;

impl PhysicsSystem {
    pub fn step(world: &mut MatchWorld, intentions: &MatchIntentions, dt: f32) {
        for (idx, action) in intentions.actions.iter().enumerate() {
            if idx >= world.transforms.len() || idx >= world.limits.len() {
                break;
            }
            match action {
                PlayerIntentAction::MoveTo { target } => {
                    let cur = world.transforms[idx].pos_ft;
                    let dir = (*target - cur).normalize_or_zero();
                    let max_speed = world.limits[idx].max_speed_ftps;
                    let max_accel = world.limits[idx].max_accel_ftps2;
                    let traction = world.limits[idx].traction_limit;

                    // 1. 意图期望速度
                    let desired_vel = dir * max_speed;
                    let mut accel = (desired_vel - world.transforms[idx].vel_ft).clamp_length_max(max_accel);

                    // 2. 地面抓地力限制侧向变向 (Traction Envelope)
                    let current_vel = world.transforms[idx].vel_ft;
                    if current_vel.length_squared() > 1.0 {
                        let fwd = current_vel.normalize();
                        let lateral_accel = accel - fwd * accel.dot(fwd);
                        if lateral_accel.length() > traction {
                            let clamped_lat = lateral_accel.normalize() * traction;
                            let forward_accel = fwd * accel.dot(fwd);
                            accel = forward_accel + clamped_lat;
                        }
                    }

                    world.transforms[idx].accel_ft = accel;
                    world.transforms[idx].vel_ft += accel * dt;
                    let vel = world.transforms[idx].vel_ft;
                    world.transforms[idx].pos_ft += vel * dt;
                    if dir.length_squared() > 0.01 {
                        world.transforms[idx].facing = dir;
                    }
                }
                PlayerIntentAction::Hold => {
                    // 抓地力自然阻尼制动 (Inertial braking)
                    let friction = 0.82_f32;
                    world.transforms[idx].vel_ft *= friction;
                    world.transforms[idx].accel_ft = Vec2::ZERO;
                    let vel = world.transforms[idx].vel_ft;
                    world.transforms[idx].pos_ft += vel * dt;
                }
                _ => {}
            }
        }
        // 更新时钟
        world.clock.current_time += dt;
        if world.clock.game_clock > dt {
            world.clock.game_clock -= dt;
        } else {
            world.clock.game_clock = 0.0;
        }
    }
}

/// 阶段四：规则判罚与账本系统 (OfficiatingSystem)
pub struct OfficiatingSystem;

impl OfficiatingSystem {
    pub fn resolve(world: &mut MatchWorld) -> Vec<String> {
        let mut events = Vec::new();
        if world.clock.game_clock == 0.0 && world.clock.period < 4 {
            world.clock.period += 1;
            world.clock.game_clock = 720.0;
            events.push(format!("第 {} 节比赛结束", world.clock.period - 1));
        }
        events
    }
}
