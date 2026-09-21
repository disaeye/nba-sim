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
pub mod projectile;
pub mod resolve;
pub mod rules;
pub mod tactics;
pub use capability::{
    drive_finishing_delta, effective_catch_radius, effective_defensive_boxout_bonus,
    effective_help_awareness, effective_max_accel, effective_max_speed,
    effective_post_defense_physicality, effective_putback_bias, effective_risk_tolerance,
    effective_transition_leakout_chance, effective_turn_decel_retention, free_throw_probability,
    poke_check_success, receive_estimate_noise,
};
pub use court::CourtGeometry;
pub use data::{
    project_display_role, CoachProfile, PlayerAttributes, PlayerData, PlayerSlotFitness,
    PlayerTendencies, SubstitutionEvent, SubstitutionReason, TeamData, TeamTraits,
};
pub use event::{GameEvent, PossessionEndCause, PossessionSummary, TimedGameEvent};
pub use flow::{transition_ball_state, BallPhase, BallState, GameFlowState, PhaseType};
pub use league::{LeagueId, LeagueProfile};
pub use possession::{BallOwnership, Possession, SubPhase};
pub use resolve::{
    BaseRates, BlockPolicy, ContactPolicy, DrivePolicy, PassPolicy, ReboundPolicy, ResolveConfig,
    ShotTypeBlockBias, ShotTypeRates,
};
pub use rules::{
    DecisionRules, DefenseRules, GameRules, ModulationRules, PotentialFieldRules, SemanticRules,
    TacticalRules, UNIMPLEMENTED_RULE_FIELDS,
};
pub use tactics::{
    DefensiveSystem, HelpDefenseConfig, MatchupRule, OffensiveSystem, OnBallDefenseConfig,
    PlayerAttributeKey, ScreenDefenseConfig, SituationalTactics, SlotBehaviour, SlotRequirement,
    TacticalAction, TacticalFormation, TacticalSetSpec, TacticalSlot, TacticalSlotSpec,
    TacticalTriggers,
};
/// Deterministic simulation step supplied by the simulation scheduler.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FixedDt(pub f32);
