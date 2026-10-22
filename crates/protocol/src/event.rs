//! 事件侧的决策审计绑定。
//!
//! 每条动作事件都带着「是哪一次决策让它发生、当时还考虑过什么、每个候选
//! 的期望得分怎么算出来的」。没有这层绑定，事实流只能说明发生了什么，
//! 无法说明为什么。

use serde::{Deserialize, Serialize};

use crate::frame::DebugEpv;

/// 单个候选动作的审计记录。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceCandidate {
    /// 动作族标签，例如 `shoot`、`pass`、`drive`、`post_up`、`hold`。
    pub action: String,
    /// 传球或掩护的目标球员。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
    /// 带 γ 的总效用。
    pub total: f32,
    /// 玻尔兹曼选择概率（全部候选合计为 1）。
    pub probability: f32,
    /// 期望得分的七项分解。
    pub epv: DebugEpv,
    pub variance: f32,
    pub clock_cost: f32,
    pub foul_cost: f32,
    pub tactical_bonus: f32,
}

/// 一次决策的完整因果快照。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DecisionTrace {
    pub trace_id: u64,
    pub actor_id: String,
    /// 实际选中的动作族。
    pub chosen: String,
    /// 逆温度 β。
    pub beta: f32,
    /// 方差偏好 γ。
    pub gamma: f32,
    /// 按概率降序排列的候选集。
    pub candidates: Vec<TraceCandidate>,
}

impl DecisionTrace {
    /// 按概率降序的前 `count` 个候选。
    pub fn top(&self, count: usize) -> &[TraceCandidate] {
        &self.candidates[..count.min(self.candidates.len())]
    }

    /// 概率最小值与最大值之比；用于识别退化的均匀分布。
    ///
    /// 候选只有一个时比值定义为 1。
    pub fn probability_spread(&self) -> f32 {
        let Some(first) = self.candidates.first() else {
            return f32::from(1u8);
        };
        let mut low = first.probability;
        let mut high = first.probability;
        for candidate in &self.candidates {
            low = low.min(candidate.probability);
            high = high.max(candidate.probability);
        }
        if low <= f32::default() {
            return f32::INFINITY;
        }
        high / low
    }
}
