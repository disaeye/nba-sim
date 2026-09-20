//! 对外值类型：比赛箱体统计与作用域导出摘要。
//!
//! 这两个类型是引擎对外的**数据格式**（与 `protocol` 的帧结构并列）：它们不持有
//! 比赛真相，只是某次运行的结果快照。放在单独模块是为了让「外部能看到什么」
//! 与 `MatchEngine` 的状态字段分开审阅。

use nba_invariants::{Violation, ViolationTaxonomy};

/// 比赛投篮分解与核心统计。
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MatchBoxScore {
    pub fg2_attempts: u32,
    pub fg2_made: u32,
    pub fg3_attempts: u32,
    pub fg3_made: u32,
    pub ft_attempts: u32,
    pub ft_made: u32,
    pub turnovers: u32,
    pub fouls: u32,
}

impl MatchBoxScore {
    pub fn fg2_pct(&self) -> f32 {
        if self.fg2_attempts == 0 {
            0.0
        } else {
            self.fg2_made as f32 / self.fg2_attempts as f32
        }
    }
    pub fn fg3_pct(&self) -> f32 {
        if self.fg3_attempts == 0 {
            0.0
        } else {
            self.fg3_made as f32 / self.fg3_attempts as f32
        }
    }
    pub fn ft_pct(&self) -> f32 {
        if self.ft_attempts == 0 {
            0.0
        } else {
            self.ft_made as f32 / self.ft_attempts as f32
        }
    }
}

/// Summary of a scoped simulation export.
#[derive(Debug)]
pub struct ExportSummary {
    /// Number of ticks written to the stream.
    pub ticks: usize,
    /// Human-readable scope description (e.g. "1 Quarter").
    pub scope_desc: String,
    /// All invariant violations detected across the run; empty means the
    /// simulation satisfied every physical/basketball rule that was checked.
    pub violations: Vec<Violation>,
    /// 比赛统计数据分解。
    pub box_score: MatchBoxScore,
    /// 违例多维分类账分析结果。
    pub taxonomy: ViolationTaxonomy,
}
