//! 版本化参考分布 fixture（quality 评判参照系规范：参照系是数据，不是代码）。
//!
//! fixture 是"分布描述"（区间/分位数），未来接入 play-by-play 真实数据时
//! 替换进同一结构，评判代码不改。变更属于校准行为（design 迭代规范）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const NBA_FIXTURE_JSON: &str = include_str!("../fixtures/nba.v1.json");
pub const NBA_V2_FIXTURE_JSON: &str = include_str!("../fixtures/nba.v2.json");
pub const FIBA_FIXTURE_JSON: &str = include_str!("../fixtures/fiba.v1.json");

/// 闭区间带（fixture 以 [min, max] 表达）。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Band {
    pub min: f32,
    pub max: f32,
}

impl Band {
    pub fn contains(&self, v: &f32) -> bool {
        v >= &self.min && v <= &self.max
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceDistributions {
    pub version: String,
    pub league: String,
    pub court_width_ft: f32,
    pub court_height_ft: f32,
    /// 传球走廊半径（ft）——与 GameRules.pass_corridor_radius_ft 对齐。
    pub pass_corridor_radius_ft: f32,
    /// 进攻篮板后的进攻时钟重置（秒）。回合时长上界的自变量：每次进攻篮板
    /// 重置时钟，因此 n 个进攻篮板的回合会多出 n-1 个窗口（gap.md §15.5
    /// 数据契约字段，不得在评判代码里硬编码）。
    pub offensive_rebound_shot_clock_seconds: f32,
    /// 回合时长上界的程序开销容忍量（秒）：死球、罚球、发球等非进攻时钟
    /// 时间。属数据契约，随联赛标定。
    pub duration_tolerance_seconds: f32,
    /// 回合时长带（秒），按结果类。
    pub duration_bands: OutcomeBands,
    /// 传球数带，按结果类。
    pub passes_bands: OutcomeBands,
    /// 比赛级失误率带（每回合）。
    pub turnover_rate_band: Band,
    /// 三分出手占比带。
    pub three_attempt_rate_band: Band,
    /// 重兵盯防（contest 高于阈值）出手视为缺陷条目。
    pub heavy_contest_threshold: f32,
    /// 阶段停留时长上限（秒）；未列出的阶段不做停留裁决。
    pub phase_dwell_max_seconds: HashMap<String, f32>,
    /// 允许的阶段转换边（from -> [to...]）；未登记的阶段不裁决。
    pub phase_transitions: HashMap<String, Vec<String>>,
    /// D2 比赛级构成准则参考带（dev 方案 §5.1）。
    /// v1 fixture 无此字段，`#[serde(default)]` 保证向后兼容——
    /// 缺省时构成准则判 `InsufficientEvidence` 而非 pass。
    #[serde(default)]
    pub composition_bands: Option<CompositionBands>,
}

/// 比赛级动作构成参考带（dev 方案 §5.1）。
/// 每条带都是"必要条件的回归网"——进带不庆祝，出带必报警；
/// 联合分布与情境条件分布不在覆盖范围（见 fixtures/blind_spots.md）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompositionBands {
    /// 数据来源标注：`prior`（先验带宽）/ `pbp`（真实 PBP 标定）。
    /// 报告按来源区分准则权重；禁止以模拟输出反标定（§5.2）。
    pub provenance: String,
    #[serde(default)]
    pub note: String,
    pub three_attempt_rate: Band,
    pub two_attempt_rate: Band,
    /// 中距离出手占 FGA 比例（中距离回归的直接证据，§6.2 D3.2）。
    pub mid_range_share_of_fga: Band,
    /// 篮下出手占 FGA 比例。
    pub rim_share_of_fga: Band,
    /// 罚球率 FTA/FGA。
    pub free_throw_rate: Band,
    /// 48 分钟等效回合数。
    pub pace_possessions_per_48min: Band,
    /// 三分命中率。
    pub three_make_pct: Band,
    /// 两分命中率。
    pub two_make_pct: Band,
    /// 助攻率 AST/FGM（事件有 AST 载荷才启用；无则登记缺口禁止伪造）。
    pub assist_rate: Band,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutcomeBands {
    pub score: Band,
    pub rebound: Band,
    pub turnover: Band,
}

impl ReferenceDistributions {
    pub fn nba_v1() -> Self {
        serde_json::from_str(NBA_FIXTURE_JSON).expect("embedded nba.v1 fixture must parse")
    }

    /// D2：带构成准则参考带的 NBA 档案（评判器默认消费此版本）。
    pub fn nba_v2() -> Self {
        serde_json::from_str(NBA_V2_FIXTURE_JSON).expect("embedded nba.v2 fixture must parse")
    }

    pub fn fiba_v1() -> Self {
        serde_json::from_str(FIBA_FIXTURE_JSON).expect("embedded fiba.v1 fixture must parse")
    }

    pub fn for_league(league_name: &str) -> Self {
        match league_name.to_ascii_uppercase().as_str() {
            "FIBA" => Self::fiba_v1(),
            // D2：NBA 默认消费带构成准则参考带的 v2 档案。
            _ => Self::nba_v2(),
        }
    }

    /// 终端事件 → 结果类（score/rebound/turnover）。
    pub fn classify_outcome(&self, terminal_event: &str) -> &'static str {
        if terminal_event == "SCORE" {
            "score"
        } else if terminal_event.starts_with("TURNOVER") {
            "turnover"
        } else {
            "rebound"
        }
    }

    pub fn duration_band(&self, outcome: &str) -> Option<Band> {
        match outcome {
            "score" => Some(self.duration_bands.score),
            "turnover" => Some(self.duration_bands.turnover),
            "rebound" => Some(self.duration_bands.rebound),
            _ => None,
        }
    }

    pub fn passes_band(&self, outcome: &str) -> Option<Band> {
        match outcome {
            "score" => Some(self.passes_bands.score),
            "turnover" => Some(self.passes_bands.turnover),
            "rebound" => Some(self.passes_bands.rebound),
            _ => None,
        }
    }

    pub fn phase_dwell_band(&self, phase: &str) -> Option<(f32, f32)> {
        self.phase_dwell_max_seconds
            .get(phase)
            .map(|max| (0.0, *max))
    }

    pub fn phase_transition_allowed(&self, from: &str, to: &str) -> bool {
        match self.phase_transitions.get(from) {
            Some(targets) => targets.iter().any(|t| t == to),
            None => true,
        }
    }
}
