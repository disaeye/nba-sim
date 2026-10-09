//! 独立篮球语义档案（validation.md §2.1）。
//!
//! `BasketballRuleProfile` 描述联赛比赛规则，`ScoringProfile` 描述官方
//! 技术统计及回合口径。每条规则记录出处与审核状态；只有 `approved`
//! 的规则可参与正式验收。

use serde::{Deserialize, Serialize};

/// 规则审核状态。非 `approved` 的条目不能进入正式验收。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleReviewStatus {
    /// 已有独立来源案例与边界案例，审核通过。
    Approved,
    /// 有草案但尚未完成审核；只能用于诊断模式。
    Draft,
    /// 仅有条目名称，依据缺失；阻止规则认证。
    Missing,
}

impl RuleReviewStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approved => "approved",
            Self::Draft => "draft",
            Self::Missing => "missing",
        }
    }
}

/// 一条联赛规则的登记条目。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleEntry {
    /// 稳定规则编号，如 `nba-shot-clock-24`。
    pub rule_id: String,
    /// 联赛标识（`nba` / `fiba` / 自定义范围）。
    pub league: String,
    /// 规则版本（年份或档案版本）。
    pub rule_version: String,
    /// 出处条款（规则书的章节引用）。
    pub source_clause: String,
    /// 解释案例：条款如何映射到仿真可检查事实。
    pub interpretation_cases: Vec<String>,
    pub review_status: RuleReviewStatus,
}

impl RuleEntry {
    pub fn is_approved(&self) -> bool {
        self.review_status == RuleReviewStatus::Approved
    }
}

/// 联赛比赛规则档案（validation.md §2.1）。
///
/// 档案本身是规则条目的集合；正式认证要求全部必需条目 `approved`。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasketballRuleProfile {
    pub profile_id: String,
    pub league: String,
    pub rule_version: String,
    pub entries: Vec<RuleEntry>,
}

impl BasketballRuleProfile {
    /// 构建空档案（条目随后按联赛规则逐条登记）。
    pub fn new(profile_id: &str, league: &str, rule_version: &str) -> Self {
        Self {
            profile_id: profile_id.to_string(),
            league: league.to_string(),
            rule_version: rule_version.to_string(),
            entries: Vec::new(),
        }
    }

    pub fn register(&mut self, entry: RuleEntry) {
        self.entries.push(entry);
    }

    /// 全部必需条目是否审核通过（正式验收前置条件）。
    pub fn all_approved(&self) -> bool {
        self.entries.iter().all(RuleEntry::is_approved)
    }

    /// 阻塞正式验收的未审核条目编号。
    pub fn blocking_ids(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| !entry.is_approved())
            .map(|entry| entry.rule_id.clone())
            .collect()
    }
}

/// 官方技术统计与回合口径档案（validation.md §2.1/§2.3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoringProfile {
    pub profile_id: String,
    pub league: String,
    pub stats_version: String,
    /// 投篮犯规未中是否计 FGA（league 口径差异点之一）。
    pub counts_fga_on_shooting_foul_miss: bool,
    /// 进攻犯规使投篮无效时是否撤销 FGA。
    pub cancels_fga_on_offensive_foul: bool,
    /// 加时是否计入常规赛分钟分母（Pace 口径）。
    pub overtime_in_pace_minutes: bool,
    /// 附加技术罚球是否建立新的普通回合。
    pub technical_ft_starts_new_possession: bool,
}

impl ScoringProfile {
    /// NBA 官方技术统计口径基线。
    pub fn nba() -> Self {
        Self {
            profile_id: "scoring-nba-v1".to_string(),
            league: "nba".to_string(),
            stats_version: "2023-24".to_string(),
            counts_fga_on_shooting_foul_miss: true,
            cancels_fga_on_offensive_foul: true,
            overtime_in_pace_minutes: true,
            technical_ft_starts_new_possession: false,
        }
    }
}
