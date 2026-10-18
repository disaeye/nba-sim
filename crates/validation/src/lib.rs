//! 五层验证基础设施（validation.md）。
//!
//! 验证器只读：从引擎流独立重建比赛语义，输出四态裁决。重建与判断
//! 不调用生产结算函数；场景执行经由引擎公开入口。

pub mod artifacts;
pub mod composition;
pub mod core_validation;
pub mod mechanism_core;
pub mod criteria;
pub mod joint;
pub mod manifest;
pub mod mech;
pub mod mech_runner;
pub mod mechanism;
pub mod movement_tact;
pub mod opportunity;
pub mod profile;
pub mod reconstruct;
pub mod reference;
pub mod rule_coverage;
pub mod runner;
pub mod sampling_coverage;
pub mod scenario;
pub mod stat_batch;
pub mod stats;
pub mod stream;
pub mod verdict;

pub use criteria::{Criterion, CriterionRegistry, Family};
pub use manifest::{PopulationSpec, ReferenceManifest, UnitsDeclaration, ValidationManifest};
pub use profile::{BasketballRuleProfile, RuleEntry, RuleReviewStatus, ScoringProfile};
pub use verdict::{CompletionState, EvidenceSummary, GateClass, Verdict};
