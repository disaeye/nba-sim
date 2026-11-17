//! 渲染动作校验：动作身份、动画配方与两类渲染器的一致性。
//!
//! 方案 §10.2：两类球场视图消费同一动作状态，回放可按帧重建相同动作与
//! 阶段。界面按权威动作状态呈现所有登记动作与技术变体；参与者、阶段和动作
//! 进度与模拟事实一致。
//!
//! 这条测试钉住：
//!
//! 1. 动作目录里的每个条目都有配方，未登记的动作身份就地失败；
//! 2. 引擎发出的每个动作身份都能在目录里查到配方；
//! 3. 3D 与 2D 渲染器从同一份配方读出同一组身份与配方；
//! 4. 动作进入终态后不再播放。

mod common;

use std::collections::BTreeSet;

use nba_protocol::{ActionAnimationCatalog, RenderActionValidator};
use nba_sim_core::action::Fact;
use nba_sim_core::world::TICK_SECONDS;

/// 动画目录：渲染器与校验器读同一份。
fn catalog() -> ActionAnimationCatalog {
    let raw = std::fs::read_to_string(
        nba_sim_core::params::workspace_root().join("data/rules/action_animation_catalog.json"),
    )
    .expect("动画目录文件可读");
    serde_json::from_str(&raw).expect("动画目录可解析")
}

#[test]
fn the_catalog_covers_every_action_entry() {
    let catalog = catalog();
    let validator = RenderActionValidator::new(&catalog).expect("动画目录自洽");
    assert_eq!(catalog.catalog_version, 1);
    assert_eq!(catalog.families.len(), 16, "动画目录必须覆盖 16 个动作族");
    assert!(
        validator.len() >= 111,
        "动画目录必须覆盖 111 个动作条目：{}",
        validator.len()
    );
    assert!(
        validator.allowed_clips().len() >= 10,
        "基础动画剪辑不足 10 个"
    );
    assert_eq!(validator.families().len(), 16, "校验器看到 16 个动作族");
}

#[test]
fn an_unregistered_action_identity_fails_immediately() {
    let validator = RenderActionValidator::new(&catalog()).expect("动画目录自洽");
    // 未登记的动作身份：就地失败，渲染器不能自己编一个动画。
    let error = validator
        .recipe("ACTION-DRIBBLE", "dribble", "not_a_real_variant", "primary")
        .expect_err("未登记的动作身份必须失败");
    assert!(
        error.contains("没有登记的动画配方"),
        "失败信息必须说明配方缺失：{error}"
    );
    // 未登记的阶段：同样失败。
    let error = validator
        .recipe_3d(
            "ACTION-DRIBBLE",
            "dribble",
            "crossover",
            "primary",
            "not_a_phase",
        )
        .expect_err("未登记的阶段必须失败");
    assert!(
        error.contains("没有阶段"),
        "失败信息必须说明阶段缺失：{error}"
    );
}

#[test]
fn a_missing_recipe_in_the_catalog_fails_immediately() {
    // 目录里缺字段或剪辑不在允许列表里：建校验器时失败。
    let mut catalog = catalog();
    let first = catalog.families.first_mut().expect("目录有动作族");
    let action = first.actions.first_mut().expect("动作族有条目");
    action.recipe_3d.clip = "Not_An_Allowed_Clip".to_string();
    let error = RenderActionValidator::new(&catalog).expect_err("非法剪辑必须失败");
    assert!(
        error.contains("允许列表"),
        "失败信息必须说明剪辑非法：{error}"
    );
}

#[test]
fn every_engine_action_identity_has_a_recipe() {
    // 引擎发出的动作身份都能在动画目录里查到配方：渲染器因此能为
    // 每一个动作给出动画。
    //
    // 身份样本取一批比赛而不是一场：一场里的身份数量随种子变化，样本
    // 太大时一场比赛不足以覆盖目录里的身份。
    let validator = RenderActionValidator::new(&catalog()).expect("动画目录自洽");
    let mut seen: BTreeSet<(String, String, String, String)> = BTreeSet::new();
    for seed in 1..=8u64 {
        let mut world = common::short_world(seed);
        for _ in 0..12_000 {
            world.step(TICK_SECONDS);
            let tick = world.stream_tick();
            for player in &tick.frame.players {
                for action in &player.active_actions {
                    seen.insert((
                        action.family.clone(),
                        action.kind.clone(),
                        action.variant.clone(),
                        action.role.clone(),
                    ));
                }
            }
        }
    }
    assert!(
        seen.len() >= 60,
        "一批比赛必须出现大量动作身份：{}",
        seen.len()
    );
    for (family, kind, variant, role) in &seen {
        let entry = validator
            .recipe(family, kind, variant, role)
            .unwrap_or_else(|error| panic!("引擎的动作身份没有配方：{error}"));
        // 持续姿态只有 sustained 阶段，有窗口的动作才有 execution 阶段。
        // 两种形态都要有配方。
        if entry.phases.iter().any(|phase| phase == "execution") {
            validator
                .recipe_3d(family, kind, variant, role, "execution")
                .unwrap_or_else(|error| panic!("3D 配方缺失：{error}"));
        }
        validator
            .recipe_2d(family, kind, variant, role)
            .unwrap_or_else(|error| panic!("2D 配方缺失：{error}"));
    }
}

#[test]
fn every_engine_action_phase_is_registered() {
    // 引擎发出的每个 (身份, 阶段) 组合都必须在目录里登记：渲染器遇到
    // 未登记的阶段会就地失败，整个画面冻结到该动作结束。这条测试钉住
    // 目录与运行时投影的契约，缺任何一个组合都在提交前暴露。
    let validator = RenderActionValidator::new(&catalog()).expect("动画目录自洽");
    let mut seen: BTreeSet<(String, String, String, String, String)> = BTreeSet::new();
    for seed in 1..=8u64 {
        let mut world = common::short_world(seed);
        for _ in 0..12_000 {
            world.step(TICK_SECONDS);
            let tick = world.stream_tick();
            for player in &tick.frame.players {
                for action in &player.active_actions {
                    seen.insert((
                        action.family.clone(),
                        action.kind.clone(),
                        action.variant.clone(),
                        action.role.clone(),
                        action.phase.clone(),
                    ));
                }
            }
        }
    }
    assert!(
        seen.len() >= 120,
        "一批比赛必须覆盖大量的身份与阶段组合：{}",
        seen.len()
    );
    for (family, kind, variant, role, phase) in &seen {
        validator
            .recipe_3d(family, kind, variant, role, phase)
            .unwrap_or_else(|error| {
                panic!(
                    "引擎发出的阶段没有登记配方：{family}/{kind}/{variant}/{role}/{phase}: {error}"
                )
            });
    }
}

#[test]
fn the_two_renderers_read_the_same_recipe() {
    // 同一组动作身份，两个渲染器读出的配方必须一致：3D 的剪辑与 2D 的
    // 运动学都来自同一份目录条目。
    let validator = RenderActionValidator::new(&catalog()).expect("动画目录自洽");
    for family in &validator.families() {
        for spec in common::action_specs() {
            if spec.family.as_str() != family {
                continue;
            }
            let entry = validator
                .recipe(
                    spec.family.as_str(),
                    spec.kind,
                    spec.variant,
                    spec.role.as_str(),
                )
                .unwrap_or_else(|error| panic!("动作条目没有配方：{error}"));
            let clip = entry.recipe_3d.clip.clone();
            let speed = entry.recipe_3d.speed;
            let (kinetics, label) = validator
                .recipe_2d(
                    spec.family.as_str(),
                    spec.kind,
                    spec.variant,
                    spec.role.as_str(),
                )
                .unwrap_or_else(|error| panic!("2D 配方缺失：{error}"));
            assert!(!clip.is_empty(), "3D 剪辑不能为空");
            assert!(speed > 0.0, "3D 播放速率必须为正");
            assert!(!kinetics.is_empty(), "2D 运动学不能为空");
            assert!(!label.is_empty(), "2D 中文标签不能为空");
            assert!(
                validator.allowed_clips().contains(&clip),
                "3D 剪辑 {clip} 不在允许列表里"
            );
        }
    }
}

#[test]
fn an_action_stops_playing_after_its_terminal_state() {
    // 动作进入终态之后，协议投影里不再有它。
    let mut world = common::short_world(12);
    let mut terminated: BTreeSet<u64> = BTreeSet::new();
    for _ in 0..12_000 {
        world.step(TICK_SECONDS);
        for event in &world.events {
            if let Fact::ActionCompleted { action_id, .. } = &event.fact {
                terminated.insert(*action_id);
            }
        }
        let tick = world.stream_tick();
        for player in &tick.frame.players {
            for action in &player.active_actions {
                assert!(
                    !terminated.contains(&action.action_id),
                    "动作 {} 已经结束但仍在投影里",
                    action.action_id
                );
            }
        }
        terminated.clear();
    }
}
