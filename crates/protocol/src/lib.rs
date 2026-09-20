//! 输出格式 (Wire Protocol)。
//!
//! 25Hz NDJSON 流的帧 DTO 与序列化格式。
//! 只依赖 serde —— 任何消费者（前端/回放器/训练管道）按此格式解码，
//! 与引擎内部结构完全解耦。

pub mod frame;

pub use frame::{
    DebugFlag, DebugProb, DebugUtility, DecisionDebug, FrameEvent, FrameRules, RenderBall,
    RenderFrame, RenderPlayer, RenderScore, RenderTeam, StreamTick,
};
