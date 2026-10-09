//! 校验专用扰动入口的验收（validation.md §7.1：只改目标输入，
//! 派生量经生产路径重算）。

use nba_engine::{MatchEngine, ValidationAttribute};

#[test]
fn perturbation_updates_derived_physics_limits() {
    let mut engine = MatchEngine::new(42);
    // 走几个 tick 让世界进入正常比赛状态。
    for _ in 0..100 {
        engine.step();
    }
    let home_player = "H_01";
    let before = engine
        .engine_snapshot()
        .physics
        .get_player(home_player)
        .expect("player registered")
        .max_speed_ftps;
    let old = engine.apply_validation_perturbation(home_player, ValidationAttribute::Speed, 0.30);
    let after = engine
        .engine_snapshot()
        .physics
        .get_player(home_player)
        .expect("player registered")
        .max_speed_ftps;
    assert!(
        (after - before).abs() > f32::EPSILON,
        "派生量 max_speed 未随属性扰动重算: {before} -> {after}"
    );
    assert!(old > 0.0 && old <= 1.0, "返回的旧值应在合法域内: {old}");
}

#[test]
fn perturbation_rejects_out_of_domain_value() {
    let should_panic = std::panic::catch_unwind(|| {
        let mut engine = MatchEngine::new(42);
        engine.apply_validation_perturbation("H_01", ValidationAttribute::Speed, 1.5);
    });
    assert!(
        should_panic.is_err(),
        "越界属性值必须被生产校验路径拒绝（快崩）"
    );
}

#[test]
fn perturbation_targets_only_named_player() {
    // 扰动 H_01 的三分属性后，其他球员的物理派生量不变。
    let mut engine = MatchEngine::new(42);
    for _ in 0..100 {
        engine.step();
    }
    let other_before = engine
        .engine_snapshot()
        .physics
        .get_player("A_01")
        .expect("player registered")
        .max_speed_ftps;
    engine.apply_validation_perturbation("H_01", ValidationAttribute::ShootingThree, 0.90);
    let other_after = engine
        .engine_snapshot()
        .physics
        .get_player("A_01")
        .expect("player registered")
        .max_speed_ftps;
    assert_eq!(
        other_before, other_after,
        "未点名球员的派生量不得被扰动波及"
    );
}

#[test]
fn perturbation_leaves_untouched_state_intact() {
    // 未触及状态一致性：扰动 H_01 的三分属性后，引擎的全局状态
    // （比分/犯规/回合/时钟/球态）与未扰动 fork 完全一致——扰动只改
    // 目标属性与其派生量，不得改写比赛真相的其他部分。
    let mut base = MatchEngine::new(42);
    for _ in 0..300 {
        base.step();
    }
    let untouched = base.experimental_fork();
    let mut perturbed = base.experimental_fork();
    perturbed.apply_validation_perturbation("H_01", ValidationAttribute::ShootingThree, 0.77);
    // 立即对比（扰动尚未传播进比赛演化）：
    let before_untouched = untouched.snapshot();
    let before_perturbed = perturbed.snapshot();
    assert_eq!(
        (
            before_untouched.frame.score.home,
            before_untouched.frame.score.away
        ),
        (
            before_perturbed.frame.score.home,
            before_perturbed.frame.score.away
        ),
        "扰动不得改写比分"
    );
    assert_eq!(
        before_untouched.frame.possession_id, before_perturbed.frame.possession_id,
        "扰动不得改写球权"
    );
    assert_eq!(
        before_untouched.frame.team_fouls_home, before_perturbed.frame.team_fouls_home,
        "扰动不得改写球队犯规"
    );
    assert_eq!(
        before_untouched.frame.t_game, before_perturbed.frame.t_game,
        "扰动不得改写比赛时钟"
    );
    assert_eq!(
        before_untouched.frame.ball.status, before_perturbed.frame.ball.status,
        "扰动不得改写球态"
    );
    // 扰动只改变物理派生量（max_speed/max_accel 不变——shooting 不驱动运动上限）：
    let physics_untouched = before_untouched
        .frame
        .players
        .iter()
        .find(|p| p.id == "H_01")
        .expect("player present");
    let physics_perturbed = before_perturbed
        .frame
        .players
        .iter()
        .find(|p| p.id == "H_01")
        .expect("player present");
    assert_eq!(
        physics_untouched.x, physics_perturbed.x,
        "扰动不得改写球员位置"
    );
}

#[test]
fn perturbation_propagates_into_simulation_outcome() {
    // 行为接线验证：扰动必须传播进比赛结果（两侧事件流分岔）。
    // 档位间的方向性（高精度 → 更高命中）是统计判定，属 MECH 框架
    // （多检查点 × 多 trial 聚合）的职责，单次比赛的小样本无法承载。
    let mut base = MatchEngine::new(7);
    for _ in 0..600 {
        base.step();
    }
    let mut low = base.experimental_fork();
    let mut high = base.experimental_fork();
    for id in [
        "H_01", "H_02", "H_03", "H_04", "H_05", "A_01", "A_02", "A_03", "A_04", "A_05",
    ] {
        low.apply_validation_perturbation(id, ValidationAttribute::ShootingThree, 0.02);
        high.apply_validation_perturbation(id, ValidationAttribute::ShootingThree, 0.98);
    }
    let mut low_shot_events = 0u32;
    let mut high_shot_events = 0u32;
    let mut diverged_at: Option<u32> = None;
    for i in 0..20_000u32 {
        let low_tick = low.step();
        let high_tick = high.step();
        for event in &low_tick.frame.event_log {
            if event.kind == "SHOT_ATTEMPT_RESOLVED" {
                low_shot_events += 1;
            }
        }
        for event in &high_tick.frame.event_log {
            if event.kind == "SHOT_ATTEMPT_RESOLVED" {
                high_shot_events += 1;
            }
        }
        // 命中结果通道分岔 = 扰动影响了随机抽样输入（make 路径消费了属性）。
        let low_makes: Vec<_> = low_tick
            .frame
            .event_log
            .iter()
            .filter(|e| e.kind == "SHOT_ATTEMPT_RESOLVED")
            .filter_map(|e| {
                e.data
                    .as_ref()
                    .and_then(|d| d.get("ShotAttemptResolved"))
                    .and_then(|p| p.get("scoring_decision"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        let high_makes: Vec<_> = high_tick
            .frame
            .event_log
            .iter()
            .filter(|e| e.kind == "SHOT_ATTEMPT_RESOLVED")
            .filter_map(|e| {
                e.data
                    .as_ref()
                    .and_then(|d| d.get("ShotAttemptResolved"))
                    .and_then(|p| p.get("scoring_decision"))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
            })
            .collect();
        if diverged_at.is_none() && low_makes != high_makes {
            diverged_at = Some(i);
        }
    }
    assert!(
        low_shot_events >= 5 && high_shot_events >= 5,
        "出手事件样本不足: low={low_shot_events} high={high_shot_events}"
    );
    assert!(
        diverged_at.is_some(),
        "扰动未传播进比赛结果（两侧结算序列完全一致——属性未被 make 路径消费）"
    );
}
