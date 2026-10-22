//! 动作效用的可解释分解与选择规则。
//!
//! 每个候选动作都产出一个 [`Utility`]：期望得分（分项列出）、结果的离散度、
//! 消耗时钟的成本、犯规风险的成本、战术先验偏置。选择用玻尔兹曼分布
//! `P(a) ∝ exp(β·U(a))`，逆温度 β 由球商与体能决定。
//!
//! 分解项存在的理由不是「方便调试」，而是「没有它就无法判定一次出手是
//! 合理的强投还是权重配错」。任何一项为 NaN 或整体退化成都属于引擎缺陷，
//! 因此这里只做断言，不做回退。

use rand::Rng;

/// 期望得分的分项。总和就是要比较的期望得分。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct EpvTerms {
    /// 能力项：执行者的技术属性把动作做成的基本产出。
    pub ability: f32,
    /// 干扰项：主要防守人的距离、朝向与封盖威胁带来的期望变化。
    pub contest: f32,
    /// 通道项：传球线路或突破走廊上的拦截与协防威胁。
    pub lane: f32,
    /// 协防密度项：第二名及之后的防守人对这次动作的期望折损。
    pub help: f32,
    /// 倾向项：球员长期习惯造成的偏好，不含任何结果信息。
    pub tendency: f32,
    /// 语境项：接球就投、补篮、掩护后窗口这类时机加成。
    pub context: f32,
    /// 失球项：这次动作把球交出去的概率乘以其代价。
    pub turnover: f32,
}

impl EpvTerms {
    pub fn sum(&self) -> f32 {
        self.ability + self.contest + self.lane + self.help + self.tendency + self.context
            + self.turnover
    }
}

/// 一次候选动作的效用。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Utility {
    pub epv: EpvTerms,
    /// 结果的离散度：领先方厌恶，落后方偏好。
    pub variance: f32,
    /// 消耗进攻时钟的成本（负值表示紧迫度带来的加成）。
    pub clock_cost: f32,
    /// 犯规风险的成本：进攻犯规、送罚球。
    pub foul_cost: f32,
    /// 战术先验偏置：选板窗口给出的锚点奖励。
    pub tactical_bonus: f32,
}

impl Utility {
    pub fn epv_base(&self) -> f32 {
        self.epv.sum()
    }

    /// `U(s,a) = EPV_base + γ·σ − C_clock − C_foul + B_tactic`
    pub fn total(&self, gamma: f32) -> f32 {
        self.epv_base() + gamma * self.variance - self.clock_cost - self.foul_cost
            + self.tactical_bonus
    }
}

/// 玻尔兹曼权重：`exp(β·(U(a) − max U))`，返回未归一化的权重。
///
/// 减去最大值只为数值稳定：它同时乘在分子分母上，不改变分布。β ≤ 0
/// 会让分布退化成均分布或反向偏好，属于引擎缺陷，直接断言。
pub fn softmax_weights(totals: &[f32], beta: f32) -> Vec<f32> {
    assert!(!totals.is_empty(), "softmax needs at least one candidate");
    assert!(
        beta > f32::default(),
        "the inverse temperature must be positive, got {beta}"
    );
    assert!(
        totals.iter().all(|value| value.is_finite()),
        "a candidate utility is not finite: {totals:?}"
    );
    let peak = totals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    totals
        .iter()
        .map(|value| (beta * (value - peak)).exp())
        .collect()
}

/// 归一化后的选择概率。
pub fn softmax_probabilities(totals: &[f32], beta: f32) -> Vec<f32> {
    let weights = softmax_weights(totals, beta);
    let total: f32 = weights.iter().sum();
    assert!(
        total.is_finite() && total > f32::default(),
        "the softmax distribution degenerated to zero mass"
    );
    weights.iter().map(|weight| weight / total).collect()
}

/// 玻尔兹曼选择：`P(a) ∝ exp(β·(U(a) − max U))`。
pub fn softmax_pick<R: Rng>(rng: &mut R, totals: &[f32], beta: f32) -> usize {
    let weights = softmax_weights(totals, beta);
    let total: f32 = weights.iter().sum();
    assert!(
        total.is_finite() && total > f32::default(),
        "the softmax distribution degenerated to zero mass"
    );
    let mut threshold = rng.gen::<f32>() * total;
    for (index, weight) in weights.iter().enumerate() {
        threshold -= weight;
        if threshold <= f32::default() {
            return index;
        }
    }
    totals.len() - 1
}
