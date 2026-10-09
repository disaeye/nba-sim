//! 参考数据登记的加载与 STAT 裁决推导（validation.md §6.3/§11）。
//!
//! `data/reference/stat_deltas.v1.json` 是唯一判据来源：每个 STAT 叶项
//! 登记为 `full`（容差已登记，等待审核后进入正式配置）或 `blocked`
//! （参考字段缺失，登记了缺失字段与获取任务）。加载器不编造数值；
//! 审核状态为非 approved 时全部条目仍保持 `InsufficientEvidence`。

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;

/// 登记值/容差的灵活类型：单值指标是数字，向量/构成指标是键值映射。
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RegistrationValue {
    Number(f64),
    Text(String),
    Map(BTreeMap<String, serde_json::Value>),
}

/// 登记文件中单条容差记录。
#[derive(Debug, Clone, Deserialize)]
pub struct DeltaEntry {
    pub criterion_id: String,
    pub metric: String,
    pub statistic_kind: String,
    #[serde(default)]
    pub reference_value: Option<RegistrationValue>,
    #[serde(default)]
    pub allowed_delta: Option<RegistrationValue>,
    #[serde(default)]
    pub rationale: String,
    pub data_coverage: String,
    #[serde(default)]
    pub reason_code: Option<String>,
    #[serde(default)]
    pub missing_fields: Vec<String>,
    #[serde(default)]
    pub data_requirement: String,
    /// Wasserstein 类条目的登记分位（如出手距离分布的 p10..p90）。
    #[serde(default)]
    pub reference_quantiles: BTreeMap<String, serde_json::Value>,
}

/// 登记文件的审核状态。
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationAudit {
    pub status: String,
    #[serde(default)]
    pub registered_by: String,
    #[serde(default)]
    pub review_required: String,
}

/// stat_deltas.v1.json 的顶层结构。
#[derive(Debug, Clone, Deserialize)]
pub struct StatDeltaRegistration {
    pub version: String,
    pub reference_manifest: ReferenceManifestLite,
    pub method: RegistrationMethod,
    pub deltas: Vec<DeltaEntry>,
    pub audit: RegistrationAudit,
}

/// 参考清单（轻量版：裁决推导只依赖身份与审核状态）。
#[derive(Debug, Clone, Deserialize)]
pub struct ReferenceManifestLite {
    pub reference_id: String,
    pub season: String,
    pub data_sha256: String,
    pub games: u32,
    pub attempts: u32,
    /// stats.nba.com PBP 归档摘要（v2 起登记）。
    #[serde(default)]
    pub pbp_data_sha256: String,
    /// pbpstats 逐回合归档摘要（v2 起登记）。
    #[serde(default)]
    pub pbpstats_data_sha256: String,
    /// pbpstats 回合总数。
    #[serde(default)]
    pub pbpstats_possessions: u32,
}

/// 比较方法声明。
#[derive(Debug, Clone, Deserialize)]
pub struct RegistrationMethod {
    pub statistic: String,
    pub cluster: String,
    pub bootstrap: String,
    pub multiple_comparison: String,
    pub decision_rule: String,
}

/// 单个准则的登记裁决。
#[derive(Debug, Clone)]
pub struct StatCriterionRegistration {
    pub criterion_id: String,
    /// `registered_pending_review`（容差已登记、待审核）或
    /// `reference_field_missing`（参考字段缺失）。
    pub status: &'static str,
    /// InsufficientEvidence 的原因码（validation.md §10 裁决字段）。
    pub reason_code: String,
    /// 人类可读的缺口说明（页面与工件展示）。
    pub detail: String,
}

/// 默认登记路径：`NBA_STAT_DELTAS` 环境变量覆盖；缺省从 crate 目录向
/// 上两级定位仓库根（测试与 CLI 的 cwd 不同，不能依赖相对 cwd）。
pub fn default_registration_path() -> std::path::PathBuf {
    if let Some(path) = std::env::var_os("NBA_STAT_DELTAS") {
        return std::path::PathBuf::from(path);
    }
    let manifest = env!("CARGO_MANIFEST_DIR");
    std::path::Path::new(manifest).join("../../data/reference/stat_deltas.v1.json")
}

/// 审核状态非 `approved` 时 `registered_pending_review` 条目依旧给
/// `InsufficientEvidence`（validation.md §2.1：只有 approved 档案参与
/// 正式验收），但原因码区分「数据已登记待审核」与「数据缺失」。
pub fn load_stat_registrations(
    path: &Path,
) -> Result<
    (
        StatDeltaRegistration,
        BTreeMap<String, StatCriterionRegistration>,
    ),
    String,
> {
    let text =
        std::fs::read_to_string(path).map_err(|error| format!("read {path:?} failed: {error}"))?;
    let registration: StatDeltaRegistration =
        serde_json::from_str(&text).map_err(|error| format!("parse stat deltas: {error}"))?;
    let mut by_criterion: BTreeMap<String, Vec<&DeltaEntry>> = BTreeMap::new();
    for entry in &registration.deltas {
        by_criterion
            .entry(entry.criterion_id.clone())
            .or_default()
            .push(entry);
    }
    let audit_approved = registration.audit.status == "approved";
    let mut out = BTreeMap::new();
    for (criterion_id, entries) in &by_criterion {
        let blocked = entries
            .iter()
            .find(|entry| entry.data_coverage == "blocked");
        if let Some(entry) = blocked {
            let pipeline = entry.reason_code.as_deref() == Some("reference_pipeline_pending");
            let detail = if pipeline {
                format!(
                    "归档在手，重建脚本待写；已登记部分：{}",
                    entry.data_requirement
                )
            } else {
                format!(
                    "缺失参考字段 {:?}；获取任务：{}",
                    entry.missing_fields, entry.data_requirement
                )
            };
            out.insert(
                criterion_id.clone(),
                StatCriterionRegistration {
                    criterion_id: criterion_id.clone(),
                    status: if pipeline {
                        "reference_pipeline_pending"
                    } else {
                        "reference_field_missing"
                    },
                    reason_code: entry
                        .reason_code
                        .clone()
                        .unwrap_or_else(|| "reference_field_missing".to_string()),
                    detail,
                },
            );
            continue;
        }
        // data_coverage 只有 full/blocked 两态（登记校验脚本把关）；
        // 非 blocked 必有 full 条目，否则登记损坏，就地崩溃。
        let full = entries
            .iter()
            .find(|entry| entry.data_coverage.starts_with("full"))
            .unwrap_or_else(|| {
                panic!("corrupt registration: {criterion_id} has neither full nor blocked entry")
            });
        if audit_approved {
            out.insert(
                criterion_id.clone(),
                StatCriterionRegistration {
                    criterion_id: criterion_id.clone(),
                    status: "registered",
                    reason_code: "registered".to_string(),
                    detail: format!(
                        "容差已登记且审核通过（{} 条指标）；进入正式运行配置",
                        entries.len()
                    ),
                },
            );
        } else {
            out.insert(
                criterion_id.clone(),
                StatCriterionRegistration {
                    criterion_id: criterion_id.clone(),
                    status: "registered_pending_review",
                    reason_code: "reference_delta_pending_review".to_string(),
                    detail: format!(
                        "容差已登记（{} 条指标），审核状态 {}；审核通过前保持证据不足",
                        entries.len(),
                        registration.audit.status
                    ),
                },
            );
        }
        let _ = full;
    }
    Ok((registration, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_path(relative: &str) -> std::path::PathBuf {
        let manifest = env!("CARGO_MANIFEST_DIR");
        std::path::Path::new(manifest).join("../../").join(relative)
    }

    #[test]
    fn registration_file_loads_and_covers_all_stat_items() {
        let path = repo_path("data/reference/stat_deltas.v1.json");
        let (registration, per_criterion) = load_stat_registrations(&path).expect("load");
        assert_eq!(registration.version, "stat_deltas.v2");
        assert_eq!(registration.reference_manifest.games, 1230);
        assert_eq!(registration.reference_manifest.attempts, 218701);
        assert_eq!(registration.reference_manifest.data_sha256.len(), 64);
        assert_eq!(registration.reference_manifest.pbp_data_sha256.len(), 64);
        assert_eq!(
            registration.reference_manifest.pbpstats_data_sha256.len(),
            64
        );
        assert_eq!(
            registration.reference_manifest.pbpstats_possessions,
            243_768
        );
        let all_stat = [
            "STAT-G01", "STAT-G02", "STAT-S01", "STAT-S02", "STAT-R01", "STAT-T01", "STAT-B01",
            "STAT-P01", "STAT-M01", "STAT-I01",
        ];
        for criterion in all_stat {
            let entry = per_criterion
                .get(criterion)
                .unwrap_or_else(|| panic!("{criterion} missing from registration"));
            match criterion {
                // audit=approved 后 full 条目进入正式运行配置：
                "STAT-S01" | "STAT-G01" | "STAT-R01" | "STAT-T01" | "STAT-B01" | "STAT-G02" => {
                    assert_eq!(entry.status, "registered");
                    assert_eq!(entry.reason_code, "registered");
                }
                // 追踪数据类缺口：
                "STAT-S02" | "STAT-P01" | "STAT-M01" => {
                    assert_eq!(entry.status, "reference_field_missing");
                    assert!(!entry.detail.is_empty());
                }
                // 数据在手、重建脚本待写：
                "STAT-I01" => {
                    assert_eq!(entry.status, "reference_pipeline_pending");
                    assert!(!entry.detail.is_empty());
                }
                _ => unreachable!("all STAT items covered"),
            }
        }
        // 追踪/管线缺口条目不因 audit 翻转。
        assert!(per_criterion
            .values()
            .all(|entry| entry.status != "registered_pending_review"));
        // 全部 JOINT 叶项同样被登记：JOINT-Q01/C04 由 pbpstats 逐回合归
        // 档解锁，C01/C05 由 PBP 时间线解锁（均 pending_review），其余仍
        // blocked。
        let all_joint = [
            "JOINT-C01",
            "JOINT-C02",
            "JOINT-C03",
            "JOINT-C04",
            "JOINT-C05",
            "JOINT-T01",
            "JOINT-T02",
            "JOINT-I01",
            "JOINT-Q01",
        ];
        for criterion in all_joint {
            let entry = per_criterion
                .get(criterion)
                .unwrap_or_else(|| panic!("{criterion} missing from registration"));
            assert!(!entry.detail.is_empty(), "{criterion}");
            match criterion {
                // 归档在手、判据可执行：pbpstats 解锁 Q01/C04；
                // C01/C05 经审查降回 pipeline_pending（条件细分与账期参照未完成）。
                "JOINT-Q01" | "JOINT-C04" => {
                    assert_eq!(
                        entry.status, "registered",
                        "{criterion} unlocked by pbpstats, audit approved"
                    );
                }
                "JOINT-C01" | "JOINT-C05" => {
                    assert_eq!(
                        entry.status, "registered",
                        "{criterion} rebuilt from frozen archives (todo #24)"
                    );
                }
                _ => {
                    assert_eq!(entry.status, "reference_field_missing", "{criterion}");
                }
            }
        }
    }

    #[test]
    fn approved_audit_flips_full_entries_to_registered() {
        // 构造一份 approved 状态的最小登记，验证翻转逻辑（不触真实文件）。
        let dir = std::env::temp_dir().join("nba_stat_deltas_test");
        std::fs::create_dir_all(&dir).expect("tmpdir");
        let path = dir.join("approved.json");
        std::fs::write(
            &path,
            r#"{
              "version": "stat_deltas.v1",
              "reference_manifest": {
                "reference_id": "t", "season": "s",
                "data_sha256": "0000000000000000000000000000000000000000000000000000000000000000",
                "games": 1, "attempts": 1
              },
              "method": {
                "statistic": "x", "cluster": "game", "bootstrap": "percentile",
                "multiple_comparison": "bonferroni", "decision_rule": "three-outcome"
              },
              "deltas": [
                {"criterion_id": "STAT-S01", "metric": "m", "statistic_kind": "mean_difference",
                 "allowed_delta": 0.03, "rationale": "r", "data_coverage": "full"}
              ],
              "audit": {"status": "approved", "registered_by": "dev", "review_required": ""}
            }"#,
        )
        .expect("write");
        let (_, per_criterion) = load_stat_registrations(&path).expect("load");
        assert_eq!(per_criterion["STAT-S01"].status, "registered");
        assert_eq!(per_criterion["STAT-S01"].reason_code, "registered");
        let _ = std::fs::remove_file(&path);
    }
}
