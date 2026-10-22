//! 动作链内核的验证基础设施。
//!
//! 验证只读：判定从引擎报告与事实流重建，不调用生产结算函数。四层判定
//! （几何、语境、分布、反事实）加箱体守恒；机制实验以种子为抽样单元。
//! 契约见 `docs/validation.md`。

pub mod core_validation;
pub mod five_dimension;
pub mod mechanism_core;
