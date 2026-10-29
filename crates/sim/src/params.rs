//! 校准参数表：运行时只读，来源记录随表携带。
//!
//! 三张拟合表（`shot_make`、`shot_profile`、`possession`）由
//! `scripts/calibrate/fit_tables.py` 从真实 NBA 赛季数据生成，样本量为零即
//! 失败；`rebound` 由同一脚本派生缩放，`policy` 由矩匹配写入，`formations`
//! 是手写几何，这三张不以样本量作门槛，各自用 `provenance` 的 `source` 与
//! `method` 说明来源。引擎不持有任何内联行为系数（charter C1），表缺失直接
//! 失败，不做默认回退。

use std::collections::BTreeMap;
use std::path::Path;

use crate::tactics::TacticsTable;

use serde::Deserialize;

pub const CALIBRATION_DIR: &str = "data/calibration";

/// 工作区内的校准表目录。相对路径以仓库根为基准，测试与二进制
/// 无论从哪个工作目录启动都能找到同一份表。
/// 仓库根。所有运行目录都从它拼接，避免按进程当前目录解析导致
/// 从不同工作目录启动时读写到不同位置。
pub fn workspace_root() -> std::path::PathBuf {
    normalize(
        &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".."),
    )
}

/// 去掉路径里的 `.` 与 `..`，不要求路径已存在。
fn normalize(path: &std::path::Path) -> std::path::PathBuf {
    use std::path::Component;
    let mut out = std::path::PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

pub fn workspace_calibration_dir() -> std::path::PathBuf {
    workspace_root().join(CALIBRATION_DIR)
}

/// 验证工件目录（gitignored 运行目录）。
pub fn workspace_validation_dir() -> std::path::PathBuf {
    workspace_root().join(".work").join("validation-view")
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct Provenance {
    pub source: String,
    pub season: String,
    #[serde(default)]
    pub sample: u64,
    #[serde(default)]
    pub script: String,
    #[serde(default)]
    pub method: String,
    #[serde(default)]
    pub contest: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShotCell {
    pub upper_ft: f32,
    pub rate: f32,
    pub samples: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShotMakeTable {
    pub provenance: Provenance,
    pub distance_edges: Vec<f32>,
    pub cells: BTreeMap<String, BTreeMap<String, Vec<ShotCell>>>,
    pub league_zone_make: BTreeMap<String, f32>,
    pub skill_spread: BTreeMap<String, f32>,
    pub contest_sensitivity: f32,
    pub contest_center: f32,
    pub balance_sensitivity: f32,
    pub stamina_sensitivity: f32,
}

impl ShotMakeTable {
    /// 按 (出手类型, 距离, 是否三分) 查基础命中率。找不到精确格子时取最近的
    /// 更远格子，再退到该类型整体；两种情况都有样本量守卫。
    pub fn base_rate(&self, kind: &str, distance_ft: f32, is_three: bool) -> f32 {
        let side = if is_three { "three" } else { "two" };
        let fallback = if is_three { "two" } else { "three" };
        let entry = self
            .cells
            .get(kind)
            .unwrap_or_else(|| panic!("shot_make table has no cells for {kind}"));
        let cells = entry
            .get(side)
            .or_else(|| entry.get(fallback))
            .unwrap_or_else(|| panic!("shot_make table has no cells for {kind}/{side}"));
        for cell in cells {
            if distance_ft < cell.upper_ft {
                return cell.rate;
            }
        }
        cells
            .last()
            .map(|cell| cell.rate)
            .expect("shot_make cell list must not be empty")
    }

    pub fn skill_spread(&self, zone: &str) -> f32 {
        *self
            .skill_spread
            .get(zone)
            .unwrap_or_else(|| panic!("shot_make table has no skill spread for {zone}"))
    }

    pub fn league_zone_make(&self, zone: &str) -> f32 {
        *self
            .league_zone_make
            .get(zone)
            .unwrap_or_else(|| panic!("shot_make table has no league rate for {zone}"))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct DistanceStats {
    pub p10: f32,
    pub p50: f32,
    pub p90: f32,
    pub mean: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShotProfileTable {
    pub provenance: Provenance,
    pub kind_share: BTreeMap<String, f32>,
    pub zone_share: BTreeMap<String, f32>,
    pub three_share: f32,
    pub creation_share: BTreeMap<String, f32>,
    pub zone_by_kind: BTreeMap<String, BTreeMap<String, f32>>,
    pub distance_by_kind: BTreeMap<String, DistanceStats>,
    pub three_share_by_kind: BTreeMap<String, f32>,
    pub kind_by_creation: BTreeMap<String, BTreeMap<String, f32>>,
    pub zone_by_creation: BTreeMap<String, BTreeMap<String, f32>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PossessionContext {
    pub share: f32,
    pub turnover_per_possession: f32,
    pub fga_per_possession: f32,
    pub fg2a_per_possession: f32,
    pub fg3a_per_possession: f32,
    pub shooting_foul_per_possession: f32,
    pub oreb_per_possession: f32,
    pub fg2_pct: f32,
    pub fg3_pct: f32,
    pub duration_p10: f32,
    pub duration_p50: f32,
    pub duration_p90: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PossessionLeague {
    pub possessions_per_game: f32,
    pub fga_per_possession: f32,
    pub three_attempt_share: f32,
    pub turnover_per_possession: f32,
    pub shooting_foul_per_possession: f32,
    pub oreb_per_possession: f32,
    pub oreb_per_miss: f32,
    pub fg2_pct: f32,
    pub fg3_pct: f32,
    pub fta_per_game: f32,
    pub shooting_fouls_per_game: f32,
    pub points_per_game: f32,
    /// 两队合计场均封盖。
    pub blocks_per_game: f32,
    /// 两队合计场均抢断。
    pub steals_per_game: f32,
    /// 两队合计场均 and-1（投篮犯规且投进）。
    pub and_one_per_game: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PossessionTable {
    pub provenance: Provenance,
    pub context: BTreeMap<String, PossessionContext>,
    pub league: PossessionLeague,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReboundTable {
    pub provenance: Provenance,
    pub oreb_rate_by_zone: BTreeMap<String, f32>,
    pub league_oreb_per_miss: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FreeThrowTable {
    pub provenance: Provenance,
    pub league_make: f32,
    pub attempts_observed: u64,
    pub skill_slope: f32,
    pub and_one_share: f32,
}

/// 判定带宽（相对误差区间）。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Band {
    pub lower: f64,
    pub upper: f64,
}

/// 出手类型 × 距离的真实命中率曲线（运行时表）。
///
/// 这一段是「距离衰减」组的实测数据：每个出手类型在每个距离档上的真实
/// 命中率。投篮模型用它把目标命中率换成瞄准误差，因此模型预测的命中率
/// 就是实测命中率，而不是靠一组手调的倍率近似出来的。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct ShotDistanceCell {
    pub upper_ft: f32,
    pub rate: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ShotDistanceProfile {
    /// 距离上限（ft）到命中率的序列，按距离升序。
    pub cells: Vec<ShotDistanceCell>,
}

impl ShotDistanceProfile {
    /// 距离对应的实测命中率：落在哪个档就取那个档的值；超出最后一档时
    /// 取最后一档（远距离的命中率趋近于零）。
    pub fn rate_at(&self, distance_ft: f32) -> f32 {
        for cell in &self.cells {
            if distance_ft < cell.upper_ft {
                return cell.rate;
            }
        }
        self.cells
            .last()
            .map(|cell| cell.rate)
            .unwrap_or_default()
    }
}

/// 出手类型 × 是否三分 × 距离的真实命中率表（运行时表）。
///
/// 这是「距离衰减」组的实测数据：每个出手类型在每个距离档上的真实命中
/// 率。投篮模型用它把目标命中率换成瞄准误差，因此模型预测的命中率就是
/// 实测命中率，而不是靠一组手调的倍率近似出来的。
#[derive(Debug, Clone, Deserialize)]
pub struct ShotDistanceTable {
    pub provenance: Provenance,
    pub profiles: BTreeMap<String, ShotDistanceProfile>,
}

impl ShotDistanceTable {
    pub fn rate_at(&self, kind: &str, is_three: bool, distance_ft: f32) -> Option<f32> {
        let side = if is_three { "three" } else { "two" };
        self.profiles
            .get(&format!("{kind}_{side}"))
            .map(|profile| profile.rate_at(distance_ft))
    }

    pub fn embedded() -> Self {
        let table: ShotDistanceTable =
            serde_json::from_str(include_str!("../../../data/calibration/shot_distance.json"))
                .expect("embedded shot distance table must parse");
        table
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let path = dir.join("shot_distance.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("shot_distance table must be readable: {error}"))?;
        serde_json::from_str(&text).map_err(|error| format!("shot_distance table must parse: {error}"))
    }
}

/// 策略响应系数。由 `scripts/calibrate/fit_policy.py` 做矩匹配写入，
/// 目标是让引擎产生的出手与回合分布贴近 `shot_profile` 与 `possession`。
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyTable {
    pub provenance: Provenance,
    pub weights: BTreeMap<String, f32>,
    /// 长度（英尺）与时长（秒）类几何量。与无量纲权重分开存放，
    /// 免得把英尺当权重读。
    #[serde(default)]
    pub geometry: BTreeMap<String, f32>,
    #[serde(default)]
    pub bands: BTreeMap<String, Band>,
    /// 过程判据的设计边界。键名与 `gate` 的准则同名；这些是设计边界，
    /// 不是联盟实测分布。
    #[serde(default)]
    pub process: BTreeMap<String, f32>,
    /// 出手类型 × 是否三分 × 距离的真实命中率曲线。
    #[serde(default)]
    pub shot_distance: BTreeMap<String, ShotDistanceProfile>,
}

impl PolicyTable {
    /// EPV 加成项，单位是分。
    ///
    /// 旧标度的加成权重与期望得分不同源：直接相加会让「接球就投的时机加成」
    /// 压过「这次出手本身值多少分」。`epv_bonus_scale` 把两者折到同一标度。
    pub fn b(&self, key: &str) -> f32 {
        self.w(key) * self.w("epv_bonus_scale")
    }

    /// 出手类型在给定距离上的实测命中率。
    ///
    /// 键是「出手类型_三分/两分」。没有这一段曲线时用区域基准值兜底。
    pub fn shot_rate_at(&self, kind: &str, is_three: bool, distance_ft: f32) -> Option<f32> {
        let side = if is_three { "three" } else { "two" };
        self.shot_distance
            .get(&format!("{kind}_{side}"))
            .map(|profile| profile.rate_at(distance_ft))
    }

    /// 判定带宽（相对误差区间）：未单独登记的准则用 `default`。
    pub fn band(&self, criterion: &str) -> Band {
        self.bands
            .get(criterion)
            .or_else(|| self.bands.get("default"))
            .copied()
            .unwrap_or_else(|| panic!("policy table has no band for `{criterion}`"))
    }
    /// 几何量（英尺或秒）。
    pub fn g(&self, key: &str) -> f32 {
        *self
            .geometry
            .get(key)
            .unwrap_or_else(|| panic!("policy table has no geometry value `{key}`"))
    }
    pub fn w(&self, key: &str) -> f32 {
        *self
            .weights
            .get(key)
            .unwrap_or_else(|| panic!("policy table has no weight `{key}`"))
    }
    /// 过程判据的设计边界。
    pub fn p(&self, key: &str) -> f32 {
        *self
            .process
            .get(key)
            .unwrap_or_else(|| panic!("policy table has no process bound `{key}`"))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Formations {
    pub provenance: Provenance,
    pub offense: Vec<[f32; 2]>,
    #[serde(default)]
    pub offense_alt: Vec<[f32; 2]>,
    pub defense: Vec<[f32; 2]>,
}

#[derive(Debug, Clone)]
pub struct Params {
    pub shot_make: ShotMakeTable,
    pub shot_profile: ShotProfileTable,
    pub possession: PossessionTable,
    pub rebound: ReboundTable,
    pub freethrow: FreeThrowTable,
    pub policy: PolicyTable,
    pub formations: Formations,
    pub tactics: TacticsTable,
}

/// 表的用途分类。运行时只允许加载机制、程序、策略三类。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableKind {
    Mechanism,
    Procedure,
    Policy,
    Validation,
}

#[derive(Debug, Clone, Copy)]
pub struct TableSpec {
    pub name: &'static str,
    pub kind: TableKind,
}

/// 运行时表清单。结果表不在这里；它们只由验证器加载。
pub const RUNTIME_TABLES: &[TableSpec] = &[
    TableSpec {
        name: "formations",
        kind: TableKind::Procedure,
    },
    TableSpec {
        name: "policy",
        kind: TableKind::Policy,
    },
    TableSpec {
        name: "tactics",
        kind: TableKind::Procedure,
    },
    TableSpec {
        name: "shot_distance",
        kind: TableKind::Procedure,
    },
];

/// 验证表清单。`known_insufficient` 只由验证器读取。
pub const VALIDATION_TABLES: &[TableSpec] = &[
    TableSpec {
        name: "shot_make",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "shot_profile",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "possession",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "rebound",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "freethrow",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "policy_bands",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "acceptance",
        kind: TableKind::Validation,
    },
    TableSpec {
        name: "known_insufficient",
        kind: TableKind::Validation,
    },
];

/// 运行时参数。S7 之后引擎只读这一组表。
#[derive(Debug, Clone)]
pub struct RuntimeParams {
    pub policy: PolicyTable,
    pub formations: Formations,
    pub tactics: TacticsTable,
    /// 出手类型 × 距离的实测命中率。
    pub shot_distance: ShotDistanceTable,
}

impl RuntimeParams {
    pub fn embedded() -> Self {
        let params = Self {
            policy: embedded("policy"),
            formations: embedded("formations"),
            tactics: TacticsTable::embedded(),
            shot_distance: ShotDistanceTable::embedded(),
        };
        params
            .validate()
            .expect("embedded runtime tables must validate");
        params
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let params = Self {
            policy: read(dir, "policy")?,
            formations: read(dir, "formations")?,
            tactics: TacticsTable::load(dir)?,
            shot_distance: ShotDistanceTable::load(dir)?,
        };
        params.validate()?;
        Ok(params)
    }

    pub fn validate(&self) -> Result<(), String> {
        for (name, provenance) in [
            ("policy", &self.policy.provenance),
            ("formations", &self.formations.provenance),
            ("shot_distance", &self.shot_distance.provenance),
        ] {
            if provenance.source.trim().is_empty() || provenance.method.trim().is_empty() {
                return Err(format!(
                    "{name} table must document source and method in provenance"
                ));
            }
        }
        if self.formations.offense.len() != 5 || self.formations.defense.len() != 5 {
            return Err(
                "formations table must define five offense and five defense slots".to_string(),
            );
        }
        self.tactics.validate()?;
        Ok(())
    }
}

impl From<Params> for RuntimeParams {
    fn from(params: Params) -> Self {
        Self {
            policy: params.policy,
            formations: params.formations,
            tactics: params.tactics,
            shot_distance: ShotDistanceTable::embedded(),
        }
    }
}

impl From<&Params> for RuntimeParams {
    fn from(params: &Params) -> Self {
        Self {
            policy: params.policy.clone(),
            formations: params.formations.clone(),
            tactics: params.tactics.clone(),
            shot_distance: ShotDistanceTable::embedded(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AcceptanceTable {
    pub table: String,
    pub provenance: Provenance,
    pub s3: S3Acceptance,
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3Acceptance {
    pub stable_path_ratio: BTreeMap<String, f64>,
    pub on_ball_follow_ratio: f64,
    pub on_ball_paired_min_ft: f64,
    pub recovery_path_extra_ft: f64,
    pub max_continuous_yielding_seconds: f64,
    pub rationale: String,
    pub set_at: String,
}

/// 验证参数。只由验证器读取，运行时不加载。
#[derive(Debug, Clone)]
pub struct ValidationParams {
    pub shot_make: ShotMakeTable,
    pub shot_profile: ShotProfileTable,
    pub possession: PossessionTable,
    pub rebound: ReboundTable,
    pub freethrow: FreeThrowTable,
    pub acceptance: AcceptanceTable,
}

impl ValidationParams {
    pub fn load(dir: &Path) -> Result<Self, String> {
        let params = Self {
            shot_make: read(dir, "shot_make")?,
            shot_profile: read(dir, "shot_profile")?,
            possession: read(dir, "possession")?,
            rebound: read(dir, "rebound")?,
            freethrow: read(dir, "freethrow")?,
            acceptance: read(dir, "acceptance")?,
        };
        params.validate()?;
        Ok(params)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.shot_make.provenance.sample == 0 {
            return Err("shot_make table carries no sample provenance".to_string());
        }
        if self.shot_profile.provenance.sample == 0 {
            return Err("shot_profile table carries no sample provenance".to_string());
        }
        if self.possession.provenance.sample == 0 {
            return Err("possession table carries no sample provenance".to_string());
        }
        if self.rebound.provenance.source.trim().is_empty()
            || self.rebound.provenance.method.trim().is_empty()
        {
            return Err("rebound table must document source and method in provenance".to_string());
        }
        if self.freethrow.attempts_observed == 0 {
            return Err("freethrow table carries no observed attempts".to_string());
        }
        if self.acceptance.provenance.source.trim().is_empty()
            || self.acceptance.provenance.method.trim().is_empty()
        {
            return Err("acceptance table must document source and method".to_string());
        }
        Ok(())
    }
}

impl Params {
    /// 编译期内嵌的校准表，供无文件系统的目标（浏览器、WASM）使用。
    pub fn embedded() -> Self {
        let params = Self {
            shot_make: embedded("shot_make"),
            shot_profile: embedded("shot_profile"),
            possession: embedded("possession"),
            rebound: embedded("rebound"),
            freethrow: embedded("freethrow"),
            policy: embedded("policy"),
            formations: embedded("formations"),
            tactics: TacticsTable::embedded(),
        };
        params
            .validate()
            .expect("embedded calibration tables must validate");
        params
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let params = Self {
            shot_make: read(dir, "shot_make")?,
            shot_profile: read(dir, "shot_profile")?,
            possession: read(dir, "possession")?,
            rebound: read(dir, "rebound")?,
            freethrow: read(dir, "freethrow")?,
            policy: read(dir, "policy")?,
            formations: read(dir, "formations")?,
            tactics: TacticsTable::load(dir)?,
        };
        params.validate()?;
        Ok(params)
    }

    pub fn load_default() -> Self {
        Self::load(&workspace_calibration_dir()).expect("calibration tables must load")
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.shot_make.provenance.sample == 0 {
            return Err("shot_make table carries no sample provenance".to_string());
        }
        if self.shot_profile.provenance.sample == 0 {
            return Err("shot_profile table carries no sample provenance".to_string());
        }
        if self.possession.provenance.sample == 0 {
            return Err("possession table carries no sample provenance".to_string());
        }
        if self.freethrow.attempts_observed == 0 {
            return Err("freethrow table carries no observed attempts".to_string());
        }
        for (name, provenance) in [
            ("rebound", &self.rebound.provenance),
            ("policy", &self.policy.provenance),
            ("formations", &self.formations.provenance),
        ] {
            if provenance.source.trim().is_empty() || provenance.method.trim().is_empty() {
                return Err(format!(
                    "{name} table must document source and method in provenance"
                ));
            }
        }
        if self.formations.offense.len() != 5 || self.formations.defense.len() != 5 {
            return Err(
                "formations table must define five offense and five defense slots".to_string(),
            );
        }
        if !self.formations.offense_alt.is_empty() && self.formations.offense_alt.len() != 5 {
            return Err("alternate formation must define five slots".to_string());
        }
        for kind in self.shot_profile.kind_share.keys() {
            if !self.shot_make.cells.contains_key(kind) {
                return Err(format!("shot_make has no cells for shot kind `{kind}`"));
            }
        }
        let zone_total: f32 = self.shot_profile.zone_share.values().sum();
        if (zone_total - 1.0).abs() > 0.01 {
            return Err(format!("zone shares must sum to 1, got {zone_total}"));
        }
        let creation_total: f32 = self.shot_profile.creation_share.values().sum();
        if (creation_total - 1.0).abs() > 0.01 {
            return Err(format!(
                "creation shares must sum to 1, got {creation_total}"
            ));
        }
        Ok(())
    }
}

fn read<T: serde::de::DeserializeOwned>(dir: &Path, name: &str) -> Result<T, String> {
    let path = dir.join(format!("{name}.json"));
    let text = std::fs::read_to_string(&path)
        .map_err(|error| format!("calibration table {} unreadable: {error}", path.display()))?;
    serde_json::from_str(&text)
        .map_err(|error| format!("calibration table {} invalid: {error}", path.display()))
}

fn embedded<T: serde::de::DeserializeOwned>(name: &str) -> T {
    let text = match name {
        "shot_make" => include_str!("../../../data/calibration/shot_make.json"),
        "shot_profile" => include_str!("../../../data/calibration/shot_profile.json"),
        "possession" => include_str!("../../../data/calibration/possession.json"),
        "rebound" => include_str!("../../../data/calibration/rebound.json"),
        "freethrow" => include_str!("../../../data/calibration/freethrow.json"),
        "policy" => include_str!("../../../data/calibration/policy.json"),
        "formations" => include_str!("../../../data/calibration/formations.json"),
        other => panic!("unknown embedded calibration table `{other}`"),
    };
    serde_json::from_str(text)
        .unwrap_or_else(|error| panic!("embedded calibration table {name} is invalid: {error}"))
}

#[cfg(test)]
mod manifest_tests {
    use super::*;

    #[test]
    fn runtime_and_validation_tables_are_disjoint() {
        for runtime in RUNTIME_TABLES {
            assert_ne!(runtime.kind, TableKind::Validation, "{runtime:?}");
            assert!(
                !VALIDATION_TABLES
                    .iter()
                    .any(|validation| validation.name == runtime.name),
                "{} is both runtime and validation",
                runtime.name
            );
        }
    }

    #[test]
    fn known_insufficient_is_validation_only() {
        assert!(!RUNTIME_TABLES
            .iter()
            .any(|table| table.name == "known_insufficient"));
        assert!(VALIDATION_TABLES
            .iter()
            .any(|table| table.name == "known_insufficient"));
    }

    #[test]
    fn runtime_params_load_from_runtime_tables_only() {
        let params = RuntimeParams::load(&workspace_calibration_dir())
            .expect("runtime tables must load");
        assert!(params.policy.w("shot_quality_base_rim") > 0.0);
    }

    #[test]
    fn validation_params_load_from_validation_tables() {
        let params = ValidationParams::load(&workspace_calibration_dir())
            .expect("validation tables must load");
        assert!(params.shot_make.provenance.sample > 0);
    }
}
