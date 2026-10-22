//! 联赛规则档案（LeagueProfile · M10 / architecture.md §6.3 / 宪章 C3）。
//!
//! 语义/规则约束（计时结构、进攻时钟、犯规政策与 bonus、几何）按联赛
//! 档案参数化：切换联赛 = 切换一份档案数据，引擎代码路径零联赛分支。
//! 物理约束（人体极限、球飞行）不在本档案——它们与联赛无关。
//!
//! 两份档案的数值住在 `data/rules/league_*.json`，经 `include_str!` 编译期
//! 内联。规则数字属数据资产：写在代码里会以「内联行为常数」的形式绕过
//! 规则通道（charter C1），而这正是 `scripts/check_inline_constants.py`
//! 要拦下的东西。

use crate::court::CourtGeometry;
use serde::{Deserialize, Serialize};

/// 支持的联赛档案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeagueId {
    Nba,
    Fiba,
}

const NBA_PROFILE: &str = include_str!("../../../data/rules/league_nba.json");
const FIBA_PROFILE: &str = include_str!("../../../data/rules/league_fiba.json");

/// 联赛规则档案：计时结构、进攻时钟与重置、犯规政策与 bonus、几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    /// 最后时段 bonus 的适用范围：从第几节起适用更严阈值（NBA 从第 4 节起，
    /// 加时同样适用）。0 表示该联赛没有最后时段特例。
    pub bonus_final_period: u32,
    /// 最后时段窗口（秒）：本节剩余时间不足该值时进入最后时段。
    pub bonus_final_seconds: f32,
    /// 最后时段内球队犯规达到该值即进入 bonus（NBA 第 2 犯）。
    pub bonus_fouls_final: u32,
    /// 最后时段对无球人犯规判给的罚球次数（NBA 为 1 罚并保留球权）。
    pub away_from_play_free_throws: u8,
    /// 每场暂停上限。
    pub timeouts_per_game: u32,
    /// 第四节可使用的暂停上限。
    pub timeouts_fourth_quarter_max: u32,
    /// 最后时段窗口（秒）：本节剩余时间不足该值时暂停上限另计。
    pub timeouts_final_seconds: f32,
    /// 最后时段内可使用的暂停上限。
    pub timeouts_final_max: u32,
    /// 每个加时的暂停配额。
    pub overtime_timeouts: u32,
    /// 三分线距离（ft，弧顶/翼位）。
    pub three_point_distance_ft: f32,
    /// 底角三分距离（ft）。NBA 为 22.0，FIBA 为 22.15（等半径，无特例时置 0）。
    pub corner_three_distance_ft: f32,
    /// 场地几何。档案 JSON 省略时取默认场地。
    #[serde(default)]
    pub court: CourtGeometry,
    /// 争球程序是否使用交替拥有箭头（FIBA 为 true，NBA 为 false 即跳球）。
    pub use_alternate_possession_arrow: bool,
}

impl LeagueProfile {
    /// NBA 基线档案（4×12min、24s/14s、6 犯、单节第 5 犯 bonus、23.75ft）。
    pub fn nba() -> Self {
        serde_json::from_str(NBA_PROFILE)
            .expect("data/rules/league_nba.json must be a valid league profile")
    }

    /// FIBA 档案（4×10min、24s/14s、5 犯、单节第 4 次犯规起 bonus、6.75m 三分线）。
    pub fn fiba() -> Self {
        let mut profile: Self = serde_json::from_str(FIBA_PROFILE)
            .expect("data/rules/league_fiba.json must be a valid league profile");
        // FIBA 场地 28m × 15m；档案只写规则数字，几何来自场地模块。
        profile.court = CourtGeometry::fiba();
        profile
    }

    /// 本节是否处于「最后时段」（bonus 与暂停的双重特例窗口）。
    ///
    /// 只有声明了 `bonus_final_period` 的联赛（NBA）才有这个窗口；
    /// 加时的节次编号大于常规节数，同样落在窗口内。
    pub fn in_final_window(&self, period: u32, period_seconds_left: f64) -> bool {
        self.bonus_final_period > 0
            && period >= self.bonus_final_period
            && period_seconds_left <= self.bonus_final_seconds as f64
    }

    /// 球队进入 bonus 所需的单节犯规数。
    pub fn bonus_foul_threshold(&self, period: u32, period_seconds_left: f64) -> u32 {
        if self.in_final_window(period, period_seconds_left) {
            self.bonus_fouls_final
        } else {
            self.bonus_fouls_per_period
        }
    }

    /// 该节球队犯规数达到阈值时的罚球次数。
    pub fn bonus_free_throws_for(
        &self,
        period_fouls: u32,
        period: u32,
        period_seconds_left: f64,
    ) -> u8 {
        if period_fouls >= self.bonus_foul_threshold(period, period_seconds_left) {
            self.bonus_free_throws
        } else {
            0
        }
    }

    /// 前场篮板后的进攻时钟。
    ///
    /// 只有球触筐才产生重置：`T_shot ← max(T_shot, 重置值)`。三不沾、
    /// 被封盖的球被进攻方抢到时时钟继续走原来的读数，既不重置也不归零。
    pub fn shot_clock_after_offensive_rebound(&self, current: f64, rim_contact: bool) -> f64 {
        if rim_contact {
            current.max(self.offensive_rebound_shot_clock_seconds as f64)
        } else {
            current
        }
    }
}

impl Default for LeagueProfile {
    fn default() -> Self {
        Self::nba()
    }
}
