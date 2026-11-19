//! 录像复核程序（ReviewProgram）。
//!
//! 方案 §8.6：录像复核与挑战使用版本化适用范围、时限、资源与恢复程序。
//! 保存物理证据和初次裁决，复核只能依规定读取证据并发布确认或更正。后续
//! 统计引用更正关系，不能直接删除原事件。
//!
//! `ReviewScope` 是类型化的复核问题：每个问题有它的适用范围、它读的证据、
//! 它能不能被推翻与推翻后改什么。档案的 `review.review_scope` 列出这个联赛
//! 承认哪些问题，因此换档案就是换一组可复核的问题。
//!
//! 复核程序是一个队列。判定层把每次裁决连同它当时读到的证据冻结下来登记；
//! 主循环在停表期间处理队列：教练在档案允许的窗口内申请挑战。裁决在停表
//! 结束时没有结论就作废：球已经打起来，原判就是既成事实。
//!
//! 判罚观察误差是单独参数化的模型量：裁判把一次接触、一条三分线或者一次
//! 犯规认定成什么，本身带一点误差。复核读的是冻结的事实，因此它在认定出错的
//! 时候把后果纠正过来。纯读取权威记录得出的判定（最后触球、时钟读数）没有
//! 认定误差：它们不需要复核就能成立。

use crate::action::{Fact, TeamSide};
use crate::draw::{purpose, Stream};
use crate::state::Phase;
use crate::world::World;

/// 类型化的复核问题。
///
/// 这些是「可以复核什么问题」。裁判当时吹了什么裁决由事件流记录，两张表
/// 因此解耦：档案只写复核的问题，事件流只写裁判的判定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum ReviewScope {
    /// 出界归属：球权给谁。
    OutOfBoundsAssignment,
    /// 干扰球：进球是否被取消。
    Goaltending,
    /// 篮筐干扰：防守人是否碰到了球。
    BasketInterference,
    /// 最后触球：出界与球权归属靠它确定。
    LastTouch,
    /// 恶意犯规升级。
    FlagrantUpgrade,
    /// 不体育道德犯规升级（FIBA）。
    UnsportsmanlikeUpgrade,
    /// 取消比赛资格升级（FIBA）。
    DisqualifyingUpgrade,
    /// 二分与三分：进球算几分。
    TwoVersusThreePoints,
    /// 节末比赛钟：出手在钟响前还是钟响后。
    GameClockAtPeriodEnd,
    /// 节末进攻钟：出手在钟响前还是钟响后。
    ShotClockAtPeriodEnd,
    /// 时钟故障。
    ClockMalfunction,
}

impl ReviewScope {
    /// 档案里的复核范围键。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OutOfBoundsAssignment => "out_of_bounds_assignment",
            Self::Goaltending => "goaltending",
            Self::BasketInterference => "basket_interference",
            Self::LastTouch => "last_touch",
            Self::FlagrantUpgrade => "flagrant_upgrade",
            Self::UnsportsmanlikeUpgrade => "unsportsmanlike_upgrade",
            Self::DisqualifyingUpgrade => "disqualifying_upgrade",
            Self::TwoVersusThreePoints => "two_versus_three_points",
            Self::GameClockAtPeriodEnd => "game_clock_at_period_end",
            Self::ShotClockAtPeriodEnd => "shot_clock_at_period_end",
            Self::ClockMalfunction => "clock_malfunction",
        }
    }

    /// 从档案的复核范围键读回复核问题。
    ///
    /// 档案里没有登记的问题返回 `None`：没登记的问题不可复核。
    pub fn from_profile_key(key: &str) -> Option<Self> {
        Some(match key {
            "out_of_bounds_assignment" => Self::OutOfBoundsAssignment,
            "goaltending" => Self::Goaltending,
            "basket_interference" => Self::BasketInterference,
            "last_touch" => Self::LastTouch,
            "flagrant_upgrade" => Self::FlagrantUpgrade,
            "unsportsmanlike_upgrade" => Self::UnsportsmanlikeUpgrade,
            "disqualifying_upgrade" => Self::DisqualifyingUpgrade,
            "two_versus_three_points" => Self::TwoVersusThreePoints,
            "game_clock_at_period_end" => Self::GameClockAtPeriodEnd,
            "shot_clock_at_period_end" => Self::ShotClockAtPeriodEnd,
            "clock_malfunction" => Self::ClockMalfunction,
            _ => return None,
        })
    }

    /// 裁判当时吹的裁决名到复核问题的对应。
    ///
    /// 未知名称返回 `None`：没登记的裁决一律不可复核。
    pub fn from_call(call: &str) -> Option<Self> {
        Self::from_profile_key(call)
    }

    /// 这个复核问题读的是哪一类证据。
    ///
    /// 证据类型决定复核能确认什么：读物理接触的复核可以改判接触，读不到
    /// 的就只能维持原判。
    pub fn evidence_kind(self) -> ReviewEvidence {
        match self {
            Self::OutOfBoundsAssignment | Self::LastTouch => ReviewEvidence::LastTouch,
            Self::Goaltending | Self::BasketInterference => ReviewEvidence::ContactOnBall,
            Self::FlagrantUpgrade
            | Self::UnsportsmanlikeUpgrade
            | Self::DisqualifyingUpgrade => ReviewEvidence::ContactOnPlayer,
            Self::TwoVersusThreePoints => ReviewEvidence::ReleasePoint,
            Self::GameClockAtPeriodEnd | Self::ShotClockAtPeriodEnd => {
                ReviewEvidence::ReleaseTime
            }
            Self::ClockMalfunction => ReviewEvidence::ClockReading,
        }
    }

    /// 这个复核问题能不能被推翻。
    ///
    /// 读得到完整证据的问题可以改判；只读得到部分证据的只能维持原判并
    /// 确认记录。
    pub fn can_overturn(self) -> bool {
        !matches!(self.evidence_kind(), ReviewEvidence::ClockReading)
    }

    /// 这个问题是不是一次「认定」。
    ///
    /// 认定带观察误差：裁判可能把接触、三分线或者犯规认定错。纯读取权威
    /// 记录得出的判定不带误差，因此不需要复核。
    pub fn is_judgement(self) -> bool {
        !matches!(
            self.evidence_kind(),
            ReviewEvidence::LastTouch | ReviewEvidence::ClockReading
        )
    }
}

/// 复核读取的证据类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewEvidence {
    /// 最后触球：球在出界前被谁碰过。
    LastTouch,
    /// 球上发生的接触：封盖与干扰球。
    ContactOnBall,
    /// 球员身上的接触：恶意犯规与不体育道德犯规。
    ContactOnPlayer,
    /// 出手点：球离开投篮人的位置。
    ReleasePoint,
    /// 出手时刻：球离手的那一刻。
    ReleaseTime,
    /// 时钟读数：钟本身是否走停。
    ClockReading,
}

impl ReviewEvidence {
    /// 证据类别的名字。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LastTouch => "last_touch",
            Self::ContactOnBall => "contact_on_ball",
            Self::ContactOnPlayer => "contact_on_player",
            Self::ReleasePoint => "release_point",
            Self::ReleaseTime => "release_time",
            Self::ClockReading => "clock_reading",
        }
    }
}

/// 复核读到的一次观察。
///
/// 判定层在作出裁决时把观察冻结下来：复核只能读这份冻结记录，不能
/// 「找到新证据」。观察决定了结论能走到哪一步。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Observation {
    /// 最后触球的队伍。
    Touch(TeamSide),
    /// 球上或球员身上发生了接触。
    Contact,
    /// 出手点所属区域。
    Zone(#[serde(deserialize_with = "crate::checkpoint::label")] crate::checkpoint::Label),
    /// 出手时刻的比赛钟与进攻钟读数。
    ReleaseClock { game_clock: f64, shot_clock: f64 },
    /// 时钟读数。
    Clock(f64),
    /// 没有可读证据。
    None,
}

/// 复核推翻原判之后要改什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewCorrection {
    /// 球权翻给另一方。
    Possession,
    /// 进球分值改判：分值由出手结算自己落地，���核程序只登记关系。
    Points { delta: i32 },
    /// 犯规升级被推翻：被逐取消。
    FoulUpgrade,
}

/// 复核结论的依据。
///
/// 复核读两样东西：冻结的观察与权威记录。依据说明结论从哪一样得出。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewBasis {
    /// 一次认定：裁判的认定带观察误差，误差抽定后结论随之确定。
    Judgement { misjudged: bool },
    /// 一次权威记录：结论直接由记录给出。
    Record { upheld: bool },
}

/// 复核结论在什么时候落地。
///
/// 复核问题各有各的时刻：干扰球在球还在飞的时候就能看清，节末时钟在结算
/// 出手时就能读出来，出界的最后触球只能等教练在窗口内申请。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewTrigger {
    /// 登记处当场复核。
    Immediate,
    /// 这次出手结算时复核。
    AtShotSettlement,
    /// 等教练申请复核；比赛重新开始就作废。
    CoachChallenge,
}

/// 一条待复核的裁决。
///
/// 裁决保存最初的判罚、它的复核问题、它读到的证据与它的来源事件。复核
/// 程序只能读这些，不能「找到新证据」。
///
/// `scope` 为 `None` 表示这条裁决没有对应的复核问题：它仍然被登记，
/// 但不可复核——裁判吹了什么与档案承认哪些问题是两件事。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewableRuling {
    /// 复核问题；`None` 表示这条裁决不在任何复核问题下。
    pub scope: Option<ReviewScope>,
    /// 原判给了谁球权。
    pub possession: TeamSide,
    /// 裁判当时吹的裁决名。
    #[serde(deserialize_with = "crate::checkpoint::label")]
    pub call: crate::checkpoint::Label,
    /// 读到的证据：能不能推翻原判。
    pub evidence: ReviewEvidence,
    /// 判定层冻结的观察。
    pub observation: Observation,
    /// 结论的依据。
    pub basis: ReviewBasis,
    /// 结论在什么时候落地。
    pub trigger: ReviewTrigger,
    /// 存在可推翻原判的证据。
    pub overturnable: bool,
    /// 推翻之后要改什么。
    pub correction: ReviewCorrection,
    /// 责任球员：这次裁决关于谁。
    pub actor: Option<usize>,
    /// 来源事件身份：复核事实引用它。
    pub source_event_ids: Vec<u64>,
}

/// 复核结论。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ReviewConclusion {
    /// 维持原判。
    Upheld,
    /// 改判：原判的后果被纠正。
    Overturned,
}

/// 复核程序的状态。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ReviewState {
    pending: Vec<ReviewableRuling>,
    concluded: Vec<(ReviewableRuling, ReviewConclusion)>,
}

impl ReviewState {
    /// 队列与结论都是空的。
    pub fn is_idle(&self) -> bool {
        self.pending.is_empty() && self.concluded.is_empty()
    }

    /// 队首的待复核裁决。
    pub fn pending(&self) -> Option<&ReviewableRuling> {
        self.pending.first()
    }

    /// 全部待复核裁决。
    pub fn pending_all(&self) -> &[ReviewableRuling] {
        &self.pending
    }

    /// 已发布结论的裁决。
    pub fn concluded(&self) -> &[(ReviewableRuling, ReviewConclusion)] {
        &self.concluded
    }

    /// 最近一次结论。
    pub fn last_conclusion(&self) -> Option<&(ReviewableRuling, ReviewConclusion)> {
        self.concluded.last()
    }

    /// 登记一条待复核裁决。
    fn register(&mut self, ruling: ReviewableRuling) {
        self.pending.push(ruling);
    }

    /// 队列里的裁决已经发布结论：移出队列并记下结论。
    fn conclude(&mut self, ruling: &ReviewableRuling, conclusion: ReviewConclusion) {
        let index = self.pending.iter().position(|item| same_ruling(item, ruling));
        let Some(index) = index else {
            return;
        };
        let ruling = self.pending.remove(index);
        self.concluded.push((ruling, conclusion));
    }

    /// 队列里的裁决作废：移出队列，不发布结论。
    ///
    /// 作废的裁决就此结束：原判留在事件流里，没有结论可发。
    fn discard(&mut self, ruling: &ReviewableRuling) {
        let index = self.pending.iter().position(|item| same_ruling(item, ruling));
        let Some(index) = index else {
            return;
        };
        self.pending.remove(index);
    }
}

/// 两条裁决是不是同一条：来源事件身份相同就是同一条。
fn same_ruling(left: &ReviewableRuling, right: &ReviewableRuling) -> bool {
    left.call == right.call && left.source_event_ids == right.source_event_ids
}

/// 该裁决是否在档案的即时回放范围内，且裁判有权发起复核。
pub fn is_reviewable(world: &World, scope: ReviewScope) -> bool {
    world.rules.league.authorizes("replay_review") && world.rules.league.reviews(scope.as_str())
}

/// 犯规升级对应的复核裁决名。
///
/// NBA 只有恶意犯规一类复核问题；FIBA 把同类行为分成不体育道德犯规与
/// 取消比赛资格犯规，因此二级用后者。档案不承认任何一类时返回 `None`：
/// 没有可复核的问题就不登记裁决。
pub fn foul_upgrade_call(
    league: &nba_domain::league::LeagueProfile,
    excessive: bool,
) -> Option<&'static str> {
    if excessive && league.reviews("disqualifying_upgrade") {
        Some("disqualifying_upgrade")
    } else if league.reviews("flagrant_upgrade") {
        Some("flagrant_upgrade")
    } else if league.reviews("unsportsmanlike_upgrade") {
        Some("unsportsmanlike_upgrade")
    } else {
        None
    }
}

impl World {
    /// 登记一条可复核的裁决。发球类裁决在写阶段时调用。
    ///
    /// 裁决来源事件就是这次登记时的事件流末尾：复核结论引用它，
    /// 因此复核事实的 `source_event_ids` 指回原判。
    pub fn record_ruling(&mut self, possession: TeamSide, call: &'static str) {
        let address = self.last_event_id;
        let misjudged = self.judge_with_error(0, address);
        self.record_ruling_with_observation(
            possession,
            call,
            None,
            Observation::None,
            ReviewCorrection::Possession,
            ReviewBasis::Judgement { misjudged },
            ReviewTrigger::CoachChallenge,
            vec![address],
        );
    }

    /// 登记一条附带冻结观察的可复核裁决。
    ///
    /// 没有登记的裁决名不进入复核程序：复核问题必须是档案承认的那一个。
    /// 来源事件身份由判定层给出：复核结论引用它，因此复核事实能指回原判
    /// 依据的事实。
    ///
    /// 依据由判定层给出：认定类的裁决带观察误差，读取权威记录得出的裁决
    /// 不带。判定层因此是唯一抽误差的地方。
    pub fn record_ruling_with_observation(
        &mut self,
        possession: TeamSide,
        call: &'static str,
        actor: Option<usize>,
        observation: Observation,
        correction: ReviewCorrection,
        basis: ReviewBasis,
        trigger: ReviewTrigger,
        source_event_ids: Vec<u64>,
    ) {
        let scope = ReviewScope::from_call(call);
        let evidence = scope.map_or(ReviewEvidence::ClockReading, ReviewScope::evidence_kind);
        let overturnable = match basis {
            ReviewBasis::Judgement { misjudged } => {
                misjudged && scope.is_some_and(ReviewScope::can_overturn)
            }
            ReviewBasis::Record { upheld } => !upheld,
        };
        self.review_state.register(ReviewableRuling {
            scope,
            possession,
            call,
            evidence,
            observation,
            basis,
            trigger,
            overturnable,
            correction,
            actor,
            source_event_ids,
        });
    }

    /// 抽一次认定观察误差：这次认定可能与冻结的事实不一致。
    ///
    /// 抽样地址包含裁决的来源事件身份：同一 tick 里的不同裁决必须抽到
    /// 不同的值，同一条裁决必须抽到同一个值。
    pub fn judge_with_error(&self, entity: usize, address: u64) -> bool {
        self.draw_review_error(entity, address)
            < self.params.policy.w("referee_observation_error_rate")
    }

    /// 登记一条依据权威记录复核的裁决。
    ///
    /// 这类复核不读认定，只读记录：节末的出手时刻、时钟读数都属于这一类。
    /// 结论由记录直接给出，因此复核不需要观察误差。
                                pub fn record_record_ruling(
                                                                &mut self,
                                                                possession: TeamSide,
                                                                call: &'static str,
                                                                actor: Option<usize>,
                                                                observation: Observation,
                                                                upheld: bool,
                                                                correction: ReviewCorrection,
                                                                trigger: ReviewTrigger,
                                                                source_event_ids: Vec<u64>,
                                ) {
                                                                self.record_ruling_with_observation(
                                                                                                possession,
                                                                                                call,
                                                                                                actor,
                                                                                                observation,
                                                                                                correction,
                                                                                                ReviewBasis::Record { upheld },
                                                                                                trigger,
                                                                                                source_event_ids,
                                                                );
                                }

    /// 裁判认定观察误差的抽样值。
    ///
    /// 抽样地址包含裁决的来源事件身份：同一 tick 里的不同裁决必须抽到
    /// 不同的值，同一条裁决必须抽到同一个值。
    pub fn draw_review_error(&self, entity: usize, address: u64) -> f32 {
        let bits = self.draws.u64_at(
            crate::draw::DrawKey {
                stream: Stream::Decision,
                possession: 0,
                tick: 0,
                entity: entity as u32,
                purpose: purpose::REVIEW_OBSERVATION,
                ordinal: 0,
            },
            address,
        ) >> 40;
        bits as f32 / ((1u32 << 24) as f32)
    }

    /// 当前待复核的裁决。
    ///
    /// 已发布结论的裁决仍然可查：挑战额度用完之后再来一次挑战必须被
    /// 额度规则拦下，而不是因为「没有裁决可复核」被拦下——这两种拒绝
    /// 指向的问题不同。
    pub fn pending_ruling(&self) -> Option<&ReviewableRuling> {
        match self.review_state.pending() {
            Some(ruling) => Some(ruling),
            None => self.review_state.last_conclusion().map(|(ruling, _)| ruling),
        }
    }

    /// 复核程序的当前状态。
    pub fn review_state(&self) -> &ReviewState {
        &self.review_state
    }

    /// 复核队列里是否还有等待处理的裁决。
    pub fn review_pending(&self) -> bool {
        self.review_state.pending().is_some()
    }

    /// 复核可以在哪些阶段进行。
    ///
    /// 复核需要停表：死球、发球、罚球、暂停与节间休息都是停表时段。
    /// 节末结算期间比赛同样停着，结算要等复核结论落地。
    fn review_phase(world: &World) -> bool {
        if world.settlement.is_some() {
            return true;
        }
        matches!(
            world.phase(),
            Phase::DeadBall { .. }
                | Phase::Inbound { .. }
                | Phase::FreeThrows { .. }
                | Phase::Timeout { .. }
                | Phase::PeriodBreak { .. }
        )
    }

    /// 裁判发起即时回放。
    ///
    /// 三个条件同时成立才受理：裁判有权发起、这条裁决在档案的复核范围内、
    /// 复核问题在规则允许的时刻（停表期间）。
    pub fn start_instant_replay(&mut self) -> Result<ReviewConclusion, &'static str> {
        if !World::review_phase(self) {
            return Err("instant replay requires a stoppage");
        }
        let Some(ruling) = self.review_state.pending().cloned() else {
            return Err("no reviewable ruling on record");
        };
        if !self.rules.league.authorizes("replay_review") {
            return Err("officials are not authorized to request replay review");
        }
        let Some(scope) = ruling.scope else {
            return Err("ruling is outside the instant-replay scope");
        };
        if !self.rules.league.reviews(scope.as_str()) {
            return Err("ruling is outside the instant-replay scope");
        }
        let conclusion = self.conclude_by_observation(&ruling);
        self.publish_review(ruling, conclusion);
        Ok(conclusion)
    }

    /// 复核结论：裁判读冻结的观察，给出确认或更正。
    ///
    /// 认定带观察误差，误差在认定那一刻就抽定：复核读到的冻结事实与
    /// 当时的认定不一致时，原判被纠正。
    fn conclude_by_observation(&mut self, ruling: &ReviewableRuling) -> ReviewConclusion {
        match ruling.basis {
            ReviewBasis::Judgement { misjudged } => {
                if misjudged {
                    ReviewConclusion::Overturned
                } else {
                    ReviewConclusion::Upheld
                }
            }
            ReviewBasis::Record { upheld } => {
                if upheld {
                    ReviewConclusion::Upheld
                } else {
                    ReviewConclusion::Overturned
                }
            }
        }
    }

    /// 发布复核结论。
    ///
    /// 结论改判时后果被纠正：球权给另一方，进球的分值改判，犯规升级被
    /// 推翻。原判与结论都留在事件流里，统计引用更正关系，不删除原事件。
    pub fn publish_review(&mut self, ruling: ReviewableRuling, conclusion: ReviewConclusion) {
        let overturned = conclusion == ReviewConclusion::Overturned;
        if overturned {
            self.apply_correction(&ruling);
        }
        self.pending_review_source_events = Some(ruling.source_event_ids.clone());
        self.emit(Fact::Review {
            team: ruling.possession,
            call: ruling.call,
            overturned,
            cost_timeout: false,
            points_delta: if overturned {
                match ruling.correction {
                    ReviewCorrection::Points { delta } => delta,
                    _ => 0,
                }
            } else {
                0
            },
            player: ruling.actor,
            time: self.now(),
        });
        self.review_state.conclude(&ruling, conclusion);
    }

    /// 把改判的后果落到权威状态上。
    fn apply_correction(&mut self, ruling: &ReviewableRuling) {
        match ruling.correction {
            ReviewCorrection::Possession => {
                let new_possession = ruling.possession.other();
                match self.phase() {
                    Phase::DeadBall { ready_at, .. } => {
                        self.set_phase(Phase::DeadBall {
                            team: new_possession,
                            ready_at,
                        });
                    }
                    Phase::Inbound { ready_at, .. } => {
                        self.set_phase(Phase::Inbound {
                            team: new_possession,
                            ready_at,
                        });
                    }
                    _ => {}
                }
                self.possession.team = new_possession;
            }
            ReviewCorrection::Points { delta } => {
                // 分值改判在出手结算时就已经落地：出手事实带着改判后的分值。
                // 复核程序在这里只确认这条关系，不重复改分。
                let _ = delta;
            }
            ReviewCorrection::FoulUpgrade => {
                if let Some(actor) = ruling.actor {
                    self.players[actor].disqualified = false;
                    self.players[actor].retained_after_foul_limit = false;
                }
            }
        }
    }

    /// 发布指定复核问题的结论。
    ///
    /// 复核程序只对登记的那条裁决生效：找不到这一类问题就地失败，不让一
    /// 条结论落在别的裁决上。
    pub fn conclude_review(
        &mut self,
        scope: ReviewScope,
        conclusion: ReviewConclusion,
    ) -> Result<(), &'static str> {
        let Some(ruling) = self
            .review_state
            .pending_all()
            .iter()
            .rev()
            .find(|item| item.scope == Some(scope))
            .cloned()
        else {
            return Err("no reviewable ruling on record");
        };
        self.publish_review(ruling, conclusion);
        Ok(())
    }

    /// 就地发布一条复核结论。
    ///
    /// 当场复核的问题在登记的地方就能看清：裁判读冻结的证据，给出确认或
    /// 更正，结论立刻落地。
    pub fn conclude_immediate_review(&mut self, scope: ReviewScope) -> Result<(), &'static str> {
        let Some(ruling) = self
            .review_state
            .pending_all()
            .iter()
            .rev()
            .find(|item| item.scope == Some(scope))
            .cloned()
        else {
            return Err("no reviewable ruling on record");
        };
        let conclusion = self.conclude_by_observation(&ruling);
        self.publish_review(ruling, conclusion);
        Ok(())
    }

    /// 教练申请复核。
    ///
    /// 停表期间、联赛规定的复核窗口内、本方还有挑战额度、且这条裁决在
    /// 档案的复核范围内，四个条件同时成立才受理。结论维持原判时按档案
    /// 付出一次暂停，认定被纠正时改判并把球权翻给挑战方。
    pub fn request_challenge(&mut self, team: TeamSide) -> Result<ChallengeOutcome, &'static str> {
        if !matches!(self.phase(), Phase::DeadBall { .. } | Phase::Inbound { .. }) {
            return Err("challenge requires a stoppage");
        }
        // 额度先查：额度用完之后再来一次挑战必须被额度规则拦下，而不是因为
        // 「没有裁决可复核」被拦下——这两种拒绝指向的问题不同。
        let index = World::team_index(team);
        if self.challenges_left[index] == 0 {
            return Err("no challenge remaining");
        }
        let Some(ruling) = self.review_state.pending().cloned() else {
            return Err("no reviewable ruling on record");
        };
        let Some(scope) = ruling.scope else {
            return Err("ruling is outside the instant-replay scope");
        };
        if !is_reviewable(self, scope) {
            return Err("ruling is outside the instant-replay scope");
        }
        if ruling.possession == team {
            return Err("only the team that lost the ruling may challenge");
        }
        let window = self.rules.league.review.challenge_final_seconds;
        let final_period = self.clock.period() >= self.rules.league.periods.regulation_periods;
        // 复核窗口在末节最后两分钟：节次与剩余时间两个条件同时成立。
        let late = final_period && self.clock.game_clock() <= f64::from(window);
        if !late {
            return Err("challenge window closed");
        }
        self.challenges_left[index] -= 1;
        let conclusion = self.conclude_by_observation(&ruling);
        if conclusion == ReviewConclusion::Overturned {
            // 挑战成功：原判被推翻，不扣除暂停，球权逆转给挑战方。
            self.publish_review(ruling, conclusion);
            Ok(ChallengeOutcome::Overturned)
        } else {
            // 复核维持原判：扣除一次暂停。
            let cost_timeout = self.timeouts[index].remaining > 0;
            if cost_timeout {
                self.timeouts[index].remaining -= 1;
            }
            self.pending_review_source_events = Some(ruling.source_event_ids.clone());
            self.emit(Fact::Review {
                team,
                call: ruling.call,
                overturned: false,
                cost_timeout,
                points_delta: 0,
                player: ruling.actor,
                time: self.now(),
            });
            self.review_state.conclude(&ruling, ReviewConclusion::Upheld);
            Ok(ChallengeOutcome::Upheld)
        }
    }

    /// 复核窗口是否打开。
    ///
    /// 窗口在末节最后两分钟：节次与剩余时间两个条件同时成立。
    pub fn challenge_window_open(&self) -> bool {
        let window = self.rules.league.review.challenge_final_seconds;
        let final_period = self.clock.period() >= self.rules.league.periods.regulation_periods;
        final_period && self.clock.game_clock() <= f64::from(window)
    }

    /// 主循环的复核步骤。
    ///
    /// 三类复核各有各的落地时刻：当场复核的裁决在登记处就给出结论，结算
    /// 时复核的裁决等这次出手结算，等教练的裁决在档案允许的窗口内由教练
    /// 申请。球重新打起来之后还没结论的裁决就此作废：原判留在事件流里，
    /// 没有结论可发。
    pub fn review_step(&mut self) {
        if !World::review_phase(self) {
            // 球已经打起来：等教练申请的裁决作废，队列不会堵住后面的裁决。
            // 当场复核与结算时复核的裁决不在这里作废：它们各自在自己的时刻
            // 落地。
            let pending: Vec<ReviewableRuling> = self
                .review_state
                .pending_all()
                .iter()
                .filter(|ruling| ruling.trigger == ReviewTrigger::CoachChallenge)
                .cloned()
                .collect();
            for ruling in pending {
                self.review_state.discard(&ruling);
            }
            return;
        }
        if self.settlement.is_some() {
            // 结算期间复核结论必须落地：否则节末结算等不到结束。
            let pending: Vec<ReviewableRuling> = self.review_state.pending_all().to_vec();
            for ruling in pending {
                let conclusion = self.conclude_by_observation(&ruling);
                self.publish_review(ruling, conclusion);
            }
            return;
        }
        let Some(ruling) = self.review_state.pending().cloned() else {
            return;
        };
        if ruling.trigger != ReviewTrigger::CoachChallenge {
            return;
        }
        if !self.challenge_window_open() {
            return;
        }
        let challenger = ruling.possession.other();
        let index = World::team_index(challenger);
        if self.challenges_left[index] == 0 {
            return;
        }
        let _ = self.request_challenge(challenger);
    }
}

/// 挑战结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeOutcome {
    /// 挑战成立，原判被推翻。
    Overturned,
    /// 挑战失败，原判维持，挑战方损失一次暂停。
    Upheld,
}
