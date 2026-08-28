//! 领域层 (Domain Layer)：篮球比赛本体词汇表。
//!
//! 只描述"比赛是什么"(场地、时钟、阶段、事件、动作窗口)，
//! 不描述"比赛怎么打"(决策/物理/裁决均在外层)。
//! 本 crate 是依赖图的最内层：除 glam 数学类型与 serde 派生外零依赖。

pub mod action_window;
pub mod court;
pub mod event;
pub mod possession;

pub use action_window::{ActionPhase, ActionTimeWindow, ActionType};
pub use event::PhysicsEvent;
pub use possession::{Possession, SubPhase};
