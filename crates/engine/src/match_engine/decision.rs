//! 决策阶段：按子阶段决定是否产生一次运球决策。
//!
//! 依据 `docs/architecture.md` §5.1：决策管线是「感知 → 候选生成 → 硬约束过滤
//! → 物理可行性 → 软约束惩罚 → 效用评分 → softmax 采样」。本模块只负责**触发
//! 条件与上下文装配**：判断本 tick 是否到达决策间隔、把持球人的体力与士气偏置
//! 与当前约束上下文交给 `DecisionSystem`，产出意图供执行阶段实施。
//!
//! 意图不是命令：执行阶段会基于**当前**世界重新过一遍硬约束（`architecture.md`
//! §5.2 执行点重校验），因此这里产出的结果可能被降级或作废。
//!
//! `rng` 在调用前后经 `std::mem::replace` 换出再换回，避免把 `&mut self` 的
//! 其余字段借给决策系统时产生借用冲突；种子重放保证同种子逐 tick 一致（C4）。

use nba_decision::pipeline::DecisionOutput;
use nba_decision::play_actions::{build_selection_context, EngineWorldInputs};
use nba_decision::{
    evaluate_active_play, select, OnBallDecisionContext, PlayExecution, PlaySelectionContext,
    StableFieldOutput,
};
use nba_domain::play::PlaySpec;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;

use super::MatchEngine;

impl MatchEngine {
    fn active_play_specs(&self, possession: Possession) -> &[PlaySpec] {
        match possession {
            Possession::Home => &self.config.home_playbook,
            Possession::Away => &self.config.away_playbook,
        }
    }

    fn active_play_book_mut(
        &mut self,
        possession: Possession,
    ) -> &mut nba_decision::PlayActivationBook {
        match possession {
            Possession::Home => &mut self.observations.home_play_activation_book,
            Possession::Away => &mut self.observations.away_play_activation_book,
        }
    }

    fn active_play_cooldowns_mut(
        &mut self,
        possession: Possession,
    ) -> &mut std::collections::BTreeMap<String, u32> {
        match possession {
            Possession::Home => &mut self.observations.home_play_cooldowns,
            Possession::Away => &mut self.observations.away_play_cooldowns,
        }
    }

    fn active_play_mut(&mut self, possession: Possession) -> &mut Option<super::state::ActivePlay> {
        match possession {
            Possession::Home => &mut self.observations.home_active_play,
            Possession::Away => &mut self.observations.away_active_play,
        }
    }

    pub(crate) fn advance_active_play(&mut self) {
        let home_cooldowns = self.observations.home_play_cooldowns.clone();
        let away_cooldowns = self.observations.away_play_cooldowns.clone();
        self.observations
            .home_play_activation_book
            .tick(&home_cooldowns);
        self.observations
            .away_play_activation_book
            .tick(&away_cooldowns);
        for (possession, active) in [
            (Possession::Home, &mut self.observations.home_active_play),
            (Possession::Away, &mut self.observations.away_active_play),
        ] {
            let entries = match possession {
                Possession::Home => self.observations.home_play_activation_book.entries(),
                Possession::Away => self.observations.away_play_activation_book.entries(),
            };
            let book_active = entries.iter().any(|entry| {
                entry.phase == nba_decision::PlayBookPhase::Active
                    && active
                        .as_ref()
                        .is_some_and(|play| play.spec.id == entry.play_id)
            });
            if !book_active {
                *active = None;
            }
        }
    }

    pub(crate) fn end_active_play(&mut self, possession: Possession) {
        let cooldowns = self.active_play_cooldowns_mut(possession).clone();
        self.active_play_book_mut(possession)
            .note_possession_end(&cooldowns);
        *self.active_play_mut(possession) = None;
        self.observations.possession_ticks = 0;
        // 回合边界作废挂起的投篮释放：回合已结束（出界/犯规/违例），
        // 冻结的出手不可能再起飞。若不在此时作废，窗口的 Exec→Follow
        // 边界会在发球程序期间触发 consume，把 sub_phase 拉回 ShotAttempt，
        // 覆盖 begin_inbound 刚设置的 Initiation，发球决策永不触发
        // （INBOUND_READY 永久滞留，实测 seed42 tick 41499 死锁）。
        // 窗口本体仍照常推进到结束（封盖后的动作余辉语义），但不再消费。
        self.observations.pending_shot_release = None;
    }

    pub(crate) fn build_play_selection_context(
        &self,
        carrier_id: &str,
    ) -> Option<PlaySelectionContext> {
        let possession = self.flow.possession;
        let carrier = self.systems.physics.get_player(carrier_id)?;
        let hoop = self
            .config
            .rules
            .court
            .hoop_pos(possession == Possession::Home);
        let roster = match possession {
            Possession::Home => &self.config.home_roster_order,
            Possession::Away => &self.config.away_roster_order,
        };
        let teammates: Vec<_> = roster
            .iter()
            .filter_map(|id| self.systems.physics.get_player(id))
            .filter(|player| player.on_court && player.team == carrier.team)
            .collect();
        let off_positions: Vec<_> = teammates.iter().map(|player| player.pos_ft).collect();
        let off_velocities: Vec<_> = teammates.iter().map(|player| player.vel_ft).collect();
        // 换人边界：旧持球人可能刚被换下（on_court=false）而球权尚未转移，
        // 他不在在场队友列表内。本 tick 跳过 Play 评估，待球权转移后恢复。
        let carrier_idx = teammates
            .iter()
            .position(|player| player.id == carrier_id)?;
        let stable_field = self
            .observations
            .field_hysteresis
            .iter()
            .filter(|(player_id, _)| {
                self.systems
                    .physics
                    .get_player(player_id)
                    .is_some_and(|player| player.team != carrier.team)
            })
            .fold(
                StableFieldOutput {
                    help_pulled_off: false,
                    weak_side_vacant: false,
                },
                |mut output, (_, state)| {
                    let stable = state.snapshot();
                    output.help_pulled_off |= stable.help_pulled_off;
                    output.weak_side_vacant |= stable.weak_side_vacant;
                    output
                },
            );
        let inputs = EngineWorldInputs {
            carrier_idx,
            off_positions,
            off_velocities,
            hoop_pos: hoop,
            court: self.config.rules.court,
            shot_clock_seconds: self.clock.shot_clock,
            carrier_possed: matches!(
                &self.ball.ball_state,
                BallTrajectoryKind::Held { carrier_id: holder } if holder == carrier_id
            ),
            halfcourt: self.clock.sub_phase == SubPhase::ActionExecution
                && self.flow.game_flow == GameFlowState::LiveBall,
            possession_ticks: self.observations.possession_ticks,
            stable_field,
        };
        Some(build_selection_context(&inputs, &self.config.rules))
    }

    fn choose_active_play(&mut self, carrier_id: &str) -> Option<PlayExecution> {
        if self.flow.game_flow != GameFlowState::LiveBall
            || self.clock.sub_phase != SubPhase::ActionExecution
        {
            return None;
        }
        let possession = self.flow.possession;
        let selection_context = self.build_play_selection_context(carrier_id)?;
        if self.active_play_mut(possession).is_none() {
            let playbook = self.active_play_specs(possession).to_vec();
            let mut rng = std::mem::replace(&mut self.systems.rng, ChaCha8Rng::seed_from_u64(0));
            let activation = {
                let book = self.active_play_book_mut(possession);
                select(&playbook, &selection_context, book, &mut rng)
            };
            self.systems.rng = rng;
            if let Some(activation) = activation {
                self.active_play_cooldowns_mut(possession)
                    .insert(activation.play_id.clone(), activation.cooldown_ticks);
                *self.active_play_mut(possession) = Some(super::state::ActivePlay {
                    spec: activation.spec,
                });
                // 激活是可观测事实：经事件通道发布，供 UI/评判器重建
                // Play 的激活/冷却时间线（tactics.md §2.2.4）。
                self.journal.pending_events.push(GameEvent::PlayActivated {
                    play_id: activation.play_id,
                    possession,
                });
            }
        }
        self.active_play_mut(possession)
            .as_ref()
            .map(|play| evaluate_active_play(&play.spec, &selection_context))
    }

    /// 持球姿态落地：新持球人确立时评估一次面框/背身技术选择。
    ///
    /// 写入物理状态与朝向（背身面向传球侧，面框面向篮筐），并把姿态
    /// 切换作为事件发布，使时间线可读出「技术选择」。PostUp 动作执行
    /// 时也会强制写入背身（见 `mark_orientation`）。
    fn refresh_ball_orientation(&mut self, carrier_id: &str) {
        if self
            .possession_ctx
            .ball_orientation
            .as_ref()
            .is_some_and(|(id, _)| id == carrier_id)
        {
            return;
        }
        let hoop = self
            .config
            .rules
            .court
            .hoop_pos(self.flow.possession == Possession::Home);
        let Some(carrier) = self.systems.physics.get_player(carrier_id) else {
            return;
        };
        let orientation =
            nba_decision::orientation::choose_orientation(&self.config.rules, carrier, hoop);
        self.mark_orientation(carrier_id, orientation, hoop);
    }

    /// 写入姿态、朝向与事件；姿态缓存与物理状态保持一致。
    fn mark_orientation(
        &mut self,
        carrier_id: &str,
        orientation: nba_domain::action_window::BallOrientation,
        hoop: glam::Vec2,
    ) {
        let is_back = orientation == nba_domain::action_window::BallOrientation::BackToBasket;
        if let Some(p) = self.systems.physics.get_player_mut(carrier_id) {
            p.ball_orientation = orientation;
            let dir = if is_back {
                p.pos_ft - hoop
            } else {
                hoop - p.pos_ft
            }
            .normalize_or_zero();
            if dir.length_squared() > f32::EPSILON {
                p.facing_dir = dir;
            }
        }
        let player_name = self
            .systems
            .physics
            .get_player(carrier_id)
            .map(|p| format!("{}号", p.jersey))
            .unwrap_or_else(|| carrier_id.to_string());
        if is_back {
            self.journal.current_callout = Some(format!(
                "{} 背身要位，用身体卡住防守凿向篮筐！",
                player_name
            ));
        } else {
            self.journal.current_callout =
                Some(format!("{} 面框三威胁，重心压低观察全场！", player_name));
        }
        // 姿态选择作为领域事件发布（帧投影的 orientation 字段与本事件同 tick）。
        self.journal
            .pending_events
            .push(nba_domain::event::GameEvent::BallOrientationChosen {
                player_id: carrier_id.to_string(),
                back_to_basket: is_back,
            });
        self.possession_ctx.ball_orientation = Some((carrier_id.to_string(), orientation));
    }

    fn decide_with_active_play(
        &mut self,
        carrier_id: &str,
        stamina: f32,
        morale_bias: f32,
        rng: &mut ChaCha8Rng,
    ) -> Option<DecisionOutput> {
        // 先完成需要 &mut self 的 Play 选择，再构建只读决策上下文，
        // 避免不可变借用与可变借用交叠。
        let play = self.choose_active_play(carrier_id);
        // 持球姿态是新持球人的第一次技术决策，先于动作候选评估。
        self.refresh_ball_orientation(carrier_id);
        let ctx = self.constraint_ctx();
        let decision = OnBallDecisionContext {
            constraint_context: &ctx,
            carrier_id,
            stamina,
            morale_bias,
            coach: &self.systems.coach,
            active_play: play.as_ref(),
        };
        self.systems
            .decision
            .decide_on_ball_with_play(&decision, rng)
    }

    pub(crate) fn decision_phase(&mut self, current_t: f32) -> Option<DecisionOutput> {
        // 挂起的投篮释放期间禁止一切持球决策——门禁必须挂在
        // decision_phase 入口而不是 ActionExecution 分支：非投篮
        // 犯规（events.rs）会把子阶段切回 Initiation，出手者仍持球、
        // pending 仍挂着，Initiation 分支若无门禁会让同一球员再次
        // 出手，撞上 pending 非空的 fast-fail（实测 seed100 全量赛
        // tick ~35994）。pending 只由窗口推进消费或回合边界作废，
        // 期间球权与球态都冻结在出手者身上，任何新决策都不合法。
        if self.observations.pending_shot_release.is_some() {
            return None;
        }
        // Phase state machine: inbound decisions are evaluated only during the
        // inbound phase; live-ball decisions use the same registry pipeline.
        let mut decision_output: Option<DecisionOutput> = None;
        match self.clock.sub_phase {
            SubPhase::Initiation => {
                if self.flow.game_flow == GameFlowState::DeadBall {
                    // 发球阶段使用专用（更短）决策间隔：发球受 5 秒规则约束，
                    // 套用阵地节奏会与之竞速（实测 37% 发球被判五秒违例）。
                    if matches!(
                        self.ball.ball_state,
                        BallTrajectoryKind::InboundReady { .. }
                    ) && current_t - self.clock.last_decision_time
                        >= self.config.rules.inbound_decision_interval_seconds
                    {
                        let carrier = self.carrier_id();
                        let stamina = self
                            .systems
                            .physics
                            .get_player(&carrier)
                            .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                            .unwrap_or(1.0);
                        let morale_bias = self.morale_bias_for(&carrier);
                        let mut rng =
                            std::mem::replace(&mut self.systems.rng, ChaCha8Rng::seed_from_u64(0));
                        let outcome =
                            self.decide_with_active_play(&carrier, stamina, morale_bias, &mut rng);
                        decision_output = outcome;
                        self.systems.rng = rng;
                    }
                } else {
                    // 第一性原理：后场推进不受「战术发起」延迟约束。
                    //
                    // 8 秒规则要求进攻方在 8 秒内把球推过中线，而
                    // `tactical_initiation_seconds = 6.5s` 会让球队在后场
                    // 干等到 6.5s 才首次决策，只剩 1.5s 窗口（决策间隔
                    // 2.4s）—— 实测因此产生 31 次/场 8 秒违例，球 x 在
                    // 8 秒内只从 11.4 移到 12.8 ft（需越过 47）。
                    //
                    // 半场阵地进攻才需要「战术发起」等待；后场是转换推进，
                    // 必须立即允许决策（`Advance` 候选随即可用）。
                    let in_backcourt = {
                        let midcourt = self.config.rules.court.width_ft / 2.0;
                        match self.flow.possession {
                            Possession::Home => self.ball.ball_pos_3d.0.x < midcourt,
                            Possession::Away => self.ball.ball_pos_3d.0.x > midcourt,
                        }
                    };
                    if in_backcourt
                        || self.clock.sub_phase_timer
                            >= self.config.rules.tactical_initiation_seconds
                    {
                        self.transition_phase(SubPhase::ActionExecution);
                        self.set_game_flow(GameFlowState::LiveBall);
                        self.journal.current_event = Some("TACTICAL_EXECUTION".to_string());
                        self.journal.current_callout =
                            Some(format!("战术发起：{}", self.config.tactical_set.name_zh()));
                    }
                }
            }
            SubPhase::ActionExecution => {
                // 挂起的投篮释放（已裁定、球在手、窗口执行段）期间禁止
                // 新的持球决策：出手者正在执行已冻结的投篮，再决策会
                // 重复出手或让窗口与球态分叉。
                if self.observations.pending_shot_release.is_none()
                    && current_t - self.clock.last_decision_time
                        >= self.config.rules.decision_interval_seconds
                {
                    let carrier = self.carrier_id();
                    let stamina = self
                        .systems
                        .physics
                        .get_player(&carrier)
                        .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                        .unwrap_or(1.0);
                    let morale_bias = self.morale_bias_for(&carrier);
                    let mut rng =
                        std::mem::replace(&mut self.systems.rng, ChaCha8Rng::seed_from_u64(0));
                    let outcome =
                        self.decide_with_active_play(&carrier, stamina, morale_bias, &mut rng);
                    decision_output = outcome;
                    self.systems.rng = rng;
                }
            }
            SubPhase::ShotAttempt | SubPhase::FlightAndRebound | SubPhase::DeadBallReset => {}
        }
        decision_output
    }
}
