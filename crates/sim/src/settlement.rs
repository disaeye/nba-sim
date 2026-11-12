//! 节末结算：待决事项与终场冻结。
//!
//! 方案 §9.3：时钟归零后进入 `Finalizing`，停止新增普通进攻动作，保留需要
//! 结算的投篮、接触、罚则与复核。维护 `PendingAdjudications`：待决出手、
//! 信号前接触、罚球、得分有效性与规则纠正。每项有来源和关闭条件。
//!
//! 所有待决事项关闭后发布 `PeriodSettlementCompleted`，随后决定休息、下一
//! 节、加时或终场。终场发布后禁止普通比分、动作与球权继续变化。
//!
//! 待决事项的开启与关闭都由引擎状态决定：判定层把观察冻结成裁决，复核
//! 程序读冻结的观察给出结论，结算层只维护清单。清单因此不依赖事件到达
//! 顺序，也不依赖谁记得关闭哪一项。

use serde::{Deserialize, Serialize};

use crate::state::Phase;
use crate::world::World;

/// 待决事项的类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PendingKind {
    /// 待决出手：球在飞，结算还没给出结果。
    Shot,
    /// 信号前接触：吹哨之前发生的接触，判罚还没登记。
    ContactBeforeSignal,
    /// 待决罚球：罚球程序还没走完。
    FreeThrow,
    /// 得分有效性：进球是否计入还没有结论。
    ScoreValidity,
    /// 规则纠正：复核或更正还没有发布结论。
    RuleCorrection,
}

impl PendingKind {
    /// 类别名。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shot => "shot",
            Self::ContactBeforeSignal => "contact_before_signal",
            Self::FreeThrow => "free_throw",
            Self::ScoreValidity => "score_validity",
            Self::RuleCorrection => "rule_correction",
        }
    }
}

/// 关闭条件：这一项在什么事实成立时算关闭。
///
/// 条件是机器可判的：结算层据此判断清单是否已经清空，不依赖事件到达
/// 顺序，也不依赖调用方记得关掉哪一项。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloseCondition {
    /// 这次出手已经结算：结果确定，随它登记的判罚也已经落地。
    ShotSettled,
    /// 罚球程序已经走完：没有下一次罚球待发。
    FreeThrowProgramDone,
    /// 得分有效性已经有结论：出手结算完成，且相关复核已经发布结论。
    ScoreValidityConcluded,
    /// 复核已经发布结论。
    ReviewConcluded,
}

/// 一项待决事项。
///
/// 每项有来源事件与关闭条件：来源事件说明这一项依据哪些事实，关闭条件
/// 说明这一项在什么事实成立时算关闭。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingAdjudication {
    /// 事项类别。
    pub kind: PendingKind,
    /// 这一项指向的对象：出手的 shot_id、罚则的 foul_id、裁决的事件身份。
    pub subject: u64,
    /// 关闭条件。
    pub close_condition: CloseCondition,
    /// 来源事件身份：这一项依据哪些事实。
    pub source_event_ids: Vec<u64>,
}

impl PendingAdjudication {
    /// 新建一项待决事项。
    pub fn new(
        kind: PendingKind,
        subject: u64,
        close_condition: CloseCondition,
        source_event_ids: Vec<u64>,
    ) -> Self {
        Self {
            kind,
            subject,
            close_condition,
            source_event_ids,
        }
    }

    /// 关闭条件是否已经满足。
    pub fn is_satisfied(&self, world: &World) -> bool {
        match self.close_condition {
            CloseCondition::ShotSettled => !world.shot_in_flight(self.subject),
            CloseCondition::FreeThrowProgramDone => {
                !matches!(world.phase(), Phase::FreeThrows { .. })
            }
            CloseCondition::ScoreValidityConcluded => {
                !world.shot_in_flight(self.subject) && !world.review_pending()
            }
            CloseCondition::ReviewConcluded => !world.review_pending(),
        }
    }
}

/// 待决事项清单。
///
/// 清单按加入顺序保存，因此结算的顺序不依赖容器遍历顺序。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PendingAdjudications {
    items: Vec<PendingAdjudication>,
}

impl PendingAdjudications {
    /// 空清单。
    pub fn new() -> Self {
        Self::default()
    }

    /// 加入一项待决事项。
    pub fn push(&mut self, item: PendingAdjudication) {
        self.items.push(item);
    }

    /// 清单里是否已经有这一类、这一个对象的待决事项。
    pub fn has(&self, kind: PendingKind, subject: u64) -> bool {
        self.items
            .iter()
            .any(|item| item.kind == kind && item.subject == subject)
    }

    /// 清单里是否有这一类待决事项。
    pub fn has_kind(&self, kind: PendingKind) -> bool {
        self.items.iter().any(|item| item.kind == kind)
    }

    /// 清单是否为空。
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 清单长度。
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 清单里的全部事项。
    pub fn items(&self) -> &[PendingAdjudication] {
        &self.items
    }

    /// 关闭条件已经满足的事项：把它们从清单里移除并返回。
    pub fn close_satisfied(&mut self, world: &World) -> Vec<PendingAdjudication> {
        let mut closed = Vec::new();
        let mut remaining = Vec::with_capacity(self.items.len());
        for item in self.items.drain(..) {
            if item.is_satisfied(world) {
                closed.push(item);
            } else {
                remaining.push(item);
            }
        }
        self.items = remaining;
        closed
    }
}

/// 一次节末结算。
///
/// 结算记录本节关闭了多少待决出手与罚球：这两项随
/// `PeriodSettlementCompleted` 一起发布，事件流因此能重建节末结算的内容。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PeriodSettlement {
    /// 本次结算的节次。
    pub period: u32,
    /// 待决事项清单。
    pub pending: PendingAdjudications,
    /// 本节关闭的待决出手数。
    pub pending_shots_resolved: u32,
    /// 本节执行的罚球次数。
    pub free_throws_resolved: u32,
}

impl PeriodSettlement {
    /// 开始一次节末结算。
    pub fn new(period: u32) -> Self {
        Self {
            period,
            pending: PendingAdjudications::new(),
            pending_shots_resolved: 0,
            free_throws_resolved: 0,
        }
    }

    /// 与引擎状态同步：补上已经出现但还没登记的待决事项，关掉条件已满足
    /// 的事项。
    ///
    /// 同步只读引擎状态：它不推进任何程序，只描述「现在还有哪些事没关」。
    pub fn sync(&mut self, world: &World) {
        if let Some(shot) = world.pending_shot.as_ref() {
            let shot_id = shot.shot_id;
            let source = vec![shot.released_event_id];
            if !self.pending.has(PendingKind::Shot, shot_id) {
                self.pending.push(PendingAdjudication::new(
                    PendingKind::Shot,
                    shot_id,
                    CloseCondition::ShotSettled,
                    source.clone(),
                ));
            }
            if shot.fouled && !self.pending.has(PendingKind::ContactBeforeSignal, shot_id) {
                self.pending.push(PendingAdjudication::new(
                    PendingKind::ContactBeforeSignal,
                    shot_id,
                    CloseCondition::ShotSettled,
                    source.clone(),
                ));
            }
            if world.reviews_game_clock_at_period_end()
                && !self.pending.has(PendingKind::ScoreValidity, shot_id)
            {
                self.pending.push(PendingAdjudication::new(
                    PendingKind::ScoreValidity,
                    shot_id,
                    CloseCondition::ScoreValidityConcluded,
                    source,
                ));
            }
        }
        if let Phase::FreeThrows { foul_id, .. } = world.phase() {
            if !self.pending.has_kind(PendingKind::FreeThrow) {
                self.pending.push(PendingAdjudication::new(
                    PendingKind::FreeThrow,
                    u64::from(foul_id),
                    CloseCondition::FreeThrowProgramDone,
                    vec![world.last_event_id],
                ));
            }
        }
        if let Some(ruling) = world.pending_ruling() {
            let subject = ruling.source_event_ids.first().copied().unwrap_or_default();
            if !self.pending.has(PendingKind::RuleCorrection, subject) {
                self.pending.push(PendingAdjudication::new(
                    PendingKind::RuleCorrection,
                    subject,
                    CloseCondition::ReviewConcluded,
                    ruling.source_event_ids.clone(),
                ));
            }
        }
        for item in self.pending.close_satisfied(world) {
            match item.kind {
                PendingKind::Shot | PendingKind::ScoreValidity => {
                    self.pending_shots_resolved += 1;
                }
                PendingKind::FreeThrow => self.free_throws_resolved += 1,
                PendingKind::ContactBeforeSignal | PendingKind::RuleCorrection => {}
            }
        }
    }

    /// 待决事项是否已经全部关闭。
    pub fn is_settled(&self) -> bool {
        self.pending.is_empty()
    }
}
