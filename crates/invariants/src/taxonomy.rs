use crate::Violation;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// 违例严重度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ViolationSeverity {
    /// 物理不可能/严重规则破坏（必须阻断/终止）
    Hard,
    /// 战术不合理但物理可能（计数警告并记录）
    Soft,
}

/// 违例所属的公理正交分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ViolationCategory {
    /// 阵容与实体守恒 (5v5、替补界外、球实体)
    RosterAndEntity,
    /// 空间拓扑与碰撞穿模 (出界、穿模)
    SpatialTopology,
    /// 运动学与物理极值 (超速、瞬移、高度)
    Kinematics,
    /// 球权排他与状态机 (两人持球、空气球)
    PossessionMutex,
    /// 时钟与生命周期 (活球停表、死球走表、24秒违例)
    ClockAndFlow,
    /// 得分因果律 (凭空得分、无进球自增)
    ScoreCausality,
    /// 回合叙事自洽性 (出手真实性、篮板距离)
    NarrativeConsistency,
}

/// 违例结构化分类账。
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct ViolationTaxonomy {
    pub total_violations: usize,
    pub hard_count: usize,
    pub soft_count: usize,
    pub by_category: HashMap<String, usize>,
    pub by_rule: HashMap<String, usize>,
}

impl ViolationTaxonomy {
    pub fn from_violations(violations: &[Violation]) -> Self {
        let mut taxonomy = Self {
            total_violations: violations.len(),
            hard_count: 0,
            soft_count: 0,
            by_category: HashMap::new(),
            by_rule: HashMap::new(),
        };

        for v in violations {
            let cat = categorize_rule(v.rule);
            let sev = v.severity;

            match sev {
                ViolationSeverity::Hard => taxonomy.hard_count += 1,
                ViolationSeverity::Soft => taxonomy.soft_count += 1,
            }

            *taxonomy
                .by_category
                .entry(format!("{:?}", cat))
                .or_insert(0) += 1;
            *taxonomy.by_rule.entry(v.rule.to_string()).or_insert(0) += 1;
        }

        taxonomy
    }
}

pub fn categorize_rule(rule: &str) -> ViolationCategory {
    match rule {
        "TEAM_ON_COURT_COUNT"
        | "BENCH_DEEP_IN_COURT"
        | "BALL_HOLDER_EXISTS"
        | "BALL_HOLDER_ON_COURT"
        | "FOUL_PLAYER_EXISTS" => ViolationCategory::RosterAndEntity,
        "PLAYER_IN_BOUNDS" | "PLAYER_SEPARATION" | "FOUL_SAME_TEAM" => {
            ViolationCategory::SpatialTopology
        }
        "PLAYER_SPEED" | "BALL_SPEED" | "BALL_HEIGHT_BOUNDS" | "BALL_TELEPORT" => {
            ViolationCategory::Kinematics
        }
        "BALL_SINGLE_HOLDER" | "BALL_WITH_HOLDER" | "BALL_HOLDER_MISMATCH" => {
            ViolationCategory::PossessionMutex
        }
        "CLOCK_MONOTONIC"
        | "SHOT_CLOCK_BOUNDS"
        | "TEAM_FOULS_MONOTONIC"
        | "TEAM_FOUL_COUNT_MISMATCH" => ViolationCategory::ClockAndFlow,
        "SCORE_MONOTONIC"
        | "SCORE_EVENT_WITHOUT_POINTS"
        | "UNCAUSED_SCORE_DELTA"
        | "SCORE_DELTA_VALIDITY" => ViolationCategory::ScoreCausality,
        "FOUL_PAYLOAD_MISSING" | "FOUL_PAYLOAD_INVALID" => ViolationCategory::NarrativeConsistency,
        _ => ViolationCategory::NarrativeConsistency,
    }
}
