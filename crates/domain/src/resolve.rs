//! 裁决参数档案（ResolveConfig - M7 并轨产物）。
//!
//! 原 officiating/config.rs 的第二 config 体系已并入 GameRules
//! 嵌套组（architecture.md 7.2 / design.md 3.1 M7 验收）：裁决的
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
            foul_rate: 0.08,
            threshold_speed_ftps: 16.0,
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
    pub lane_risk_weight: f32,
    pub openness_weight: f32,
    pub passer_skill_weight: f32,
    pub receiver_control_weight: f32,
    pub catch_equilibrium_weight: f32,
}

impl Default for PassPolicy {
    fn default() -> Self {
        Self {
            lane_risk_weight: 0.35,
            openness_weight: 0.10,
            passer_skill_weight: 0.12,
            receiver_control_weight: 0.08,
            catch_equilibrium_weight: 0.06,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseRates {
    pub pass_success: f32,
    pub handoff_success: f32,
    pub drive_success: f32,
    pub shot_make_2pt: f32,
    pub shot_make_3pt: f32,
    pub ft_make: f32,
    pub steal_attempt_success: f32,
    pub foul_on_drive_rate: f32,
    pub offensive_rebound_rate: f32,
    pub block_rate: f32,
    /// Pass interception adjudication shape.
    pub intercept_steal_slope: f32,
    pub intercept_tip_slope: f32,
    pub intercept_clearance_scale_ft: f32,
    pub intercept_steal_floor: f32,
    pub intercept_steal_ceiling: f32,
    pub intercept_tip_floor: f32,
    pub intercept_tip_ceiling: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotTypeRates {
    pub catch_shoot_2pt: f32,
    pub catch_shoot_3pt: f32,
    pub pull_up_2pt: f32,
    pub pull_up_3pt: f32,
    pub post_2pt: f32,
    pub drive_finish_2pt: f32,
    pub other_2pt: f32,
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
            base_rates: BaseRates {
                pass_success: 0.94,
                handoff_success: 0.97,
                drive_success: 0.78,
                shot_make_2pt: 0.565,
                shot_make_3pt: 0.34,
                ft_make: 0.77,
                steal_attempt_success: 0.005,
                foul_on_drive_rate: 0.10,
                offensive_rebound_rate: 0.26,
                block_rate: 0.05,
                intercept_steal_slope: 0.25,
                intercept_tip_slope: 0.40,
                intercept_clearance_scale_ft: 2.5,
                intercept_steal_floor: 0.05,
                intercept_steal_ceiling: 0.40,
                intercept_tip_floor: 0.10,
                intercept_tip_ceiling: 0.60,
            },
            shot_type_rates: ShotTypeRates {
                catch_shoot_2pt: 0.46,
                catch_shoot_3pt: 0.33,
                pull_up_2pt: 0.40,
                pull_up_3pt: 0.27,
                post_2pt: 0.45,
                drive_finish_2pt: 0.60,
                other_2pt: 0.42,
            },
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
        }
    }
}

impl ResolveConfig {
    /// Rejects non-finite and out-of-range policy before simulation.
    pub fn validate(&self) -> Result<(), String> {
        let probabilities = [
            self.base_rates.pass_success,
            self.base_rates.handoff_success,
            self.base_rates.drive_success,
            self.base_rates.shot_make_2pt,
            self.base_rates.shot_make_3pt,
            self.base_rates.ft_make,
            self.base_rates.steal_attempt_success,
            self.base_rates.foul_on_drive_rate,
            self.base_rates.offensive_rebound_rate,
            self.base_rates.block_rate,
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
            self.pass.lane_risk_weight,
            self.pass.openness_weight,
            self.pass.passer_skill_weight,
            self.pass.receiver_control_weight,
            self.pass.catch_equilibrium_weight,
            self.contact.defender_skill_foul_scale,
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
