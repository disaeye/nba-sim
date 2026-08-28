pub mod constraint;
pub mod decision;
pub mod court;
pub mod movement;
pub mod spatial;
pub mod ballistics;
pub mod action_window;
pub mod events;
pub mod resolution;
pub mod protocol;
pub mod tactics;
pub mod modulation;
pub mod adjudication;
pub mod simulation;

pub use court::Court;
pub use movement::{PhysicsWorld, PlayerPhysicsState, LocomotionState};
pub use spatial::{SpatialGeometry, PassCorridorStatus, OpennessMetric};
pub use ballistics::{BallisticsEngine, BallTrajectoryKind, ReboundLandingSpot};
pub use action_window::{ActionTimeWindow, ActionPhase, ActionType};
pub use events::PhysicsEvent;
pub use resolution::{ResolutionLayer, ResolutionOutcome};
pub use protocol::{StreamTick, RenderPlayer, RenderBall, RenderFrame, DecisionDebug};
pub use constraint::{
    Constraint, ConstraintDomain, Severity, ConstraintScope, ConstraintTiming, PhaseType,
    CandidateAction, ConstraintContext, ConstraintResult, ConstraintStatus, EnforcementAction,
    ViolationKind, ConstraintRegistry, ScoredCandidate,
};
pub use decision::{DecisionSystem, DecisionOutput, DecisionTrace, DecisionWeights};
pub use simulation::MatchEngine;
