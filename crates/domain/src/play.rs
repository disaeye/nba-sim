//! Play 规则模型（tactics.md §2.2.4、ADR-019）：回合级规则化行为覆盖的数据层。
//!
//! Play 是回合级的窗口化覆盖：触发后在 `window_seconds` 内注入候选偏好、
//! 候选抑制与行为规则，全部经效用管线与约束层生效，不声明动作顺序、
//! 不写坐标、不含球员 id（charter C1 / TA8）。本模块只承载 schema、
//! 反序列化与结构校验；选板器与每 tick 规则评估层在外层 crate。
//!
//! 场输出谓词（`HelpShadingOff` / `WeakSideVacated`）消费势能场输出量。
//! tactics.md §2.5 裁定场量逐 tick 抖动，此类谓词必须先过场侧滞回稳定
//! 通道（双阈值 + 最小保持时间）才可进入开关型判定；本 schema 层只
//! 标记谓词类别，滞回本身由场优化链实现。

use serde::{Deserialize, Serialize};

/// Play 档案的两级 kind（tactics.md §2.2.4）。
///
/// `System` 变体属于既有档案体系（`TacticalSetSpec` 等槽位适配档案），
/// `PlaySpec` 只允许携带 [`PlayKind::Play`]；选板器与守卫按 kind 分流，
/// 两库互不混用，任何一边引用另一边的字段都是 schema 错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayKind {
    System,
    Play,
}

/// 持球人候选动作族（封闭枚举）。
///
/// 对应 `crates/decision` 效用管线的候选族（`CandidateAction` 的
/// Shoot/Drive/PostUp/Pass/Dwell 等变体）。domain 不依赖 decision：
/// 枚举在 domain 定义，decision 侧消费时做映射。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum DecisionActionFamily {
    /// 投篮族。
    Shoot,
    /// 持球突破族。
    Drive,
    /// 背身单打族（`CandidateAction::PostUp`）。
    PostUp,
    /// 传球族。
    Pass,
    /// 三威胁试探族（`CandidateAction::TripleThreatJab`）。
    TripleThreatJab,
    /// 原地持球停顿族（`CandidateAction::Dwell`）。
    Dwell,
}

impl DecisionActionFamily {
    /// 档案与错误信息里使用的字符串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shoot => "Shoot",
            Self::Drive => "Drive",
            Self::PostUp => "PostUp",
            Self::Pass => "Pass",
            Self::TripleThreatJab => "TripleThreatJab",
            Self::Dwell => "Dwell",
        }
    }
}

/// Play 谓词（封闭枚举，tactics.md §2.2.4 谓词词汇表）。
///
/// serde tag 取 snake_case 字符串（如 `"beyond_circle_ft"`），保证档案
/// 可读；带参谓词的字段随 tag 平铺在同一对象里。词汇表外的 tag 字符串
/// 在反序列化时直接报错，不静默忽略（§2.2.4 纪律 2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "predicate", rename_all = "snake_case")]
pub enum PlayPredicate {
    // ---- 球态谓词 ----
    /// 本队持球且评估对象是持球人。
    CarrierPossed,
    /// 前场阵地战（非快攻）。
    HalfcourtPossession,
    /// 进攻时钟低于阈值（秒）。
    ShotClockUrgent {
        /// 阈值秒数，非负且有限。
        threshold_s: f32,
    },

    // ---- 几何谓词（全部由 `SpatialGeometry` 派生，ADR-015）----
    /// 持球人距篮超过 r 英尺。
    BeyondCircleFt {
        /// 距篮半径（英尺），非负且有限。
        r: f32,
    },
    /// 掩护已建立：持球人与掩护人距离小于 `screen_detection_radius_ft`
    /// 且掩护人站定。
    ScreenEstablished,
    /// 指定侧底角有本队球员。
    CornerOccupied {
        /// 底角侧别。
        side: PlayCourtSide,
    },

    // ---- 场输出谓词（必须先过场侧滞回，tactics.md §2.5）----
    /// 对位协防人被护筐引力拉离持球人走廊（`threat_ratio` 超阈值且
    /// 目标点位移稳定超过滞回带宽）。
    HelpShadingOff,
    /// 弱侧出现真空（`void_ratio` 超阈值且稳定）。
    WeakSideVacated,
}

/// 底角侧别（`CornerOccupied` 的参数）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayCourtSide {
    Left,
    Right,
}

/// 动作词表（封闭枚举，tactics.md §2.2.4）。
///
/// 动词映射到目标生成偏置与动作窗口类型（`domain::action_window` 的
/// `ActionType`）；动词不直接执行动作，只改变目标与候选语境。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum PlayVerb {
    /// 掩护人顺下。
    ScreenRoll,
    /// 掩护人外弹。
    ScreenPop,
    /// 定点落位。
    SpotUp,
    /// 弱侧转移。
    Relocate,
    /// 背切。
    CutBackdoor,
    /// 上提接应。
    Lift,
}

impl PlayVerb {
    /// 档案与错误信息里使用的字符串形式。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ScreenRoll => "ScreenRoll",
            Self::ScreenPop => "ScreenPop",
            Self::SpotUp => "SpotUp",
            Self::Relocate => "Relocate",
            Self::CutBackdoor => "CutBackdoor",
            Self::Lift => "Lift",
        }
    }
}

/// 持球人候选族偏好：效用加项，经 `DecisionRules` 通道的权重系数缩放后
/// 加性进入效用管线（architecture.md §5.1 的加性修正项）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayCarrierPreference {
    pub action_family: DecisionActionFamily,
    /// 效用增量，非负且有限。
    pub bonus: f32,
}

/// 候选抑制模式。
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum PlayInhibitionMode {
    /// 效用惩罚（加性项，量级为 `penalty`）。
    Soft {
        /// 效用惩罚量，非负且有限。
        penalty: f32,
    },
    /// 把该族从候选集中移除。
    Hard,
}

/// 候选抑制声明。mode 的 tag（`"soft"` / `"hard"`）与 `penalty` 平铺在
/// 同一对象里：`{"action_family": "Dwell", "mode": "hard"}`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayInhibition {
    pub action_family: DecisionActionFamily,
    #[serde(flatten)]
    pub mode: PlayInhibitionMode,
}

/// 行为规则：激活窗口内每 tick 重估 `when`，全真时 `then` 生效。
///
/// `then.slot` 必须引用当前 System 档案已声明的槽位 id（Play 不发明
/// System 里不存在的槽位）；实在性校验在引擎接线时做，本层只查非空。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayRule {
    /// 规则标识（档案内唯一）。
    pub id: String,
    /// 非空规则谓词列表，按 AND 求值；全部为真时 `then` 生效。
    pub when: Vec<PlayPredicate>,
    pub then: PlayAction,
    /// 该规则生效时的持球人候选族偏好；同族加项与顶层及其他命中规则累加。
    #[serde(default)]
    pub carrier_preferences: Vec<PlayCarrierPreference>,
}

/// 规则生效动作：动词 + 目标槽位。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayAction {
    pub verb: PlayVerb,
    /// 目标槽位 id（引用 System 档案，非空）。
    pub slot: String,
}

/// 选板触发：每决策 tick 评估，非空 `when` 中全部谓词为真时进入选板候选；
/// 激活持续 `window_seconds`，结束或回合终结后进入 `cooldown_seconds` 冷却。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayTrigger {
    /// 非空触发谓词列表，按 AND 求值；全部为真时本 Play 进入选板候选。
    pub when: Vec<PlayPredicate>,
    /// 激活窗口时长（秒），非零且有限。
    pub window_seconds: f32,
    /// 冷却时长（秒），非负且有限。
    pub cooldown_seconds: f32,
}

/// Play 档案：回合级规则化行为覆盖的完整 schema（tactics.md §2.2.4）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlaySpec {
    /// schema 版本，当前必须为 1。
    pub schema_version: u32,
    /// 档案标识，非空。
    pub id: String,
    /// 中文名称，非空。
    pub name_zh: String,
    /// 两级 kind；`PlaySpec` 只允许携带 [`PlayKind::Play`]。
    pub kind: PlayKind,
    /// 顶层持球人候选族偏好；与本 tick 命中规则中的同族项累加。
    #[serde(default)]
    pub carrier_preferences: Vec<PlayCarrierPreference>,
    /// 候选抑制声明。
    #[serde(default)]
    pub inhibitions: Vec<PlayInhibition>,
    /// 行为规则。
    #[serde(default)]
    pub rules: Vec<PlayRule>,
    /// 选板触发（至少一个，且每项 `when` 非空）。
    pub triggers: Vec<PlayTrigger>,
}

/// `PlaySpec::validate` 的错误：带具体字段定位的结构化描述。
///
/// fast-fail：第一个非法点就地报错，不 fallback、不部分修复。
#[derive(Debug, Clone, PartialEq)]
pub enum PlaySpecError {
    /// schema 版本不受支持。
    SchemaVersion {
        id: String,
        found: u32,
        expected: u32,
    },
    /// `id` 为空白。
    EmptyId,
    /// `name_zh` 为空白。
    EmptyNameZh { id: String },
    /// `kind` 携带了 `Play` 以外的变体。
    InvalidKind { id: String, found: PlayKind },
    /// 无任何选板触发。
    NoTriggers { id: String },
    /// 触发条件 when 为空。
    EmptyTriggerWhen { id: String, trigger_index: usize },
    /// 触发窗口/冷却时长非法（非有限或超出符号约束）。
    InvalidTriggerTiming {
        id: String,
        trigger_index: usize,
        field: &'static str,
        value: f32,
    },
    /// hard 抑制覆盖了全部候选族，未保留合法出路（tactics.md §2.2.4）。
    HardInhibitionCoversAllFamilies {
        id: String,
        hard_families: Vec<&'static str>,
    },
    /// 同一偏好列表内动作族重复声明。
    DuplicateFamily {
        id: String,
        section: &'static str,
        family: &'static str,
    },
    /// 规则 `then.slot` 为空白。
    EmptyRuleSlot { id: String, rule_id: String },
    /// 规则条件 when 为空。
    EmptyRuleWhen { id: String, rule_id: String },
    /// 多条规则引用同一槽位，无法保证同 tick 只有一个动作。
    DuplicateRuleSlot { id: String, slot: String },
    /// 规则标识为空白。
    EmptyRuleId { id: String, rule_index: usize },
    /// 规则标识重复。
    DuplicateRuleId { id: String, rule_id: String },
    /// 规则谓词参数非法（非有限或负值）。
    InvalidRulePredicate {
        id: String,
        rule_id: String,
        predicate: &'static str,
        field: &'static str,
        value: f32,
    },
    /// 触发谓词参数非法（非有限或负值）。
    InvalidTriggerPredicate {
        id: String,
        trigger_index: usize,
        predicate: &'static str,
        field: &'static str,
        value: f32,
    },
    /// 偏好 bonus / 抑制 penalty 非法（非有限或负值）。
    InvalidNumeric {
        id: String,
        section: &'static str,
        family: &'static str,
        field: &'static str,
        value: f32,
    },
    /// 反序列化失败：未知谓词/动词/族/kind 字符串、未知字段或形状不符。
    ///
    /// 谓词、动词、动作族都是封闭枚举（tactics.md §2.2.4 纪律 2）：
    /// 词汇表外的字符串必须在此处被拒绝，不静默忽略。
    Deserialization { message: String },
}

impl std::fmt::Display for PlaySpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SchemaVersion {
                id,
                found,
                expected,
            } => write!(
                f,
                "play `{id}` has schema_version {found}, expected {expected}"
            ),
            Self::EmptyId => write!(f, "play declares an empty `id`"),
            Self::EmptyNameZh { id } => write!(f, "play `{id}` declares an empty `name_zh`"),
            Self::InvalidKind { id, found } => write!(
                f,
                "play `{id}` carries kind `{}`, only `Play` may be carried by PlaySpec",
                found.as_str()
            ),
            Self::NoTriggers { id } => write!(f, "play `{id}` declares no triggers"),
            Self::EmptyTriggerWhen { id, trigger_index } => write!(
                f,
                "play `{id}` trigger #{trigger_index} has an empty `when` list"
            ),
            Self::InvalidTriggerTiming {
                id,
                trigger_index,
                field,
                value,
            } => write!(
                f,
                "play `{id}` trigger #{trigger_index} has invalid `{field}`: {value}"
            ),
            Self::HardInhibitionCoversAllFamilies { id, hard_families } => write!(
                f,
                "play `{id}` hard-inhibits every action family {hard_families:?}; \
                 at least one family must survive (tactics.md §2.2.4)"
            ),
            Self::DuplicateFamily {
                id,
                section,
                family,
            } => write!(
                f,
                "play `{id}` declares duplicate `{family}` in `{section}`"
            ),
            Self::EmptyRuleSlot { id, rule_id } => {
                write!(f, "play `{id}` rule `{rule_id}` has an empty `then.slot`")
            }
            Self::EmptyRuleWhen { id, rule_id } => {
                write!(f, "play `{id}` rule `{rule_id}` has an empty `when` list")
            }
            Self::DuplicateRuleSlot { id, slot } => {
                write!(f, "play `{id}` declares multiple rules for slot `{slot}`")
            }
            Self::EmptyRuleId { id, rule_index } => {
                write!(f, "play `{id}` rule #{rule_index} has an empty id")
            }
            Self::DuplicateRuleId { id, rule_id } => {
                write!(f, "play `{id}` declares duplicate rule id `{rule_id}`")
            }
            Self::InvalidRulePredicate {
                id,
                rule_id,
                predicate,
                field,
                value,
            } => write!(
                f,
                "play `{id}` rule `{rule_id}` predicate `{predicate}` has invalid \
                 `{field}`: {value}"
            ),
            Self::InvalidTriggerPredicate {
                id,
                trigger_index,
                predicate,
                field,
                value,
            } => write!(
                f,
                "play `{id}` trigger #{trigger_index} predicate `{predicate}` has \
                 invalid `{field}`: {value}"
            ),
            Self::InvalidNumeric {
                id,
                section,
                family,
                field,
                value,
            } => write!(
                f,
                "play `{id}` `{section}` entry `{family}` has invalid `{field}`: {value}"
            ),
            Self::Deserialization { message } => {
                write!(f, "play spec deserialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for PlaySpecError {}

impl PlayKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Play => "Play",
        }
    }
}

/// 谓词的稳定字符串名（错误定位用，与 serde tag 一致）。
fn predicate_name(predicate: &PlayPredicate) -> &'static str {
    match predicate {
        PlayPredicate::CarrierPossed => "carrier_possed",
        PlayPredicate::HalfcourtPossession => "halfcourt_possession",
        PlayPredicate::ShotClockUrgent { .. } => "shot_clock_urgent",
        PlayPredicate::BeyondCircleFt { .. } => "beyond_circle_ft",
        PlayPredicate::ScreenEstablished => "screen_established",
        PlayPredicate::CornerOccupied { .. } => "corner_occupied",
        PlayPredicate::HelpShadingOff => "help_shading_off",
        PlayPredicate::WeakSideVacated => "weak_side_vacated",
    }
}

/// 谓词参数校验的错误定位：规则谓词记规则 id，触发谓词记触发下标。
enum PredicateSite {
    Trigger(usize),
    Rule(String),
}

impl PlaySpec {
    /// 从 JSON 反序列化并校验（fast-fail：非法档案在此就地报错）。
    pub fn from_json(json_str: &str) -> Result<Self, PlaySpecError> {
        let spec: Self =
            serde_json::from_str(json_str).map_err(|err| PlaySpecError::Deserialization {
                message: err.to_string(),
            })?;
        spec.validate()?;
        Ok(spec)
    }

    /// 结构自洽校验（tactics.md §2.2.4 字段语义 + hard 抑制合法出路不变量）。
    pub fn validate(&self) -> Result<(), PlaySpecError> {
        if self.id.trim().is_empty() {
            return Err(PlaySpecError::EmptyId);
        }
        if self.name_zh.trim().is_empty() {
            return Err(PlaySpecError::EmptyNameZh {
                id: self.id.clone(),
            });
        }
        if self.schema_version != 1 {
            return Err(PlaySpecError::SchemaVersion {
                id: self.id.clone(),
                found: self.schema_version,
                expected: 1,
            });
        }
        if self.kind != PlayKind::Play {
            return Err(PlaySpecError::InvalidKind {
                id: self.id.clone(),
                found: self.kind,
            });
        }
        if self.triggers.is_empty() {
            return Err(PlaySpecError::NoTriggers {
                id: self.id.clone(),
            });
        }
        for (index, trigger) in self.triggers.iter().enumerate() {
            if trigger.when.is_empty() {
                return Err(PlaySpecError::EmptyTriggerWhen {
                    id: self.id.clone(),
                    trigger_index: index,
                });
            }
            if !(trigger.window_seconds.is_finite() && trigger.window_seconds > 0.0) {
                return Err(PlaySpecError::InvalidTriggerTiming {
                    id: self.id.clone(),
                    trigger_index: index,
                    field: "window_seconds",
                    value: trigger.window_seconds,
                });
            }
            if !(trigger.cooldown_seconds.is_finite() && trigger.cooldown_seconds >= 0.0) {
                return Err(PlaySpecError::InvalidTriggerTiming {
                    id: self.id.clone(),
                    trigger_index: index,
                    field: "cooldown_seconds",
                    value: trigger.cooldown_seconds,
                });
            }
            for predicate in &trigger.when {
                self.validate_predicate(predicate, &PredicateSite::Trigger(index))?;
            }
        }

        // 抑制：bonus/penalty 非负有限；hard 集合不得覆盖全部族。
        let mut hard_families: Vec<DecisionActionFamily> = Vec::new();
        for inhibition in &self.inhibitions {
            if let PlayInhibitionMode::Soft { penalty } = inhibition.mode {
                if !(penalty.is_finite() && penalty >= 0.0) {
                    return Err(PlaySpecError::InvalidNumeric {
                        id: self.id.clone(),
                        section: "inhibitions",
                        family: inhibition.action_family.as_str(),
                        field: "penalty",
                        value: penalty,
                    });
                }
            } else if !hard_families.contains(&inhibition.action_family) {
                hard_families.push(inhibition.action_family);
            }
        }
        if hard_families.len() == DecisionActionFamily::ALL.len() {
            return Err(PlaySpecError::HardInhibitionCoversAllFamilies {
                id: self.id.clone(),
                hard_families: hard_families.iter().map(|f| f.as_str()).collect(),
            });
        }

        // 顶层偏好：数值非负有限，族不得在列表内重复。
        let mut top_families: Vec<DecisionActionFamily> = Vec::new();
        for preference in &self.carrier_preferences {
            if !(preference.bonus.is_finite() && preference.bonus >= 0.0) {
                return Err(PlaySpecError::InvalidNumeric {
                    id: self.id.clone(),
                    section: "carrier_preferences",
                    family: preference.action_family.as_str(),
                    field: "bonus",
                    value: preference.bonus,
                });
            }
            if !top_families.contains(&preference.action_family) {
                top_families.push(preference.action_family);
            } else {
                return Err(PlaySpecError::DuplicateFamily {
                    id: self.id.clone(),
                    section: "carrier_preferences",
                    family: preference.action_family.as_str(),
                });
            }
        }

        // 规则：id 非空且唯一、slot 非空、谓词参数合法，规则内偏好数值合法。
        let mut seen_rule_ids: Vec<&str> = Vec::new();
        let mut seen_rule_slots: Vec<&str> = Vec::new();
        for (index, rule) in self.rules.iter().enumerate() {
            if rule.id.trim().is_empty() {
                return Err(PlaySpecError::EmptyRuleId {
                    id: self.id.clone(),
                    rule_index: index,
                });
            }
            if seen_rule_ids.contains(&rule.id.as_str()) {
                return Err(PlaySpecError::DuplicateRuleId {
                    id: self.id.clone(),
                    rule_id: rule.id.clone(),
                });
            }
            seen_rule_ids.push(rule.id.as_str());
            if rule.when.is_empty() {
                return Err(PlaySpecError::EmptyRuleWhen {
                    id: self.id.clone(),
                    rule_id: rule.id.clone(),
                });
            }
            for predicate in &rule.when {
                self.validate_predicate(predicate, &PredicateSite::Rule(rule.id.clone()))?;
            }
            if rule.then.slot.trim().is_empty() {
                return Err(PlaySpecError::EmptyRuleSlot {
                    id: self.id.clone(),
                    rule_id: rule.id.clone(),
                });
            }
            if seen_rule_slots.contains(&rule.then.slot.as_str()) {
                return Err(PlaySpecError::DuplicateRuleSlot {
                    id: self.id.clone(),
                    slot: rule.then.slot.clone(),
                });
            }
            seen_rule_slots.push(rule.then.slot.as_str());

            let mut rule_families: Vec<DecisionActionFamily> = Vec::new();
            for preference in &rule.carrier_preferences {
                if !(preference.bonus.is_finite() && preference.bonus >= 0.0) {
                    return Err(PlaySpecError::InvalidNumeric {
                        id: self.id.clone(),
                        section: "rules[].carrier_preferences",
                        family: preference.action_family.as_str(),
                        field: "bonus",
                        value: preference.bonus,
                    });
                }
                if rule_families.contains(&preference.action_family) {
                    return Err(PlaySpecError::DuplicateFamily {
                        id: self.id.clone(),
                        section: "rules[].carrier_preferences",
                        family: preference.action_family.as_str(),
                    });
                }
                rule_families.push(preference.action_family);
            }
        }
        Ok(())
    }

    /// 谓词参数校验（非负且有限），错误按 `site` 定位。
    fn validate_predicate(
        &self,
        predicate: &PlayPredicate,
        site: &PredicateSite,
    ) -> Result<(), PlaySpecError> {
        let params: &[(&'static str, f32)] = match predicate {
            PlayPredicate::CarrierPossed
            | PlayPredicate::HalfcourtPossession
            | PlayPredicate::ScreenEstablished
            | PlayPredicate::HelpShadingOff
            | PlayPredicate::WeakSideVacated
            | PlayPredicate::CornerOccupied { .. } => &[],
            PlayPredicate::ShotClockUrgent { threshold_s } => &[("threshold_s", *threshold_s)],
            PlayPredicate::BeyondCircleFt { r } => &[("r", *r)],
        };
        for &(field, value) in params {
            if !value.is_finite() || value < 0.0 {
                return Err(match site {
                    PredicateSite::Trigger(trigger_index) => {
                        PlaySpecError::InvalidTriggerPredicate {
                            id: self.id.clone(),
                            trigger_index: *trigger_index,
                            predicate: predicate_name(predicate),
                            field,
                            value,
                        }
                    }
                    PredicateSite::Rule(rule_id) => PlaySpecError::InvalidRulePredicate {
                        id: self.id.clone(),
                        rule_id: rule_id.clone(),
                        predicate: predicate_name(predicate),
                        field,
                        value,
                    },
                });
            }
        }
        Ok(())
    }
}

impl DecisionActionFamily {
    /// 全部候选族（合法出路不变量的全集）。
    pub const ALL: [DecisionActionFamily; 6] = [
        Self::Shoot,
        Self::Drive,
        Self::PostUp,
        Self::Pass,
        Self::TripleThreatJab,
        Self::Dwell,
    ];
}
