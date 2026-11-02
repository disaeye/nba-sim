//! 比赛程序：暂停、换人与轮换。
//!
//! 跳球、发球、罚球、复核与节末的结算在 `world` 与 `resolve` 中，
//! 这里放独立的程序模块。

pub mod rotation;
pub mod timeouts;
