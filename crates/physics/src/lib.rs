//! 物理子系统 (Physics Subsystem)。
//!
//! 刚体世界、运动学 FSM、篮球三维弹道与空间几何查询。
//! 依赖领域层词汇，不知道任何决策与裁决逻辑。

pub mod ballistics;
pub mod movement;
pub mod spatial;

pub use ballistics::{BallisticsEngine, BallTrajectoryKind, ReboundLandingSpot};
pub use movement::{
    LocomotionState, PhysicsWorld, PlayerPhysicsState, MAX_PLAYER_ACCEL_FTPS2,
    MAX_PLAYER_SPEED_FTPS, PLAYER_RADIUS_FT,
};
pub use spatial::{OpennessMetric, PassCorridorStatus, SpatialGeometry};
