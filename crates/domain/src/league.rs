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
    /// 常规节数。
    pub regulation_periods: u32,
    /// 每节时长，单位为秒。
    pub period_duration_seconds: f32,
    /// 加时时长，单位为秒。
    pub overtime_duration_seconds: f32,
    /// 进攻时钟时长，单位为秒。
    pub shot_clock_seconds: f32,
    /// 进攻篮板后的进攻时钟重置值，单位为秒。
    pub offensive_rebound_shot_clock_seconds: f32,
    /// 个人犯满离场次数。
    pub max_personal_fouls: u8,
    /// 赛前名单上限（NBA 15，FIBA 12）。
    #[serde(default = "default_roster_limit")]
    pub roster_limit: u32,
    /// 每队每场的教练挑战额度（NBA 1，无挑战的档案为 0）。
    #[serde(default = "default_challenges_per_game")]
    pub challenges_per_game: u32,
    /// 复核窗口：本节剩余时间不超过该值时才能申请复核。
    #[serde(default = "default_challenge_final_seconds")]
    pub challenge_final_seconds: f32,
    /// 罚球人必须在这么多秒内出手，否则违例。
    #[serde(default = "default_free_throw_time_limit_seconds")]
    pub free_throw_time_limit_seconds: f32,
    /// 投篮犯规基础罚球数。
    pub shooting_foul_free_throws: u8,
    /// 球队犯规进入罚则后的罚球数。
    pub bonus_free_throws: u8,
    /// 一般节球队犯规进入罚则的序号。
    pub bonus_fouls_per_period: u32,
    /// 末段规则适用的起始节次；零值表示未启用。
    pub bonus_final_period: u32,
    /// 末段规则适用的剩余时间门限，单位为秒。
    pub bonus_final_seconds: f32,
    /// 末段球队犯规进入罚则的序号。
    pub bonus_fouls_final: u32,
    /// NBA 末段无球犯规判给的罚球数。
    pub away_from_play_free_throws: u8,
    /// 每场暂停额度。
    pub timeouts_per_game: u32,
    /// 第四节暂停额度。
    pub timeouts_fourth_quarter_max: u32,
    /// 暂停末段限制适用的剩余时间门限，单位为秒。
    pub timeouts_final_seconds: f32,
    /// 暂停末段额度。
    pub timeouts_final_max: u32,
    /// 每个加时暂停额度。
    pub overtime_timeouts: u32,
    /// 三分线弧顶距离，单位为英尺。
    pub three_point_distance_ft: f32,
    /// 底角三分线距离，单位为英尺；零值表示无单独底角距离。
    pub corner_three_distance_ft: f32,
    /// 球场几何。
    #[serde(default)]
    pub court: CourtGeometry,
    /// 是否使用交替拥有箭头程序。
    pub use_alternate_possession_arrow: bool,
    /// 场地中心圆的半径（英尺），用于跳球站位。
    #[serde(default = "default_center_circle_radius_ft")]
    pub center_circle_radius_ft: f32,
    /// 跳球时场上接应队员距中圈的最小站位距离（英尺）。
    #[serde(default = "default_jump_ball_receiver_distance_ft")]
    pub jump_ball_receiver_distance_ft: f32,
    /// 持球人被紧逼盯防的最长持球秒数（NBA 五秒、FIBA 五秒）。
    #[serde(default = "default_closely_guarded_seconds")]
    pub closely_guarded_seconds: f32,
    /// 判定为「紧逼盯防」的防守距离上限（英尺）。
    #[serde(default = "default_closely_guarded_distance_ft")]
    pub closely_guarded_distance_ft: f32,
}

fn default_closely_guarded_seconds() -> f32 {
    5.0
}

fn default_roster_limit() -> u32 {
    15
}

fn default_challenges_per_game() -> u32 {
    1
}

fn default_challenge_final_seconds() -> f32 {
    120.0
}

fn default_free_throw_time_limit_seconds() -> f32 {
    10.0
}

fn default_closely_guarded_distance_ft() -> f32 {
    6.0
}

fn default_center_circle_radius_ft() -> f32 {
    6.0
}

fn default_jump_ball_receiver_distance_ft() -> f32 {
    8.0
}

/// 节次策略族：常规节数、节时长与加时时长。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PeriodPolicy {
    pub regulation_periods: u32,
    pub period_duration_seconds: f32,
    pub overtime_duration_seconds: f32,
}

/// 时钟策略族：进攻钟、进攻篮板重置与紧逼盯防判定。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockPolicy {
    pub shot_clock_seconds: f32,
    pub offensive_rebound_shot_clock_seconds: f32,
    pub closely_guarded_seconds: f32,
    pub closely_guarded_distance_ft: f32,
}

/// 犯规策略族：犯满离场、罚球数与罚则门槛。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FoulPolicy {
    pub max_personal_fouls: u8,
    pub shooting_foul_free_throws: u8,
    pub bonus_free_throws: u8,
    pub bonus_fouls_per_period: u32,
    pub bonus_final_period: u32,
    pub bonus_final_seconds: f32,
    pub bonus_fouls_final: u32,
    pub away_from_play_free_throws: u8,
}

/// 暂停策略族：额度、末段限制与加时额度。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeoutPolicy {
    pub timeouts_per_game: u32,
    pub timeouts_fourth_quarter_max: u32,
    pub timeouts_final_seconds: f32,
    pub timeouts_final_max: u32,
    pub overtime_timeouts: u32,
}

impl LeagueProfile {
    /// 节次策略族。引擎读节数与时长时走这里，不直接摸扁平字段。
    pub fn periods(&self) -> PeriodPolicy {
        PeriodPolicy {
            regulation_periods: self.regulation_periods,
            period_duration_seconds: self.period_duration_seconds,
            overtime_duration_seconds: self.overtime_duration_seconds,
        }
    }

    /// 时钟策略族。
    pub fn clocks(&self) -> ClockPolicy {
        ClockPolicy {
            shot_clock_seconds: self.shot_clock_seconds,
            offensive_rebound_shot_clock_seconds: self.offensive_rebound_shot_clock_seconds,
            closely_guarded_seconds: self.closely_guarded_seconds,
            closely_guarded_distance_ft: self.closely_guarded_distance_ft,
        }
    }

    /// 犯规策略族。
    pub fn fouls(&self) -> FoulPolicy {
        FoulPolicy {
            max_personal_fouls: self.max_personal_fouls,
            shooting_foul_free_throws: self.shooting_foul_free_throws,
            bonus_free_throws: self.bonus_free_throws,
            bonus_fouls_per_period: self.bonus_fouls_per_period,
            bonus_final_period: self.bonus_final_period,
            bonus_final_seconds: self.bonus_final_seconds,
            bonus_fouls_final: self.bonus_fouls_final,
            away_from_play_free_throws: self.away_from_play_free_throws,
        }
    }

    /// 暂停策略族。
    pub fn timeouts(&self) -> TimeoutPolicy {
        TimeoutPolicy {
            timeouts_per_game: self.timeouts_per_game,
            timeouts_fourth_quarter_max: self.timeouts_fourth_quarter_max,
            timeouts_final_seconds: self.timeouts_final_seconds,
            timeouts_final_max: self.timeouts_final_max,
            overtime_timeouts: self.overtime_timeouts,
        }
    }
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
        let mut profile: Self = serde_json::from_str(FIBA_PROFILE)
            .expect("data/rules/league_fiba.json must be a valid league profile");
        assert_eq!(profile.id, LeagueId::Fiba);
        assert_eq!(profile.ruleset_version, "2026-v1.1");
        profile
            .source
            .validate()
            .expect("FIBA rule source metadata must be complete");
        profile.court = CourtGeometry::fiba();
        profile
            .validate()
            .expect("FIBA jump-ball settings must be valid");
        profile
    }

    pub fn period_starts_with_jump_ball(&self, period: u32) -> bool {
        assert!(period > 0, "period number must be positive");
        period == 1 || (!self.use_alternate_possession_arrow && period > self.regulation_periods)
    }

    /// 判断比赛时钟是否处于球队犯规末段规则的时间范围。
    pub fn in_final_window(&self, period: u32, period_seconds_left: f64) -> bool {
        self.bonus_final_period > 0
            && period >= self.bonus_final_period
            && period_seconds_left <= f64::from(self.bonus_final_seconds)
    }

    /// 返回当前节适用的球队犯规罚则序号。
    pub fn bonus_foul_threshold(&self, period: u32, period_seconds_left: f64) -> u32 {
        if self.in_final_window(period, period_seconds_left) {
            self.bonus_fouls_final
        } else {
            self.bonus_fouls_per_period
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
            self.bonus_free_throws
        } else {
            0
        }
    }

    /// 前场篮板后的进攻时钟重置：只补足至档案时长，不缩短已有时长。
    pub fn shot_clock_after_offensive_rebound(&self, current: f64, rim_contact: bool) -> f64 {
        if rim_contact {
            current.max(f64::from(self.offensive_rebound_shot_clock_seconds))
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
    fn policy_families_mirror_the_profile() {
        for league in [LeagueProfile::nba(), LeagueProfile::fiba()] {
            let periods = league.periods();
            assert_eq!(periods.regulation_periods, league.regulation_periods);
            assert_eq!(
                periods.period_duration_seconds,
                league.period_duration_seconds
            );
            assert_eq!(
                periods.overtime_duration_seconds,
                league.overtime_duration_seconds
            );
            let clocks = league.clocks();
            assert_eq!(clocks.shot_clock_seconds, league.shot_clock_seconds);
            assert_eq!(
                clocks.offensive_rebound_shot_clock_seconds,
                league.offensive_rebound_shot_clock_seconds
            );
            assert_eq!(
                clocks.closely_guarded_seconds,
                league.closely_guarded_seconds
            );
            let fouls = league.fouls();
            assert_eq!(fouls.max_personal_fouls, league.max_personal_fouls);
            assert_eq!(
                fouls.bonus_fouls_per_period,
                league.bonus_fouls_per_period
            );
            assert_eq!(fouls.bonus_free_throws, league.bonus_free_throws);
            let timeouts = league.timeouts();
            assert_eq!(timeouts.timeouts_per_game, league.timeouts_per_game);
            assert_eq!(timeouts.overtime_timeouts, league.overtime_timeouts);
        }
    }
}
