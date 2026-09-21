// 触筐反弹分布校准门（ADR-017 第二步）：
// 打铁后的落点距离不再均匀采样，而由「入射镜像反射 × 恢复系数 + 受控散射」
// 的抛体自然产生。本文件守住真实篮板的两个结构特征：
//
// 1. 距离分层：远投打铁回弹更远（入射水平速度随出手距离增大）；
// 2. 总体分布：混合出手结构下大多数落点靠近筐（真实 NBA 大多数篮板
//    在筐附近被收走），同时保留长投打铁的长回弹尾部；
// 3. 方向偏置：落点偏向出手点一侧（回弹）多于前穿侧。
//
// 出手距离混合近似比赛出手结构（近筐密集、三分次之、中距离最少）。
use glam::Vec2;
use nba_domain::GameRules;
use nba_physics::BallisticsEngine;
use rand::rngs::StdRng;
use rand::SeedableRng;

fn sample(dist: f32, n: usize, rng: &mut StdRng, rules: &GameRules, hoop: Vec2) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let origin = hoop - Vec2::new(dist, 0.0);
        let peak = (rules.shot_peak_base_ft + dist * rules.shot_peak_distance_factor)
            .min(rules.ball_z_max_ft);
        let flight = BallisticsEngine::shot_duration(dist, peak, rules);
        let spot =
            BallisticsEngine::compute_rebound_landing(origin, hoop, flight, rng, rules);
        out.push((spot.landing_pos - hoop).length());
    }
    out
}

fn median(v: &mut [f32]) -> f32 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

#[test]
fn rim_rebound_distance_stratifies_with_shot_distance() {
    let rules = GameRules::default();
    let hoop = Vec2::new(88.75, 25.0);
    let mut rng = StdRng::seed_from_u64(42);
    let close = median(&mut sample(3.0, 200, &mut rng, &rules, hoop));
    let mid = median(&mut sample(14.0, 200, &mut rng, &rules, hoop));
    let three = median(&mut sample(25.0, 200, &mut rng, &rules, hoop));
    // 单调分层：上篮打铁基本落在筐边，三分打铁明显更远。
    assert!(close < mid, "close {close} must be shorter than mid {mid}");
    assert!(mid < three, "mid {mid} must be shorter than three {three}");
    // 量级带（实测 p50：上篮 ≈1.3、中距 ≈5.9、三分 ≈9.4）。
    assert!(close < 3.0, "close median {close} unexpectedly far");
    assert!(
        (2.0..=14.0).contains(&three),
        "three median {three} outside physical band"
    );
}

#[test]
fn rim_rebound_mixed_distribution_matches_rebound_structure() {
    let rules = GameRules::default();
    let hoop = Vec2::new(88.75, 25.0);
    let mut rng = StdRng::seed_from_u64(42);
    let mix: [(f32, usize); 5] = [(3.0, 300), (8.0, 150), (14.0, 150), (22.0, 100), (25.0, 300)];
    let mut all: Vec<f32> = Vec::new();
    for (dist, n) in mix {
        all.extend(sample(dist, n, &mut rng, &rules, hoop));
    }
    let total = all.len() as f32;
    let within6 = all.iter().filter(|d| **d <= 6.0).count() as f32 / total;
    let within12 = all.iter().filter(|d| **d <= 12.0).count() as f32 / total;
    let beyond12 = 1.0 - within12;
    // 实测：within6 ≈ 0.63、within12 ≈ 0.999、beyond12 ≈ 0.001。
    // 真实篮板大多数在筐附近（0-6 ft）收走。单次触筐反弹的物理上限：
    // 有界恢复系数（≤0.7）下三分打铁首触地约 11-12 ft；15+ ft 的真实
    // 长篮板来自篮板接触与触地后的二次弹跳（第三步碰撞几何范畴），
    // 在引擎侧经 LooseBall 延续，不在此门的单一抛体内。
    assert!(
        (0.50..=0.72).contains(&within6),
        "within-6ft share {within6} outside band"
    );
    assert!(
        (0.97..=1.0).contains(&within12),
        "within-12ft share {within12} outside band"
    );
    assert!(beyond12 < 0.03, "beyond-12ft share {beyond12} too fat");
}

#[test]
fn rim_rebound_direction_biases_back_toward_shooter() {
    let rules = GameRules::default();
    let hoop = Vec2::new(88.75, 25.0);
    let mut rng = StdRng::seed_from_u64(42);
    let mut back = 0usize;
    let n = 600;
    for _ in 0..n {
        let origin = hoop - Vec2::new(25.0, 0.0);
        let peak = (rules.shot_peak_base_ft + 25.0 * rules.shot_peak_distance_factor)
            .min(rules.ball_z_max_ft);
        let flight = BallisticsEngine::shot_duration(25.0, peak, &rules);
        let spot =
            BallisticsEngine::compute_rebound_landing(origin, hoop, flight, &mut rng, &rules);
        let to_shooter = (origin - hoop).normalize();
        let to_landing = (spot.landing_pos - hoop).normalize();
        if to_shooter.dot(to_landing) > 0.0 {
            back += 1;
        }
    }
    let share = back as f32 / n as f32;
    // 接触点扇形以近筐沿为中心，镜像反射把多数球弹回出手侧（实测 ≈ 0.85）。
    assert!(share > 0.6, "back-share {share} lost the reflection bias");
}
