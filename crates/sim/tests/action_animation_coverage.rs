//! 动作动画覆盖目录与协议投影逐项验证。
//!
//! 检验全部 16 个动作族及技术变体在目录中登记完整、
//! 3D/2D 配方参数合法、测试映射函数真实存在，
//! 且在仿真流输出中 `RenderPlayer.active_actions` 字段完整有效。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchSetup, Params, World};

mod common;

#[test]
fn action_animation_catalog_covers_all_families_and_valid_recipes() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().parent().unwrap().to_path_buf();
    let catalog_path = root.join("data/rules/action_animation_catalog.json");
    assert!(catalog_path.is_file(), "动作动画目录文件不存在");

    let content = std::fs::read_to_string(&catalog_path).expect("读取目录文件");
    let json: serde_json::Value = serde_json::from_str(&content).expect("解析目录 JSON");

    assert_eq!(json["catalog_version"].as_u64(), Some(1));
    let allowed_clips: BTreeSet<String> = json["allowed_clips"]
        .as_array()
        .expect("allowed_clips 数组")
        .iter()
        .map(|v| v.as_str().expect("clip 字符串").to_string())
        .collect();
    assert!(allowed_clips.len() >= 10, "基础动画剪辑不足 10 个");

    let families = json["families"].as_array().expect("families 数组");
    assert_eq!(families.len(), 16, "动作族数量必须严格等于 16");

    let mut total_actions = 0;
    for family in families {
        let fam_id = family["id"].as_str().expect("family id");
        let actions = family["actions"].as_array().expect("actions 数组");
        assert!(!actions.is_empty(), "动作族 {fam_id} 不能为空");

        for action in actions {
            total_actions += 1;
            let name = action["name"].as_str().expect("action name");
            let kind = action["kind"].as_str().expect("action kind");
            let variant = action["variant"].as_str().expect("action variant");
            let role = action["role"].as_str().expect("action role");
            let phases = action["phases"].as_array().expect("action phases");

            assert!(!name.is_empty());
            assert!(!kind.is_empty());
            assert!(!variant.is_empty());
            assert!(!role.is_empty());
            assert!(!phases.is_empty());

            let recipe_3d = &action["recipe_3d"];
            let clip = recipe_3d["clip"].as_str().expect("clip");
            assert!(allowed_clips.contains(clip), "剪辑 {clip} 不在允许列表中");
            let speed = recipe_3d["speed"].as_f64().expect("speed");
            assert!(speed > 0.0, "播放速率必须大于 0");

            let recipe_2d = &action["recipe_2d"];
            let kinetics = recipe_2d["kinetics"].as_str().expect("kinetics");
            let label_zh = recipe_2d["label_zh"].as_str().expect("label_zh");
            assert!(!kinetics.is_empty());
            assert!(!label_zh.is_empty());

            let test_ref = action["scenario_test"].as_str().expect("scenario_test");
            assert!(!test_ref.is_empty());
            if let Some((file, func)) = test_ref.split_once("::") {
                let test_file_path = root.join(file);
                assert!(test_file_path.is_file(), "关联测试文件不存在: {file}");
                let file_content = std::fs::read_to_string(&test_file_path).expect("读取测试文件");
                assert!(
                    file_content.contains(&format!("fn {func}")),
                    "测试函数 {func} 在 {file} 中未找到"
                );
            }
        }
    }
    assert!(total_actions >= 90, "总动作细分数需大于等于 90，实测: {total_actions}");
}

#[test]
fn active_actions_stream_projection_contains_authoritative_data() {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    let mut world = World::new(MatchSetup::builtin(GameRules::default()), params, 42);

    let mut observed_families: BTreeSet<String> = BTreeSet::new();
    let mut observed_phases: BTreeSet<String> = BTreeSet::new();
    let mut action_counts: BTreeMap<String, usize> = BTreeMap::new();

    for _ in 0..1_200 {
        let tick = world.stream_tick();
        for player in &tick.frame.players {
            for act in &player.active_actions {
                assert!(act.action_id > 0, "action_id 必须大于 0");
                assert!(!act.family.is_empty(), "family 不能为空");
                assert!(!act.kind.is_empty(), "kind 不能为空");
                assert!(!act.variant.is_empty(), "variant 不能为空");
                assert!(!act.role.is_empty(), "role 不能为空");
                assert!(!act.phase.is_empty(), "phase 不能为空");
                assert!(act.started_at >= 0.0);
                assert!(act.phase_started_at >= act.started_at);
                if let Some(ends) = act.ends_at {
                    assert!(ends >= act.phase_started_at);
                }

                observed_families.insert(act.family.clone());
                observed_phases.insert(act.phase.clone());
                *action_counts.entry(act.family.clone()).or_default() += 1;
            }
        }
        world.step(TICK_SECONDS);
    }

    assert!(
        observed_families.contains("ACTION-DEFENSE-ON-BALL"),
        "必须观测到持球防守动作状态"
    );
    assert!(
        observed_phases.contains("sustained") || observed_phases.contains("execution"),
        "必须观测到持续姿态或执行阶段"
    );
    assert!(!action_counts.is_empty(), "必须观测到活动动作");
}
