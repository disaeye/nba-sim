//! Play 动词映射与谓词装配测试（plan_play.md #14 / tactics.md §2.2.4）。
//!
//! 覆盖：六个动词各自的方向性（偏置方向正确）；谓词装配的几何派生
//! （`beyond_circle_ft` / `screen_established` / `corner_occupied` 真假
//! 两侧）；场输出稳定值透传；偏移量级上限；收界语义。

use glam::Vec2;
use nba_decision::play_actions::{
    build_selection_context, resolve_verb, resolve_verb_target, EngineWorldInputs, VerbContext,
};
use nba_decision::play_selector::eval_predicate;
use nba_decision::potential_field::StableFieldOutput;
use nba_domain::action_window::ActionType;
use nba_domain::{CourtGeometry, GameRules, PlayPredicate, PlayVerb};

/// 攻右篮的篮筐位置（CourtGeometry 默认值）。
const HOOP: Vec2 = Vec2::new(88.75, 25.0);

/// 标准动词语境：掩护点在弧顶内侧，持球人在翼位。
fn verb_ctx() -> VerbContext {
    VerbContext {
        actor_pos: Vec2::new(70.0, 25.0),
        hoop_pos: HOOP,
        carrier_pos: Vec2::new(66.0, 34.0),
        defender_pos: Some(Vec2::new(68.0, 24.0)),
        court: CourtGeometry::default(),
        clamp_margin_ft: 1.8,
        three_point_distance_ft: 23.75,
    }
}

/// 全假场输出稳定值（未拉离、未真空，与 `DefenseHysteresisState`
/// 初始态一致）。
fn field_off() -> StableFieldOutput {
    StableFieldOutput {
        help_pulled_off: false,
        weak_side_vacant: false,
    }
}

/// 进攻方世界快照：可自由指定各球员位置与速度。
fn world_inputs(
    off_positions: Vec<Vec2>,
    off_velocities: Vec<Vec2>,
    carrier_idx: usize,
    stable_field: StableFieldOutput,
) -> EngineWorldInputs {
    EngineWorldInputs {
        carrier_idx,
        off_positions,
        off_velocities,
        hoop_pos: HOOP,
        court: CourtGeometry::default(),
        shot_clock_seconds: 14.0,
        carrier_possed: true,
        halfcourt: true,
        possession_ticks: 10,
        stable_field,
    }
}

// ---- 功能 1：六个动词的方向性 ----

#[test]
fn screen_roll_offset_points_toward_hoop() {
    let ctx = verb_ctx();
    let r = resolve_verb(PlayVerb::ScreenRoll, &ctx);
    let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize();
    assert!(r.target_offset.dot(to_hoop) > 0.0, "顺下偏置必须朝篮筐");
    assert_eq!(r.window_action, None, "顺下不预置出手窗口");
}

#[test]
fn screen_pop_offset_extends_radially_to_three_point_line() {
    let ctx = verb_ctx();
    let r = resolve_verb(PlayVerb::ScreenPop, &ctx);
    // 外弹沿「篮筐 → 掩护点」射线向外：与径向同向，且终点恰好落在三分线。
    let radial = (ctx.actor_pos - ctx.hoop_pos).normalize();
    assert!(r.target_offset.dot(radial) > 0.0, "外弹偏置必须远离篮筐");
    let target = ctx.actor_pos + r.target_offset;
    let dist_to_hoop = (target - ctx.hoop_pos).length();
    assert!(
        (dist_to_hoop - ctx.three_point_distance_ft).abs() < 1e-3,
        "外弹终点应落在三分线上，实测 {dist_to_hoop}"
    );
    assert_eq!(r.window_action, Some(ActionType::JumpShot));
}

#[test]
fn screen_pop_at_arc_already_has_zero_offset() {
    // 已站在三分线上时无需外弹：偏置为零，窗口语义不变。
    let mut ctx = verb_ctx();
    ctx.actor_pos = ctx.hoop_pos + Vec2::new(-ctx.three_point_distance_ft, 0.0);
    let r = resolve_verb(PlayVerb::ScreenPop, &ctx);
    assert!(r.target_offset.length() < 1e-4);
    assert_eq!(r.window_action, Some(ActionType::JumpShot));
}

#[test]
fn spot_up_offset_is_near_zero_nudge() {
    let ctx = verb_ctx();
    let r = resolve_verb(PlayVerb::SpotUp, &ctx);
    // 就地微调：偏置量级远小于场上移动尺度，方向朝篮筐。
    assert!(r.target_offset.length() < 5.0, "定点落位只做近零微调");
    let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize();
    assert!(r.target_offset.dot(to_hoop) > 0.0);
    assert_eq!(r.window_action, Some(ActionType::JumpShot));
}

#[test]
fn relocate_offset_points_to_weak_side() {
    let ctx = verb_ctx();
    let r = resolve_verb(PlayVerb::Relocate, &ctx);
    // 持球人在 y=34（高于中轴 25），弱侧在 y 较低一侧：偏置的 y 分量应为负。
    assert!(
        r.target_offset.y < 0.0,
        "持球人在上半场时弱侧转移应向 y 较低一侧"
    );
    assert_eq!(r.window_action, Some(ActionType::JumpShot));
}

#[test]
fn cut_backdoor_veers_to_defender_far_side() {
    let ctx = verb_ctx();
    let r = resolve_verb(PlayVerb::CutBackdoor, &ctx);
    // 背切朝篮筐方向前进：与「到篮筐方向」点积为正。
    let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize();
    assert!(r.target_offset.dot(to_hoop) > 0.0, "背切必须朝篮筐");
    // 防守人在径向线下方（y=24 < 25），远侧为 y 分量更高的一侧：偏置的
    // y 分量应高于纯朝筐直线的 y 分量。
    let pure = to_hoop * r.target_offset.length();
    assert!(
        r.target_offset.y > pure.y,
        "背切应偏向防守人（y=24，在切线下方）的远侧"
    );
    assert_eq!(r.window_action, None);
}

#[test]
fn cut_backdoor_without_defender_goes_straight_to_hoop() {
    let mut ctx = verb_ctx();
    ctx.defender_pos = None;
    let r = resolve_verb(PlayVerb::CutBackdoor, &ctx);
    let to_hoop = (ctx.hoop_pos - ctx.actor_pos).normalize();
    assert!(r.target_offset.dot(to_hoop) > 0.0);
}

#[test]
fn lift_offset_moves_toward_top_of_the_arc() {
    let mut ctx = verb_ctx();
    // 低位：篮筐正下方偏中圈侧。
    ctx.actor_pos = Vec2::new(84.0, 25.0);
    let r = resolve_verb(PlayVerb::Lift, &ctx);
    // 弧顶在篮筐沿中圈方向（-x）：上提偏置的 x 分量应为负。
    assert!(r.target_offset.x < 0.0, "低位上提应朝弧顶（中圈方向）");
    // 步长封顶：低位到弧顶全程约 23.75 ft，偏置至多 16 ft。
    assert!(r.target_offset.length() <= 16.0 + 1e-3);
    assert_eq!(r.window_action, None);
}

// ---- 偏移量级 ----

#[test]
fn all_verb_offsets_stay_within_field_scale() {
    let ctx = verb_ctx();
    for verb in [
        PlayVerb::ScreenRoll,
        PlayVerb::ScreenPop,
        PlayVerb::SpotUp,
        PlayVerb::Relocate,
        PlayVerb::CutBackdoor,
        PlayVerb::Lift,
    ] {
        let r = resolve_verb(verb, &ctx);
        assert!(
            r.target_offset.length() <= 30.0,
            "动词 `{}` 的偏置量级 {} ft 超出场上尺度",
            verb.as_str(),
            r.target_offset.length()
        );
    }
}

// ---- 功能 2：谓词装配 ----

#[test]
fn beyond_circle_ft_true_and_false_sides() {
    let rules = GameRules::default();
    // 持球人距篮 22.75 ft。
    let inputs = world_inputs(
        vec![Vec2::new(66.0, 25.0)],
        vec![Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&inputs, &rules);
    assert!(
        (ctx.carrier_dist_to_hoop_ft - 22.75).abs() < 1e-3,
        "距篮距离应从位置快照派生，实测 {}",
        ctx.carrier_dist_to_hoop_ft
    );
    assert!(eval_predicate(
        &PlayPredicate::BeyondCircleFt { r: 20.0 },
        &ctx
    ));
    assert!(!eval_predicate(
        &PlayPredicate::BeyondCircleFt { r: 25.0 },
        &ctx
    ));
}

#[test]
fn screen_established_requires_distance_and_stationary() {
    let rules = GameRules::default();
    // 站定阈值：22.0 × 0.20 = 4.4 ft/s。
    let stationary_limit =
        rules.max_player_speed_ftps * rules.semantics.screen_stationary_speed_ratio;
    assert!(stationary_limit < rules.max_player_speed_ftps);

    // 真侧：队友在掩护判定半径内且站定（速度为零）。
    let near_stationary = world_inputs(
        vec![Vec2::new(66.0, 25.0), Vec2::new(72.0, 28.0)],
        vec![Vec2::ZERO, Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&near_stationary, &rules);
    assert!((Vec2::new(72.0, 28.0) - Vec2::new(66.0, 25.0)).length() < 16.0);
    assert!(ctx.screen_established);

    // 假侧一：同位置队友在移动（速度超站定阈值）。
    let near_moving = world_inputs(
        vec![Vec2::new(66.0, 25.0), Vec2::new(72.0, 28.0)],
        vec![Vec2::ZERO, Vec2::new(rules.max_player_speed_ftps, 0.0)],
        0,
        field_off(),
    );
    assert!(!build_selection_context(&near_moving, &rules).screen_established);

    // 假侧二：站定但距离超出掩护判定半径。
    let far_stationary = world_inputs(
        vec![Vec2::new(66.0, 25.0), Vec2::new(90.0, 40.0)],
        vec![Vec2::ZERO, Vec2::ZERO],
        0,
        field_off(),
    );
    assert!(!build_selection_context(&far_stationary, &rules).screen_established);
}

#[test]
fn corner_occupied_distinguishes_sides_and_halves() {
    let rules = GameRules::default();
    let depth = CourtGeometry::default().corner_zone_depth_ft();

    // 真侧：左底角（y 低）与右底角（y 高）各有一名球员，均在前场。
    let both = world_inputs(
        vec![
            Vec2::new(86.0, depth / 2.0),
            Vec2::new(86.0, 50.0 - depth / 2.0),
        ],
        vec![Vec2::ZERO, Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&both, &rules);
    assert!(ctx.corner_left_occupied);
    assert!(ctx.corner_right_occupied);

    // 假侧一：同样 y 坐标但在后场（底角线只存在于进攻半场）。
    let backcourt = world_inputs(
        vec![
            Vec2::new(6.0, depth / 2.0),
            Vec2::new(6.0, 50.0 - depth / 2.0),
        ],
        vec![Vec2::ZERO, Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&backcourt, &rules);
    assert!(!ctx.corner_left_occupied);
    assert!(!ctx.corner_right_occupied);

    // 假侧二：前场但距边线超过底角带深度（翼位）。
    let wings = world_inputs(
        vec![Vec2::new(80.0, 10.0), Vec2::new(80.0, 40.0)],
        vec![Vec2::ZERO, Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&wings, &rules);
    assert!(!ctx.corner_left_occupied);
    assert!(!ctx.corner_right_occupied);

    // 单侧：只有右底角有人。
    let right_only = world_inputs(
        vec![Vec2::new(86.0, 50.0 - depth / 2.0)],
        vec![Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&right_only, &rules);
    assert!(ctx.corner_right_occupied);
    assert!(!ctx.corner_left_occupied);
}

#[test]
fn corner_predicate_maps_side_enum() {
    let rules = GameRules::default();
    let depth = CourtGeometry::default().corner_zone_depth_ft();
    let inputs = world_inputs(
        vec![Vec2::new(86.0, depth / 2.0)],
        vec![Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&inputs, &rules);
    assert!(eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Left
        },
        &ctx
    ));
    assert!(!eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Right
        },
        &ctx
    ));
}

// ---- 场输出透传 ----

#[test]
fn stable_field_output_flows_into_context() {
    let rules = GameRules::default();
    let inputs = world_inputs(
        vec![Vec2::new(66.0, 25.0)],
        vec![Vec2::ZERO],
        0,
        StableFieldOutput {
            help_pulled_off: true,
            weak_side_vacant: true,
        },
    );
    let ctx = build_selection_context(&inputs, &rules);
    assert!(ctx.help_shading_off_stable);
    assert!(ctx.weak_side_vacated_stable);
    assert!(eval_predicate(&PlayPredicate::HelpShadingOff, &ctx));
    assert!(eval_predicate(&PlayPredicate::WeakSideVacated, &ctx));

    let off = world_inputs(
        vec![Vec2::new(66.0, 25.0)],
        vec![Vec2::ZERO],
        0,
        field_off(),
    );
    let ctx = build_selection_context(&off, &rules);
    assert!(!ctx.help_shading_off_stable);
    assert!(!ctx.weak_side_vacated_stable);
}

// ---- 收界 ----

#[test]
fn resolved_target_is_clamped_inside_playable_area() {
    let ctx = verb_ctx();
    for verb in [
        PlayVerb::ScreenRoll,
        PlayVerb::ScreenPop,
        PlayVerb::SpotUp,
        PlayVerb::Relocate,
        PlayVerb::CutBackdoor,
        PlayVerb::Lift,
    ] {
        // 把作用点贴到边线附近：收界后目标必须仍在可站立区域内。
        let mut near_sideline = ctx;
        near_sideline.actor_pos = Vec2::new(88.0, 2.0);
        let target = resolve_verb_target(verb, &near_sideline);
        let margin = near_sideline.clamp_margin_ft;
        assert!(
            target.x >= margin
                && target.x <= 94.0 - margin
                && target.y >= margin
                && target.y <= 50.0 - margin,
            "动词 `{}` 的目标点 {target:?} 出界",
            verb.as_str()
        );
    }
}

// ---- fast-fail：装配输入非法时就地崩溃 ----

#[test]
#[should_panic(expected = "off_positions and off_velocities must have the same length")]
fn build_context_fails_fast_on_length_mismatch() {
    let rules = GameRules::default();
    let inputs = world_inputs(vec![Vec2::new(66.0, 25.0)], vec![], 0, field_off());
    build_selection_context(&inputs, &rules);
}

#[test]
#[should_panic(expected = "carrier_idx 1 out of bounds (1)")]
fn build_context_fails_fast_on_carrier_index_out_of_bounds() {
    let rules = GameRules::default();
    let inputs = world_inputs(
        vec![Vec2::new(66.0, 25.0)],
        vec![Vec2::ZERO],
        1,
        field_off(),
    );
    build_selection_context(&inputs, &rules);
}
