//! 持球姿态（面框/背身）的技术选择。
//!
//! 接球后的第一件事不是选动作，是选姿态：面框进入三威胁，
//! 还是背身要位。这决定了后续动作族的可选集与朝向语义——
//! 决策层在新持球确立时评估一次，期间保持，失去球权即重置。

use glam::Vec2;
use nba_domain::action_window::BallOrientation;
use nba_domain::GameRules;
use nba_physics::movement::PlayerPhysicsState;

/// 评估持球人应采取的持球姿态。
///
/// 背身亲和度 = 内线技术加权（strength / shooting_near / finishing）
/// − 外线技术加权（ball_handling / shooting_three），再乘接球区域
/// 因子：低位满 1，向 `zone_fade_ft` 线性衰减到 0。亲和度超过
/// 阈值选背身，否则面框。权重与阈值全部走 `DecisionRules` 通道。
pub fn choose_orientation(
    rules: &GameRules,
    carrier: &PlayerPhysicsState,
    hoop: Vec2,
) -> BallOrientation {
    let d = &rules.decision;
    let one = f32::from(1u8);
    let zero = f32::from(0u8);
    let dist = (carrier.pos_ft - hoop).length();
    let zone = if dist <= d.orientation_post_zone_ft {
        one
    } else {
        let fade = d.orientation_zone_fade_ft - d.orientation_post_zone_ft;
        ((d.orientation_zone_fade_ft - dist) / fade).clamp(zero, one)
    };
    if zone <= zero {
        // 外线接球不存在背身选项；快攻推进同理（距篮远，区域因子为 0）。
        return BallOrientation::FaceUp;
    }
    let a = &carrier.attributes;
    let affinity = (a.strength * d.orientation_strength_weight
        + a.shooting_near * d.orientation_near_weight
        + a.finishing * d.orientation_finishing_weight
        - a.ball_handling * d.orientation_handling_penalty
        - a.shooting_three * d.orientation_three_penalty)
        .clamp(zero, one)
        * zone;
    if affinity >= d.orientation_threshold {
        BallOrientation::BackToBasket
    } else {
        BallOrientation::FaceUp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nba_domain::PlayerAttributes;

    fn carrier(pos: Vec2, attributes: PlayerAttributes) -> PlayerPhysicsState {
        PlayerPhysicsState {
            id: "p1".to_string(),
            jersey: "1".to_string(),
            team: "home".to_string(),
            pos_ft: pos,
            vel_ft: Vec2::ZERO,
            accel_ft: Vec2::ZERO,
            target_pos_ft: pos,
            target_speed_ftps: 0.0,
            max_speed_ftps: 20.0,
            max_accel_ftps2: 30.0,
            has_ball: true,
            on_court: true,
            action: "Idle".to_string(),
            slot: "top".to_string(),
            morale: "Normal".to_string(),
            stamina: 100.0,
            max_stamina: 100.0,
            foul_count: 0,
            locomotion: nba_physics::movement::LocomotionState::Idle,
            facing_dir: Vec2::X,
            ball_orientation: BallOrientation::FaceUp,
            turn_decel_timer: 0.0,
            is_locked_kinematics: false,
            out_of_bounds_placement: false,
            is_receiving_pass: false,
            is_driving_to_rim: false,
            boundary_cross_latched: false,
            attributes,
            tendencies: nba_domain::PlayerTendencies::default(),
        }
    }

    /// 内线大个低位接球 → 背身。
    #[test]
    fn big_in_low_post_chooses_back_to_basket() {
        let rules = GameRules::default();
        let hoop = rules.court.hoop_pos(true);
        // 低位 6ft：强壮中锋、控球与三分贫弱。
        let big = carrier(
            hoop + Vec2::new(-6.0, 0.0),
            PlayerAttributes {
                strength: 0.92,
                shooting_near: 0.82,
                finishing: 0.88,
                ball_handling: 0.42,
                shooting_three: 0.28,
                ..PlayerAttributes::default()
            },
        );
        assert_eq!(
            choose_orientation(&rules, &big, hoop),
            BallOrientation::BackToBasket
        );
    }

    /// 控卫弧顶持球 → 面框（区域因子为 0 直接短路）。
    #[test]
    fn guard_on_perimeter_chooses_face_up() {
        let rules = GameRules::default();
        let hoop = rules.court.hoop_pos(true);
        let guard = carrier(
            hoop + Vec2::new(-24.0, 0.0),
            PlayerAttributes {
                strength: 0.35,
                shooting_near: 0.40,
                finishing: 0.55,
                ball_handling: 0.92,
                shooting_three: 0.88,
                ..PlayerAttributes::default()
            },
        );
        assert_eq!(
            choose_orientation(&rules, &guard, hoop),
            BallOrientation::FaceUp
        );
    }

    /// 同一个大个，区域拉远后亲和度衰减 → 面框。
    #[test]
    fn same_big_faded_zone_chooses_face_up() {
        let rules = GameRules::default();
        let hoop = rules.court.hoop_pos(true);
        let big = carrier(
            hoop + Vec2::new(-17.0, 0.0),
            PlayerAttributes {
                strength: 0.92,
                shooting_near: 0.82,
                finishing: 0.88,
                ball_handling: 0.42,
                shooting_three: 0.28,
                ..PlayerAttributes::default()
            },
        );
        // 17ft 处区域因子 = (18-17)/(18-12) ≈ 0.167，亲和度被压到阈值下。
        assert_eq!(
            choose_orientation(&rules, &big, hoop),
            BallOrientation::FaceUp
        );
    }
}
