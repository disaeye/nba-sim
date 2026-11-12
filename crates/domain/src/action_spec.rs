//! 动作规格表：111 个动作条目的运行时契约。
//!
//! 方案 §6：每个动作条目有种类、技术细分、执行人、对象、准备窗口、执行
//! 窗口、恢复窗口、控制要求、运动约束和取消条件。技术细分参与可执行性、
//! 时长、球和身体路线、防守反应、失误风险、体能与接触条件；每种动作都
//! 需要独立消费者，不允许只添加名称。
//!
//! 这张表把「有哪些动作条目」从散落在若干枚举里的名字变成一份带契约的
//! 清单：动作族、类别、变体、角色、阶段、生产候选来源、生产结算器、
//! 对手响应与场景测试都在同一处。渲染器、验证器与文档守卫都从这张表
//! 取值，因此目录与运行时不可能各说各话。

use serde::{Deserialize, Serialize};

/// 动作族标识符。
///
/// 族是协议的稳定分组：渲染器按族选择动画片段，验证器按族核对事件。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionFamily {
    ActionDribble,
    ActionDrive,
    ActionFinish,
    ActionShot,
    ActionPost,
    ActionPass,
    ActionFake,
    ActionBallProtection,
    ActionScreen,
    ActionCut,
    ActionRelocate,
    ActionDefenseOnBall,
    ActionDefenseOffBall,
    ActionCloseout,
    ActionRebound,
    ActionTransition,
}

impl ActionFamily {
    /// 全部动作族。
    pub const ALL: [ActionFamily; 16] = [
        ActionFamily::ActionDribble,
        ActionFamily::ActionDrive,
        ActionFamily::ActionFinish,
        ActionFamily::ActionShot,
        ActionFamily::ActionPost,
        ActionFamily::ActionPass,
        ActionFamily::ActionFake,
        ActionFamily::ActionBallProtection,
        ActionFamily::ActionScreen,
        ActionFamily::ActionCut,
        ActionFamily::ActionRelocate,
        ActionFamily::ActionDefenseOnBall,
        ActionFamily::ActionDefenseOffBall,
        ActionFamily::ActionCloseout,
        ActionFamily::ActionRebound,
        ActionFamily::ActionTransition,
    ];

    /// 动作族的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ActionDribble => "ACTION-DRIBBLE",
            Self::ActionDrive => "ACTION-DRIVE",
            Self::ActionFinish => "ACTION-FINISH",
            Self::ActionShot => "ACTION-SHOT",
            Self::ActionPost => "ACTION-POST",
            Self::ActionPass => "ACTION-PASS",
            Self::ActionFake => "ACTION-FAKE",
            Self::ActionBallProtection => "ACTION-BALL-PROTECTION",
            Self::ActionScreen => "ACTION-SCREEN",
            Self::ActionCut => "ACTION-CUT",
            Self::ActionRelocate => "ACTION-RELOCATE",
            Self::ActionDefenseOnBall => "ACTION-DEFENSE-ON-BALL",
            Self::ActionDefenseOffBall => "ACTION-DEFENSE-OFF-BALL",
            Self::ActionCloseout => "ACTION-CLOSEOUT",
            Self::ActionRebound => "ACTION-REBOUND",
            Self::ActionTransition => "ACTION-TRANSITION",
        }
    }

    /// 从档案键读回动作族。
    pub fn from_str(key: &str) -> Option<Self> {
        Some(match key {
            "ACTION-DRIBBLE" => Self::ActionDribble,
            "ACTION-DRIVE" => Self::ActionDrive,
            "ACTION-FINISH" => Self::ActionFinish,
            "ACTION-SHOT" => Self::ActionShot,
            "ACTION-POST" => Self::ActionPost,
            "ACTION-PASS" => Self::ActionPass,
            "ACTION-FAKE" => Self::ActionFake,
            "ACTION-BALL-PROTECTION" => Self::ActionBallProtection,
            "ACTION-SCREEN" => Self::ActionScreen,
            "ACTION-CUT" => Self::ActionCut,
            "ACTION-RELOCATE" => Self::ActionRelocate,
            "ACTION-DEFENSE-ON-BALL" => Self::ActionDefenseOnBall,
            "ACTION-DEFENSE-OFF-BALL" => Self::ActionDefenseOffBall,
            "ACTION-CLOSEOUT" => Self::ActionCloseout,
            "ACTION-REBOUND" => Self::ActionRebound,
            "ACTION-TRANSITION" => Self::ActionTransition,
            _ => return None,
        })
    }
}

/// 动作条目在交互中的角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionRole {
    /// 主执行人。
    Primary,
    /// 掩护人。
    Screener,
    /// 借掩护的切入人。
    CutPartner,
    /// 对位防守人。
    OnBallDefender,
    /// 其他防守人。
    Defender,
    /// 抢篮板的人。
    Rebounder,
    /// 出手人。
    Shooter,
}

impl ActionRole {
    /// 角色的档案键。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Primary => "primary",
            Self::Screener => "screener",
            Self::CutPartner => "cut_partner",
            Self::OnBallDefender => "on_ball_defender",
            Self::Defender => "defender",
            Self::Rebounder => "rebounder",
            Self::Shooter => "shooter",
        }
    }

    /// 从档案键读回角色。
    pub fn from_str(key: &str) -> Option<Self> {
        Some(match key {
            "primary" => Self::Primary,
            "screener" => Self::Screener,
            "cut_partner" => Self::CutPartner,
            "on_ball_defender" => Self::OnBallDefender,
            "defender" => Self::Defender,
            "rebounder" => Self::Rebounder,
            "shooter" => Self::Shooter,
            _ => return None,
        })
    }
}

/// 动作条目。
///
/// 一个条目就是一个可被候选生成、可被执行、可被取消、可被渲染的最小
/// 动作语义单元。`candidate_source` 与 `resolver` 是生产路径：
/// 前者是生成这个动作候选的函数，后者是执行它的结算函数。两者都是
/// 代码位置，因此「只有名字没有实现」的条目在守卫里立刻暴露。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSpec {
    /// 动作族。
    pub family: ActionFamily,
    /// 动作条目名。
    pub name: &'static str,
    /// 协议里的类别。
    pub kind: &'static str,
    /// 协议里的技术变体。
    pub variant: &'static str,
    /// 交互中的角色。
    pub role: ActionRole,
    /// 动作是否有阶段窗口。持续姿态没有窗口，它的阶段是 sustained。
    pub windowed: bool,
    /// 生成这个动作候选的生产路径。
    pub candidate_source: &'static str,
    /// 执行这个动作的生产路径。
    pub resolver: &'static str,
    /// 对手响应：对手看到这次动作之后做什么。
    pub opponent_response: &'static str,
    /// 场景测试：`文件::函数`。
    pub scenario_test: &'static str,
}

/// 动作规格表。
///
/// 表里的每一项都对应一个生产路径：候选来源与结算器都在引擎里，缺失就
/// 在读表时失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionSpecTable {
    specs: Vec<ActionSpec>,
}

impl ActionSpecTable {
    /// 由条目列表建表。重复的 (族, 类别, 变体, 角色) 就地失败。
    pub fn new(specs: Vec<ActionSpec>) -> Self {
        let table = Self { specs };
        table.validate();
        table
    }

    /// 表里没有重复条目，且每个条目都带生产路径与场景测试。
    fn validate(&self) {
        let mut seen = std::collections::BTreeSet::new();
        for spec in &self.specs {
            assert!(
                seen.insert((spec.family, spec.kind, spec.variant, spec.role)),
                "动作条目重复：{} {} {} {}",
                spec.family.as_str(),
                spec.kind,
                spec.variant,
                spec.role.as_str()
            );
            assert!(
                !spec.candidate_source.is_empty(),
                "动作条目 {} 没有生产候选来源",
                spec.name
            );
            assert!(
                !spec.resolver.is_empty(),
                "动作条目 {} 没有生产结算器",
                spec.name
            );
            assert!(
                !spec.opponent_response.is_empty(),
                "动作条目 {} 没有对手响应",
                spec.name
            );
            assert!(
                spec.scenario_test.contains("::"),
                "动作条目 {} 的场景测试必须指向一个测试函数",
                spec.name
            );
        }
    }

    /// 表里的全部条目。
    pub fn specs(&self) -> &[ActionSpec] {
        &self.specs
    }

    /// 条目总数。
    pub fn len(&self) -> usize {
        self.specs.len()
    }

    /// 表是否为空。
    pub fn is_empty(&self) -> bool {
        self.specs.is_empty()
    }

    /// 某个族的全部条目。
    pub fn family(&self, family: ActionFamily) -> Vec<&ActionSpec> {
        self.specs
            .iter()
            .filter(|spec| spec.family == family)
            .collect()
    }

    /// 按 (族, 类别, 变体, 角色) 查条目。
    pub fn lookup(
        &self,
        family: ActionFamily,
        kind: &str,
        variant: &str,
        role: ActionRole,
    ) -> Option<&ActionSpec> {
        self.specs.iter().find(|spec| {
            spec.family == family && spec.kind == kind && spec.variant == variant && spec.role == role
        })
    }

    /// 按名字查条目。
    ///
    /// 动作名在族之间不唯一：「勾手」既是跳投也是背身步法，因此调用方
    /// 必须给出族。
    pub fn by_name(&self, name: &str) -> Option<&ActionSpec> {
        self.specs.iter().find(|spec| spec.name == name)
    }

    /// 按族与名字查条目。
    pub fn by_family_name(&self, family: ActionFamily, name: &str) -> Option<&ActionSpec> {
        self.specs
            .iter()
            .find(|spec| spec.family == family && spec.name == name)
    }

    /// 表里出现过的全部动作族。
    pub fn families(&self) -> Vec<ActionFamily> {
        let mut families: Vec<ActionFamily> = self
            .specs
            .iter()
            .map(|spec| spec.family)
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        families.sort();
        families
    }
}
