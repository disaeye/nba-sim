//! 渲染动作校验：动作身份与动画配方的一致性。
//!
//! 方案 §10.2：两类球场视图消费同一动作状态，回放可按帧重建相同动作与
//! 阶段。界面按权威动作状态呈现所有登记动作与技术变体。
//!
//! 这份校验器把「动作身份」与「动画配方」绑在一起：
//!
//! - 动作族、类别、变体与角色必须是动画目录里登记过的组合；
//! - 动作的每个阶段必须在配方里给出；
//! - 两类渲染器读同一份配方，因此同一帧显示相同的动作身份。
//!
//! 未登记的组合与缺失的配方就地失败：渲染器不能自己猜一个动画出来。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// 动画目录里的一个动作配方。
///
/// 配方是 3D 渲染器用的剪辑与速度，以及 2D 渲染器用的运动学与中文标签。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecipe {
    /// 3D 剪辑名。
    pub clip: String,
    /// 3D 播放速率。
    pub speed: f32,
    /// 3D 是否循环播放。
    pub loop_: bool,
    /// 2D 运动学名称。
    pub kinetics: String,
    /// 2D 中文标签。
    pub label_zh: String,
}

/// 一个动作条目的配方。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecipeEntry {
    /// 动作条目名。
    pub name: String,
    /// 协议类别。
    pub kind: String,
    /// 技术变体。
    pub variant: String,
    /// 交互角色。
    pub role: String,
    /// 动作的阶段列表。
    pub phases: Vec<String>,
    /// 3D 配方。
    pub recipe_3d: Recipe3d,
    /// 2D 配方。
    pub recipe_2d: Recipe2d,
}

/// 3D 配方。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe3d {
    /// 剪辑名。
    pub clip: String,
    /// 播放速率。
    pub speed: f32,
    /// 是否循环。
    #[serde(rename = "loop")]
    pub loop_: bool,
}

/// 2D 配方。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Recipe2d {
    /// 运动学名称。
    pub kinetics: String,
    /// 中文标签。
    pub label_zh: String,
}

/// 动画目录。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionAnimationCatalog {
    /// 目录版本。
    pub catalog_version: u32,
    /// 允许使用的 3D 剪辑。
    pub allowed_clips: Vec<String>,
    /// 动作族。
    pub families: Vec<AnimationFamily>,
}

/// 动画目录里的一个动作族。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationFamily {
    /// 动作族。
    pub id: String,
    /// 动作条目。
    pub actions: Vec<ActionRecipeEntry>,
}

/// 渲染动作校验器。
///
/// 校验器持有动画目录：动作身份在目录里查得到配方。两类渲染器都从校验器
/// 取配方，因此同一帧显示相同的动作身份与相同的动画。
#[derive(Debug, Clone)]
pub struct RenderActionValidator {
    entries: BTreeMap<(String, String, String, String), ActionRecipeEntry>,
    clips: Vec<String>,
}

impl RenderActionValidator {
    /// 由动画目录建校验器。
    ///
    /// 目录里重复的条目、缺字段的条目与不在允许列表里的剪辑就地失败：
    /// 动画目录本身必须自洽。
    pub fn new(catalog: &ActionAnimationCatalog) -> Result<Self, String> {
        let mut entries: BTreeMap<(String, String, String, String), ActionRecipeEntry> =
            BTreeMap::new();
        for family in &catalog.families {
            for action in &family.actions {
                if action.recipe_3d.speed <= 0.0 {
                    return Err(format!(
                        "动作条目 {} 的播放速率必须为正：{}",
                        action.variant, action.recipe_3d.speed
                    ));
                }
                if !catalog.allowed_clips.contains(&action.recipe_3d.clip) {
                    return Err(format!(
                        "动作条目 {} 的剪辑 {} 不在允许列表里",
                        action.variant, action.recipe_3d.clip
                    ));
                }
                if action.phases.is_empty() {
                    return Err(format!("动作条目 {} 没有阶段", action.variant));
                }
                if action.recipe_2d.kinetics.is_empty() {
                    return Err(format!("动作条目 {} 没有 2D 运动学", action.variant));
                }
                let key = (
                    family.id.clone(),
                    action.kind.clone(),
                    action.variant.clone(),
                    action.role.clone(),
                );
                if entries.insert(key.clone(), action.clone()).is_some() {
                    return Err(format!("动画目录里有重复条目：{}", key.1));
                }
            }
        }
        Ok(Self {
            entries,
            clips: catalog.allowed_clips.clone(),
        })
    }

    /// 允许的 3D 剪辑。
    pub fn allowed_clips(&self) -> &[String] {
        &self.clips
    }

    /// 目录里的条目总数。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 目录是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 查一个动作条目的配方。
    ///
    /// 动作身份不在目录里就地失败：渲染器不能给一个没有登记的动作
    /// 编一个动画。
    pub fn recipe(
        &self,
        family: &str,
        kind: &str,
        variant: &str,
        role: &str,
    ) -> Result<&ActionRecipeEntry, String> {
        self.entries
            .get(&(family.to_string(), kind.to_string(), variant.to_string(), role.to_string()))
            .ok_or_else(|| {
                format!(
                    "动作身份没有登记的动画配方：{family} {kind} {variant} {role}"
                )
            })
    }

    /// 查一个动作条目在某个阶段的 3D 配方。
    ///
    /// 阶段不在条目里就地失败：动作进入那个阶段时必须知道播什么。
    pub fn recipe_3d(
        &self,
        family: &str,
        kind: &str,
        variant: &str,
        role: &str,
        phase: &str,
    ) -> Result<(String, f32, bool), String> {
        let entry = self.recipe(family, kind, variant, role)?;
        if !entry.phases.iter().any(|entry_phase| entry_phase == phase) {
            return Err(format!(
                "动作条目 {family} {kind} {variant} 没有阶段 {phase} 的配方：登记的阶段是 {:?}",
                entry.phases
            ));
        }
        Ok((
            entry.recipe_3d.clip.clone(),
            entry.recipe_3d.speed,
            entry.recipe_3d.loop_,
        ))
    }

    /// 查一个动作条目的 2D 配方。
    pub fn recipe_2d(
        &self,
        family: &str,
        kind: &str,
        variant: &str,
        role: &str,
    ) -> Result<(String, String), String> {
        let entry = self.recipe(family, kind, variant, role)?;
        Ok((
            entry.recipe_2d.kinetics.clone(),
            entry.recipe_2d.label_zh.clone(),
        ))
    }

    /// 目录里出现过的动作族。
    pub fn families(&self) -> Vec<String> {
        let mut families: Vec<String> = self
            .entries
            .keys()
            .map(|(family, _, _, _)| family.clone())
            .collect();
        families.sort();
        families.dedup();
        families
    }

    /// 目录里出现过的角色。
    pub fn roles(&self) -> Vec<String> {
        let mut roles: Vec<String> = self
            .entries
            .keys()
            .map(|(_, _, _, role)| role.clone())
            .collect();
        roles.sort();
        roles.dedup();
        roles
    }
}
