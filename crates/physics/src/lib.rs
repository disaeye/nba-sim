//! 物理子系统 (Physics Subsystem)。
//!
//! 刚体世界、运动学 FSM、篮球三维弹道与空间几何查询。
//! 依赖领域层词汇，不知道任何决策与裁决逻辑。

pub mod ballistics;
pub mod movement;
pub mod spatial;

pub use ballistics::{BallTrajectoryKind, BallisticsEngine, ReboundLandingSpot};
pub use movement::{
    EntityFilter, LocomotionState, PhysicsBackend, PhysicsFact, PhysicsWorld, PlayerPhysicsState,
    RawContact, RayHit, ShapeCastHit, SimpleCirclePhysics, SpatialPhysics,
};
pub use spatial::{OpennessMetric, PassCorridorStatus, SpatialGeometry};
