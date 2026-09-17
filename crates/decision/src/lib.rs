//! 决策子系统 (Decision Subsystem)。
//!
//! 约束驱动的智能体决策：候选生成 → 硬约束过滤 → 软约束惩罚 →
//! 效用评分 → softmax 个性化采样；以及战术规划与体能/士气调制。

pub mod constraint;
pub mod defense;
pub mod modulation;
pub mod pipeline;
pub mod potential_field;
pub mod tactics;

pub use constraint::{
    CandidateAction, Constraint, ConstraintContext, ConstraintDomain, ConstraintFinding,
    ConstraintRegistry, ConstraintResult, ConstraintScope, ConstraintStatus, ConstraintTiming,
    EnforcementAction, PhaseType, ScoredCandidate, Severity, ViolationKind,
};
pub use modulation::{CoachStrategy, MoraleState, PlayerModulationState};
pub use pipeline::{DecisionOutput, DecisionSystem, DecisionTrace, DecisionWeights};
pub use potential_field::{
    DefensePotentialFieldSolver, EmergentDefenseTarget, PotentialFieldConfig,
};
pub use tactics::{
    DefensiveRole, DefensiveScheme, DefensiveTactic, OffensiveRole, TacticalPlanner, TacticalSet,
    TeamIntent,
};
// Domain owns the canonical phase, possession, and lifecycle vocabulary.
