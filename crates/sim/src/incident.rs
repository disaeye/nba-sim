//! 原始事件（RawIncident）与罚则队列（PenaltyQueue）。
//!
//! 方案 §5.3 的推进链条是：原始事实 → 裁决批次 → 原子提交。原始事件是
//! 「发生了什么」，裁决批次是「按规则这意味着什么」。两层分开，才能让
//! 同一份原始事实在不同档案下得到不同的裁决。
//!
//! `IncidentKind` 是原始事件的类别：接触、边界、动作执行、时限到期。
//! 每种类别有自己的判定函数（`detect_*`），判定只读事实与账本，不改
//! 比赛状态。
//!
//! `PenaltyQueue` 是同一中断中的多个罚则的队列：一次中断可能同时产生
//! 多个罚则（例如技术犯规穿插在投篮犯规的罚球之间），队列给出它们的
//! 顺序、执行人、得分处理与球权恢复。

use nba_domain::event::FoulKind;
use nba_domain::identity::ProgramId;
use serde::{Deserialize, Serialize};

use crate::action::TeamSide;

/// 原始事件类别。
///
/// 这些是「发生了什么」，不是「这意味着什么」。裁决按类别读取事实并按
/// 档案给出后果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncidentKind {
    /// 球越过边界。
    OutOfBounds,
    /// 进攻方八秒未过半场。
    EightSecond,
    /// 已过中线又把球带回后场。
    Backcourt,
    /// 持球人被紧逼盯防超过时限。
    CloselyGuarded,
    /// 进攻人在限制区连续停留超过时限。
    OffensiveThreeSecond,
    /// 防守人在限制区连续停留超过时限。
    DefensiveThreeSecond,
    /// 进攻人背身超过时限。
    BackToBasket,
    /// 进攻钟走完。
    ShotClock,
    /// 持球人走步。
    Travel,
    /// 持球人二次运球。
    DoubleDribble,
    /// 发球人超过时限没有把球传入场内。
    InboundTimeout,
    /// 罚球人超过时限没有出手。
    FreeThrowTimeout,
    /// 罚球离手之前有球员提前进入限制区。
    FreeThrowLaneViolation,
    /// 掩护人没有落位就撞上防守人。
    IllegalScreen,
    /// 防守人用手或身体阻止对方。
    PersonalFoul,
    /// 防守人在投篮过程中碰到投篮人。
    ShootingFoul,
    /// 进攻人撞倒防守人。
    OffensiveFoul,
    /// 双方各记一次个人犯规。
    DoubleFoul,
    /// 技术犯规：责任主体是球队或教练。
    TechnicalFoul,
    /// 恶意犯规。
    FlagrantFoul,
    /// 清晰路径犯规。
    ClearPathFoul,
    /// 转换进攻中的故意犯规。
    TransitionTakeFoul,
    /// 球员受伤。
    Injury,
    /// 球队技术犯规：教练或替补席抗议判罚。
    ///
    /// 规则责任主体是球队或教练，不是场上球员。
    TeamTechnicalFoul,
}

impl IncidentKind {
    /// 类别名：事件流与档案用同一个名字。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OutOfBounds => "out_of_bounds",
            Self::EightSecond => "eight_second",
            Self::Backcourt => "backcourt",
            Self::CloselyGuarded => "closely_guarded",
            Self::OffensiveThreeSecond => "offensive_three_second",
            Self::DefensiveThreeSecond => "defensive_three_second",
            Self::BackToBasket => "back_to_basket",
            Self::ShotClock => "shot_clock",
            Self::Travel => "travel",
            Self::DoubleDribble => "double_dribble",
            Self::InboundTimeout => "inbound_timeout",
            Self::FreeThrowTimeout => "free_throw_timeout",
            Self::FreeThrowLaneViolation => "free_throw_lane_violation",
            Self::IllegalScreen => "illegal_screen",
            Self::PersonalFoul => "personal_foul",
            Self::ShootingFoul => "shooting_foul",
            Self::OffensiveFoul => "offensive_foul",
            Self::DoubleFoul => "double_foul",
            Self::TechnicalFoul => "technical_foul",
            Self::FlagrantFoul => "flagrant_foul",
            Self::ClearPathFoul => "clear_path_foul",
            Self::TransitionTakeFoul => "transition_take_foul",
            Self::Injury => "injury",
            Self::TeamTechnicalFoul => "team_technical_foul",
        }
    }

    /// 该类别对应的犯规种类；不是犯规的类别返回 `None`。
    ///
    /// 犯规种类与原始事件类别的对应由档案决定：FIBA 与 NBA 对同一行为
    /// 的归类不完全相同，因此这里给出默认对应，档案可以覆盖。
    pub fn foul_kind(self) -> Option<FoulKind> {
        match self {
            Self::ShootingFoul => Some(FoulKind::Shooting),
            Self::PersonalFoul => Some(FoulKind::Personal),
            Self::OffensiveFoul | Self::IllegalScreen => Some(FoulKind::Offensive),
            Self::DoubleFoul => Some(FoulKind::Double),
            Self::TechnicalFoul | Self::DefensiveThreeSecond => Some(FoulKind::Technical),
            Self::FlagrantFoul => Some(FoulKind::Flagrant),
            Self::ClearPathFoul | Self::TransitionTakeFoul => Some(FoulKind::LooseBall),
            _ => None,
        }
    }

    /// 该类别对应的违例种类；不是违例的类别返回 `None`。
    pub fn violation_kind(self) -> Option<crate::adjudicate::ViolationKind> {
        use crate::adjudicate::ViolationKind;
        match self {
            Self::OutOfBounds => Some(ViolationKind::OutOfBounds),
            Self::EightSecond => Some(ViolationKind::EightSecond),
            Self::Backcourt => Some(ViolationKind::Backcourt),
            Self::CloselyGuarded => Some(ViolationKind::CloselyGuarded),
            Self::OffensiveThreeSecond => Some(ViolationKind::OffensiveThreeSecond),
            Self::BackToBasket => Some(ViolationKind::BackToBasket),
            Self::ShotClock => Some(ViolationKind::ShotClock),
            Self::Travel => Some(ViolationKind::Travel),
            Self::DoubleDribble => Some(ViolationKind::DoubleDribble),
            Self::InboundTimeout => Some(ViolationKind::InboundTimeout),
            Self::FreeThrowTimeout => Some(ViolationKind::FreeThrowTimeout),
            Self::FreeThrowLaneViolation => None,
            _ => None,
        }
    }

    /// 该类别是否让比赛停表。
    ///
    /// 违例与犯规停表；伤病与技术犯规停表，因为它们都要换程序。
    pub fn stops_play(self) -> bool {
        match self {
            Self::Injury | Self::TeamTechnicalFoul | Self::DefensiveThreeSecond => true,
            _ => true,
        }
    }
}

/// 原始事件：发生了什么、在谁身上、依据什么。
///
/// 原始事件不带后果：罚球数、球权归属与恢复方式由裁决按档案给出。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawIncident {
    /// 事件类别。
    pub kind: IncidentKind,
    /// 事件发生的比赛时刻（秒）。
    pub at_seconds: f64,
    /// 主要责任方：犯事的球员。
    pub actor: Option<usize>,
    /// 次要责任方：被侵犯的球员。
    pub subject: Option<usize>,
    /// 事件所属队伍。
    pub team: Option<TeamSide>,
    /// 判定所依据的数值：距离、速度、时限读数。
    pub measurements: Vec<(String, f32)>,
    /// 触发这次判定的因果事件。
    pub source_event_ids: Vec<u64>,
    /// 事件所属比赛程序。
    pub program_id: Option<ProgramId>,
    /// 罚球次数：投篮犯规判几次罚球。
    pub free_throws: u8,
    /// 投篮是否打进：打进的投篮犯规是加罚。
    pub made: bool,
    /// 恶意犯规的等级：一级两罚，二级两罚加被逐。
    pub excessive: bool,
    /// 罚球禁区：进攻方提前进区。
    pub offense_early: bool,
    /// 罚球禁区：防守方提前进区。
    pub defense_early: bool,
    /// 出界：最后触球的是进攻方。
    pub offensive_touch: bool,
    /// 出界：球在投篮飞行中飞出界。
    pub shot_in_air: bool,
}

impl RawIncident {
    /// 新建一条原始事件。
    pub fn new(kind: IncidentKind, at_seconds: f64) -> Self {
        Self {
            kind,
            at_seconds,
            actor: None,
            subject: None,
            team: None,
            measurements: Vec::new(),
            source_event_ids: Vec::new(),
            program_id: None,
            free_throws: 0,
            made: false,
            excessive: false,
            offense_early: false,
            defense_early: false,
            offensive_touch: false,
            shot_in_air: false,
        }
    }

    /// 记下责任方与受影响方。
    pub fn with_parties(mut self, actor: Option<usize>, subject: Option<usize>) -> Self {
        self.actor = actor;
        self.subject = subject;
        self
    }

    /// 记下所属队伍。
    pub fn with_team(mut self, team: TeamSide) -> Self {
        self.team = Some(team);
        self
    }

    /// 记下一项判定依据。
    pub fn with_measurement(mut self, name: &str, value: f32) -> Self {
        self.measurements.push((name.to_string(), value));
        self
    }

    /// 记下因果事件。
    pub fn with_sources(mut self, sources: Vec<u64>) -> Self {
        self.source_event_ids = sources;
        self
    }

    /// 记下事件所属比赛程序。
    pub fn with_program(mut self, program: ProgramId) -> Self {
        self.program_id = Some(program);
        self
    }

    /// 记下罚球次数与投篮是否打进。
    pub fn with_free_throws(mut self, free_throws: u8, made: bool) -> Self {
        self.free_throws = free_throws;
        self.made = made;
        self
    }

    /// 记下恶意犯规的等级。
    pub fn with_excessive(mut self, excessive: bool) -> Self {
        self.excessive = excessive;
        self
    }

    /// 记下罚球禁区：哪一方提前进区。
    pub fn with_lane_early(mut self, offense_early: bool, defense_early: bool) -> Self {
        self.offense_early = offense_early;
        self.defense_early = defense_early;
        self
    }

    /// 记下出界的两条判定：最后触球的一方与球是否在投篮飞行中。
    pub fn with_out_of_bounds(mut self, offensive_touch: bool, shot_in_air: bool) -> Self {
        self.offensive_touch = offensive_touch;
        self.shot_in_air = shot_in_air;
        self
    }
}

/// 罚则队列里的一项。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PenaltyEntry {
    /// 产生这条罚则的犯规身份。
    pub foul_id: u64,
    /// 罚则的犯规种类。
    pub kind: FoulKind,
    /// 罚球次数。
    pub free_throws: u8,
    /// 罚球人。
    pub shooter: Option<usize>,
    /// 罚球之后是否保留球权。
    pub retain_possession: bool,
    /// 罚球结束后的恢复方式：继续罚球还是交还给罚则队列的下一项。
    pub after: PenaltyAfter,
}

/// 一条罚则执行完之后做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PenaltyAfter {
    /// 罚球结束，回到队列的下一项。
    NextInQueue,
    /// 罚球结束，按原罚则计划恢复比赛。
    ResumeGame,
    /// 罚球结束，由指定队伍发球。
    Inbound { team: TeamSide },
}

/// 罚则队列：同一中断中的多个罚则按规则顺序执行。
///
/// 队列是普通次序：投篮犯规的罚球先执行，技术犯规的罚球随后穿插进来。
/// 「穿插」由 `PenaltyAfter::NextInQueue` 与 `ResumeGame` 表达。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PenaltyQueue {
    entries: Vec<PenaltyEntry>,
}

impl PenaltyQueue {
    /// 空队列。
    pub fn new() -> Self {
        Self::default()
    }

    /// 队列长度。
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// 队列是否为空。
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// 加入一条罚则。
    pub fn push(&mut self, entry: PenaltyEntry) {
        self.entries.push(entry);
    }

    /// 队首的罚则。
    pub fn first(&self) -> Option<&PenaltyEntry> {
        self.entries.first()
    }

    /// 取出队首的罚则。
    pub fn pop(&mut self) -> Option<PenaltyEntry> {
        if self.entries.is_empty() {
            None
        } else {
            Some(self.entries.remove(0))
        }
    }

    /// 队列里的全部罚则。
    pub fn entries(&self) -> &[PenaltyEntry] {
        &self.entries
    }

    /// 队列里剩余的罚球总数。
    pub fn remaining_free_throws(&self) -> u8 {
        self.entries.iter().map(|entry| entry.free_throws).sum()
    }
}

/// 一次裁决批次：同一时刻的全部裁决结果。
///
/// 批次是一次原子提交的全部内容：球状态、控制、程序、时钟、罚则与事件
/// 一起落下，中途不留半套状态。
#[derive(Debug, Clone, PartialEq)]
pub struct AdjudicationBatch {
    /// 触发这次裁决的原始事件。
    pub incidents: Vec<RawIncident>,
    /// 罚则队列。
    pub penalties: PenaltyQueue,
    /// 这次裁决引用的因果事件。
    pub source_event_ids: Vec<u64>,
    /// 裁决发生的比赛时刻。
    pub at_seconds: f64,
}

impl AdjudicationBatch {
    /// 新建一个裁决批次。
    pub fn new(at_seconds: f64) -> Self {
        Self {
            incidents: Vec::new(),
            penalties: PenaltyQueue::new(),
            source_event_ids: Vec::new(),
            at_seconds,
        }
    }

    /// 加入一条原始事件。
    pub fn with_incident(mut self, incident: RawIncident) -> Self {
        self.source_event_ids
            .extend(incident.source_event_ids.iter().copied());
        self.incidents.push(incident);
        self
    }

    /// 加入一条罚则。
    pub fn with_penalty(mut self, entry: PenaltyEntry) -> Self {
        self.penalties.push(entry);
        self
    }
}
