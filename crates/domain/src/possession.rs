//! 球权与比赛阶段：领域状态机词汇。

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

/// 领域权威球权归属状态机（docs/architecture.md §3 球权模型）。
///
/// 严格区分"球权归属"（谁控制球/谁是进攻方）与物理层"弹道轨迹"（抛物线坐标插值）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BallOwnership {
    /// 明确由某球员双手持球控制。
    Held { carrier_id: String },
    /// 投篮或传球飞行中（无活球持球人，原出手/传球者在飞行完成前保持关联）。
    InFlight { origin_player_id: String },
    /// 活球争抢/脱手/篮板下落状态（无明确持有人）。
    Loose,
    /// 死球状态（出界、进球后、违例、判罚停表）。
    Dead,
    /// 发球准备状态（发球员站在界外待发球）。
    InboundSetup { inbounder_id: String },
}

impl BallOwnership {
    /// 判断从当前状态到目标状态的转移是否符合篮球比赛因果公理。
    pub fn can_transition_to(&self, next: &BallOwnership) -> bool {
        match (self, next) {
            // 相同状态允许自旋刷新（如更新持球人坐标）
            (a, b) if a == b => true,

            // Held 可以转移到：传球/投篮飞行、脱手 Loose、死球 Dead
            (BallOwnership::Held { .. }, BallOwnership::InFlight { .. }) => true,
            (BallOwnership::Held { .. }, BallOwnership::Loose) => true,
            (BallOwnership::Held { .. }, BallOwnership::Dead) => true,
            // 允许抢断或直接手递手传球到新的 Held
            (BallOwnership::Held { .. }, BallOwnership::Held { .. }) => true,

            // InFlight 可以转移到：接球/抢断 Held、篮板弹地 Loose、进球/出界 Dead
            (BallOwnership::InFlight { .. }, BallOwnership::Held { .. }) => true,
            (BallOwnership::InFlight { .. }, BallOwnership::Loose) => true,
            (BallOwnership::InFlight { .. }, BallOwnership::Dead) => true,

            // Loose 争抢可以转移到：控制住球 Held、争抢出界/犯规 Dead
            (BallOwnership::Loose, BallOwnership::Held { .. }) => true,
            (BallOwnership::Loose, BallOwnership::Dead) => true,

            // Dead 死球可以转移到：发球准备 InboundSetup、活球争球 Loose（如跳球）
            (BallOwnership::Dead, BallOwnership::InboundSetup { .. }) => true,
            (BallOwnership::Dead, BallOwnership::Loose) => true,
            (BallOwnership::Dead, BallOwnership::Held { .. }) => true, // 罚球直接持球

            // InboundSetup 发球准备可以转移到：发球飞行 InFlight、发球成功 Held、发球超时 Dead
            (BallOwnership::InboundSetup { .. }, BallOwnership::InFlight { .. }) => true,
            (BallOwnership::InboundSetup { .. }, BallOwnership::Held { .. }) => true,
            (BallOwnership::InboundSetup { .. }, BallOwnership::Dead) => true,

            // 其他跨越因果律的跳跃均视为非法
            _ => false,
        }
    }

    /// 获取当前处于确定控制状态的持球人 ID。
    pub fn carrier_id(&self) -> Option<&str> {
        match self {
            BallOwnership::Held { carrier_id } => Some(carrier_id.as_str()),
            BallOwnership::InboundSetup { inbounder_id } => Some(inbounder_id.as_str()),
            _ => None,
        }
    }
}
