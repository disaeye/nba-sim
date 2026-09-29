//! 比赛生命周期、阶段与球状态词汇。
//!
//! 这些类型只描述比赛世界的事实，不包含决策、物理或裁判实现。

use crate::possession::Possession;
use glam::Vec2;
use serde::{Deserialize, Serialize};

/// 比赛生命周期。它决定时钟和活球动作是否推进。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GameFlowState {
    Pregame,
    TipOff,
    LiveBall,
    DeadBall,
    Timeout,
    FreeThrow,
    QuarterEnd,
    Halftime,
    Overtime,
    GameEnd,
}

impl GameFlowState {
    /// 活球动作是否允许发生。
    pub fn allows_live_ball_actions(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    /// 比赛时钟只在实际活球期间推进；罚球和死球均停表。
    pub fn advances_game_clock(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    /// 进攻时钟只在活球期间推进。
    pub fn advances_shot_clock(self) -> bool {
        matches!(self, Self::LiveBall | Self::Overtime)
    }

    pub fn is_terminal(self) -> bool {
        self == Self::GameEnd
    }

    /// Current lifecycle states in which no live-ball action may execute.
    pub fn is_dead_ball(self) -> bool {
        matches!(
            self,
            Self::Pregame
                | Self::TipOff
                | Self::DeadBall
                | Self::Timeout
                | Self::FreeThrow
                | Self::QuarterEnd
                | Self::Halftime
                | Self::GameEnd
        )
    }

    pub fn allows_dead_ball_setup(self) -> bool {
        matches!(
            self,
            Self::DeadBall | Self::Timeout | Self::QuarterEnd | Self::Halftime
        )
    }

    /// Legal macro-lifecycle edges. Same-state assignment is idempotent.
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        matches!(
            (self, next),
            (Self::Pregame, Self::TipOff | Self::GameEnd)
                | (
                    Self::TipOff,
                    Self::LiveBall | Self::DeadBall | Self::FreeThrow | Self::GameEnd
                )
                | (
                    Self::LiveBall,
                    Self::DeadBall
                        | Self::Timeout
                        | Self::FreeThrow
                        | Self::QuarterEnd
                        | Self::Halftime
                        | Self::Overtime
                        | Self::GameEnd
                )
                | (
                    Self::DeadBall,
                    Self::LiveBall
                        | Self::Timeout
                        | Self::FreeThrow
                        | Self::QuarterEnd
                        | Self::Halftime
                        | Self::Overtime
                        | Self::GameEnd
                )
                | (
                    Self::Timeout,
                    Self::DeadBall | Self::LiveBall | Self::GameEnd
                )
                | (
                    Self::FreeThrow,
                    Self::LiveBall
                        | Self::DeadBall
                        | Self::QuarterEnd
                        | Self::Halftime
                        | Self::GameEnd,
                )
                | (
                    Self::QuarterEnd,
                    Self::Halftime
                        | Self::LiveBall
                        | Self::Overtime
                        | Self::DeadBall
                        | Self::GameEnd,
                )
                | (
                    Self::Halftime,
                    Self::DeadBall | Self::LiveBall | Self::GameEnd
                )
                | (
                    Self::Overtime,
                    Self::DeadBall | Self::Timeout | Self::QuarterEnd | Self::GameEnd,
                )
        )
    }
}

/// possession 内的阶段。阶段是约束激活和决策目标的边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhaseType {
    TipOff,
    Inbound,
    Transition,
    SetPlay,
    Resolution,
    Rebound,
    FreeThrow,
    Timeout,
    DeadBallReset,
}

impl PhaseType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::TipOff => "TIP_OFF",
            Self::Inbound => "INBOUND",
            Self::Transition => "TRANSITION",
            Self::SetPlay => "SET_PLAY",
            Self::Resolution => "RESOLUTION",
            Self::Rebound => "REBOUND",
            Self::FreeThrow => "FREE_THROW",
            Self::Timeout => "TIMEOUT",
            Self::DeadBallReset => "DEAD_BALL_RESET",
        }
    }
}

/// 球的归属状态 · 单一事实源（architecture 统一规范）。
///
/// 归属语义连同飞行/接球点参数一起住在领域层；physics 只把该状态当作
/// 采样参数载体（闭式采样位置是时间的纯函数），不再承载任何归属判断。
/// `has_ball` 标志、持球人、球权队全部由此派生（P1）。
///
/// 宏观态映射：Held/Drive = 持球族；ControlTransfer = 合球飞行；
/// InboundTransfer/InboundReady = 发球准备；Pass/Shot = 飞行族；
/// LooseBall/RimRebound = 松球族；Dead = 死球。
#[derive(Debug, Clone)]
pub enum BallState {
    /// 持球（突破中的持球亦是 Held，突破由 Drive 描述运动细节）。
    Held { carrier_id: String },
    /// 死球发球准备：球从原位飞向界外发球点。
    InboundTransfer {
        from_pos: Vec2,
        from_z: f32,
        baseline_pos: Vec2,
        inbounder_id: String,
        start_time: f32,
        duration: f32,
    },
    /// 死球发球准备完成：球停在界外发球点，等待发球人释放。
    InboundReady {
        baseline_pos: Vec2,
        inbounder_id: String,
    },
    /// 突破：球仍由突破者控制，语义结果由裁决层决定。
    Drive {
        driver_id: String,
        from_pos: Vec2,
        target_pos: Vec2,
        start_time: f32,
        duration: f32,
        move_kind: Option<crate::action_window::DribbleMoveKind>,
    },
    /// 控球交接 / 发球递交的短飞行。
    ControlTransfer {
        from_pos: Vec2,
        from_z: f32,
        /// Frozen receiving point captured when the transfer is created.
        target_pos: Vec2,
        target_z: f32,
        carrier_id: String,
        start_time: f32,
        duration: f32,
    },
    /// 传球飞行（含界外发球传球，inbound = true）。
    /// `from_z` 是释放时刻的球高：传球从球的实际高度出发上升到接球人
    /// 胸口，避免地板球直接跳到胸口高度的单帧位移。
    Pass {
        from_pos: Vec2,
        from_z: f32,
        to_pos: Vec2,
        target_id: String,
        start_time: f32,
        duration: f32,
        peak_z: f32,
        inbound: bool,
    },
    /// 投篮飞行。
    Shot {
        shooter_id: String,
        from_pos: Vec2,
        hoop_pos: Vec2,
        /// 实际瞄准点；释放时按命中概率输入抽取。
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        is_three: bool,
        peak_z: f32,
        /// 释放时刻的命中概率输入。命中在触筐结算时决定。
        make_probability: f32,
        /// 释放时刻可见的干扰强度。投篮犯规在出手窗口内的接触事实出现时发布。
        contest_intensity: f32,
    },
    /// 松球（传球脱手/篮板弹地）。`last_touch_team` 记录最后触球方
    /// （architecture：InFlight/Loose→最后触球队），是飞行期
    /// possession 派生的唯一依据。`last_touch_player` 是物理上的
    /// 最后触球人（可空）：与 `Dead` 的同名载荷同源，使「谁最后触球」
    /// 能从权威球态直接读出（P1）；由松球派生的停球继承该载荷后，
    /// 违例归因不再依赖旁路字段（`last_passer_id`）的存活期
    /// （seed 14 实测：拨掉的松球在节末转为停球后，旁路字段已清空，
    /// 归因链断在 None）。
    LooseBall {
        pos: Vec2,
        vel: Vec2,
        z: f32,
        vel_z: f32,
        last_touch_team: Possession,
        last_touch_player: Option<String>,
    },
    /// 罚球从当前球位移动到罚球线的准备过程。
    FreeThrowSetup {
        shooter_id: String,
        from_pos: Vec2,
        from_z: f32,
        to_pos: Vec2,
        to_z: f32,
        start_time: f32,
        duration: f32,
        forced_result: Option<bool>,
        is_final: bool,
    },
    /// 罚球飞向篮筐的飞行。不中后由实际到达位置生成篮板飞行。
    FreeThrow {
        shooter_id: String,
        from_pos: Vec2,
        hoop_pos: Vec2,
        aim_pos: Vec2,
        start_time: f32,
        duration: f32,
        peak_z: f32,
        is_final: bool,
    },
    /// 打铁触筐后的篮板飞行。`last_touch_team` 为出手方（触筐不改 Team control）。
    /// `last_touch_player` 为出手人，与 `LooseBall` 同一归因用途。
    RimRebound {
        from_pos: Vec2,
        from_z: f32,
        hoop_pos: Vec2,
        target_landing: Vec2,
        start_time: f32,
        duration: f32,
        peak_z: f32,
        last_touch_team: Possession,
        last_touch_player: Option<String>,
    },
    /// 终局冻结的死球位置。死球期 possession 由上一个回合的归属决定，
    /// `last_touch_team` 在进入死球时快照。
    Dead {
        pos: Vec2,
        z: f32,
        last_touch_team: Possession,
        /// 最后触球的**球员**（可空）。
        ///
        /// ## 为什么需要它（round-9 审计修复）
        ///
        /// `Dead` 此前只带 `last_touch_team`，因此死球回合的**责任球员**无法从
        /// 球态本身派生，只能回退到引擎里另存的 `last_passer_id`。而该字段会在
        /// 进入下一次进攻时被清空，于是当 8 秒违例等死球终结发生在清空之后，
        /// 归因链断在 `None` 上——实测 `TURNOVER_ACTOR_CONSISTENCY` 8 seed 报
        /// 3 条 Hard（`turnover_player_id` 为空）。
        ///
        /// 这是架构问题而非缺一个 fallback：按 P1「状态是唯一事实源」，
        /// 责任球员应当能从**权威球态**读出，而不是依赖旁路字段的存活期。
        /// 补上该载荷后，`BallState` 自身即可回答「谁最后触球」。
        last_touch_player: Option<String>,
    },
}

/// 球的宏观相位标签（由 [`BallState`] 派生，供决策上下文等窄消费面使用）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BallPhase {
    Held,
    ControlTransfer,
    PassFlight,
    ShotFlight,
    Drive,
    Loose,
    Rebound,
    InboundTransfer,
    Dead,
}

impl BallState {
    /// 当前持球人（仅持球族有值）。
    pub fn carried_by(&self) -> Option<&str> {
        match self {
            BallState::Held { carrier_id }
            | BallState::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        }
    }

    /// 持球/飞行族的关联球员（M2：取代 `carrier_idx` 的读取方）。
    ///
    /// - 持球族（Held/Drive/ControlTransfer）→ 持球人；
    /// - Pass → 接球人；Shot → 出手人；
    /// - InboundTransfer/InboundReady → 发球人；
    /// - Loose/RimRebound/Dead → 无关联球员（possession 由
    ///   [`BallState::possessing_team`] 从最后触球方派生）。
    pub fn associated_player(&self) -> Option<&str> {
        match self {
            BallState::Held { carrier_id }
            | BallState::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            BallState::ControlTransfer { .. } => None,
            BallState::Pass { target_id, .. } => Some(target_id.as_str()),
            BallState::Shot { shooter_id, .. }
            | BallState::FreeThrowSetup { shooter_id, .. }
            | BallState::FreeThrow { shooter_id, .. } => Some(shooter_id.as_str()),
            BallState::InboundTransfer { inbounder_id, .. }
            | BallState::InboundReady { inbounder_id, .. } => Some(inbounder_id.as_str()),
            BallState::LooseBall {
                last_touch_player, ..
            }
            | BallState::RimRebound {
                last_touch_player, ..
            } => last_touch_player.as_deref(),
            // 死球：回放进入死球时快照的最后触球人（可空）。
            BallState::Dead {
                last_touch_player, ..
            } => last_touch_player.as_deref(),
        }
    }

    /// 派生球权归属（architecture：取代 `Possession` 独立字段的读取方）。
    ///
    /// - 持球/交接/突破 → 持球人所在队；
    /// - 传球飞行 → 接球人所在队（进攻方内部转移不改归属）；被抢断时
    ///   抢断即是一次状态转移（→ Held{defender}），不在此处体现；
    /// - 投篮/篮板/松球 → 最后触球方（`last_touch_team` 载荷）；
    /// - 发球族 → 发球人所在队；
    /// - 死球 → 进入死球时快照的 `last_touch_team`。
    ///
    /// 注意：防守方碰掉球进入 LooseBall 时 `last_touch_team` 记防守方
    /// （物理上确实最后触球），但引擎的 team possession 语义按 FIBA
    /// 14-3 的 deflection 不结束控制处理——该语义在引擎层由写入口
    /// 构造载荷时决定，本派生只忠实回放载荷。
    pub fn possessing_team(&self) -> Option<Possession> {
        match self {
            BallState::Held { .. }
            | BallState::Drive { .. }
            | BallState::ControlTransfer { .. }
            | BallState::Pass { .. }
            | BallState::Shot { .. }
            | BallState::FreeThrowSetup { .. }
            | BallState::FreeThrow { .. }
            | BallState::InboundTransfer { .. }
            | BallState::InboundReady { .. } => None,
            BallState::LooseBall {
                last_touch_team, ..
            }
            | BallState::RimRebound {
                last_touch_team, ..
            }
            | BallState::Dead {
                last_touch_team, ..
            } => Some(*last_touch_team),
        }
    }

    /// 派生相位标签。
    pub fn phase(&self) -> BallPhase {
        match self {
            BallState::Held { .. } => BallPhase::Held,
            BallState::ControlTransfer { .. } => BallPhase::ControlTransfer,
            BallState::InboundTransfer { .. } | BallState::InboundReady { .. } => {
                BallPhase::InboundTransfer
            }
            BallState::Pass { .. } => BallPhase::PassFlight,
            BallState::Drive { .. } => BallPhase::Drive,
            BallState::Shot { .. } | BallState::FreeThrow { .. } => BallPhase::ShotFlight,
            BallState::FreeThrowSetup { .. } => BallPhase::Dead,
            BallState::LooseBall { .. } => BallPhase::Loose,
            BallState::RimRebound { .. } => BallPhase::Rebound,
            BallState::Dead { .. } => BallPhase::Dead,
        }
    }
}

/// 唯一写入口的纯函数转换表（architecture 状态机规范）。
///
/// 输入当前状态与事实驱动的下一状态；非法边被拒绝并给出原因。
/// 非法 = 语义上不可能的归属跃迁（如飞行中的投篮被直接拿住、死球直接
/// 进入活球飞行而不经发球程序）。合法边覆盖引擎当前产出的全部转移。
pub fn transition_ball_state(cur: &BallState, next: BallState) -> Result<BallState, String> {
    if edge_allowed(cur, &next) {
        Ok(next)
    } else {
        Err(format!("{:?} -> {:?}", cur.phase(), next.phase()))
    }
}

fn edge_allowed(cur: &BallState, next: &BallState) -> bool {
    use BallState as B;
    matches!(
        (cur, next),
        // 持球：可继续持球（重持）、突破、传球、投篮、交接、死球、
        // 或进入发球程序（持球违例，如 5 秒/8 秒/走步——死球化与发球
        // 转移合并为单步，见下注）。
        //
        // `Held -> LooseBall`（持球被切掉）是 round-13 新增：真实 NBA
        // 失误中「带球丢球」占 53.6%（82games 2024-25 IND），是占比最大
        // 的一类，而此前状态机里没有这条边，以致该事实无法表达。
        (B::Held { .. }, B::Held { .. })
            | (B::Held { .. }, B::Drive { .. })
            | (B::Held { .. }, B::Pass { .. })
            | (B::Held { .. }, B::Shot { .. })
            | (B::Held { .. }, B::ControlTransfer { .. })
            | (B::Held { .. }, B::LooseBall { .. })
            | (B::Held { .. }, B::Dead { .. })
            | (B::Held { .. }, B::InboundTransfer { .. })
        // 罚球出手在飞行结束时结算，并按下一罚、命中结束或不中篮板转移。
            | (B::Held { .. }, B::RimRebound { .. })
            | (B::Dead { .. }, B::FreeThrowSetup { .. })
            | (B::Dead { .. }, B::FreeThrow { .. })
            | (B::Dead { .. }, B::RimRebound { .. })
        // 突破：结算为持球 / 突分传球 / 急停投篮 / 攻框命中后的发球转移（合并建模，同上）/
        // 篮板飞行 / 松球 / 死球。
            | (B::Drive { .. }, B::Held { .. })
            | (B::Drive { .. }, B::Pass { .. })
            | (B::Drive { .. }, B::Shot { .. })
            | (B::Drive { .. }, B::RimRebound { .. })
            | (B::Drive { .. }, B::InboundTransfer { .. })
            | (B::Drive { .. }, B::LooseBall { .. })
            | (B::Drive { .. }, B::Dead { .. })
        // 传球飞行：到达 / 被断（持球）/ 脱手（松球）/ 出界或持球违例
        // 判罚直接进入对方发球程序（出界即失球权，合并建模同上）。
            | (B::Pass { .. }, B::Pass { .. })
            | (B::Pass { .. }, B::ControlTransfer { .. })
            | (B::Pass { .. }, B::Held { .. })
            | (B::Pass { .. }, B::LooseBall { .. })
            | (B::Pass { .. }, B::Dead { .. })
            | (B::Pass { .. }, B::InboundTransfer { .. })
        // 投篮飞行：命中 / 打铁触筐（篮板飞行）/ 出界。
        // 命中后"回场发球转移"是 Dead{MadeBasket} 瞬时态与发球飞行
        // 的合并建模（引擎单步完成，状态机摘要表的等价展开）。
            | (B::Shot { .. }, B::InboundTransfer { .. })
            | (B::Shot { .. }, B::Dead { .. })
            | (B::Shot { .. }, B::RimRebound { .. })
            | (B::Shot { .. }, B::LooseBall { .. })
            | (B::FreeThrowSetup { .. }, B::FreeThrow { .. })
            | (B::FreeThrowSetup { .. }, B::Dead { .. })
            | (B::FreeThrowSetup { .. }, B::FreeThrowSetup { .. })
            | (B::FreeThrow { .. }, B::FreeThrowSetup { .. })
            | (B::FreeThrow { .. }, B::Dead { .. })
            | (B::FreeThrow { .. }, B::RimRebound { .. })
            | (B::FreeThrow { .. }, B::LooseBall { .. })
            | (B::FreeThrow { .. }, B::FreeThrow { .. })
        // 篮板飞行：被收下（持球）/ 直接一传（outlet，收下即传的原子转移）
        // / 弹出界 / 过渡交接给抢板人（ControlTransfer 合球平滑）。
            | (B::RimRebound { .. }, B::Held { .. })
            | (B::RimRebound { .. }, B::ControlTransfer { .. })
            | (B::RimRebound { .. }, B::Pass { .. })
            // 篮板飞行途中撞到球员身体：弹开并重解飞行（ADR-017 第三步）。
            // 自由球阶段不变，仍是篮板飞行——只是起点与落点被接触几何改写。
            | (B::RimRebound { .. }, B::RimRebound { .. })
            | (B::RimRebound { .. }, B::LooseBall { .. })
            | (B::RimRebound { .. }, B::Dead { .. })
            | (B::RimRebound { .. }, B::InboundTransfer { .. })
        // 松球：被收下 / 交接拾取 / 继续弹跳 / 出界。
            | (B::LooseBall { .. }, B::Held { .. })
            | (B::LooseBall { .. }, B::ControlTransfer { .. })
            | (B::LooseBall { .. }, B::LooseBall { .. })
            | (B::LooseBall { .. }, B::Dead { .. })
            | (B::LooseBall { .. }, B::InboundTransfer { .. })
        // 交接短飞行：到达即持球；持球违例判罚时死球化与发球程序
        // 合并为单步（否则拒绝写回会让引擎停滞在"死球+持球"无出口状态，
        // 2026-09-02 GAP 复审 seed 2/4 实证）。
            | (B::ControlTransfer { .. }, B::Held { .. })
            | (B::ControlTransfer { .. }, B::Dead { .. })
            | (B::ControlTransfer { .. }, B::InboundTransfer { .. })
        // 死球：只允许进入发球程序或保持死球。
            | (B::Dead { .. }, B::InboundTransfer { .. })
            | (B::Dead { .. }, B::Dead { .. })
        // 发球转移：到达发球点就绪，或被中断回死球。
            | (B::InboundTransfer { .. }, B::InboundReady { .. })
            | (B::InboundTransfer { .. }, B::InboundTransfer { .. })
            | (B::InboundTransfer { .. }, B::Dead { .. })
        // 发球就绪：发球人传出（Pass{inbound}）、违例换边或回死球。
            | (B::InboundReady { .. }, B::Pass { .. })
            | (B::InboundReady { .. }, B::InboundTransfer { .. })
            | (B::InboundReady { .. }, B::Dead { .. })
    )
}
