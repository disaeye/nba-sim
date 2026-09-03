//! 领域层（Domain Layer）：篮球比赛本体词汇表。
//!
//! 只描述“比赛是什么”（场地、时钟、阶段、事件、动作窗口、规则参数），
//! 不描述“比赛怎么打”（决策、物理、裁判实现均在外层）。
//! 本 crate 是依赖图的最内层：除 glam 数学类型与 serde 派生外零依赖。
use serde::{Deserialize, Serialize};

pub mod action_window;
pub mod capability;
pub mod court;
pub mod data;
pub mod event;
pub mod flow;
pub mod league;
pub mod possession;
pub mod resolve;
pub mod rules;
pub mod tactics;
pub use tactics::{
    OffensiveSystem, TacticalAction, TacticalFormation, TacticalSetSpec, TacticalSlot,
    TacticalSlotSpec, TacticalTriggers,
};
pub use capability::{
    drive_finishing_delta, effective_decision_risk_tolerance, effective_defense_factor,
    effective_max_accel, effective_max_speed, free_throw_probability,
};
pub use court::CourtGeometry;
pub use data::{PlayerAttributes, PlayerData, PlayerRole, PlayerTendencies, TeamData, TeamTraits};
pub use event::{GameEvent, PossessionSummary, TimedGameEvent};
pub use flow::{transition_ball_state, BallPhase, BallState, GameFlowState, MatchClockState, MatchScoreState, PhaseType};
pub use league::{LeagueId, LeagueProfile};
pub use possession::{BallOwnership, Possession, SubPhase};
pub use resolve::{BaseRates, ContactPolicy, DrivePolicy, PassPolicy, ReboundPolicy,
    ResolveConfig, ShotTypeBlockBias, ShotTypeRates};
pub use rules::{DecisionRules, GameRules, ModulationRules, SemanticRules, TacticalRules};
/// Deterministic simulation step supplied by the simulation scheduler.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FixedDt(pub f32);
