//! 子命令实现：每个子命令一个模块，`main.rs` 负责参数解析与分派。
//!
//! 拆分依据：子命令彼此独立，只经 `main` 的 argv 分派相连；
//! 公共辅助（参数解析、工件落盘、Hard 门）留在 `main.rs`，
//! 因为每个子命令都要用它们。

pub(crate) mod batch;
pub(crate) mod convert;
pub(crate) mod simulate;
