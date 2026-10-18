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

/// 策略响应系数。由 `scripts/calibrate/fit_policy.py` 做矩匹配写入，
/// 目标是让引擎产生的出手与回合分布贴近 `shot_profile` 与 `possession`。
#[derive(Debug, Clone, Deserialize)]
pub struct PolicyTable {
    pub provenance: Provenance,
    pub weights: BTreeMap<String, f32>,
    #[serde(default)]
    pub bands: BTreeMap<String, f32>,
}

impl PolicyTable {
    /// 判定带宽：未单独登记的准则用 `default`。
    pub fn band(&self, criterion: &str) -> f64 {
        self.bands
            .get(criterion)
            .or_else(|| self.bands.get("default"))
            .copied()
            .unwrap_or_else(|| panic!("policy table has no band for `{criterion}`")) as f64
    }
    pub fn w(&self, key: &str) -> f32 {
        *self
            .weights
            .get(key)
            .unwrap_or_else(|| panic!("policy table has no weight `{key}`"))
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
