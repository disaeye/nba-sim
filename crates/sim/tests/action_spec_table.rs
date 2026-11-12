//! 动作规格表与动作目录的一致性守卫。
//!
//! 方案 §6：每种动作都需要独立消费者，不允许只添加名称。§10.2：渲染帧里的
//! 球员动作状态必须来自权威运行时，包含稳定动作身份、动作族、技术变体、
//! 参与者角色、阶段与动作时间区间。
//!
//! 这条守卫把三份清单绑在一起：动作目录（语义覆盖）、动作动画目录（渲染
//! 配方）与动作规格表（生产契约）。三份清单对不上就地失败。

mod common;

use std::collections::BTreeSet;

use nba_domain::action_spec::{ActionFamily, ActionRole};
use nba_sim_core::action_spec::action_spec_table;
use serde_json::Value;

fn workspace_root() -> std::path::PathBuf {
    nba_sim_core::params::workspace_root()
}

fn load_json(path: &str) -> Value {
    let raw = std::fs::read_to_string(workspace_root().join(path)).expect("catalog file readable");
    serde_json::from_str(&raw).expect("catalog file parses")
}

#[test]
fn the_action_catalog_covers_every_family_and_entry() {
    let catalog = load_json("data/rules/action_catalog.json");
    assert_eq!(catalog["catalog_version"], 1);
    let families = catalog["families"].as_array().expect("families array");
    assert_eq!(families.len(), 16, "动作族必须是 16 个");
    let mut total = 0usize;
    for family in families {
        let id = family["id"].as_str().expect("family id");
        assert!(
            ActionFamily::from_str(id).is_some(),
            "档案里的动作族 {id} 必须有领域定义"
        );
        let actions = family["actions"].as_array().expect("actions array");
        assert!(!actions.is_empty(), "{id} 没有动作条目");
        total += actions.len();
    }
    assert_eq!(total, 111, "动作条目总数必须是 111");
}

#[test]
fn the_spec_table_has_one_entry_per_catalog_action() {
    let catalog = load_json("data/rules/action_catalog.json");
    let table = action_spec_table();
    assert_eq!(
        table.len(),
        111,
        "动作规格表必须覆盖 111 个动作条目，实际 {}",
        table.len()
    );
    let mut names: BTreeSet<(String, String)> = BTreeSet::new();
    for family in catalog["families"].as_array().expect("families array") {
        let family_id = family["id"].as_str().expect("family id");
        for name in family["actions"].as_array().expect("actions array") {
            let name = name.as_str().expect("action name");
            assert!(
                names.insert((family_id.to_string(), name.to_string())),
                "动作目录里有重复条目：{family_id}::{name}"
            );
            let spec = table
                .by_family_name(
                    ActionFamily::from_str(family_id).expect("family has a domain type"),
                    name,
                )
                .unwrap_or_else(|| panic!("动作规格表缺少条目：{family_id}::{name}"));
            assert_eq!(
                spec.family.as_str(),
                family_id,
                "动作 {name} 的族与目录不一致"
            );
        }
    }
    for spec in table.specs() {
        assert!(
            names.contains(&(spec.family.as_str().to_string(), spec.name.to_string())),
            "动作规格表有目录里没有的条目：{}::{}",
            spec.family.as_str(),
            spec.name
        );
    }
}

#[test]
fn the_animation_catalog_and_the_spec_table_agree_on_identity() {
    let animation = load_json("data/rules/action_animation_catalog.json");
    let table = action_spec_table();
    let families = animation["families"].as_array().expect("families array");
    let mut total = 0usize;
    for family in families {
        let family_id = family["id"].as_str().expect("family id");
        let family_type = ActionFamily::from_str(family_id).expect("family has a domain type");
        for entry in family["actions"].as_array().expect("actions array") {
            total += 1;
            let name = entry["name"].as_str().expect("action name");
            let spec = table
                .by_family_name(family_type, name)
                .unwrap_or_else(|| panic!("动作规格表缺少条目：{family_id}::{name}"));
            assert_eq!(
                spec.family.as_str(),
                family_id,
                "动作 {name} 的族与动画目录不一致"
            );
            assert_eq!(
                spec.kind,
                entry["kind"].as_str().expect("kind"),
                "动作 {name} 的类别与动画目录不一致"
            );
            assert_eq!(
                spec.variant,
                entry["variant"].as_str().expect("variant"),
                "动作 {name} 的变体与动画目录不一致"
            );
            assert_eq!(
                spec.role.as_str(),
                entry["role"].as_str().expect("role"),
                "动作 {name} 的角色与动画目录不一致"
            );
            let phases = entry["phases"].as_array().expect("phases array");
            if spec.windowed {
                assert!(
                    phases.iter().any(|phase| phase == "execution"),
                    "有窗口的动作 {name} 必须有执行阶段"
                );
            } else {
                assert!(
                    phases.iter().any(|phase| phase == "sustained"),
                    "无窗口的动作 {name} 必须是持续姿态"
                );
            }
        }
    }
    assert_eq!(total, 111, "动画目录必须覆盖 111 个动作条目");
}

#[test]
fn every_spec_names_a_real_production_path() {
    let table = action_spec_table();
    let workspace = workspace_root();
    for spec in table.specs() {
        for path in [spec.candidate_source, spec.resolver] {
            let (file, function) = path
                .split_once("::")
                .unwrap_or_else(|| panic!("生产路径必须带函数名：{path}"));
            let relative = file.strip_prefix("crates/sim/src/").unwrap_or(file);
            let full = workspace.join("crates/sim/src").join(relative);
            let source = std::fs::read_to_string(&full)
                .unwrap_or_else(|error| panic!("生产路径的文件不存在 {file}: {error}"));
            assert!(
                source.contains(&format!("fn {function}")),
                "生产路径指向的函数不存在：{path}"
            );
        }
    }
}

#[test]
fn every_spec_names_a_real_scenario_test() {
    let table = action_spec_table();
    let workspace = workspace_root();
    for spec in table.specs() {
        let (file, function) = spec
            .scenario_test
            .split_once("::")
            .unwrap_or_else(|| panic!("场景测试必须带函数名：{}", spec.scenario_test));
        let full = workspace.join(file);
        let source = std::fs::read_to_string(&full)
            .unwrap_or_else(|error| panic!("场景测试的文件不存在 {file}: {error}"));
        assert!(
            source.contains(&format!("fn {function}")),
            "场景测试指向的函数不存在：{}",
            spec.scenario_test
        );
    }
}

#[test]
fn every_role_is_defined_in_the_domain_layer() {
    for role in [
        "primary",
        "screener",
        "cut_partner",
        "on_ball_defender",
        "defender",
        "rebounder",
        "shooter",
    ] {
        assert!(
            ActionRole::from_str(role).is_some(),
            "角色 {role} 必须有领域定义"
        );
    }
}
