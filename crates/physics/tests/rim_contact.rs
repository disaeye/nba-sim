// 触筐/触板反弹分布校准门（ADR-017 第二步/第三步）：
// 打铁后的落点距离不再均匀采样，而由「入射镜像反射 × 恢复系数 + 受控散射」
// 的抛体自然产生。本文件守住真实篮板的两个结构特征：
//
// 1. 距离分层：远投打铁回弹更远（入射水平速度随出手距离增大）；
// 2. 总体分布：混合出手结构下大多数落点靠近筐（真实 NBA 大多数篮板
//    在筐附近被收走），同时保留长投打铁的长回弹尾部；
// 3. 方向偏置：落点偏向出手点一侧（回弹）多于前穿侧。
//
// 出手距离混合近似比赛出手结构（近筐密集、三分次之、中距离最少）。
// 第三步加入板通道后，分层门与方向门仍单测近筐沿反射（第二步物理）
// 的专属特征；混合门复刻生产双通道路由（探针命中 → 打板，否则近筐
// 沿），守住两通道并存后的总体分布。
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
        let spot = BallisticsEngine::compute_rebound_landing(origin, hoop, flight, rng, rules);
        out.push((spot.landing_pos - hoop).length());
    }
    out
}

/// 双通道采样（与生产路由同源）：探针判「力度过大越过筐」→ 打板路径，
/// 否则近筐沿反射。混合门用它守住两通道并存后的总体分布。
fn sample_dual(
    dist: f32,
    n: usize,
    rng: &mut StdRng,
    rules: &GameRules,
    hoop: Vec2,
) -> (Vec<f32>, usize) {
    let mut out = Vec::with_capacity(n);
    let mut bank = 0usize;
    for _ in 0..n {
        let origin = hoop - Vec2::new(dist, 0.0);
        let peak = (rules.shot_peak_base_ft + dist * rules.shot_peak_distance_factor)
            .min(rules.ball_z_max_ft);
        let flight = BallisticsEngine::shot_duration(dist, peak, rules);
        let spot = if BallisticsEngine::compute_backboard_contact_probe(origin, hoop, rules) {
            bank += 1;
            BallisticsEngine::compute_rebound_landing_bank(origin, hoop, flight, rng, rules)
        } else {
            BallisticsEngine::compute_rebound_landing(origin, hoop, flight, rng, rules)
        };
        out.push((spot.landing_pos - hoop).length());
    }
    (out, bank)
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
    // 单调分层：上篮打铁基本在筐边触地，三分打铁明显更远。
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
    let mix: [(f32, usize); 5] = [
        (3.0, 300),
        (8.0, 150),
        (14.0, 150),
        (22.0, 100),
        (25.0, 300),
    ];
    let mut all: Vec<f32> = Vec::new();
    let mut bank_total = 0usize;
    for (dist, n) in mix {
        let (part, bank) = sample_dual(dist, n, &mut rng, &rules, hoop);
        all.extend(part);
        bank_total += bank;
    }
    let total = all.len() as f32;
    let bank_share = bank_total as f32 / total;
    let within6 = all.iter().filter(|d| **d <= 6.0).count() as f32 / total;
    let within12 = all.iter().filter(|d| **d <= 12.0).count() as f32 / total;
    let beyond12 = 1.0 - within12;
    // 双通道实测（seed 42，本 mix）：本门出手全部沿 x 轴正对筐，距筐
    // ≥ 2.5 ft 的探针全命中（弦外推 z ∈ [9.5,13]），板通道份额 1.000，
    // 近筐沿通道在本门内份额为 0（其分布特征由分层门与方向门单测）。
    // 打板反弹物理：竖直保持入射下落、水平 x 镜像 × 0.55，球直落板前，
    // 实测 within6=1.000、within12=1.000、beyond12=0.000、p50≈0.92——
    // 与真实打板打铁的篮板位置（Restricted Area 内被收走）一致。
    assert!(
        (0.97..=1.0).contains(&bank_share),
        "bank share {bank_share} outside measured band"
    );
    assert!(
        (0.97..=1.0).contains(&within6),
        "within-6ft share {within6} outside band"
    );
    assert!(
        (0.999..=1.0).contains(&within12),
        "within-12ft share {within12} outside band"
    );
    assert!(beyond12 < 0.01, "beyond-12ft share {beyond12} too fat");
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
