//! 裁决子系统 (Officiating Subsystem)。
//!
//! 对抗性结果的概率裁决：传球拦截、篮板归属、接触犯规。
//! 输入物理事实和领域事件，输出裁决结果 (ResolutionOutcome)。

pub mod resolution;

pub use resolution::{ResolutionLayer, ResolutionOutcome};
