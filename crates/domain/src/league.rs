//! 联赛规则档案（LeagueProfile · M10 / architecture.md §6.3 / 宪章 C3）。
//!
//! 计时、球队犯规罚则与场地几何使用版本化档案。不同联赛共用同一套规则程序。

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

/// 官方规则来源和生效版本。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleSource {
    pub title: String,
    pub url: String,
    pub published: String,
    pub effective: String,
    pub reviewed: String,
}

impl RuleSource {
    pub fn validate(&self) -> Result<(), String> {
        if self.title.trim().is_empty()
            || !self.url.starts_with("https://")
            || self.published.trim().is_empty()
            || self.effective.trim().is_empty()
            || self.reviewed.trim().is_empty()
        {
            return Err("league rule source metadata is incomplete".to_string());
        }
        Ok(())
    }
}

/// 联赛时长、罚则基础值、暂停额度与球场几何。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeagueProfile {
    pub id: LeagueId,
    pub name: String,
    pub ruleset_version: String,
    pub source: RuleSource,
    /// 节次策略族：常规节数、节时长与加时时长。
    #[serde(flatten)]
    pub periods: PeriodPolicy,
    /// 时钟策略族：进攻钟、进攻篮板重置与紧逼盯防判定。
    #[serde(flatten)]
    pub clocks: ClockPolicy,
    /// 犯规策略族：犯满离场、罚球数与罚则门槛。
    #[serde(flatten)]
    pub fouls: FoulPolicy,
    /// 暂停策略族：额度、末段限制与加时额度。
    #[serde(flatten)]
    pub timeouts: TimeoutPolicy,
    /// 赛前名单上限（NBA 15，FIBA 12）。
    pub roster_limit: u32,
    /// 每队每场的教练挑战额度（NBA 1，无挑战的档案为 0）。
    pub challenges_per_game: u32,
    /// 复核窗口：本节剩余时间不超过该值时才能申请复核。
    pub challenge_final_seconds: f32,
    /// 罚球人必须在这么多秒内出手，否则违例。
    pub free_throw_time_limit_seconds: f32,
    /// 三分线弧顶距离，单位为英尺。
    pub three_point_distance_ft: f32,
    /// 底角三分线距离，单位为英尺；零值表示无单独底角距离。
    pub corner_three_distance_ft: f32,
    /// 球场几何。
    pub court: CourtGeometry,
    /// 是否使用交替拥有箭头程序。
    pub use_alternate_possession_arrow: bool,
    /// 场地中心圆的半径（英尺），用于跳球站位。
    pub center_circle_radius_ft: f32,
    /// 跳球时场上接应队员距中圈的最小站位距离（英尺）。
    pub jump_ball_receiver_distance_ft: f32,
}

/// 节次策略族：常规节数、节时长与加时时长。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PeriodPolicy {
    pub regulation_periods: u32,
    pub period_duration_seconds: f32,
    pub overtime_duration_seconds: f32,
}

/// 时钟策略族：进攻钟、进攻篮板重置与紧逼盯防判定。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ClockPolicy {
    pub shot_clock_seconds: f32,
    pub offensive_rebound_shot_clock_seconds: f32,
    pub closely_guarded_seconds: f32,
    pub closely_guarded_distance_ft: f32,
}

/// 犯规策略族：犯满离场、罚球数与罚则门槛。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FoulPolicy {
    pub max_personal_fouls: u8,
    pub shooting_foul_free_throws: u8,
    pub bonus_free_throws: u8,
    pub bonus_fouls_per_period: u32,
    pub bonus_final_period: u32,
    pub bonus_final_seconds: f32,
    pub bonus_fouls_final: u32,
    pub away_from_play_free_throws: u8,
    /// 加时节的球队犯规罚则门槛：零表示加时沿用常规节门槛。
    ///
    /// NBA 2025-26 Rule 12B §V：加时每节前三次普通球队犯规只发球，
    /// 第四次起进罚则。FIBA 2026 Article 42.1.3：加时犯规计入第四节
    /// 累计数，因此沿用常规节门槛（该值为零）。
    #[serde(default)]
    pub bonus_fouls_overtime: u32,
    /// 加时球队犯规是否计入第四节累计。
    ///
    /// NBA 为否（加时单独计数），FIBA 为是（Article 42.1.3）。
    #[serde(default)]
    pub overtime_fouls_carry_from_fourth: bool,
}

/// 暂停策略族：额度、末段限制与加时额度。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TimeoutPolicy {
    pub timeouts_per_game: u32,
    pub timeouts_fourth_quarter_max: u32,
    pub timeouts_final_seconds: f32,
    pub timeouts_final_max: u32,
    pub overtime_timeouts: u32,
}

impl LeagueProfile {
    pub fn validate(&self) -> Result<(), String> {
        if !self.center_circle_radius_ft.is_finite()
            || self.center_circle_radius_ft <= 0.0
            || !self.jump_ball_receiver_distance_ft.is_finite()
            || self.jump_ball_receiver_distance_ft <= self.center_circle_radius_ft
            || self.jump_ball_receiver_distance_ft >= self.court.width_ft * 0.5
            || self.center_circle_radius_ft >= self.court.width_ft * 0.25
        {
            return Err("jump-ball court geometry is invalid".to_string());
        }
        Ok(())
    }

    /// 读取 NBA 2025-26 官方规则档案。
    pub fn nba() -> Self {
        let profile: Self = serde_json::from_str(NBA_PROFILE)
            .expect("data/rules/league_nba.json must be a valid league profile");
        assert_eq!(profile.id, LeagueId::Nba);
        assert_eq!(profile.ruleset_version, "2025-26");
        profile
            .source
            .validate()
            .expect("NBA rule source metadata must be complete");
        profile
            .validate()
            .expect("NBA jump-ball settings must be valid");
        profile
    }

    /// 读取 FIBA 2026 v1.1 官方规则档案。
    pub fn fiba() -> Self {
        let profile: Self = serde_json::from_str(FIBA_PROFILE)
            .expect("data/rules/league_fiba.json must be a valid league profile");
        assert_eq!(profile.id, LeagueId::Fiba);
        assert_eq!(profile.ruleset_version, "2026-v1.1");
        profile
            .source
            .validate()
            .expect("FIBA rule source metadata must be complete");
        profile
            .validate()
            .expect("FIBA jump-ball settings must be valid");
        profile
    }

    pub fn period_starts_with_jump_ball(&self, period: u32) -> bool {
        assert!(period > 0, "period number must be positive");
        period == 1
            || (!self.use_alternate_possession_arrow && period > self.periods.regulation_periods)
    }

    /// 判断比赛时钟是否处于球队犯规末段规则的时间范围。
    pub fn in_final_window(&self, period: u32, period_seconds_left: f64) -> bool {
        self.fouls.bonus_final_period > 0
            && period >= self.fouls.bonus_final_period
            && period_seconds_left <= f64::from(self.fouls.bonus_final_seconds)
    }

    /// 返回当前节适用的球队犯规罚则序号。
    ///
    /// 顺序：加时节的独立门槛先于末段规则，最后是常规节门槛。
    /// NBA 加时前三次普通犯规只发球（Rule 12B §V），FIBA 加时沿用
    /// 第四节累计（Article 42.1.3）。
    pub fn bonus_foul_threshold(&self, period: u32, period_seconds_left: f64) -> u32 {
        if self.in_final_window(period, period_seconds_left) {
            self.fouls.bonus_fouls_final
        } else if period > self.periods.regulation_periods
            && self.fouls.bonus_fouls_overtime > 0
        {
            self.fouls.bonus_fouls_overtime
        } else {
            self.fouls.bonus_fouls_per_period
        }
    }

    /// 某节开始时球队已经累计的犯规数（加时是否带入第四节）。
    ///
    /// FIBA Article 42.1.3：加时节的球队犯规计入第四节累计，因此加时
    /// 的罚则门槛按「第四节累计 + 本节犯规」判定。NBA 单独计数。
    pub fn carried_fouls(&self, period: u32, fourth_quarter_fouls: u32) -> u32 {
        if period > self.periods.regulation_periods && self.fouls.overtime_fouls_carry_from_fourth {
            fourth_quarter_fouls
        } else {
            0
        }
    }

    /// 按档案中的球队犯规序号计算普通犯规罚球。
    pub fn bonus_free_throws_for(
        &self,
        period_fouls: u32,
        period: u32,
        period_seconds_left: f64,
    ) -> u8 {
        let threshold = self.bonus_foul_threshold(period, period_seconds_left);
        if threshold > 0 && period_fouls >= threshold {
            self.fouls.bonus_free_throws
        } else {
            0
        }
    }

    /// 前场篮板后的进攻时钟重置：只补足至档案时长，不缩短已有时长。
    pub fn shot_clock_after_offensive_rebound(&self, current: f64, rim_contact: bool) -> f64 {
        if rim_contact {
            current.max(f64::from(self.clocks.offensive_rebound_shot_clock_seconds))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_families_survive_the_json_round_trip() {
        // 磁盘上仍是扁平键：`serde(flatten)` 把四个策略族写回顶层，
        // 读回时又落回策略族。
        for league in [LeagueProfile::nba(), LeagueProfile::fiba()] {
            let text = serde_json::to_string(&league).expect("league profile serializes");
            let parsed: LeagueProfile =
                serde_json::from_str(&text).expect("league profile deserializes");
            assert_eq!(parsed.periods, league.periods);
            assert_eq!(parsed.clocks, league.clocks);
            assert_eq!(parsed.fouls, league.fouls);
            assert_eq!(parsed.timeouts, league.timeouts);
            assert!(text.contains("\"period_duration_seconds\""));
            assert!(text.contains("\"shot_clock_seconds\""));
            assert!(text.contains("\"max_personal_fouls\""));
        }
    }
}
