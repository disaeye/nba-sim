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
    pub fn on_court_players(&self) -> impl Iterator<Item = (&PlayerRuntimeComponent, &TransformComponent)> {
        self.states
            .iter()
            .zip(self.transforms.iter())
            .filter(|(s, _)| s.on_court)
    }
}
