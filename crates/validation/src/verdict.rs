//! 四态裁决模型（validation.md §1.2–§1.3）。
//!
//! `Verdict` 四态与三条完成状态是本仓库验证体系的固定词表；gate_class
//! 决定一条缺陷阻断哪一类认证。综合指数不得抵消失败或未判定。

use serde::{Deserialize, Serialize};

/// 裁决四态（validation.md §1.2）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// 证据充分且符合判据。
    Pass,
    /// 充分证据证明不符合判据。
    Defect,
    /// 适用条件明确为假，且检查范围完整。
    NotApplicable,
    /// 字段、参考、样本量或检验精度不足，或统计区间无法决定等效性。
    InsufficientEvidence,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Defect => "defect",
            Self::NotApplicable => "not_applicable",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }
}

/// 门类（validation.md §1.3）：前三类分别阻断对应认证，`Diagnostic`
/// 只说明观察事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateClass {
    Integrity,
    Mechanism,
    Distribution,
    Diagnostic,
}

/// 三个完成状态（validation.md §1.3）。任何必需项的
/// `InsufficientEvidence` 都阻止对应状态成立。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompletionState {
    /// 登记的检查、解析、重建、场景、工件和 CLI 均实现并测试通过。
    ValidatorImplemented,
    /// 必需结构场景、全场结构检查和机制实验通过。
    EngineConformant,
    /// 指定联赛、规则版本、阵容范围下第 3、5 层全部必需参考与判据通过。
    RealismValidated,
}

impl CompletionState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ValidatorImplemented => "validator_implemented",
            Self::EngineConformant => "engine_conformant",
            Self::RealismValidated => "realism_validated",
        }
    }
}

/// 一次评估的证据汇总（validation.md §1.2 触发机会等式）。
///
/// 触发机会数 = `passes + defects + insufficient`；`not_applicable`
/// 单列，不进入分母。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EvidenceSummary {
    pub opportunities: u64,
    pub passes: u64,
    pub defects: u64,
    pub not_applicable: u64,
    pub insufficient: u64,
}

impl EvidenceSummary {
    pub fn record(&mut self, verdict: Verdict) {
        match verdict {
            Verdict::Pass => self.passes += 1,
            Verdict::Defect => self.defects += 1,
            Verdict::NotApplicable => self.not_applicable += 1,
            Verdict::InsufficientEvidence => self.insufficient += 1,
        }
        if verdict != Verdict::NotApplicable {
            self.opportunities += 1;
        }
    }

    /// 证据覆盖率（validation.md §1.2）：有裁决的机会占触发机会的份额。
    pub fn decided_ratio(&self) -> f64 {
        if self.opportunities == 0 {
            return 0.0;
        }
        let decided = self.passes + self.defects;
        decided as f64 / self.opportunities as f64
    }
}
