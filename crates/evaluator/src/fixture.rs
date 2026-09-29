//! 版本化参考分布 fixture（quality 评判参照系规范：参照系是数据，不是代码）。
//!
//! fixture 是"分布描述"（区间/分位数），未来接入 play-by-play 真实数据时
//! 替换进同一结构，评判代码不改。变更属于校准行为（design 迭代规范）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const NBA_FIXTURE_JSON: &str = include_str!("../fixtures/nba.v1.json");
pub const NBA_V2_FIXTURE_JSON: &str = include_str!("../fixtures/nba.v2.json");
pub const NBA_V3_FIXTURE_JSON: &str = include_str!("../fixtures/nba.v3.json");
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
    /// 传球走廊半径（ft）——取自 GameRules.pass_corridor_radius_ft。
    pub pass_corridor_radius_ft: f32,
    /// 进攻篮板后的进攻时钟重置（秒）。回合时长上界的自变量：每次进攻篮板
    /// 重置时钟，因此 n 个进攻篮板的回合会多出 n-1 个窗口（gap.md §15.5
    /// 数据规格字段，不得在评判代码里硬编码）。
    pub offensive_rebound_shot_clock_seconds: f32,
    /// 回合时长上界的程序开销容忍量（秒）：死球、罚球、发球等非进攻时钟
    /// 时间。属数据规格，随联赛标定。
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
    /// v3 已登记来源与统计口径的联合和情境参考带。
    #[serde(default)]
    pub joint_situational_bands: Option<JointSituationalBands>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointSituationalBands {
    pub provenance: ReferenceMetadata,
    pub shot_zone_make: ZoneMakeBands,
    pub q4_late_three_attempt_share: ClutchThreeAttemptBands,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceMetadata {
    pub source: String,
    pub source_version: String,
    pub season: String,
    pub games: usize,
    pub attempts: usize,
    pub calculation: String,
    pub source_url: String,
    pub source_sha256: String,
    #[serde(default)]
    pub external_check: Option<ExternalCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalCheck {
    pub source: String,
    pub source_url: String,
    pub metric: String,
    pub value: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZoneMakeBands {
    pub minimum_attempts_per_zone: usize,
    pub source_minimum_attempts_per_zone: usize,
    pub source_season: String,
    pub three_point_distance_ft: f32,
    pub corner_three_distance_ft: f32,
    pub rim: Band,
    pub near: Band,
    pub mid: Band,
    pub three: Band,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClutchThreeAttemptBands {
    pub late_window_seconds: f32,
    pub minimum_attempts_per_window: usize,
    pub source_minimum_late_attempts: usize,
    pub source_minimum_early_attempts: usize,
    pub delta: Band,
    pub interval_method: String,
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
    /// 中距离出手占 FGA 比例（≥ 14 ft 且三分线内；中距离回归的直接证据，§6.2 D3.2）。
    /// v1 fixture 无此带时准则按 NotApplicable 处理（向后兼容）。
    pub mid_range_share_of_fga: Option<Band>,
    /// 近筐出手占 FGA 比例（5–14 ft，attributes.md §2.3a）。
    pub near_range_share_of_fga: Option<Band>,
    /// 篮下出手占 FGA 比例（< 5 ft）。
    pub rim_share_of_fga: Option<Band>,
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

    pub fn nba_v3() -> Self {
        let fixture: Self =
            serde_json::from_str(NBA_V3_FIXTURE_JSON).expect("embedded nba.v3 fixture must parse");
        fixture.validate_joint_situational_bands();
        fixture
    }

    pub fn fiba_v1() -> Self {
        serde_json::from_str(FIBA_FIXTURE_JSON).expect("embedded fiba.v1 fixture must parse")
    }

    pub fn for_league(league_name: &str) -> Self {
        match league_name.to_ascii_uppercase().as_str() {
            "FIBA" => Self::fiba_v1(),
            // NBA 默认档案仍为 nba.v2，待 ADR-009 验收后再切换。
            _ => Self::nba_v2(),
        }
    }

    fn validate_joint_situational_bands(&self) {
        let Some(bands) = &self.joint_situational_bands else {
            return;
        };
        assert_eq!(
            self.version, "nba.v3",
            "joint reference fixture version mismatch"
        );
        assert_eq!(
            self.league, "NBA",
            "joint reference fixture league mismatch"
        );
        assert_eq!(
            bands.provenance.games, 1230,
            "reference game count must match archive manifest"
        );
        assert_eq!(
            bands.provenance.season, "2023-24 NBA regular season",
            "reference season must match official independent benchmark"
        );
        assert_eq!(
            bands.provenance.attempts, 218701,
            "reference attempt count must match archive manifest"
        );
        assert!(
            !bands.provenance.source.is_empty(),
            "reference source is required"
        );
        assert!(
            !bands.provenance.source_version.is_empty(),
            "source version is required"
        );
        assert!(
            !bands.provenance.season.is_empty(),
            "reference season is required"
        );
        assert!(
            bands.provenance.games > 0,
            "reference game count must be positive"
        );
        assert!(
            bands.provenance.attempts > 0,
            "reference attempt count must be positive"
        );
        assert!(
            !bands.provenance.calculation.is_empty(),
            "reference calculation is required"
        );
        let benchmark = bands
            .provenance
            .external_check
            .as_ref()
            .expect("independent benchmark metadata is required");
        assert!(
            benchmark.source_url.starts_with("https://"),
            "independent benchmark URL must be HTTPS"
        );
        assert!(
            !benchmark.source.is_empty()
                && !benchmark.metric.is_empty()
                && benchmark.value.is_finite()
                && (0.0..=1.0).contains(&benchmark.value),
            "independent benchmark metadata is required"
        );
        assert!(
            bands.provenance.source_url.starts_with("https://"),
            "reference URL must be HTTPS"
        );
        assert_eq!(
            bands.provenance.source_sha256.len(),
            64,
            "source SHA-256 must be complete"
        );
        let zone = &bands.shot_zone_make;
        assert!(
            zone.minimum_attempts_per_zone > 0
                && zone.source_minimum_attempts_per_zone > 0
                && zone.minimum_attempts_per_zone <= zone.source_minimum_attempts_per_zone
                && zone.source_season == bands.provenance.season
                && bands
                    .provenance
                    .source_version
                    .contains("shotdetail_2023.tar.xz"),
            "zone source sample coverage must support the evaluator threshold"
        );
        assert!(
            zone.source_minimum_attempts_per_zone <= bands.provenance.attempts,
            "zone source minimum exceeds total reference attempts"
        );
        assert!(
            zone.three_point_distance_ft > 0.0,
            "three-point distance must be positive"
        );
        assert!(
            zone.corner_three_distance_ft > 0.0
                && zone.corner_three_distance_ft < zone.three_point_distance_ft,
            "corner three distance must be valid"
        );
        for band in [&zone.rim, &zone.near, &zone.mid, &zone.three] {
            assert!(
                band.min >= 0.0 && band.max <= 1.0 && band.min < band.max,
                "invalid shot make-rate band"
            );
        }
        let clutch = &bands.q4_late_three_attempt_share;
        assert!(
            !clutch.interval_method.is_empty(),
            "clutch interval method is required"
        );
        assert!(
            clutch.late_window_seconds > 0.0,
            "late-game window must be positive"
        );
        assert!(
            clutch.minimum_attempts_per_window > 0
                && clutch.source_minimum_late_attempts > 0
                && clutch.source_minimum_early_attempts > 0
                && clutch.minimum_attempts_per_window <= clutch.source_minimum_late_attempts
                && clutch.minimum_attempts_per_window <= clutch.source_minimum_early_attempts
                && clutch.source_minimum_late_attempts <= bands.provenance.attempts
                && clutch.source_minimum_early_attempts <= bands.provenance.attempts,
            "Q4 source sample coverage must support the evaluator threshold"
        );
        assert!(
            clutch.delta.min >= -1.0
                && clutch.delta.max <= 1.0
                && clutch.delta.min < clutch.delta.max,
            "invalid clutch shot-rate difference band"
        );
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
