//! 联赛规则档案（LeagueProfile · M10 / architecture.md §6.3 / 宪章 C3）。
//!
//! 语义/规则约束（计时结构、进攻时钟、犯规政策与 bonus、几何）按联赛
//! 档案参数化：切换联赛 = 切换一份档案数据，引擎代码路径零联赛分支。
//! 物理约束（人体极限、球飞行）不在本档案——它们与联赛无关。

use crate::court::CourtGeometry;
use serde::{Deserialize, Serialize};

/// 支持的联赛档案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeagueId {
    Nba,
    Fiba,
}

/// 联赛规则档案：计时结构、进攻时钟与重置、犯规政策与 bonus、几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LeagueProfile {
    pub id: LeagueId,
    pub name: String,
    /// 常规节数（NBA/FIBA 4，NCAA 2）。
    pub regulation_periods: u32,
    /// 每节时长（秒）。
    pub period_duration_seconds: f32,
    /// 加时赛时长（秒）。
    pub overtime_duration_seconds: f32,
    /// 进攻时钟（秒）。
    pub shot_clock_seconds: f32,
    /// 前场篮板后的进攻时钟重置（秒）。
    pub offensive_rebound_shot_clock_seconds: f32,
    /// 个人犯满离场阈值（NBA 6，FIBA 5）。
    pub max_personal_fouls: u8,
    /// 投篮犯规的罚球次数。
    pub shooting_foul_free_throws: u8,
    /// bonus 状态下的罚球次数。
    pub bonus_free_throws: u8,
    /// 单节球队犯规达到该值后进入 bonus。
    pub bonus_fouls_per_period: u32,
    /// 三分线距离（ft）。
    pub three_point_distance_ft: f32,
    /// 场地几何。
    pub court: CourtGeometry,
}

impl LeagueProfile {
    /// NBA 基线档案（4×12min、24s/14s、6 犯、单节第 5 犯 bonus、23.75ft）。
    pub fn nba() -> Self {
        Self {
            id: LeagueId::Nba,
            name: "NBA".to_string(),
            regulation_periods: 4,
            period_duration_seconds: 12.0 * 60.0,
            overtime_duration_seconds: 5.0 * 60.0,
            shot_clock_seconds: 24.0,
            offensive_rebound_shot_clock_seconds: 14.0,
            max_personal_fouls: 6,
            shooting_foul_free_throws: 2,
            bonus_free_throws: 2,
            bonus_fouls_per_period: 5,
            three_point_distance_ft: 23.75,
            court: CourtGeometry::default(),
        }
    }

    /// FIBA 档案（4×10min、24s/14s、5 犯、单节第 4 次犯规起 bonus、6.75m 三分线）。
    pub fn fiba() -> Self {
        Self {
            id: LeagueId::Fiba,
            name: "FIBA".to_string(),
            regulation_periods: 4,
            period_duration_seconds: 10.0 * 60.0,
            overtime_duration_seconds: 5.0 * 60.0,
            shot_clock_seconds: 24.0,
            offensive_rebound_shot_clock_seconds: 14.0,
            max_personal_fouls: 5,
            shooting_foul_free_throws: 2,
            bonus_free_throws: 2,
            bonus_fouls_per_period: 4,
            // 6.75 m ≈ 22.15 ft；FIBA 场地 28m × 15m ≈ 91.86 × 49.21 ft。
            three_point_distance_ft: 22.15,
            court: CourtGeometry::fiba(),
        }
    }
}

impl Default for LeagueProfile {
    fn default() -> Self {
        Self::nba()
    }
}
