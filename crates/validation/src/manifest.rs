//! 验证清单与参考清单（validation.md §3.1 / §6.1）。
//!
//! `ValidationManifest` 固定一次认证运行的输入包摘要、代码版本、随机
//! 算法、种子、scope、准则集合摘要与预算；`ReferenceManifest` 固定参考
//! 数据的联赛、版本、来源与审核人；`PopulationSpec` 声明模拟阵容集合。

use serde::{Deserialize, Serialize};

/// 数值单位与坐标声明（validation.md §3.1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnitsDeclaration {
    /// 长度单位（`feet`）。
    pub length: String,
    /// 时间单位（`seconds`）。
    pub time: String,
    /// 坐标方向（`x_along_baseline_left_to_right` 等自描述串）。
    pub coordinates: String,
    /// 固定步长（秒）。
    pub tick_seconds: f32,
}

/// 一次认证运行的完整清单（validation.md §3.1）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationManifest {
    /// 清单 schema 版本。
    pub manifest_version: String,
    /// 输入包摘要（引擎流文件 sha256）。
    pub input_digest: String,
    /// 引擎代码版本（git commit 或构建标识）。
    pub code_version: String,
    /// 随机算法版本（如 `chacha8`）。
    pub rng_algorithm: String,
    pub seed: u64,
    /// 比赛范围（`full` / 缩短范围描述）。
    pub scope: String,
    /// 协议版本。
    pub protocol_version: String,
    /// 参与判定的准则集合摘要（编号排序列表的 sha256）。
    pub criteria_digest: String,
    /// 参考清单摘要（无参考时为空串）。
    #[serde(default)]
    pub reference_digest: String,
    pub units: UnitsDeclaration,
    /// 资源预算：最大 tick 数。
    pub budget_max_ticks: u64,
    /// 资源预算：最大字节数。
    pub budget_max_bytes: u64,
    /// 预期比赛数（批次运行）。
    pub expected_games: u32,
    /// 认证范围声明（哪些联赛、阵容、规则版本）。
    pub certification_scope: String,
}

/// 参考数据清单（validation.md §6.1）：固定来源、版本与审核人。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReferenceManifest {
    pub reference_id: String,
    pub league: String,
    /// 规则年份。
    pub rule_year: String,
    /// 赛事类别（`regular_season` 等）。
    pub competition: String,
    /// 比赛名单来源与数量。
    pub game_count: u32,
    /// 数据许可说明。
    pub license: String,
    /// 数据文件摘要（sha256）。
    pub data_sha256: String,
    /// 获取与转换程序版本。
    pub pipeline_version: String,
    /// 缺失率（0..1）。
    pub missing_rate: f64,
    /// 坐标映射描述（参考坐标系到仿真坐标系的变换）。
    pub coordinate_mapping: String,
    /// 统计定义版本。
    pub stats_definition: String,
    /// 审核人（角色，非真名）。
    pub reviewer: String,
}

impl ReferenceManifest {
    /// 数据依赖是否满足正式验收要求。
    pub fn is_complete(&self) -> bool {
        self.reviewer != "none" && self.data_sha256.len() == 64
    }
}

/// 模拟人群声明（validation.md §6.1）：阵容集合、权重与联赛范围。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PopulationSpec {
    pub population_id: String,
    /// 阵容集合标识列表（指向引擎 setup 档案）。
    pub lineup_ids: Vec<String>,
    /// 阵容抽样权重（与 `lineup_ids` 等长，和为 1）。
    pub lineup_weights: Vec<f64>,
    /// 对手配对规则描述。
    pub pairing: String,
    /// 球员能力映射方式（`identity` / 经审核的映射表标识）。
    pub ability_mapping: String,
    /// 联赛范围。
    pub league_scope: String,
}

impl PopulationSpec {
    /// 权重是否合规（等长、和为 1）。
    pub fn weights_valid(&self) -> bool {
        self.lineup_ids.len() == self.lineup_weights.len()
            && (self.lineup_weights.iter().sum::<f64>() - 1.0).abs() < 1e-9
    }
}
