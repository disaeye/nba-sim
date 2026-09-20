//! 只读访问器：比赛真相（球权/球态/时钟/比分/阶段与回合上下文）的观察通道。
//!
//! 依据 `current/plan.md` D7 与 `gap.md` §20.1：权威时间、球权和终态没有外部
//! 可写真相字段。字段在 `MatchEngine` 上私有，外部只能经这里的只读访问器或
//! `step()` / `snapshot()` 推进与观测；需要变可写的测试场景走
//! `test_hooks.rs` 里显式命名的 `*_for_test` 钩子。
//!
//! 这些字段曾为 `pub`，使任何一个库调用方都能直接改写比赛状态（如
//! `engine.rules.tick_seconds = f32::MAX`），因此访问器集中在本文件便于审计
//! 「外部究竟能观察什么」。
//!
//! 访问器集中在本文件便于审计「外部究竟能观察什么」。

use glam::Vec2;
use nba_domain::{GameEvent, GameFlowState, GameRules, Possession, SubPhase};
use nba_invariants::Violation;
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::PhysicsWorld;

use super::{MatchBoxScore, MatchEngine};

impl MatchEngine {
    // ==================== D4.2 真相字段只读访问器 ====================
    // 比赛真相（球权/球态/时钟/比分/阶段）不再对外暴露可变字段；
    // 外部只能经这些访问器读取，或经 `step()` / `snapshot()` 推进与观测
    // （dev 方案 §7.1 D4.2）。写入一律走引擎内部唯一入口。

    /// 当前宏观生命周期状态。
    pub fn game_flow(&self) -> GameFlowState {
        self.flow.game_flow
    }

    /// 回合序号（每次球权转移递增）。
    pub fn possession_id(&self) -> u32 {
        self.flow.possession_id
    }

    /// 回合子阶段。
    pub fn sub_phase(&self) -> SubPhase {
        self.clock.sub_phase
    }

    /// 权威球态（球权真相的唯一载体）。
    pub fn ball_state(&self) -> &BallTrajectoryKind {
        &self.ball.ball_state
    }

    /// 球的三维位置（ft）。
    pub fn ball_pos_3d(&self) -> (Vec2, f32) {
        self.ball.ball_pos_3d
    }

    /// 比赛时钟（秒，节内倒计时）。
    pub fn game_clock(&self) -> f32 {
        self.clock.game_clock
    }

    /// 进攻时钟（秒）。
    pub fn shot_clock(&self) -> f32 {
        self.clock.shot_clock
    }

    /// 单调仿真时间（秒）。
    pub fn current_time(&self) -> f32 {
        self.clock.current_time
    }

    /// 当前节次。
    pub fn period(&self) -> u32 {
        self.clock.period
    }

    /// 主队比分。
    pub fn home_score(&self) -> u32 {
        self.ledger.home_score
    }

    /// 客队比分。
    pub fn away_score(&self) -> u32 {
        self.ledger.away_score
    }

    /// 客队团队犯规数。
    pub fn team_fouls_away(&self) -> u32 {
        self.ledger.team_fouls_away
    }

    /// 当前罚球执行者（只读）。
    pub fn free_throw_shooter(&self) -> Option<&str> {
        self.ledger.free_throw_shooter.as_deref()
    }

    /// 本回合剩余罚球次数。
    pub fn free_throws_remaining(&self) -> u8 {
        self.ledger.free_throws_remaining
    }

    /// 后场连续持球时间（秒），8 秒违例判据。
    pub fn backcourt_elapsed(&self) -> f32 {
        self.clock.backcourt_elapsed
    }

    /// 已完成回合数。
    pub fn completed_possessions(&self) -> usize {
        self.flow.completed_possessions
    }

    /// 比赛统计分解（2P/3P/FT、失误、犯规）——只读快照。
    pub fn box_score(&self) -> &MatchBoxScore {
        &self.ledger.box_score
    }

    // ------------------------------------------------------------------
    // 只读访问器（`current/plan.md` D7：外部不能改真相，只能观察）。
    //
    // 这些字段曾为 `pub`，使任何一个库调用方都能直接改写比赛状态（如
    // `engine.rules.tick_seconds = f32::MAX`），违反 `gap.md` §20.1
    // 「权威时间、球权和终态没有外部可写真相字段」。现改为私有 +
    // 只读访问器；需要变可写的测试场景走显式命名的 `*_for_test` 钩子。
    // ------------------------------------------------------------------

    /// 单调固定步索引（事实与回放消费者用它定位 tick）。
    pub fn tick_index(&self) -> u64 {
        self.clock.tick_index
    }

    /// 本场生效的规则（只读）。外部需自定义规则时用 `with_rules` 构造。
    pub fn rules(&self) -> &GameRules {
        &self.config.rules
    }

    /// 物理世界（只读）：位置、属性、openness 等查询走这里。
    pub fn physics(&self) -> &PhysicsWorld {
        &self.systems.physics
    }

    /// 所属方名单顺序（只读）；用于验证顺序不携带语义（ADR-005）。
    pub fn away_roster_order(&self) -> &[String] {
        &self.config.away_roster_order
    }

    /// FIBA 交替拥有箭头指向（只读，D20）。
    pub fn possession_arrow(&self) -> Option<Possession> {
        self.flow.possession_arrow
    }

    /// 裁决争球 / 纠缠球（Held Ball，D20）。
    ///
    /// 在 FIBA 模式下（use_alternate_possession_arrow=true）依据球权箭头裁定并
    /// 翻转箭头；在 NBA 模式下执行跳球争顶程序（保持当前球权）。
    ///
    /// ## 为什么这里不做运行时状态改写（ADR-016）
    ///
    /// 它曾经直接改写 `flow.possession`（球权）并把它当作可写真相，
    /// 而球权按 ADR-010 应当从权威球态派生。现改为返回裁定结果：
    /// 真实比赛中争球的接续动作是“把球交给被裁定的一方”（一次显式球态转移），
    /// 那时球队归属自然从球态的 `team` 载荷读出，不需要另写一个字段。
    /// NBA 分支对应跳球后的松球争夺，同样由球态决定归属。
    pub fn resolve_held_ball(&mut self) -> Possession {
        if self.config.rules.league.use_alternate_possession_arrow {
            let awarded = self.flow.possession_arrow.unwrap_or(Possession::Away);
            let next_arrow = match awarded {
                Possession::Home => Possession::Away,
                Possession::Away => Possession::Home,
            };
            self.flow.possession_arrow = Some(next_arrow);
            awarded
        } else {
            self.flow.possession
        }
    }

    /// 最近一次 `step()` 产生的不变量违反（只读）。
    pub fn last_tick_violations(&self) -> &[Violation] {
        &self.audit.last_tick_violations
    }

    /// 发球基线位置（只读）。
    pub fn inbound_baseline(&self) -> Vec2 {
        self.flow.inbound_baseline
    }

    /// 待发布事件队列（只读；写入走引擎内部路径）。
    pub fn pending_events(&self) -> &[GameEvent] {
        &self.journal.pending_events
    }

    /// 球权归属（只读）。
    ///
    /// ## 它是派生量（ADR-016）
    ///
    /// 权威球态 `BallState` 的每一个变体都携带球队归属：持球族带持球人
    /// 所在队（`team`），飞行/松球/死球族带 `last_touch_team`。本访问器
    /// 直接从球态派生，不再有独立的可写 `possession` 字段可供与球态分叉。
    /// 球态载荷里的球队是构造时的断言：调用方在写球态时就知道球权归谁，
    /// 之后双方永远一致。
    pub fn possession(&self) -> nba_domain::Possession {
        self.ball
            .ball_state
            .possessing_team()
            .unwrap_or(self.flow.possession)
    }
}
