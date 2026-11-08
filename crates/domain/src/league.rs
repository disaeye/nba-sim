//! 联赛规则档案（LeagueProfile · M10 / architecture.md §6.3 / 宪章 C3）。
//!
//! 计时、球队犯规罚则与场地几何使用版本化档案。不同联赛共用同一套规则程序。
//!
//! 档案按**程序**组织成 11 个策略族：节次、时钟、进攻钟、犯规、罚球、发球、
//! 跳球、暂停、换人、违例与复核。每个策略族是一段完整的程序语义，读它的
//! 代码只在一处。

use crate::court::CourtGeometry;
use serde::{Deserialize, Serialize};

/// 支持的联赛档案。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeagueId {
    Nba,
    Fiba,
}

impl LeagueId {
    /// 该档案标识对应的规则档案。
    pub fn profile(self) -> LeagueProfile {
        match self {
            LeagueId::Nba => LeagueProfile::nba(),
            LeagueId::Fiba => LeagueProfile::fiba(),
        }
    }

    /// 档案标识的字符串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            LeagueId::Nba => "nba",
            LeagueId::Fiba => "fiba",
        }
    }
}

impl std::str::FromStr for LeagueId {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text {
            "nba" => Ok(LeagueId::Nba),
            "fiba" => Ok(LeagueId::Fiba),
            other => Err(format!(
                "unknown league `{other}`: expected `nba` or `fiba`"
            )),
        }
    }
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
#[serde(deny_unknown_fields)]
pub struct LeagueProfile {
    pub id: LeagueId,
    pub name: String,
    pub ruleset_version: String,
    pub source: RuleSource,
    /// 节次策略族：常规节数、节时长与加时时长。
    #[serde(flatten)]
    pub periods: PeriodPolicy,
    /// 时钟策略族：紧逼盯防判定与节次停顿。
    #[serde(flatten)]
    pub clocks: ClockPolicy,
    /// 进攻钟策略族：进攻钟时长与前场篮板后的重置。
    #[serde(flatten)]
    pub shot_clock: ShotClockPolicy,
    /// 犯规策略族：犯满离场与罚则门槛。
    #[serde(flatten)]
    pub fouls: FoulPolicy,
    /// 罚球策略族：各情形下的罚球数与出手时限。
    #[serde(flatten)]
    pub free_throws: FreeThrowPolicy,
    /// 发球策略族：发球时限与发球准备时长。
    #[serde(flatten)]
    pub inbound: InboundPolicy,
    /// 跳球策略族：跳球站位几何与交替拥有箭头。
    #[serde(flatten)]
    pub jump_ball: JumpBallPolicy,
    /// 暂停策略族：额度、末段限制、加时额度与强制暂停。
    #[serde(flatten)]
    pub timeouts: TimeoutPolicy,
    /// 换人策略族：赛前名单上限与死球换人窗口。
    #[serde(flatten)]
    pub substitutions: SubstitutionPolicy,
    /// 违例策略族：该联赛认可的违例种类。
    pub violations: ViolationPolicy,
    /// 复核策略族：裁判权限、录像复核范围与教练挑战。
    pub review: ReviewPolicy,
    /// 犯规分类族：认可的犯规种类与技术犯规子类。
    pub foul_kinds: FoulKindPolicy,
    /// 三分线弧顶距离，单位为英尺。
    pub three_point_distance_ft: f32,
    /// 底角三分线距离，单位为英尺；零值表示无单独底角距离。
    pub corner_three_distance_ft: f32,
    /// 球场几何。
    pub court: CourtGeometry,
}

/// 节次策略族：常规节数、节时长与加时时长。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeriodPolicy {
    pub regulation_periods: u32,
    pub period_duration_seconds: f32,
    pub overtime_duration_seconds: f32,
}

/// 时钟策略族：紧逼盯防判定与进攻钟之外的计时条件。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockPolicy {
    pub closely_guarded_seconds: f32,
    pub closely_guarded_distance_ft: f32,
}

/// 进攻钟策略族：进攻钟时长与前场篮板后的重置。
///
/// 进攻钟与比赛钟分属两个程序：比赛钟回答「这一节还剩多久」，进攻钟回答
/// 「这一回合还剩多久」。两者的规则来源不同，因此单独成族。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShotClockPolicy {
    pub shot_clock_seconds: f32,
    pub offensive_rebound_shot_clock_seconds: f32,
}

/// 犯规策略族：犯满离场、罚球数与罚则门槛。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoulPolicy {
    pub max_personal_fouls: u8,
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
    pub bonus_fouls_overtime: u32,
    /// 加时球队犯规是否计入第四节累计。
    ///
    /// NBA 为否（加时单独计数），FIBA 为是（Article 42.1.3）。
    pub overtime_fouls_carry_from_fourth: bool,
}

/// 罚球策略族：各情形下的罚球数与出手时限。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FreeThrowPolicy {
    /// 投篮犯规（非加罚）的罚球数。
    pub shooting_foul_free_throws: u8,
    /// 罚球人必须在这么多秒内出手，否则违例。
    pub free_throw_time_limit_seconds: f32,
    /// 两次罚球之间与第一次罚球前的准备间隔。
    pub free_throw_interval_seconds: f32,
}

/// 发球策略族：发球时限与发球准备时长。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InboundPolicy {
    /// 发球人必须在这么多秒内把球传出，否则违例。
    pub inbound_seconds: f32,
    /// 发球程序从开始到发球人可以把球传出的准备时长。
    pub inbound_setup_seconds: f32,
}

/// 跳球策略族：跳球站位几何与交替拥有箭头。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JumpBallPolicy {
    /// 球场中心圆的半径，单位为英尺。
    pub center_circle_radius_ft: f32,
    /// 跳球时场上接应队员距中圈的最小站位距离，单位为英尺。
    pub jump_ball_receiver_distance_ft: f32,
    /// 是否使用交替拥有箭头程序。
    pub use_alternate_possession_arrow: bool,
}

/// 暂停策略族：额度、末段限制、加时额度、时长与强制暂停。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutPolicy {
    pub timeouts_per_game: u32,
    pub timeouts_fourth_quarter_max: u32,
    /// 末段暂停限制的生效节次；零表示该联赛没有这条限制。
    ///
    /// NBA 2025-26 Rule 8：第四节最后三分钟每队最多两次暂停。FIBA 2026
    /// Article 18 没有末段暂停限制，这个值为零。
    pub timeouts_final_period: u32,
    pub timeouts_final_seconds: f32,
    pub timeouts_final_max: u32,
    pub overtime_timeouts: u32,
    /// 暂停占用的比赛时长，按来源区分。
    pub timeout_durations: TimeoutDurations,
    /// 强制暂停的触发点：每到这些时点，下一次死球自动停表。
    ///
    /// NBA 2025-26 Rule 5 §VI：第二节与第四节各有一次强制暂停，
    /// 分别发生在时钟走到 6:59 与 2:59 以下后的第一次死球。FIBA 2026
    /// Article 18 没有强制暂停，这个表为空。
    #[serde(default)]
    pub mandatory_timeouts: Vec<MandatoryTimeout>,
}

/// 暂停时长按来源区分：完整暂停、短暂停与加时暂停。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeoutDurations {
    pub full_seconds: f32,
    pub short_seconds: f32,
    pub overtime_seconds: f32,
}

/// 一个强制暂停触发点。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MandatoryTimeout {
    /// 生效的节次。
    pub period: u32,
    /// 本节剩余时间不超过该值（秒）后的第一次死球触发。
    pub remaining_seconds: f32,
    /// 一次触发给每支球队记几次暂停。
    pub charge: u32,
    /// 这次强制暂停占用的时长（秒）。
    pub duration_seconds: f32,
}

/// 换人策略族：赛前名单上限与死球换人窗口。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubstitutionPolicy {
    /// 赛前名单上限（NBA 15，FIBA 12）。
    pub roster_limit: u32,
    /// 死球时可以换人的比赛程序。
    ///
    /// 名单里的这些程序在死球期间允许换人：换人窗口由规则给出，
    /// 不由引擎的阶段表猜。
    pub dead_ball_windows: Vec<String>,
}

/// 违例策略族：该联赛认可的违例种类与各类时限。
///
/// 这份表把「哪些行为算违例」从代码里抽到档案里：FIBA 与 NBA 的违例集合
/// 不完全相同，未登记的违例在这个档案下不成立。各类违例的时限也在这里，
/// 因为时限就是违例规则的一部分。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViolationPolicy {
    /// 该档案认可的违例种类，按 [`crate::event::ViolationKind`] 的序列化名书写。
    pub recognized: Vec<String>,
    /// 各类违例的时限。
    pub limits: ViolationLimits,
}

/// 各类违例的时限（秒）。
///
/// 零值表示该联赛没有这条规则：FIBA 没有守方三秒，也没有背身五秒。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViolationLimits {
    /// 攻方三秒时限：进攻人在限制区连续停留超过该时限即违例。
    pub offensive_three_second_seconds: f32,
    /// 守方三秒时限；零表示该联赛没有守方三秒规则。
    pub defensive_three_second_seconds: f32,
    /// 守方三秒违例是否判技术犯规（NBA 是，FIBA 无此规则）。
    pub defensive_three_second_is_technical: bool,
    /// 背身五秒时限；零表示该联赛没有这条规则。
    pub back_to_basket_seconds: f32,
    /// 八秒运球时限：控球方在后场停留超过该时限即违例。
    pub backcourt_seconds: f32,
}

/// 复核策略族：裁判权限、录像复核范围与教练挑战。
///
/// 两个档案的差异在数据里：谁有权吹哪类判罚，录像复核能推翻哪几类裁决，
/// 教练挑战有多少次、在什么时间窗口内可以申请。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewPolicy {
    /// 裁判可以单方面作出的判罚类别。
    pub authorization: Vec<String>,
    /// 录像复核可以推翻的裁决类别。
    pub review_scope: Vec<String>,
    /// 每队每场的教练挑战额度（NBA 1，无挑战的档案为 0）。
    pub challenges_per_game: u32,
    /// 复核窗口：本节剩余时间不超过该值时才能申请复核。
    pub challenge_final_seconds: f32,
}

/// 犯规分类族：该联赛认可哪些犯规种类，以及技术犯规的子类。
///
/// 这份表把「犯规叫什么」从代码里抽到档案里：NBA 2025-26 没有破坏性
/// 犯规这一类，同类行为归入恶意犯规；FIBA 2026 Article 36 把它单独
/// 成一类，并把技术犯规分成两个等级。判罚与记账都读这张表。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FoulKindPolicy {
    /// 该联赛认可的犯规种类，按 [`FoulKind`] 的序列化名书写。
    pub recognized: Vec<String>,
    /// 技术犯规的子类；空表示不区分等级。
    #[serde(default)]
    pub technical_categories: Vec<String>,
    /// 破坏性犯规是否是与恶意犯规分开的独立种类。
    pub disruptive_is_separate: bool,
}

impl LeagueProfile {
    /// 按档案标识读取档案。
    pub fn for_id(id: LeagueId) -> Self {
        id.profile()
    }

    pub fn validate(&self) -> Result<(), String> {
        if !self.center_circle_radius_ft_is_valid() {
            return Err("jump-ball court geometry is invalid".to_string());
        }
        if self.shot_clock.shot_clock_seconds <= 0.0
            || self.shot_clock.offensive_rebound_shot_clock_seconds <= 0.0
        {
            return Err("shot clock durations must be positive".to_string());
        }
        if self.free_throws.free_throw_time_limit_seconds <= 0.0
            || self.free_throws.free_throw_interval_seconds <= 0.0
        {
            return Err("free throw timing must be positive".to_string());
        }
        if self.inbound.inbound_seconds <= 0.0 || self.inbound.inbound_setup_seconds <= 0.0 {
            return Err("inbound timing must be positive".to_string());
        }
        if self.substitutions.roster_limit == 0 {
            return Err("roster limit must be positive".to_string());
        }
        if self.violations.limits.backcourt_seconds <= 0.0
            || self.violations.limits.offensive_three_second_seconds <= 0.0
            || self.violations.limits.defensive_three_second_seconds < 0.0
            || self.violations.limits.back_to_basket_seconds < 0.0
        {
            return Err("violation limits must be non-negative".to_string());
        }
        if self.violations.limits.defensive_three_second_seconds > 0.0
            && !self.violations.limits.defensive_three_second_is_technical
        {
            return Err(
                "a league with a defensive three-second rule must grade it as a technical foul"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn center_circle_radius_ft_is_valid(&self) -> bool {
        self.jump_ball.center_circle_radius_ft.is_finite()
            && self.jump_ball.center_circle_radius_ft > 0.0
            && self.jump_ball.jump_ball_receiver_distance_ft.is_finite()
            && self.jump_ball.jump_ball_receiver_distance_ft
                > self.jump_ball.center_circle_radius_ft
            && self.jump_ball.jump_ball_receiver_distance_ft < self.court.width_ft * 0.5
            && self.jump_ball.center_circle_radius_ft < self.court.width_ft * 0.25
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
            .expect("NBA league profile must be internally consistent");
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
            .expect("FIBA league profile must be internally consistent");
        profile
    }

    pub fn period_starts_with_jump_ball(&self, period: u32) -> bool {
        assert!(period > 0, "period number must be positive");
        period == 1
            || (!self.jump_ball.use_alternate_possession_arrow
                && period > self.periods.regulation_periods)
    }

    /// 判断比赛时钟是否处于球队犯规末段规则的时间范围。
    pub fn in_final_window(&self, period: u32, period_seconds_left: f64) -> bool {
        self.fouls.bonus_final_period > 0
            && period >= self.fouls.bonus_final_period
            && period_seconds_left <= f64::from(self.fouls.bonus_final_seconds)
    }

    /// 该档案的犯规分类是否把破坏性犯规当成独立种类。
    pub fn disruptive_foul_is_separate(&self) -> bool {
        self.foul_kinds.disruptive_is_separate
    }

    /// 该档案是否认可这类违例。
    pub fn recognizes_violation(&self, kind: &str) -> bool {
        self.violations.recognized.iter().any(|entry| entry == kind)
    }

    /// 该档案是否在这个比赛程序里开放换人窗口。
    pub fn allows_substitution_in(&self, program: &str) -> bool {
        self.substitutions
            .dead_ball_windows
            .iter()
            .any(|entry| entry == program)
    }

    /// 录像复核是否可以推翻这类裁决。
    pub fn reviews(&self, scope: &str) -> bool {
        self.review
            .review_scope
            .iter()
            .any(|entry| entry == scope)
    }

    /// 裁判是否有权单方面作出这类判罚。
    pub fn authorizes(&self, judgement: &str) -> bool {
        self.review
            .authorization
            .iter()
            .any(|entry| entry == judgement)
    }

    /// 返回当前节适用的球队犯规罚则序号。
    ///
    /// 顺序：加时节的独立门槛先于末段规则，最后是常规节门槛。
    /// NBA 加时前三次普通犯规只发球（Rule 12B §V），FIBA 加时沿用
    /// 第四节累计（Article 42.1.3）。
    pub fn bonus_foul_threshold(&self, period: u32, period_seconds_left: f64) -> u32 {
        if self.in_final_window(period, period_seconds_left) {
            self.fouls.bonus_fouls_final
        } else if period > self.periods.regulation_periods && self.fouls.bonus_fouls_overtime > 0 {
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
            current.max(f64::from(self.shot_clock.offensive_rebound_shot_clock_seconds))
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
    fn the_eleven_policy_families_survive_the_json_round_trip() {
        // 磁盘上仍是扁平键：`serde(flatten)` 把各策略族写回顶层，
        // 读回时又落回策略族。
        for league in [LeagueProfile::nba(), LeagueProfile::fiba()] {
            let text = serde_json::to_string(&league).expect("league profile serializes");
            let parsed: LeagueProfile =
                serde_json::from_str(&text).expect("league profile deserializes");
            assert_eq!(parsed.periods, league.periods);
            assert_eq!(parsed.clocks, league.clocks);
            assert_eq!(parsed.shot_clock, league.shot_clock);
            assert_eq!(parsed.fouls, league.fouls);
            assert_eq!(parsed.free_throws, league.free_throws);
            assert_eq!(parsed.inbound, league.inbound);
            assert_eq!(parsed.jump_ball, league.jump_ball);
            assert_eq!(parsed.timeouts, league.timeouts);
            assert_eq!(parsed.substitutions, league.substitutions);
            assert_eq!(parsed.violations, league.violations);
            assert_eq!(parsed.review, league.review);
            assert!(text.contains("\"period_duration_seconds\""));
            assert!(text.contains("\"shot_clock_seconds\""));
            assert!(text.contains("\"max_personal_fouls\""));
        }
    }
}
