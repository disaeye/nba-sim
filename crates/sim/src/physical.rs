//! 属性 → 物理量的唯一映射。
//!
//! 决策层与结算层都只认这里算出来的英尺与英尺每秒。属性本身不直接进入
//! 任何概率公式：否则同一个「弹跳 0.7」在判断干扰时和在结算封盖时会
//! 被换算成两种不同的东西，调参也就无法归因。

use crate::params::PolicyTable;
use crate::state::PlayerState;

#[derive(Debug, Clone, Copy)]
pub struct Physical {
    pub speed_ftps: f32,
    pub accel_ftps2: f32,
    /// 起跳高度（英尺）。
    pub vertical_ft: f32,
    /// 站立摸高（英尺）：身高换算，加上手臂与肩宽的固定补偿。
    pub standing_reach_ft: f32,
}

/// 从校准表读取的映射系数。
///
/// 这些系数每 tick 会被每名球员读一遍，所以在运动积分开始前解析一次，
/// 循环内只做乘法。行为与逐次查表完全一致：同一张表的同一批键。
#[derive(Debug, Clone, Copy)]
pub struct Coefficients {
    pub speed_base_ftps: f32,
    pub speed_gain_ftps: f32,
    pub speed_cap_ftps: f32,
    pub accel_base_ftps2: f32,
    pub accel_gain_ftps2: f32,
    pub accel_cap_ftps2: f32,
    pub stamina_floor: f32,
    pub stamina_span: f32,
    pub vertical_base_ft: f32,
    pub vertical_gain_ft: f32,
    pub reach_arm_bonus_ft: f32,
    pub reach_per_cm_ft: f32,
    pub reach_vertical_share: f32,
    pub contest_reach_baseline_ft: f32,
}

impl Coefficients {
    /// 解析一次映射系数。
    pub fn from_policy(policy: &PolicyTable) -> Self {
        Self {
            speed_base_ftps: policy.g("motion_speed_base_ftps"),
            speed_gain_ftps: policy.g("motion_speed_gain_ftps"),
            speed_cap_ftps: policy.g("motion_speed_cap_ftps"),
            accel_base_ftps2: policy.g("motion_accel_base_ftps2"),
            accel_gain_ftps2: policy.g("motion_accel_gain_ftps2"),
            accel_cap_ftps2: policy.g("motion_accel_cap_ftps2"),
            stamina_floor: policy.w("motion_stamina_floor"),
            stamina_span: policy.w("motion_stamina_span"),
            vertical_base_ft: policy.g("vertical_base_ft"),
            vertical_gain_ft: policy.g("vertical_gain_ft"),
            reach_arm_bonus_ft: policy.g("reach_arm_bonus_ft"),
            reach_per_cm_ft: policy.g("reach_per_cm_ft"),
            reach_vertical_share: policy.g("reach_vertical_share"),
            contest_reach_baseline_ft: policy.g("contest_reach_baseline_ft"),
        }
    }
}

impl Physical {
    pub fn of(player: &PlayerState, policy: &PolicyTable) -> Self {
        Self::with(player, &Coefficients::from_policy(policy))
    }

    /// 用已解析的系数计算物理量。
    pub fn with(player: &PlayerState, coefficients: &Coefficients) -> Self {
        let attributes = &player.data.attributes;
        let base = coefficients.speed_base_ftps
            + coefficients.speed_gain_ftps * attributes.speed;
        let stamina =
            coefficients.stamina_floor + coefficients.stamina_span * player.stamina;
        let speed_ftps = (base * stamina).min(coefficients.speed_cap_ftps);
        let accel_ftps2 = (coefficients.accel_base_ftps2
            + coefficients.accel_gain_ftps2 * attributes.acceleration)
            .min(coefficients.accel_cap_ftps2);
        let vertical_ft =
            coefficients.vertical_base_ft + coefficients.vertical_gain_ft * attributes.vertical;
        let standing_reach_ft = coefficients.reach_arm_bonus_ft
            + coefficients.reach_per_cm_ft * f32::from(player.data.height_cm);
        Self {
            speed_ftps,
            accel_ftps2,
            vertical_ft,
            standing_reach_ft,
        }
    }

    /// 对抗与封盖用的触及高度：站立摸高加上起跳。
    pub fn contest_reach_ft(&self, policy: &PolicyTable) -> f32 {
        self.contest_reach_ft_with(policy.g("reach_vertical_share"))
    }

    /// 用已解析的起跳分享系数计算触及高度。
    pub fn contest_reach_ft_with(&self, vertical_share: f32) -> f32 {
        self.standing_reach_ft + self.vertical_ft * vertical_share
    }

    /// 超出基准摸高的部分（英尺）：同样的距离，摸得更高的人干扰更强。
    pub fn reach_excess_ft(&self, policy: &PolicyTable) -> f32 {
        self.reach_excess_ft_with(
            policy.g("reach_vertical_share"),
            policy.g("contest_reach_baseline_ft"),
        )
    }

    /// 用已解析的系数计算超出基准摸高的部分。
    pub fn reach_excess_ft_with(&self, vertical_share: f32, baseline_ft: f32) -> f32 {
        (self.contest_reach_ft_with(vertical_share) - baseline_ft).max(f32::default())
    }
}
