//! 球权归属词汇。

use serde::{Deserialize, Serialize};

/// 球权归属。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Possession {
    Home,
    Away,
}

impl Possession {
    pub fn other(self) -> Self {
        match self {
            Possession::Home => Possession::Away,
            Possession::Away => Possession::Home,
        }
    }
}

/// 回合内子阶段（macro-possession 状态机的状态词汇）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubPhase {
    /// 回合发起（发球/抢断转换后的推进起点）
    Initiation,
    /// 阵地战术执行
    ActionExecution,
    /// 出手飞行与裁决
    ShotAttempt,
    /// 篮板争抢
    FlightAndRebound,
    /// 死球重置
    DeadBallReset,
}
