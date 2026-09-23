//! 比赛流程：阶段标签投影、宏观生命周期迁移与子阶段迁移。
//!
//! 依据 `docs/architecture.md` §2 与 `gap.md` §4：权威时间与生命周期边界只能
//! 经这里的迁移函数改写；调用方读到的 `GameFlowState` / `SubPhase` / `PhaseType`
//! 都是经显式迁移得到的标签，不存在第二份独立状态。
//!
//! `set_game_flow` 在进入活球前校验「场上恰好 10 人」的因果前置条件：人数不足
//! 直接拒绝迁移并登记，避免出现「活球但没有合法阵容」的不可解释状态。

use nba_decision::constraint::{ConstraintContext, PhaseType};
use nba_decision::modulation::CoachStrategy;
use nba_domain::{GameEvent, GameFlowState, GameRules, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;

use super::{MatchEngine, ScopeBoundary};

impl MatchEngine {
    /// 当前阶段映射到约束阶段类型；宏观生命周期决定发球、推进和罚球语义。
    pub(crate) fn phase_type(&self) -> PhaseType {
        if self.flow.game_flow == GameFlowState::FreeThrow {
            return PhaseType::FreeThrow;
        }
        if self.flow.game_flow == GameFlowState::TipOff {
            return PhaseType::TipOff;
        }
        match self.clock.sub_phase {
            SubPhase::Initiation if self.flow.game_flow == GameFlowState::DeadBall => {
                PhaseType::Inbound
            }
            SubPhase::Initiation => PhaseType::Transition,
            SubPhase::ActionExecution => PhaseType::SetPlay,
            SubPhase::ShotAttempt => PhaseType::Resolution,
            SubPhase::FlightAndRebound => PhaseType::Rebound,
            SubPhase::DeadBallReset => PhaseType::DeadBallReset,
        }
    }
    pub fn constraint_ctx<'a>(&'a self) -> ConstraintContext<'a> {
        let possession_team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        ConstraintContext {
            physics: &self.systems.physics,
            ball_pos: self.ball.ball_pos_3d.0,
            possession_team,
            shot_clock: self.clock.shot_clock,
            game_clock: self.clock.game_clock,
            phase: self.phase_type(),
            game_flow: self.flow.game_flow,
            ball_phase: self.ball_phase(),
            inbound_elapsed: self.clock.inbound_elapsed,
            backcourt_elapsed: self.clock.backcourt_elapsed,
            rules: &self.config.rules,
            team_traits: &self.config.team_traits,
            possession_had_shot: self.possession_ctx.current_possession_shooter.is_some(),
        }
    }
    /// 球的宏观相位（由领域层 BallState 派生，M2：标签不再是独立状态）。
    pub(crate) fn ball_phase(&self) -> nba_domain::BallPhase {
        if self.ledger.free_throws_remaining > 0 {
            return nba_domain::BallPhase::Dead;
        }
        self.ball.ball_state.phase()
    }

    /// 依 ADR-010 裁定从权威球态派生焦点持球人 / 战术发起人 ID。
    pub fn active_carrier_or_focus_id(&self) -> String {
        match &self.ball.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::InboundTransfer {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::ControlTransfer { carrier_id, .. } => carrier_id.clone(),
            BallTrajectoryKind::Pass { target_id, .. } => target_id.clone(),
            BallTrajectoryKind::Shot { shooter_id, .. } => shooter_id.clone(),
            _ => self
                .current_turnover_player_id()
                .unwrap_or_else(|| self.new_possession_pg()),
        }
    }

    /// 持球人 id（ADR-010：由球态派生焦点人，不再使用独立 carrier_idx 字段）。
    pub(crate) fn carrier_id(&self) -> String {
        self.active_carrier_or_focus_id()
    }

    pub(crate) fn sync_team_tactics(&mut self) {
        self.config.tactical_set = match self.flow.possession {
            Possession::Home => self.config.home_offense_tactic,
            Possession::Away => self.config.away_offense_tactic,
        };
        // round-6 审计修复：防守方案必须成为因果输入。
        //
        // 此前 `home/away_defensive_tactic` 只被用于生成展示字符串，物理与
        // 决策管线从不消费——实测 6 种方案各跑一场全场模拟逐字节相同。
        // 现在把**防守方**的方案经规则通道写入 `rules.tactics.defense`，
        // 由防守目标点生成消费（charter C1/C3：数据通道，非代码分支）。
        //
        // 未知 id 不静默回退：保留上一 tick 的参数并在强制项里登记，
        // 避免又一个「声明了但无效」的隐形参数。
        let defending_side = match self.flow.possession {
            Possession::Home => self.config.away_defensive_tactic,
            Possession::Away => self.config.home_defensive_tactic,
        };
        match nba_domain::DefenseRules::for_scheme(defending_side.id()) {
            Some(d) => {
                // ## 方案只覆写它自己声明的字段，不重置整组
                //
                // `schemes.json` 的档案只声明 `sag_multiplier` /
                // `on_ball_gap_multiplier` / `help_priority` /
                // `switch_aggressiveness` / `screen_defense` 四项。
                // 直接 `= d` 会把 `potential_field` 一起重置为
                // `PotentialFieldRules::default()`——而那组系数是 `--rules`
                // 可覆盖的行为参数。实测后果：把 `k_threat_base` 与
                // `low_man_threat_gain` 归零后，`ROTATE_RIM_HELP` 的帧数
                // 逐位不变（1084/2149 完全相同），即校准通道被静默屏蔽。
                let tuned = self.config.rules.tactics.defense.potential_field;
                self.config.rules.tactics.defense = nba_domain::DefenseRules {
                    potential_field: tuned,
                    ..d
                };
            }
            None => self
                .journal
                .current_enforcements
                .push(format!("UNKNOWN_DEFENSE_SCHEME:{}", defending_side.id())),
        }
    }

    /// Re-evaluate coach strategy at every possession boundary.
    pub(crate) fn update_coach_strategy(&mut self) {
        let score_diff = self.ledger.home_score as i32 - self.ledger.away_score as i32;
        self.systems.coach = CoachStrategy::evaluate(
            score_diff,
            self.clock.period,
            self.clock.game_clock,
            &self.config.rules,
        );
    }
    pub fn set_game_flow(&mut self, flow: GameFlowState) {
        if self.flow.game_flow == flow {
            return;
        }
        // 当试图进入活球阶段时，执行严格的因果前置图前置条件校验
        if flow == GameFlowState::LiveBall {
            let active_count = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court)
                .count();
            if active_count != 10 {
                eprintln!(
                    "[CAUSAL_DAG] Transition to LiveBall rejected: strictly 10 on-court players required, found {}",
                    active_count
                );
                return;
            }
        }
        self.flow.game_flow = flow;
    }

    pub(crate) fn sync_game_flow(&mut self) {
        if matches!(
            self.flow.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) && self.break_finished()
        {
            self.clock.period += 1;
            self.clock.game_clock = self.config.rules.period_duration(self.clock.period);
            self.clock.shot_clock = self.config.rules.league.shot_clock_seconds;
            self.ledger.team_fouls_home = 0;
            self.ledger.team_fouls_away = 0;
            self.clock.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::LiveBall);
            self.transition_phase(SubPhase::Initiation);
            // 节间开场必须显式重建球的可取性（architecture §3 球态规范）：
            // 节末结算可能把在飞的球留成停球状态，若直接以活球流程运行，
            // 会出现「流程活球、球不可取」的停滞（seed 14 第 4 节开场实测）。
            self.start_period_ball_program();
            self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
            self.journal.current_event = Some("PERIOD_START".to_string());
            self.journal.current_callout = Some(format!("第{}节开始", self.clock.period));
        }
    }
    pub(crate) fn transition_phase(&mut self, next: SubPhase) {
        let previous = self.phase_type();
        self.clock.sub_phase = next;
        self.clock.sub_phase_timer = 0.0;
        if next == SubPhase::Initiation {
            self.clock.inbound_elapsed = 0.0;
            self.clock.backcourt_elapsed = 0.0;
            // 死球换人窗口随活球恢复而关闭（gap.md G6）：下一个死球窗口
            // 重新评估轮换。窗口在 `evaluate_dead_ball_rotation` 入口置位。
            self.observations.rotation_window_done = false;
        }
        let next_phase = self.phase_type();
        if previous != next_phase {
            self.journal
                .pending_events
                .push(GameEvent::PhaseTransition {
                    from: previous,
                    to: next_phase,
                });
        }
    }

    /// 在回合转换边界发射 L2 回合语义总结事件（docs/quality.md §2.2）。
    /// 终结原因是 `PossessionEndCause` 显式枚举——没有兜底值，调用方
    /// 必须在编译期说明回合为什么结束（dev 方案 §3.2 D0.1）。
    pub fn emit_possession_summary(
        &mut self,
        terminal_event: nba_domain::PossessionEndCause,
        rebounder_id: Option<String>,
        turnover_player_id: Option<String>,
        rebound_distance_ft: Option<f32>,
    ) {
        let start_clock = self.possession_ctx.current_possession_start_clock;
        let end_clock = self.clock.game_clock;
        let duration_seconds =
            (self.clock.current_time - self.possession_ctx.current_possession_start_time).max(0.0);
        let offense_team = match self.flow.possession {
            Possession::Home => "home".to_string(),
            Possession::Away => "away".to_string(),
        };
        let summary = nba_domain::PossessionSummary {
            possession_index: self.flow.completed_possessions as u64,
            offense_team,
            start_clock,
            end_clock,
            duration_seconds,
            passes_count: self.possession_ctx.current_possession_passes,
            terminal_event,
            shooter_id: self.possession_ctx.current_possession_shooter.clone(),
            rebounder_id,
            turnover_player_id,
            shot_contest_intensity: self.possession_ctx.current_possession_contest,
            rebound_distance_ft,
        };
        // ## 失误计数单一入口（evidence/problem.md §23.10）
        //
        // 此前 `box_score.turnovers` 只有两个自增点
        // （`start_violation_turnover` / `start_steal_transition`），
        // 而 `PossessionEndCause` 有五种 Turnover* 终结：
        // PassTipped / PassDropped / LooseBall 三条路径只发总结、不计箱体，
        // 实测 seed42 事件流 52 次失误终结 vs 箱体 12 次（低估约 4 倍）。
        //
        // 账本后果：守恒式 `possessions ≈ FGA + TO + 0.44·FTA − OREB` 的
        // TO 项失真，回合残差由 +4 撑到 +44。
        //
        // 修正方式：把计数收敛到**唯一入口**（本函数是所有权终结的漏斗），
        // 而不是在五条路径上各自补一次——后者迟早会再次漏掉新增的终结类型。
        if matches!(
            terminal_event,
            nba_domain::PossessionEndCause::TurnoverViolation
                | nba_domain::PossessionEndCause::TurnoverSteal
                | nba_domain::PossessionEndCause::TurnoverPassTipped
                | nba_domain::PossessionEndCause::TurnoverPassDropped
                | nba_domain::PossessionEndCause::TurnoverLooseBall
        ) {
            self.ledger.box_score.turnovers += 1;
        }
        self.possession_ctx.last_possession_summary_index = Some(summary.possession_index);
        self.journal
            .pending_events
            .push(nba_domain::GameEvent::PossessionSummary(summary));
        // 重置下一个回合的上下文
        self.possession_ctx
            .begin(self.clock.game_clock, self.clock.current_time);
        self.observations.possession_ticks = 0;
        self.end_active_play(self.flow.possession);
        self.journal.current_callout = None;
    }

    /// Parses a scope string into a possession budget and human description.
    /// Malformed scopes are rejected instead of silently changing the request.
    pub fn parse_scope(scope: &str) -> Result<(usize, String), String> {
        Self::parse_scope_with_rules(scope, &GameRules::default())
    }

    pub fn parse_scope_with_rules(
        scope: &str,
        rules: &GameRules,
    ) -> Result<(usize, String), String> {
        let normalized = scope.trim().to_ascii_lowercase();
        let possessions = match normalized.as_str() {
            "1p" => 1usize,
            "5p" => 5usize,
            "10p" => 10usize,
            "1q" => rules.estimated_possessions_per_period as usize,
            "full" => (rules.estimated_possessions_per_period as usize)
                .checked_mul(rules.league.regulation_periods as usize)
                .ok_or_else(|| "scope possession budget overflowed".to_string())?,
            value if value.ends_with('p') => value[..value.len() - 1]
                .parse::<usize>()
                .map_err(|_| format!("invalid scope: {scope}"))?,
            _ => return Err(format!("invalid scope: {scope}")),
        };
        if possessions == 0 {
            return Err("scope must request at least one possession".to_string());
        }
        let description = if normalized == "1q" {
            format!("1 Quarter (approx {possessions} possessions)")
        } else if normalized == "full" {
            format!("Full Game (approx {possessions} possessions)")
        } else {
            format!("{possessions} Possessions")
        };
        Ok((possessions, description))
    }
    pub fn set_scope(&mut self, scope: &str) -> Result<String, String> {
        let (target_possessions, description) =
            Self::parse_scope_with_rules(scope, &self.config.rules)?;
        let normalized = scope.trim().to_ascii_lowercase();
        self.flow.target_possessions = target_possessions;
        self.flow.scope_active = true;
        self.flow.scope_boundary = match normalized.as_str() {
            "1q" => ScopeBoundary::Period {
                last_period: self.clock.period,
            },
            "full" => ScopeBoundary::Game,
            _ => ScopeBoundary::Possessions,
        };
        self.flow.simulation_complete = match self.flow.scope_boundary {
            ScopeBoundary::Possessions => self.flow.completed_possessions >= target_possessions,
            ScopeBoundary::Period { .. } | ScopeBoundary::Game => false,
        };
        if self.flow.simulation_complete {
            self.settle_scope_ball(self.ball.ball_pos_3d);
        }
        Ok(description)
    }

    pub(crate) fn is_clutch_situation(&self) -> bool {
        let policy = &self.config.rules.modulation;
        self.clock.period >= policy.clutch_period
            && self.clock.game_clock <= policy.clutch_time_remaining
            && (self.ledger.home_score as i32 - self.ledger.away_score as i32).abs()
                <= policy.clutch_score_margin
    }

    /// 当前士气偏置。
    ///
    /// 关键时刻（`is_clutch_situation`）的偏置在此单独叠加，不另立
    /// `MoraleState::Clutch` 变体：该变体没有任何赋值点，且一旦被赋值
    /// 会使 `clutch_bias` 在此处与本分支各计一次。
    pub(crate) fn morale_bias_for(&self, player_id: &str) -> f32 {
        let policy = &self.config.rules.modulation;
        let base_bias = match self
            .observations
            .modulation
            .get(player_id)
            .map(|m| m.morale)
        {
            Some(nba_decision::modulation::MoraleState::HotHand) => policy.hot_hand_bias,
            Some(nba_decision::modulation::MoraleState::Normal) => 0.0,
            Some(nba_decision::modulation::MoraleState::Frustrated) => policy.frustrated_bias,
            Some(nba_decision::modulation::MoraleState::Exhausted) => policy.exhausted_bias,
            None => 0.0,
        };
        if self.is_clutch_situation() {
            base_bias + policy.clutch_bias
        } else {
            base_bias
        }
    }

    pub(crate) fn update_scope_completion(&mut self) {
        if !self.flow.scope_active || self.flow.simulation_complete {
            return;
        }
        let reached = match self.flow.scope_boundary {
            ScopeBoundary::Possessions => {
                self.flow.completed_possessions >= self.flow.target_possessions
            }
            ScopeBoundary::Period { last_period } => {
                self.clock.period > last_period
                    || (self.clock.period == last_period
                        && matches!(
                            self.flow.game_flow,
                            GameFlowState::QuarterEnd | GameFlowState::Halftime
                        ))
            }
            ScopeBoundary::Game => self.flow.game_flow == GameFlowState::GameEnd,
        };
        if reached {
            self.flow.simulation_complete = true;
            self.settle_scope_ball(self.ball.ball_pos_3d);
        }
    }

    // ========================================================================
    // Possession boundary
    pub(crate) fn complete_possession(&mut self) {
        // D0.1（dev 方案 §3.2）：`UNATTRIBUTED_END` 兜底已物理删除。
        // 到达回合边界时必须已有带显式 `PossessionEndCause` 的总结；
        // 缺总结 = 因果链破缺，构成 Hard 缺陷，测试断言其不发生而非兜底。
        let count = self.flow.completed_possessions as u64;
        debug_assert_eq!(
            self.possession_ctx.last_possession_summary_index,
            Some(count),
            "possession {count} reached boundary without an attributed summary"
        );
        self.flow.completed_possessions = self.flow.completed_possessions.saturating_add(1);
        self.update_scope_completion();
    }

    pub(crate) fn defensive_tactic_name(&self) -> String {
        match self.flow.possession {
            Possession::Home => self.config.away_defensive_tactic.name_zh().to_string(),
            Possession::Away => self.config.home_defensive_tactic.name_zh().to_string(),
        }
    }
}
