use crate::action_window::{ActionPhase, ActionType};
use crate::flow::PhaseType;
use crate::Possession;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimedGameEvent {
    pub time: f32,
    pub sequence: u64,
    pub event: GameEvent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GameEvent {
    Contact {
        player_a: String,
        player_b: String,
        impact_speed: f32,
        contact_normal: (f32, f32),
        is_screen: bool,
        semantic_kind: String,
        semantic_severity: String,
        possessor_id: Option<String>,
        legal_position: bool,
    },
    /// Ball trajectory intersects defender contest/reach cylinder during flight
    PathIntersection {
        ball_pos: (f32, f32, f32),
        defender_id: String,
        clearance_dist: f32,
        flight_time: f32,
    },
    /// Ball arrives at hoop cylinder
    HoopArrival {
        shooter_id: String,
        shot_origin: (f32, f32),
        is_made: bool,
        is_three: bool,
        contest_intensity: f32,
    },
    /// 防守方在出手飞行中触及球并将其终止（封盖）。
    ///
    /// ## 为何是独立事实（而非 HoopArrival 的一个布尔）
    ///
    /// 封盖与“投失”是两种不同的比赛事件：封盖把球打向别处（后续是松球
    /// 争夺），投失则是球触筐弹出（后续是篮板）。同时封盖的**责任主体是
    /// 防守人**，而投失没有防守责任方——归因账本需要能区分。
    ///
    /// `phase` 记录封盖发生时出手者的动作阶段：按 `architecture.md` §6
    /// 与 `action_window.rs`，合法的封盖窗口只在 `Execution`（起跳上升与
    /// 出手瞬间），球出手后的“封盖”是不合法事实，由不变量层报错。
    BlockedShot {
        shooter_id: String,
        blocker_id: String,
        /// 封盖发生位置的球坐标。
        ball_pos: (f32, f32, f32),
        /// 封盖者触及球的高度（决定后续松球的下坠起点）。
        contact_height_ft: f32,
        /// 出手被终止时，出手者所处的动作阶段。
        phase: ActionPhase,
        /// 该次出手是否原本会命中（封盖前的裁定）。
        would_have_made: bool,
        is_three: bool,
    },
    /// Player crosses boundary line
    BoundaryCross {
        player_id: String,
        pos: (f32, f32),
        boundary_name: String,
    },
    /// Action time-window phase transition
    WindowTransition {
        player_id: String,
        action_type: ActionType,
        new_phase: ActionPhase,
    },
    /// Pass release and arrival are explicit facts, not direct state mutation.
    PassRelease {
        passer_id: String,
        receiver_id: String,
        from_pos: (f32, f32),
        to_pos: (f32, f32),
    },
    PassReceived {
        receiver_id: String,
        /// Frozen physical position where the receiver secured the pass.
        position: (f32, f32),
    },
    /// Pass arrives without being secured by the receiver.
    PassDropped {
        passer_id: String,
        receiver_id: String,
        position: (f32, f32),
    },
    /// A defender touched the pass without securing it.
    PassTipped {
        passer_id: String,
        receiver_id: String,
        defender_id: String,
        position: (f32, f32),
    },
    /// A defender poked the ball loose from the on-ball handler (on-ball strip).
    ///
    /// 这是「带球丢球」（real NBA 失误占比最大的一类，53.6%）的事实源。
    BallPokedLoose {
        handler_id: String,
        defender_id: String,
        position: (f32, f32),
    },
    /// A defender secured the ball during a pass flight.
    PassIntercepted {
        passer_id: String,
        receiver_id: String,
        defender_id: String,
        position: (f32, f32),
    },
    /// A ball handler starts a semantically resolved drive toward a target.
    DriveInitiated {
        driver_id: String,
        from_pos: (f32, f32),
        target_pos: (f32, f32),
    },
    /// A drive reaches its resolution boundary and records the outcome.
    DriveOutcome {
        driver_id: String,
        successful: bool,
        finish_made: bool,
    },
    /// A loose ball was secured after a failed pass or tip.
    LooseBallSecured {
        player_id: String,
        position: (f32, f32),
    },
    /// A free-throw attempt is resolved by the rules layer.
    FreeThrowAttempt {
        shooter_id: String,
        attempt: u8,
        made: bool,
    },
    /// Shot release fact; the semantic result is resolved before flight.
    ShotRelease {
        shooter_id: String,
        pos: (f32, f32),
        is_three: bool,
        contest_level: f32,
        make_probability: f32,
    },
    /// Rebound contest trigger
    ReboundContest {
        rebounder_id: String,
        landing_pos: (f32, f32),
        is_offensive: bool,
    },
    /// Macro phase transition observed by the application event loop.
    PhaseTransition { from: PhaseType, to: PhaseType },
    /// Officiating result derived from a physical contact fact.
    Foul {
        fouled_player_id: String,
        fouler_id: String,
        is_shooting: bool,
    },
    /// A semantic rule was violated after constraint evaluation.
    RuleViolation {
        constraint_id: String,
        reason: String,
    },
    /// 球离开场地（第一性原理：出界是**球的位置事实**，不是球员违例）。
    ///
    /// 与 `BoundaryCross` 严格区分：后者语义是「**球员**越过边界」，由约束层
    /// 判定球员违例；本事件只陈述球已出界，并携带最后触球方与责任球员，
    /// 供因果账本把「出界导致的失误」与「违例导致的失误」区分开。
    BallOutOfBounds {
        position: [f32; 2],
        last_touch_team: String,
        responsible_player_id: Option<String>,
    },
    /// 传球接球点修正事实（层 A，P-1 有限信息）。
    ///
    /// ## 为什么需要这个事实
    ///
    /// 传球人在 release 时冻结 `to_pos`（他的**意图**）。但接球人只能按
    /// **自己的估计**跑位（P-1），因此接球成功时球的到达位置可能与 `to_pos`
    /// 不同。这一差异是**设计内的**（大个策应传提前量，小个可能预估不同），
    /// 不是缺陷。
    ///
    /// 但事实账本必须自洽：`PassReceived.position` 携带了与冻结点不同的位置，
    /// 消费方（评判器）会看到"事件声明的位置与它引用的冻结事实不可调和"。
    /// 因此引擎必须**显式发布修正**，把"意图"与"实际"的差异登记为事实，
    /// 而不是让下游去猜（gap.md §9.5：禁止三处各自解释同一传球）。
    PassLandingCorrected {
        receiver_id: String,
        /// 传球人冻结的意图接球点。
        intended: (f32, f32),
        /// 接球人的实际到达位置。
        actual: (f32, f32),
        /// 两者距离（ft）——即本回合的预估误差量。
        divergence_ft: f32,
    },
    /// A normalized enforcement intent has been applied by the application layer.
    EnforcementApplied {
        constraint_id: String,
        action: String,
    },
    /// 回合结束时的完整语义与因果图总结（用于 L2 叙事一致性校验）。
    PossessionSummary(PossessionSummary),
    /// 换人事实（tactics.md §2.3.3 / gap.md G6）：`reason` 区分强制
    /// （犯满）与主动（枯竭、轮休）轮换，评判器用它核算轮换与出场时间。
    Substitution {
        team: String,
        out_player: String,
        in_player: String,
        reason: crate::data::SubstitutionReason,
    },
    /// 双方倒地争抢地板球导致争球（Held Ball / Jump Ball）。
    JumpBallTriggered {
        player_a_id: String,
        player_b_id: String,
    },
    /// 离散位置重置（gap.md 第四节第三小节）：发球人从界外 placement 到场内合法位置。
    /// 这是显式生命周期事实，不属于普通运动，检查器据此豁免瞬移判定。
    PlacementApplied {
        player_id: String,
        from: (f32, f32),
        to: (f32, f32),
        reason: String,
        phase: String,
    },
    /// 选板器激活一个 Play（tactics.md §2.2.4）：`possession` 标注激活方。
    /// 这是可观测性事实：决策 trace 与帧投影的 `active_play_id` 与本事件
    /// 同 tick 对应，消费方（UI/评判器）据此重建 Play 的激活/冷却时间线。
    PlayActivated {
        play_id: String,
        possession: Possession,
    },
}

/// 回合终结的显式归因（dev 方案 §3.2 D0.1）。
///
/// 每个回合结束必须携带一个终结原因；**没有兜底变体**——历史
/// `UNATTRIBUTED_END` 被物理删除，任何到达回合边界却拿不出原因的
/// 路径在编译期就写不出这条总结。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PossessionEndCause {
    /// 得分（投篮命中或罚球终结）。
    Score,
    /// 防守篮板终结回合（含篮板后的控制转移）。
    DefensiveRebound,
    /// 传球/运球被直接抢断。
    TurnoverSteal,
    /// 传球被点掉后由对方控制松球。
    TurnoverPassTipped,
    /// 传球掉球（未被点掉）后由对方控制。
    TurnoverPassDropped,
    /// 松球易主（其他无法细分到上述三类的松球转换）。
    TurnoverLooseBall,
    /// 违例（24 秒/8 秒/回场/出界等）。
    TurnoverViolation,
    /// 节末/终场导致的回合终结。
    PeriodEnd,
}

impl PossessionEndCause {
    pub fn as_str(&self) -> &'static str {
        match self {
            PossessionEndCause::Score => "SCORE",
            PossessionEndCause::DefensiveRebound => "DEFENSIVE_REBOUND",
            PossessionEndCause::TurnoverSteal => "TURNOVER_STEAL",
            PossessionEndCause::TurnoverPassTipped => "TURNOVER_PASS_TIPPED",
            PossessionEndCause::TurnoverPassDropped => "TURNOVER_PASS_DROPPED",
            PossessionEndCause::TurnoverLooseBall => "TURNOVER_LOOSE_BALL",
            PossessionEndCause::TurnoverViolation => "TURNOVER_VIOLATION",
            PossessionEndCause::PeriodEnd => "PERIOD_END",
        }
    }

    pub fn is_turnover(&self) -> bool {
        self.as_str().starts_with("TURNOVER")
    }
}

impl std::fmt::Display for PossessionEndCause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// 回合完整因果语义总结。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PossessionSummary {
    pub possession_index: u64,
    pub offense_team: String,
    pub start_clock: f32,
    pub end_clock: f32,
    pub duration_seconds: f32,
    pub passes_count: u32,
    /// 终结原因（向后兼容：旧流中的字符串经 serde 映射到本枚举；
    /// 历史 `UNATTRIBUTED_END` 不再是合法值，解析旧流遇到它将报错——
    /// 严格解析是设计特性，见 dev 方案 §4 D1.3）。
    pub terminal_event: PossessionEndCause,
    #[serde(default)]
    pub shooter_id: Option<String>,
    #[serde(default)]
    pub rebounder_id: Option<String>,
    #[serde(default)]
    pub turnover_player_id: Option<String>,
    #[serde(default)]
    pub shot_contest_intensity: Option<f32>,
    #[serde(default)]
    pub rebound_distance_ft: Option<f32>,
}

impl GameEvent {
    pub fn event_type_str(&self) -> &'static str {
        match self {
            GameEvent::Contact { is_screen, .. } => {
                if *is_screen {
                    "SCREEN_CONTACT"
                } else {
                    "CONTACT_BUMP"
                }
            }
            GameEvent::PathIntersection { .. } => "PASS_INTERCEPT_OPPORTUNITY",
            GameEvent::HoopArrival { is_made, .. } => {
                if *is_made {
                    "SCORE"
                } else {
                    "SHOT_MISS"
                }
            }
            GameEvent::BlockedShot { .. } => "BLOCKED_SHOT",
            GameEvent::BoundaryCross { .. } => "OUT_OF_BOUNDS",
            GameEvent::WindowTransition { .. } => "ACTION_WINDOW_SHIFT",
            GameEvent::PassRelease { .. } => "PASS",
            GameEvent::PassReceived { .. } => "PASS_RECEIVED",
            GameEvent::PassLandingCorrected { .. } => "PASS_LANDING_CORRECTED",
            GameEvent::PassDropped { .. } => "PASS_DROPPED",
            GameEvent::PassTipped { .. } => "PASS_TIPPED",
            GameEvent::BallPokedLoose { .. } => "BALL_POKED_LOOSE",
            GameEvent::PassIntercepted { .. } => "STEAL",
            GameEvent::DriveInitiated { .. } => "DRIVE_INITIATED",
            GameEvent::DriveOutcome {
                successful,
                finish_made,
                ..
            } => {
                if *finish_made {
                    "DRIVE_SCORE"
                } else if *successful {
                    "DRIVE_MISS"
                } else {
                    "DRIVE_STOPPED"
                }
            }
            GameEvent::LooseBallSecured { .. } => "LOOSE_BALL_SECURED",
            GameEvent::FreeThrowAttempt { .. } => "FREE_THROW",
            GameEvent::ShotRelease { .. } => "SHOT_RELEASE",
            GameEvent::ReboundContest { .. } => "REBOUND",
            GameEvent::PhaseTransition { .. } => "PHASE_TRANSITION",
            GameEvent::Foul { .. } => "FOUL",
            GameEvent::RuleViolation { .. } => "VIOLATION",
            GameEvent::BallOutOfBounds { .. } => "OUT_OF_BOUNDS",
            GameEvent::EnforcementApplied { .. } => "ENFORCEMENT_APPLIED",
            GameEvent::PossessionSummary(_) => "POSSESSION_SUMMARY",
            GameEvent::Substitution { .. } => "SUBSTITUTION",
            GameEvent::JumpBallTriggered { .. } => "JUMP_BALL_TRIGGERED",
            GameEvent::PlacementApplied { .. } => "PLACEMENT_APPLIED",
            GameEvent::PlayActivated { .. } => "PLAY_ACTIVATED",
        }
    }
}
