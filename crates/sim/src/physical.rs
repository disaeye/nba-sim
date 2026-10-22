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

impl Physical {
    pub fn of(player: &PlayerState, policy: &PolicyTable) -> Self {
        let attributes = &player.data.attributes;
        let base = policy.g("motion_speed_base_ftps")
            + policy.g("motion_speed_gain_ftps") * attributes.speed;
        let stamina =
            policy.w("motion_stamina_floor") + policy.w("motion_stamina_span") * player.stamina;
        let speed_ftps = (base * stamina).min(policy.g("motion_speed_cap_ftps"));
        let accel_ftps2 = (policy.g("motion_accel_base_ftps2")
            + policy.g("motion_accel_gain_ftps2") * attributes.acceleration)
            .min(policy.g("motion_accel_cap_ftps2"));
        let vertical_ft =
            policy.g("vertical_base_ft") + policy.g("vertical_gain_ft") * attributes.vertical;
        let standing_reach_ft = policy.g("reach_arm_bonus_ft")
            + policy.g("reach_per_cm_ft") * f32::from(player.data.height_cm);
        Self {
            speed_ftps,
            accel_ftps2,
            vertical_ft,
            standing_reach_ft,
        }
    }

    /// 对抗与封盖用的触及高度：站立摸高加上起跳。
    pub fn contest_reach_ft(&self, policy: &PolicyTable) -> f32 {
        self.standing_reach_ft + self.vertical_ft * policy.g("reach_vertical_share")
    }

    /// 超出基准摸高的部分（英尺）：同样的距离，摸得更高的人干扰更强。
    pub fn reach_excess_ft(&self, policy: &PolicyTable) -> f32 {
        (self.contest_reach_ft(policy) - policy.g("contest_reach_baseline_ft")).max(f32::default())
    }
}
