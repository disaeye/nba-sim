//! 应用层 (Application Layer)：比赛引擎编排。
//! 唯一同时认识所有子系统的层。`MatchEngine::step()` 纯调度管线。

pub mod match_engine;
pub mod service;
pub mod setup;
pub mod snapshot;
pub mod world;

pub use service::{MatchInfo, MatchService, SessionState};

pub use setup::{LineupConfig, MatchSetup};
pub use snapshot::{BallStateView, EngineSnapshot, GameStateView, LineupStateView};

pub use match_engine::{
    frame_rules_from_game_rules, ExportSummary, MatchBoxScore, MatchEngine, Simulation, StreamMode,
};
