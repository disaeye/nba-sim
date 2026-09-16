//! 应用层 (Application Layer)：比赛引擎编排。
//!
//! 唯一同时认识所有子系统的层。`MatchEngine::step()` 只做调度：
//! 时钟推进 → 世界级约束检查 → 阶段状态机 → 决策执行 → 物理步进 →
//! 弹道裁决 → 阶段转换 → 协议输出。业务规则全部下沉到各子系统 crate。

pub mod match_engine;
pub mod service;
pub mod setup;
pub mod snapshot;

pub use service::{MatchInfo, MatchService, SessionState};

pub use setup::{LineupConfig, MatchSetup};
pub use snapshot::{BallStateView, EngineSnapshot, GameStateView, LineupStateView};

pub use match_engine::{
    frame_rules_from_game_rules, ExportSummary, MatchBoxScore, MatchEngine, Simulation, StreamMode,
};
