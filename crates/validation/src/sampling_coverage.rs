//! 采样分支覆盖工件（validation 当系统级测试的覆盖报告缺口）。
//!
//! 采样永远有运气成分：模拟 N 场，罕见局面（恶意犯规、争球、压哨）可能
//! 一次都没出现。本模块统计批量比赛里被触发过的事实类型（事件种类、
//! 违例变体、犯规类别、转换来源），与规则域登记（`rule_coverage`）对照，
//! 输出三档：**已采样覆盖**（流内证据 ≥1）/ **仅场景覆盖**（无采样证据
//! 但有专门场景）/ **双未覆盖**（两者皆无，如实写进认证范围限制）。
//!
//! 与场景矩阵的分工：采样层管整体统计，场景层管主动构造；覆盖报告把
//! 两边的证据合并成一张表，谁看认证结论都能看到「哪些分支从来没跑到」。

use std::collections::BTreeMap;

use nba_protocol::StreamTick;
use serde::Serialize;

/// 批量采样的事实类型覆盖统计。
#[derive(Debug, Default, Serialize)]
pub struct SamplingFacts {
    /// 事件种类 → 出现次数（全部比赛合计）。
    pub event_kinds: BTreeMap<String, u64>,
    /// 违例（VIOLATION）描述变体 → 次数。
    pub violation_variants: BTreeMap<String, u64>,
    /// 犯规类别（Foul payload foul_kind）→ 次数。
    pub foul_kinds: BTreeMap<String, u64>,
    /// 转换来源（TRANSITION_STARTED origin）→ 次数。
    pub transition_origins: BTreeMap<String, u64>,
    /// 回合终结原因（POSSESSION_SUMMARY terminal_event）→ 次数。
    pub terminal_causes: BTreeMap<String, u64>,
}

impl SamplingFacts {
    /// 合并另一份采样事实（并行批量：每线程单场聚合后合并）。
    pub fn merge(&mut self, other: SamplingFacts) {
        for (key, count) in other.event_kinds {
            *self.event_kinds.entry(key).or_insert(0) += count;
        }
        for (key, count) in other.violation_variants {
            *self.violation_variants.entry(key).or_insert(0) += count;
        }
        for (key, count) in other.foul_kinds {
            *self.foul_kinds.entry(key).or_insert(0) += count;
        }
        for (key, count) in other.transition_origins {
            *self.transition_origins.entry(key).or_insert(0) += count;
        }
        for (key, count) in other.terminal_causes {
            *self.terminal_causes.entry(key).or_insert(0) += count;
        }
    }

    /// 从单场比赛的流聚合（并行批量用；直接复用 from_runs 的口径）。
    pub fn from_single_run(ticks: &[StreamTick]) -> Self {
        Self::from_runs(&[ticks])
    }

    /// 从一批比赛的流聚合。
    pub fn from_runs(runs: &[&[StreamTick]]) -> Self {
        let mut facts = SamplingFacts::default();
        for ticks in runs {
            for tick in *ticks {
                for event in &tick.frame.event_log {
                    *facts.event_kinds.entry(event.kind.clone()).or_insert(0) += 1;
                    match event.kind.as_str() {
                        "VIOLATION" => {
                            let name = event
                                .data
                                .as_ref()
                                .and_then(serde_json::Value::as_object)
                                .and_then(|d| d.keys().next().cloned())
                                .unwrap_or_default();
                            *facts.violation_variants.entry(name).or_insert(0) += 1;
                        }
                        "FOUL" => {
                            let kind = event
                                .data
                                .as_ref()
                                .and_then(|d| d.get("Foul"))
                                .and_then(|f| f.get("foul_kind"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("?")
                                .to_string();
                            *facts.foul_kinds.entry(kind).or_insert(0) += 1;
                        }
                        "TRANSITION_STARTED" => {
                            let origin = event
                                .data
                                .as_ref()
                                .and_then(|d| d.get("TransitionStarted"))
                                .and_then(|t| t.get("origin"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("?")
                                .to_string();
                            *facts.transition_origins.entry(origin).or_insert(0) += 1;
                        }
                        "POSSESSION_SUMMARY" => {
                            let cause = event
                                .data
                                .as_ref()
                                .and_then(|d| d.get("PossessionSummary"))
                                .and_then(|p| p.get("terminal_event"))
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or("?")
                                .to_string();
                            *facts.terminal_causes.entry(cause).or_insert(0) += 1;
                        }
                        _ => {}
                    }
                }
            }
        }
        facts
    }

    /// 指定证据键是否在采样中出现过。
    pub fn has_sampled_evidence(&self, keys: &[&str]) -> bool {
        keys.iter().any(|key| {
            self.event_kinds.contains_key(*key)
                || self.violation_variants.keys().any(|v| v.contains(key))
                || self.foul_kinds.contains_key(*key)
                || self.transition_origins.contains_key(*key)
                || self.terminal_causes.contains_key(*key)
        })
    }
}

/// 单个规则域的覆盖档位。
#[derive(Debug, Serialize)]
pub struct DomainCoverage {
    pub rule_id: &'static str,
    pub description: &'static str,
    /// sampled | scenario_only | uncovered
    pub status: &'static str,
    pub scenarios: &'static [&'static str],
    pub sampling_keys: &'static [&'static str],
    pub gap_note: &'static str,
}

/// 覆盖报告（分支覆盖工件的核心载荷）。
#[derive(Debug, Serialize)]
pub struct BranchCoverageReport {
    pub sampled_games: u32,
    pub domains: Vec<DomainCoverage>,
    /// 三档计数。
    pub sampled: u32,
    pub scenario_only: u32,
    pub uncovered: u32,
    /// 采样证据明细（供核对 status 判定）。
    pub facts: SamplingFacts,
    /// 认证范围限制（uncovered 域的说明清单；如实写进认证结论）。
    pub certification_limits: Vec<String>,
}

/// 生成覆盖报告（采样事实 + 规则域登记对照）。
pub fn branch_coverage_report(sampled_games: u32, facts: &SamplingFacts) -> BranchCoverageReport {
    use crate::rule_coverage::{rule_domain_registry, RuleCoverage};

    let mut domains = Vec::new();
    let mut sampled = 0u32;
    let mut scenario_only = 0u32;
    let mut uncovered = 0u32;
    let mut certification_limits = Vec::new();
    for entry in rule_domain_registry() {
        let (status, counts_toward_limit) = match entry.coverage {
            RuleCoverage::Covered => {
                if facts.has_sampled_evidence(entry.sampling_keys) {
                    sampled += 1;
                    ("sampled", false)
                } else {
                    scenario_only += 1;
                    ("scenario_only", false)
                }
            }
            RuleCoverage::SamplingOnly | RuleCoverage::Missing => {
                uncovered += 1;
                ("uncovered", true)
            }
        };
        if counts_toward_limit {
            certification_limits.push(format!(
                "{}（{}）：{}",
                entry.rule_id, entry.description, entry.gap_note
            ));
        }
        domains.push(DomainCoverage {
            rule_id: entry.rule_id,
            description: entry.description,
            status,
            scenarios: entry.scenarios,
            sampling_keys: entry.sampling_keys,
            gap_note: entry.gap_note,
        });
    }
    BranchCoverageReport {
        sampled_games,
        domains,
        sampled,
        scenario_only,
        uncovered,
        facts: SamplingFacts {
            event_kinds: facts.event_kinds.clone(),
            violation_variants: facts.violation_variants.clone(),
            foul_kinds: facts.foul_kinds.clone(),
            transition_origins: facts.transition_origins.clone(),
            terminal_causes: facts.terminal_causes.clone(),
        },
        certification_limits,
    }
}
