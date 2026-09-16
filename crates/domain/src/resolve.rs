//! 裁决参数档案（ResolveConfig - M7 并轨产物）。
//!
//! 原 officiating/config.rs 的第二 config 体系已并入 GameRules
//! 嵌套组（architecture.md §6 / protocol.md §2.1 M7 验收）：裁决的
//! 全部概率与权重只经 GameRules.resolve 一条规则通道注入。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ResolveConfig {
    pub foundation_version: String,
    /// 罚球概率夹取范围（技能调制的边界，规则通道）。
    pub ft_probability_floor: f32,
    pub ft_probability_ceiling: f32,
    pub model: String,
    pub base_rates: BaseRates,
    pub shot_type_rates: ShotTypeRates,
    pub shot_type_block_bias: ShotTypeBlockBias,
    /// Contact policy is data, not a threshold embedded in adjudication.
    pub contact: ContactPolicy,
    /// Attribute contribution to a player's effective shooting probability.
    pub player_skill: PlayerSkillPolicy,
    /// Attribute, spacing, and contact policy used by drive adjudication.
    pub drive: DrivePolicy,
    /// Physical distance and player capability policy used for rebounds.
    pub rebound: ReboundPolicy,
    /// Physical lane and player capability policy used for passes.
    pub pass: PassPolicy,
    /// On-ball ball-security policy: pressure, containment, and poke-check shape.
    pub ball_security: BallSecurityPolicy,
}

/// 持球安全（on-ball security）裁决策略。
///
/// ## 为什么需要它（round-13 结构发现）
///
/// 真实 NBA 的失误构成（82games 2024-25 IND）为：带球丢球 **53.6%**、
/// 传球失误 33.6%、进攻犯规/违例 12.1%。此前引擎只有「传球失败」一条
/// 失误路径（`TurnoverPassDropped` / `TurnoverPassTipped` / `TurnoverSteal`），
/// `TurnoverLooseBall` 在 718 回合中只出现 1 次 —— 即**占比最大的失误
/// 类型在模型里不存在**。
///
/// 本策略为「持球人被贴身施压后丢球」提供事实路径：几何可达 → 概率 →
/// 抽样，形状与 `BaseRates` 的 `intercept_*` 一致（释放时裁定、逐 tick
/// 回放），因此不引入逐 tick 概率累积的错误语义。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BallSecurityPolicy {
    /// 防守者可发起切球的接触半径（ft）：防守人与此半径内才算贴身。
    pub poke_pressure_radius_ft: f32,
    /// 单位时间切球尝试的基础速率（每秒尝试次数）。
    pub poke_attempt_rate_per_sec: f32,
    /// 单次切球尝试的成功概率上限（技能调制前）。
    pub poke_success_ceiling: f32,
    /// 单次切球尝试的成功概率下限。
    pub poke_success_floor: f32,
    /// 防守人 `steal` 能力对成功概率的权重。
    pub poke_defender_skill_weight: f32,
    /// 持球人 `ball_handling` 能力对成功概率的抑制权重。
    pub poke_handler_skill_weight: f32,
    /// 持球人护球倾向对成功概率的抑制权重。
    pub poke_handler_tendency_weight: f32,
    /// 切球成功时球的弹出方向相对防守人→球方向的随机偏转上限（弧度）。
    pub poke_deflection_spread_rad: f32,
    /// 切球弹出速度上限（ft/s）。
    ///
    /// 切球是**小幅拨离**，不是全速发射：球原本在持球人手里（近乎静止），
    /// 被拨一下只能获得有限初速。硬用 `ball_max_speed_ftps` 会让球“瞬移”，
    /// 并使 3D 速度（水平 + 重力下落的竖直分量）越过 `BALL_SPEED` 不变量
    /// （实测 seed 31337 tick 3755：水平 83.5 + 竖直 16.0 = 85.0 > 上限）。
    pub poke_ball_speed_ftps: f32,
    /// 切球弹出速度占 `本次球速上限` 的比例（保底用，使不同规则档案下
    /// 仍不越过不变量）。
    pub poke_ball_speed_ratio: f32,
}

impl Default for BallSecurityPolicy {
    fn default() -> Self {
        Self {
            poke_pressure_radius_ft: 4.5,
            // round-16 标定（A/B 证据 §17.7）：0.55 → 2.2。
            // 失误构成对齐真实：丢球占比 24%(r1.5)/40%(r3.0) 的插值点，
            // 丢球率 0.078/回合 ≈ 真实 0.0776（82games 53.6% × 0.145）。
            poke_attempt_rate_per_sec: 2.2,
            poke_success_ceiling: 0.16,
            poke_success_floor: 0.008,
            poke_defender_skill_weight: 0.55,
            poke_handler_skill_weight: 0.70,
            poke_handler_tendency_weight: 0.25,
            poke_deflection_spread_rad: 0.9,
            poke_ball_speed_ftps: 14.0,
            poke_ball_speed_ratio: 0.18,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerSkillPolicy {
    pub shooting_weight: f32,
    pub finishing_weight: f32,
    pub free_throw_weight: f32,
    pub passing_weight: f32,
    pub defense_weight: f32,
    pub rebounding_weight: f32,
}

impl Default for PlayerSkillPolicy {
    fn default() -> Self {
        Self {
            shooting_weight: 0.18,
            finishing_weight: 0.12,
            free_throw_weight: 0.30,
            passing_weight: 0.12,
            defense_weight: 0.12,
            rebounding_weight: 0.15,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DrivePolicy {
    /// Normalized paint density penalty applied to reaching the rim.
    pub lane_density_penalty: f32,
    /// Defender contest penalty applied to reaching the rim.
    pub contest_penalty: f32,
    /// Additional foul likelihood contributed by contest intensity.
    pub foul_contest_weight: f32,
    /// Additional contest penalty applied to the finishing attempt.
    pub finish_contest_penalty: f32,
}

impl Default for DrivePolicy {
    fn default() -> Self {
        Self {
            lane_density_penalty: 0.20,
            contest_penalty: 0.22,
            foul_contest_weight: 0.5,
            finish_contest_penalty: 0.18,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ContactPolicy {
    pub foul_rate: f32,
    pub threshold_speed_ftps: f32,
    pub impact_scale_ftps: f32,
    pub screen_foul_multiplier: f32,
    pub defender_skill_foul_scale: f32,
    pub legal_position_foul_multiplier: f32,
    pub illegal_position_foul_multiplier: f32,
}

impl Default for ContactPolicy {
    fn default() -> Self {
        Self {
            foul_rate: 0.12,
            threshold_speed_ftps: 9.0,
            impact_scale_ftps: 16.0,
            screen_foul_multiplier: 1.0,
            defender_skill_foul_scale: 0.35,
            legal_position_foul_multiplier: 0.15,
            illegal_position_foul_multiplier: 1.0,
        }
    }
}

/// Rebound contest policy. Geometry and player attributes remain separate
/// inputs, so no roster identity can be baked into the adjudication.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ReboundPolicy {
    pub base_offensive_rate: f32,
    pub distance_weight: f32,
    pub attribute_weight: f32,
    pub stamina_weight: f32,
    pub positioning_weight: f32,
    pub strength_rebound_weight: f32,
    pub strength_positioning_weight: f32,
    pub strength_stamina_weight: f32,
}

impl Default for ReboundPolicy {
    fn default() -> Self {
        Self {
            base_offensive_rate: 0.26,
            distance_weight: 1.0,
            attribute_weight: 0.35,
            stamina_weight: 0.15,
            positioning_weight: 0.20,
            strength_rebound_weight: 0.70,
            strength_positioning_weight: 0.20,
            strength_stamina_weight: 0.10,
        }
    }
}

/// Pass-resolution weights keep physical lane facts and player capability
/// contributions configurable at the competition boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PassPolicy {
    pub openness_weight: f32,
    pub passer_skill_weight: f32,
    pub receiver_control_weight: f32,
    pub catch_equilibrium_weight: f32,
}

impl Default for PassPolicy {
    fn default() -> Self {
        Self {
            openness_weight: 0.10,
            passer_skill_weight: 0.12,
            receiver_control_weight: 0.08,
            catch_equilibrium_weight: 0.06,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BaseRates {
    pub pass_success: f32,
    pub drive_success: f32,
    /// 篮下（`dist_to_hoop < GameRules.rim_shot_distance_ft`）投篮命中基准。
    pub shot_make_2pt: f32,
    /// 中距离（廊下之外、三分线内）投篮命中基准。
    ///
    /// 契约依据：`docs/attributes.md` §4 / `docs/quality.md` §2.1 要求命中率
    /// 与出手区域匹配。真实篮球中距离命中（约 0.42）显著低于廊下（约 0.63）；
    /// 二者共用一个基准会把中距离按廊下结算，形成结构性偏高。
    /// 分区而非硬编码在引擎里，以遵守 `charter` C1（行为参数走数据通道）。
    pub shot_make_mid: f32,
    pub shot_make_3pt: f32,
    pub ft_make: f32,
    pub foul_on_drive_rate: f32,
    /// 跳投（含三分）被干扰时造成投篮犯规的概率。
    ///
    /// 为什么需要单独一条（evidence/problem.md §23.9）：全仓
    /// `shooting_foul` 原本只在 `DriveResolution` 产生，即**只有突破能被犯规**，
    /// 跳投在被干扰时没有任何造犯规可能。实测 seed42 全场仅 12 次犯规
    /// （真实 NBA 约 40），`free_throw_rate` 因此只有 0.110（带 [0.20,0.35]）。
    /// 这是缺失的程序路径，不是参数偏差，所以新建字段而非调已有值。
    ///
    /// 已知未闭合：该路径使罚球率只到 0.118，仍越带；瓶颈在接触强度
    /// 分布而非本参数（evidence/problem.md §23.12），不得调大凑数。
    pub foul_on_shot_rate: f32,
    /// Pass interception adjudication shape.
    pub intercept_steal_slope: f32,
    pub intercept_tip_slope: f32,
    pub intercept_clearance_scale_ft: f32,
    pub intercept_steal_floor: f32,
    pub intercept_steal_ceiling: f32,
    pub intercept_tip_floor: f32,
    pub intercept_tip_ceiling: f32,
}

impl Default for BaseRates {
    fn default() -> Self {
        Self {
            pass_success: 0.94,
            drive_success: 0.78,
            shot_make_2pt: 0.565,
            // 中距离基准：公开赛季口径约 0.42。此前与廊下共用 0.565，
            // 使 8ft–三分线的出手被按廊下结算（evidence/problem.md §21.3）。
            shot_make_mid: 0.42,
            shot_make_3pt: 0.34,
            ft_make: 0.77,
            foul_on_drive_rate: 0.18,
            // 跳投犯规基准：真实 NBA 每场约 40 次犯规，其中相当部分来自
            // 跳投犯规（三分犯规 / 中距离投篮犯规 / and-one）。
            // 干扰强度在上层作为自变量乘入，此处为“受到实质干扰时”的基准。
            foul_on_shot_rate: 0.06,
            // round-16 调整（A/B 证据 §17.6）：传球选择修复（距离衰减）后，
            // 拦截斜率减半：失败率 10.7%→7.5%、e 0.231→0.178、n 2.47。
            // 选择层已不再系统性喂长传，裁决层的惩罚强度相应回调。
            intercept_steal_slope: 0.06,
            intercept_tip_slope: 0.10,
            intercept_clearance_scale_ft: 2.5,
            intercept_steal_floor: 0.01,
            intercept_steal_ceiling: 0.25,
            intercept_tip_floor: 0.02,
            intercept_tip_ceiling: 0.40,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShotTypeRates {
    pub catch_shoot_2pt: f32,
    pub catch_shoot_3pt: f32,
    pub pull_up_2pt: f32,
    pub pull_up_3pt: f32,
    pub post_2pt: f32,
    pub drive_finish_2pt: f32,
    pub other_2pt: f32,
}

impl Default for ShotTypeRates {
    fn default() -> Self {
        Self {
            catch_shoot_2pt: 0.46,
            catch_shoot_3pt: 0.33,
            pull_up_2pt: 0.40,
            pull_up_3pt: 0.27,
            post_2pt: 0.45,
            drive_finish_2pt: 0.60,
            other_2pt: 0.42,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotTypeBlockBias {
    pub drive_finish: f32,
    pub post: f32,
    pub catch_shoot: f32,
    pub pull_up: f32,
    pub other: f32,
}

impl Default for ResolveConfig {
    fn default() -> Self {
        Self {
            foundation_version: "0.15.0".to_string(),
            ft_probability_floor: 0.40,
            ft_probability_ceiling: 0.96,
            model: "homogeneous_v0_1".to_string(),
            base_rates: BaseRates::default(),
            shot_type_rates: ShotTypeRates::default(),
            shot_type_block_bias: ShotTypeBlockBias {
                drive_finish: 1.4,
                post: 1.2,
                catch_shoot: 0.7,
                pull_up: 0.8,
                other: 1.0,
            },
            contact: ContactPolicy::default(),
            player_skill: PlayerSkillPolicy::default(),
            drive: DrivePolicy::default(),
            rebound: ReboundPolicy::default(),
            pass: PassPolicy::default(),
            ball_security: BallSecurityPolicy::default(),
        }
    }
}

impl ResolveConfig {
    /// Rejects non-finite and out-of-range policy before simulation.
    pub fn validate(&self) -> Result<(), String> {
        let probabilities = [
            self.base_rates.pass_success,
            self.base_rates.drive_success,
            self.base_rates.shot_make_2pt,
            self.base_rates.shot_make_3pt,
            self.base_rates.shot_make_mid,
            self.base_rates.ft_make,
            self.base_rates.foul_on_drive_rate,
            self.base_rates.foul_on_shot_rate,
            self.base_rates.intercept_steal_floor,
            self.base_rates.intercept_steal_ceiling,
            self.base_rates.intercept_tip_floor,
            self.base_rates.intercept_tip_ceiling,
            self.shot_type_rates.catch_shoot_2pt,
            self.shot_type_rates.catch_shoot_3pt,
            self.shot_type_rates.pull_up_2pt,
            self.shot_type_rates.pull_up_3pt,
            self.shot_type_rates.post_2pt,
            self.shot_type_rates.drive_finish_2pt,
            self.shot_type_rates.other_2pt,
            self.contact.foul_rate,
            self.rebound.base_offensive_rate,
            self.ball_security.poke_success_floor,
            self.ball_security.poke_success_ceiling,
        ];
        if probabilities
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("resolve probabilities must be finite and within 0..=1".to_string());
        }
        if self.base_rates.intercept_steal_floor > self.base_rates.intercept_steal_ceiling
            || self.base_rates.intercept_tip_floor > self.base_rates.intercept_tip_ceiling
            || self.base_rates.intercept_clearance_scale_ft <= 0.0
            || !self.base_rates.intercept_clearance_scale_ft.is_finite()
        {
            return Err("interception policy bounds are invalid".to_string());
        }
        if self.ball_security.poke_success_floor > self.ball_security.poke_success_ceiling
            || self.ball_security.poke_pressure_radius_ft <= 0.0
            || !self.ball_security.poke_pressure_radius_ft.is_finite()
            || !(0.0..=1.0).contains(&self.ball_security.poke_ball_speed_ratio)
        {
            return Err("ball-security policy bounds are invalid".to_string());
        }
        if [
            self.player_skill.shooting_weight,
            self.player_skill.finishing_weight,
            self.player_skill.passing_weight,
            self.player_skill.defense_weight,
            self.player_skill.rebounding_weight,
            self.drive.lane_density_penalty,
            self.drive.contest_penalty,
            self.drive.foul_contest_weight,
            self.drive.finish_contest_penalty,
            self.rebound.distance_weight,
            self.rebound.attribute_weight,
            self.rebound.stamina_weight,
            self.rebound.positioning_weight,
            self.pass.openness_weight,
            self.pass.passer_skill_weight,
            self.pass.receiver_control_weight,
            self.pass.catch_equilibrium_weight,
            self.contact.defender_skill_foul_scale,
            self.ball_security.poke_attempt_rate_per_sec,
            self.ball_security.poke_defender_skill_weight,
            self.ball_security.poke_handler_skill_weight,
            self.ball_security.poke_handler_tendency_weight,
            self.ball_security.poke_deflection_spread_rad,
            self.ball_security.poke_ball_speed_ftps,
            self.ball_security.poke_ball_speed_ratio,
        ]
        .iter()
        .any(|value| !value.is_finite() || *value < 0.0)
        {
            return Err("resolve policy weights must be finite and non-negative".to_string());
        }
        if !self.contact.threshold_speed_ftps.is_finite()
            || !self.contact.impact_scale_ftps.is_finite()
            || !self.contact.screen_foul_multiplier.is_finite()
            || self.contact.threshold_speed_ftps < 0.0
            || self.contact.impact_scale_ftps <= 0.0
            || self.contact.screen_foul_multiplier < 0.0
        {
            return Err("contact policy contains an invalid value".to_string());
        }
        if self.shot_type_block_bias.drive_finish.is_sign_negative()
            || self.shot_type_block_bias.post.is_sign_negative()
            || self.shot_type_block_bias.catch_shoot.is_sign_negative()
            || self.shot_type_block_bias.pull_up.is_sign_negative()
            || self.shot_type_block_bias.other.is_sign_negative()
        {
            return Err("shot block biases must be non-negative".to_string());
        }
        Ok(())
    }
}
