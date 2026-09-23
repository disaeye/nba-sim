//! Play 选板器（tactics.md §2.2.4 选板触发语义、ADR-019 裁定第 5 条）。
//!
//! 纯函数 `(playbook, context) -> Option<PlayActivation>`：每个决策 tick
//! 对在场 Play 库逐个评估触发谓词，全部为真的进入候选，经确定性打分后
//! 至多一个 Play 进入激活态。随机性（同分 tie-break）经 `rng` 参数注入，
//! 不接触任何全局熵源（TA4）；`PlayActivationBook` 是调用方跨 tick 持有
//! 的簿记状态，选板器自身无状态。
//!
//! 场输出谓词（`HelpShadingOff` / `WeakSideVacated`）消费的稳定值由调用
//! 方从 `StableFieldOutput`（场侧滞回通道输出，tactics.md §2.5）填入
//! [`PlaySelectionContext`]；本模块不接触原始 `threat_ratio` /
//! `void_ratio` 抖动量，未经滞回的场量不得进入开关型判定（ADR-019
//! 裁定第 4 条）。

use rand::Rng;
use std::collections::BTreeMap;

use nba_domain::play::{
    DecisionActionFamily, PlayCourtSide, PlayInhibitionMode, PlayPredicate, PlaySpec, PlayTrigger,
};

/// 选板上下文：调用方（引擎决策侧）组装的一 tick 评估快照。
///
/// 几何量由 `SpatialGeometry`（ADR-015 单一事实源）派生后填入；
/// `screen_established` 按档案的 `screen_detection_radius_ft` 判定；
/// 两个场输出稳定值必须先过场侧滞回通道再注入，本结构只接受稳定值。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlaySelectionContext {
    /// 本回合已进行 tick 数。
    pub possession_ticks: u32,
    /// 进攻时钟剩余秒数。
    pub shot_clock_seconds: f32,
    /// 引擎 tick 时长（秒）：`GameRules::tick_seconds`，窗口与冷却折算的
    /// 唯一频率事实源（charter C1）。
    pub tick_seconds: f32,
    /// 本队持球且评估对象是持球人。
    pub carrier_possed: bool,
    /// 前场阵地战（非快攻）。
    pub halfcourt: bool,
    /// 持球人距篮距离（英尺）。
    pub carrier_dist_to_hoop_ft: f32,
    /// 掩护已建立（持球人与站定掩护人距离小于 `screen_detection_radius_ft`）。
    pub screen_established: bool,
    /// 左底角有本队球员。
    pub corner_left_occupied: bool,
    /// 右底角有本队球员。
    pub corner_right_occupied: bool,
    /// 对位协防人被拉离持球人走廊的稳定值（`threat_ratio` 经滞回）。
    pub help_shading_off_stable: bool,
    /// 弱侧真空的稳定值（`void_ratio` 经滞回）。
    pub weak_side_vacated_stable: bool,
}

/// 一次选板的输出：进入激活态的 Play 及其窗口长度和冷却时长。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayActivation {
    /// 激活的档案标识（`PlaySpec::id`）。
    pub play_id: String,
    /// 激活窗口长度（tick），由所选触发声明的 `window_seconds` 折算。
    pub window_ticks: u32,
    /// 冷却时长（tick），由所选触发声明的 `cooldown_seconds` 折算。
    pub cooldown_ticks: u32,
    /// 激活档案的克隆；调用方跨 tick 读取偏好、抑制与规则。
    pub spec: PlaySpec,
}

/// 激活或冷却的剩余 tick 计数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayPhaseTicks {
    /// 激活剩余 tick；耗尽后转入冷却。
    Active { remaining: u32 },
    /// 冷却剩余 tick；归零后移出簿记。
    Cooldown { remaining: u32 },
}

/// 簿记相位：激活剩余窗口中，或冷却剩余时间中。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayBookPhase {
    Active,
    Cooldown,
}

/// 簿记读取项：单个 Play 的当前相位与剩余 tick。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayBookEntry {
    pub play_id: String,
    pub phase: PlayBookPhase,
    pub remaining_ticks: u32,
}

/// 逐 Play 的激活簿记状态（调用方跨 tick 持有；选板器自身无状态）。
///
/// 键按 `play_id` 升序遍历（`BTreeMap`，quality.md §5 确定性红线）：
/// 选板评估顺序与同分 tie-break 的 rng 抽取顺序都由该序决定，与档案
/// 载入顺序和运行环境无关，跨运行可复现。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlayActivationBook {
    states: BTreeMap<String, PlayPhaseTicks>,
}

impl PlayActivationBook {
    /// 创建空簿记。
    pub fn new() -> Self {
        Self::default()
    }

    /// 按 play_id 升序读取全部占用条目（激活 + 冷却）。
    pub fn entries(&self) -> Vec<PlayBookEntry> {
        self.states
            .iter()
            .map(|(play_id, phase)| PlayBookEntry {
                play_id: play_id.clone(),
                phase: match phase {
                    PlayPhaseTicks::Active { .. } => PlayBookPhase::Active,
                    PlayPhaseTicks::Cooldown { .. } => PlayBookPhase::Cooldown,
                },
                remaining_ticks: match phase {
                    PlayPhaseTicks::Active { remaining }
                    | PlayPhaseTicks::Cooldown { remaining } => *remaining,
                },
            })
            .collect()
    }

    /// 指定 play_id 是否处于激活或冷却任一占用态。
    fn is_occupied(&self, play_id: &str) -> bool {
        self.states.contains_key(play_id)
    }

    /// 写入一次激活：占用该 play_id 的激活窗口剩余 tick。
    ///
    /// 调用契约：写入前该 play_id 必须空闲（`select` 只对空闲 Play 评估
    /// 触发，天然满足）；重复写入在此就地崩溃（fast-fail）。
    fn record_activation(&mut self, play_id: &str, window_ticks: u32) {
        let previous = self.states.insert(
            play_id.to_owned(),
            PlayPhaseTicks::Active {
                remaining: window_ticks,
            },
        );
        assert!(
            previous.is_none(),
            "play `{play_id}` already occupied in activation book"
        );
    }

    /// 每 tick 推进簿记：激活剩余递减，耗尽后按 play_id 查
    /// `cooldowns_by_play_id` 得冷却时长并转入冷却；冷却剩余递减，归零
    /// 后移出簿记（该 Play 恢复可被评估）。转换与递减发生在同一次调用。
    ///
    /// `cooldowns_by_play_id` 以 play_id 为键（调用方从在场 Play 库的
    /// 触发声明折算）；激活条目查不到冷却时长时在此就地崩溃——簿记只
    /// 应包含在场库的 Play，缺席是调用方 bug，不静默回退（fast-fail）。
    pub fn tick(&mut self, cooldowns_by_play_id: &BTreeMap<String, u32>) {
        for (play_id, phase) in self.states.iter_mut() {
            *phase = match *phase {
                PlayPhaseTicks::Active { remaining } => {
                    debug_assert!(remaining > 0, "active entry must have positive ticks");
                    if remaining > 1 {
                        PlayPhaseTicks::Active {
                            remaining: remaining - 1,
                        }
                    } else {
                        let cooldown = cooldowns_by_play_id
                            .get(play_id)
                            .copied()
                            .unwrap_or_else(|| {
                                panic!(
                                    "play `{play_id}` active in book but absent from playbook cooldowns"
                                )
                            });
                        PlayPhaseTicks::Cooldown {
                            remaining: cooldown,
                        }
                    }
                }
                PlayPhaseTicks::Cooldown { remaining } => {
                    debug_assert!(remaining > 0, "cooldown entry must have positive ticks");
                    PlayPhaseTicks::Cooldown {
                        remaining: remaining - 1,
                    }
                }
            };
        }
        self.states.retain(|_, phase| match *phase {
            PlayPhaseTicks::Active { .. } => true,
            PlayPhaseTicks::Cooldown { remaining } => remaining > 0,
        });
    }

    /// 回合终结：立即结束全部激活并按 play_id 查表进入冷却；冷却为 0 的
    /// Play 不进入簿记（立即恢复可评估）。查不到冷却时长时就地崩溃。
    pub fn note_possession_end(&mut self, cooldowns_by_play_id: &BTreeMap<String, u32>) {
        let mut next: BTreeMap<String, PlayPhaseTicks> = BTreeMap::new();
        let old = std::mem::take(&mut self.states);
        for (play_id, phase) in old {
            match phase {
                PlayPhaseTicks::Active { .. } => {
                    let cooldown =
                        cooldowns_by_play_id
                            .get(&play_id)
                            .copied()
                            .unwrap_or_else(|| {
                                panic!(
                                    "play `{play_id}` active in book but absent from playbook cooldowns"
                                )
                            });
                    if cooldown > 0 {
                        next.insert(
                            play_id,
                            PlayPhaseTicks::Cooldown {
                                remaining: cooldown,
                            },
                        );
                    }
                }
                cooldown @ PlayPhaseTicks::Cooldown { .. } => {
                    next.insert(play_id, cooldown);
                }
            }
        }
        self.states = next;
    }
}

/// 逐谓词求值（tactics.md §2.2.4 谓词词汇表；封闭枚举穷尽匹配）。
///
/// 几何谓词的原始几何由调用方经 `SpatialGeometry` 派生后填入上下文；
/// 场输出谓词消费的是注入的滞回稳定值（tactics.md §2.5）。
pub fn eval_predicate(predicate: &PlayPredicate, ctx: &PlaySelectionContext) -> bool {
    match *predicate {
        // ---- 球态谓词 ----
        PlayPredicate::CarrierPossed => ctx.carrier_possed,
        PlayPredicate::HalfcourtPossession => ctx.halfcourt,
        PlayPredicate::ShotClockUrgent { threshold_s } => ctx.shot_clock_seconds < threshold_s,

        // ---- 几何谓词 ----
        PlayPredicate::BeyondCircleFt { r } => ctx.carrier_dist_to_hoop_ft > r,
        PlayPredicate::ScreenEstablished => ctx.screen_established,
        PlayPredicate::CornerOccupied { side } => match side {
            PlayCourtSide::Left => ctx.corner_left_occupied,
            PlayCourtSide::Right => ctx.corner_right_occupied,
        },

        // ---- 场输出谓词（稳定值端口） ----
        PlayPredicate::HelpShadingOff => ctx.help_shading_off_stable,
        PlayPredicate::WeakSideVacated => ctx.weak_side_vacated_stable,
    }
}

/// 一个 Play 的非空触发谓词全部为真才算触发。
fn eval_trigger(trigger: &PlayTrigger, ctx: &PlaySelectionContext) -> bool {
    assert!(
        !trigger.when.is_empty(),
        "play trigger `when` must not be empty"
    );
    trigger.when.iter().all(|p| eval_predicate(p, ctx))
}

/// hard 抑制的动作族集合（合法出路不变量由 domain 校验器拒绝违反者）。
fn hard_inhibited_families(spec: &PlaySpec) -> Vec<DecisionActionFamily> {
    let mut families = Vec::new();
    for inhibition in &spec.inhibitions {
        if matches!(inhibition.mode, PlayInhibitionMode::Hard)
            && !families.contains(&inhibition.action_family)
        {
            families.push(inhibition.action_family);
        }
    }
    families
}

/// 激活输出合法性校验（tactics.md §2.2.4：hard 抑制后候选集必须非空）。
///
/// domain 校验器已在档案层拒绝覆盖全部族的 hard 抑制集合；本断言是
/// 选板器出口的防御性 fast-fail。
fn assert_activation_leaves_viable_actions(
    spec: &PlaySpec,
    hard_families: &[DecisionActionFamily],
) {
    assert!(
        hard_families.len() < DecisionActionFamily::ALL.len(),
        "play `{}` hard-inhibits every action family",
        spec.id
    );
}

/// 纯函数选板器：评估在场 Play 库的触发谓词，至多选出一个激活。
///
/// - 过滤：簿记占用（激活中或冷却中）的 Play 不评估，已激活的不重复激活；
/// - 触发：[`eval_trigger`] 全真才进入候选；
/// - 每个 Play 取命中触发项中谓词数最多者；同分保留声明顺序首项；该项决定
///   激活窗口与冷却；不同 Play 的最高分再参与全局选板；
/// - 打分：谓词数量多者优先（更特异的触发优先），不同 Play 同分用 `rng`
///   均匀抽取；不引入与档案无关的隐藏权重；
/// - 输入：每份 Play 在参与选择前经过 schema 校验，非法档案立即失败；
/// - 评估顺序：book 的 play_id 升序（`BTreeMap` 序）。
///
/// 返回 `None` 表示本 tick 无激活。
pub fn select(
    playbook: &[PlaySpec],
    ctx: &PlaySelectionContext,
    book: &mut PlayActivationBook,
    rng: &mut impl Rng,
) -> Option<PlayActivation> {
    let mut candidates: Vec<(&PlaySpec, &PlayTrigger, u32)> = Vec::new();
    for spec in playbook {
        spec.validate().unwrap_or_else(|error| {
            panic!("play `{}` is invalid at selector input: {error}", spec.id)
        });
        if book.is_occupied(&spec.id) {
            continue;
        }

        // 每个 Play 仅保留分数最高的命中触发项；同分时不替换，保留声明顺序首项。
        let mut best_trigger: Option<(&PlayTrigger, u32)> = None;
        for trigger in &spec.triggers {
            if !eval_trigger(trigger, ctx) {
                continue;
            }
            let trigger_score = score(trigger.when.len());
            let replace = match best_trigger {
                None => true,
                Some((_, best_score)) => trigger_score > best_score,
            };
            if replace {
                best_trigger = Some((trigger, trigger_score));
            }
        }
        if let Some((trigger, trigger_score)) = best_trigger {
            candidates.push((spec, trigger, trigger_score));
        }
    }

    let winning_score = candidates
        .iter()
        .map(|(_, _, candidate_score)| *candidate_score)
        .max()?;
    let mut tied_candidates: Vec<_> = candidates
        .into_iter()
        .filter(|(_, _, candidate_score)| *candidate_score == winning_score)
        .collect();
    let chosen_index = if tied_candidates.len() == 1 {
        0
    } else {
        rng.gen_range(0..tied_candidates.len())
    };
    let (chosen, trigger, _) = tied_candidates.swap_remove(chosen_index);

    // 任一触发激活即写入簿记；簿记只保存活动 tick，冷却值随输出提供给调用方。
    let window_ticks = seconds_to_ticks(trigger.window_seconds, ctx.tick_seconds);
    let cooldown_ticks = seconds_to_ticks(trigger.cooldown_seconds, ctx.tick_seconds);
    let hard_families = hard_inhibited_families(chosen);
    assert_activation_leaves_viable_actions(chosen, &hard_families);
    book.record_activation(&chosen.id, window_ticks);

    Some(PlayActivation {
        play_id: chosen.id.clone(),
        window_ticks,
        cooldown_ticks,
        spec: chosen.clone(),
    })
}

/// 秒数折算 tick：`GameRules::tick_seconds` 是唯一频率事实源
/// （charter C1：行为相关折算走规则通道），向上取整（正秒数至少 1 tick）。
pub fn seconds_to_ticks(seconds: f32, tick_seconds: f32) -> u32 {
    debug_assert!(tick_seconds.is_finite() && tick_seconds > 0.0);
    ((seconds / tick_seconds).ceil()) as u32
}

/// 打分：触发谓词数量（更特异的触发优先）。
fn score(predicate_count: usize) -> u32 {
    predicate_count as u32
}
