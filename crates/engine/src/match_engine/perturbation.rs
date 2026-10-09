//! 校验专用扰动入口（validation.md §7.1 直接机制实验的输入侧）。
//!
//! 扰动只允许走 [`MatchEngine::apply_validation_perturbation`]：属性写入
//! 与 `PlayerAttributes::validate` 同一套生产校验，派生量（物理运动上限）
//! 经 `nba_domain::effective_*` 映射层重算，与 `construction.rs` 初始化
//! 同源。本模块定义扰动目标属性的枚举——编译期限定合法输入，字符串
//! 拼接的属性名是拼写错误温床。
//!
//! 覆盖范围：首批 MECH 实验登记的目标输入（MECH-S01 区域精度、MECH-M01
//! 运动类、MECH-R01 罚球与决策、MECH-B01 对抗与篮板）。其余属性按实验
//! 登记需要逐个加入，不预设全集。

use nba_domain::PlayerAttributes;

/// 校验扰动目标属性。
///
/// `get`/`set` 是字段访问的唯一收编点（新属性接入 = 枚举加变体 +
/// 两个 match 臂，编译器强制覆盖）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationAttribute {
    /// 三分精度（MECH-S01：区域精度对 shooting 输入的响应）。
    ShootingThree,
    /// 近筐精度（MECH-S01）。
    ShootingClose,
    /// 中距离精度（MECH-S01）。
    ShootingMid,
    /// 罚球（MECH-R01）。
    FreeThrow,
    /// 速度（MECH-M01：运动类输入响应）。
    Speed,
    /// 加速度（MECH-M01）。
    Acceleration,
    /// 体力（MECH-M01：消耗与恢复）。
    Stamina,
    /// 力量（MECH-B01：对抗响应）。
    Strength,
    /// 进攻篮板（MECH-B01）。
    OffensiveRebound,
    /// 防守篮板（MECH-B01）。
    DefensiveRebound,
    /// 决策智商（MECH-R01：决策质量响应）。
    DecisionIq,
}

impl ValidationAttribute {
    /// 读取当前值。
    pub fn get(self, attributes: &PlayerAttributes) -> f32 {
        match self {
            ValidationAttribute::ShootingThree => attributes.shooting_three,
            ValidationAttribute::ShootingClose => attributes.shooting_close,
            ValidationAttribute::ShootingMid => attributes.shooting_mid,
            ValidationAttribute::FreeThrow => attributes.free_throw,
            ValidationAttribute::Speed => attributes.speed,
            ValidationAttribute::Acceleration => attributes.acceleration,
            ValidationAttribute::Stamina => attributes.stamina,
            ValidationAttribute::Strength => attributes.strength,
            ValidationAttribute::OffensiveRebound => attributes.offensive_rebound,
            ValidationAttribute::DefensiveRebound => attributes.defensive_rebound,
            ValidationAttribute::DecisionIq => attributes.decision_iq,
        }
    }

    /// 写入新值（合法性由调用方的 `PlayerAttributes::validate` 把关）。
    pub fn set(self, attributes: &mut PlayerAttributes, value: f32) {
        match self {
            ValidationAttribute::ShootingThree => attributes.shooting_three = value,
            ValidationAttribute::ShootingClose => attributes.shooting_close = value,
            ValidationAttribute::ShootingMid => attributes.shooting_mid = value,
            ValidationAttribute::FreeThrow => attributes.free_throw = value,
            ValidationAttribute::Speed => attributes.speed = value,
            ValidationAttribute::Acceleration => attributes.acceleration = value,
            ValidationAttribute::Stamina => attributes.stamina = value,
            ValidationAttribute::Strength => attributes.strength = value,
            ValidationAttribute::OffensiveRebound => attributes.offensive_rebound = value,
            ValidationAttribute::DefensiveRebound => attributes.defensive_rebound = value,
            ValidationAttribute::DecisionIq => attributes.decision_iq = value,
        }
    }

    /// 登记用的稳定名称（实验规格文件与工件输出用）。
    pub fn name(self) -> &'static str {
        match self {
            ValidationAttribute::ShootingThree => "shooting_three",
            ValidationAttribute::ShootingClose => "shooting_close",
            ValidationAttribute::ShootingMid => "shooting_mid",
            ValidationAttribute::FreeThrow => "free_throw",
            ValidationAttribute::Speed => "speed",
            ValidationAttribute::Acceleration => "acceleration",
            ValidationAttribute::Stamina => "stamina",
            ValidationAttribute::Strength => "strength",
            ValidationAttribute::OffensiveRebound => "offensive_rebound",
            ValidationAttribute::DefensiveRebound => "defensive_rebound",
            ValidationAttribute::DecisionIq => "decision_iq",
        }
    }
}
