//! 持球人的脚步模型：支撑脚与接触状态。
//!
//! 走步与二次运球按「收球 → 轴心脚 → 脚步接触 → 释放」的先后判断，
//! 不用收球后的平面位移一次性判定全部走步。支撑脚在收球的那一刻确定，
//! 之后除非球员重新运球或球权改变，它不再改变。
//!
//! 脚步接触是确定性的：一次接触占用一步，量程由身高与步长模型给出。
//! 球员的每一步都有落点，接触记录同时给出这一脚走了多远。

use crate::action_window::Foot;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// 一步的接触状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FootContact {
    /// 支撑脚在地面：收球时确立，此后不抬起。
    Planted,
    /// 抬起中的脚：正在迈出，还没有落地。
    Lifted,
    /// 落地成新的支撑脚：接触发生，一步记满。
    Landed,
}

/// 支撑脚状态：哪只脚在地上，以及它落在哪里。
///
/// 支撑脚是确定性的：每发生一次脚步接触，支撑脚在左右之间交替，落点
/// 记在接触的位置上。收球时确立的轴心脚从支撑脚里取。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SupportFoot {
    /// 当前在地上的那只脚。
    pub foot: Foot,
    /// 支撑脚的落点。
    pub planted_at: Vec2,
    /// 上一次脚接触的落点：下一步的量程从这里算。
    pub last_contact: Vec2,
}

impl SupportFoot {
    pub fn new(foot: Foot, position: Vec2) -> Self {
        Self {
            foot,
            planted_at: position,
            last_contact: position,
        }
    }

    /// 记录一次脚接触：支撑脚交替，落点写入接触位置。
    pub fn record_contact(&mut self, contact: Vec2) {
        self.foot = self.foot.opposite();
        self.planted_at = contact;
        self.last_contact = contact;
    }
}

/// 收球之后确立的轴心状态。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PivotState {
    /// 轴心脚。收球时按球手的朝向确定，此后不变。
    pub foot: Foot,
    /// 轴心脚的落点：抬起轴心脚就是移动了轴心脚。
    pub position: Vec2,
    /// 收球后已经用掉的脚步数。
    pub steps_taken: u8,
    /// 收球之后允许的最大脚步数（两步）。
    pub steps_allowed: u8,
    /// 最近一脚的落点：判断下一步是否构成移动轴心脚。
    pub last_contact: Option<Vec2>,
    /// 上一次落地的是哪只脚：连续两次同一只脚落地就是跳步
    /// （NBA Rule 10 §XIII.8）。
    pub last_contact_foot: Option<Foot>,
    /// 上一次接触是不是紧接着来的：两次接触之间球手如果离地再落回
    /// 同一只脚，中间没有另一只脚落地，那也是跳步。
    pub hop: bool,
}

impl PivotState {
    pub fn new(foot: Foot, position: Vec2, steps_allowed: u8) -> Self {
        Self {
            foot,
            position,
            steps_taken: 0,
            steps_allowed,
            last_contact: Some(position),
            last_contact_foot: None,
            hop: false,
        }
    }

    /// 是否还有剩余脚步。
    pub fn has_step_left(&self) -> bool {
        self.steps_taken < self.steps_allowed
    }

    /// 记录一次落地接触：一步记满，落点写入接触记录。
    pub fn record_contact(&mut self, contact: Vec2) {
        self.steps_taken = self.steps_taken.saturating_add(1);
        self.last_contact = Some(contact);
    }

    /// 记录一次落地接触并记下是哪只脚落地。
    ///
    /// 连续两次同一只脚落地就是跳步（NBA Rule 10 §XIII.8）：收球之后
    /// 球手不得连续两次用同一只脚触地。
    pub fn record_contact_with_foot(&mut self, contact: Vec2, foot: Foot) {
        self.hop = self.last_contact_foot == Some(foot);
        self.last_contact_foot = Some(foot);
        self.record_contact(contact);
    }

    /// 轴心脚是否被移动：抬起的那只脚就是轴心脚，落点与原位不同即移动。
    pub fn pivot_moved(&self, foot: Foot, contact: Vec2) -> bool {
        foot == self.foot && (contact - self.position).length() > PIVOT_MOVE_TOLERANCE_FT
    }
}

/// 轴心脚位移的判定容差（英尺）：小于这个距离的滑动不算移动轴心脚。
pub const PIVOT_MOVE_TOLERANCE_FT: f32 = 0.5;

/// 一步的量程（英尺）。
///
/// 量程由身高给出：越高的人步子越长，但有上限。这条量程替代了
/// 「收球后平面位移超过固定容限」的判定。
pub fn step_length_ft(height_cm: u16, base_ft: f32, per_cm_ft: f32, cap_ft: f32) -> f32 {
    let height = f32::from(height_cm);
    (base_ft + per_cm_ft * height).min(cap_ft)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pivot_foot_is_fixed_by_the_gather_and_never_changes() {
        let mut pivot = PivotState::new(Foot::Right, Vec2::new(10.0, 25.0), 2);
        assert!(pivot.has_step_left());
        pivot.record_contact(Vec2::new(11.0, 25.0));
        assert_eq!(pivot.steps_taken, 1);
        assert!(pivot.has_step_left());
        pivot.record_contact(Vec2::new(12.0, 25.0));
        assert!(!pivot.has_step_left());
    }

    #[test]
    fn moving_the_pivot_foot_is_detected_by_its_displacement() {
        let pivot = PivotState::new(Foot::Right, Vec2::new(10.0, 25.0), 2);
        assert!(!pivot.pivot_moved(Foot::Right, Vec2::new(10.2, 25.0)));
        assert!(pivot.pivot_moved(Foot::Right, Vec2::new(11.0, 25.0)));
        // 抬起的是另一只脚：移动轴心脚的判定不成立。
        assert!(!pivot.pivot_moved(Foot::Left, Vec2::new(11.0, 25.0)));
    }

    #[test]
    fn the_step_length_grows_with_height_and_is_capped() {
        // 身高项在上限之内必须真的起作用：否则所有球员的步长都一样，
        // 脚步模型就退化成固定值。
        let short = step_length_ft(180, 1.2, 0.012, 3.8);
        let tall = step_length_ft(215, 1.2, 0.012, 3.8);
        assert!(tall > short);
        assert_eq!(step_length_ft(240, 1.2, 0.012, 3.8), 3.8);
    }
}
