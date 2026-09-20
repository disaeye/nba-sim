//! 球权与生命周期状态转移：死球、失误、抢断、篮板发球、出界、界外发球、节末。
//!
//! 依据 `docs/architecture.md` §3 的球权状态机：一切球权变更集中在本模块，
//! 并统一经 `transition_ball_state` 唯一写入口落地。

use glam::Vec2;
use nba_decision::constraint::{PhaseType, ViolationKind};
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;

use super::projection::opposite;
use super::MatchEngine;

impl MatchEngine {
    /// 构造 `Dead` 球态的唯一入口：从**当前权威球态**派生最后触球人。
    ///
    /// ## 为什么需要它（round-9 架构修复）
    ///
    /// `Dead` 此前只带 `last_touch_team`，责任球员只能回退到旁路字段
    /// `last_passer_id`——而该字段在进入下一次进攻时被清空，于是死球终结
    /// （如 8 秒违例）的归因会断在 `None`（实测 `TURNOVER_ACTOR_CONSISTENCY`
    /// 8 seed 报 3 条 Hard）。
    ///
    /// 按 P1「状态是唯一事实源」，责任球员必须能从权威球态读出。因此把
    /// 「进入死球时的最后触球人」作为载荷写进 `Dead`，而不是在各调用点
    /// 各自去猜一个回退链。所有 `Dead` 构造都经此函数，保证载荷一致。
    pub(crate) fn dead_state(&self, pos: Vec2, z: f32) -> BallTrajectoryKind {
        BallTrajectoryKind::Dead {
            pos,
            z,
            last_touch_team: self.flow.possession,
            last_touch_player: self.current_turnover_player_id(),
        }
    }

    /// 阶段转换器：从权威球态派生本回合的责任球员。
    pub(crate) fn current_turnover_player_id(&self) -> Option<String> {
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
            // `ControlTransfer` 的载荷里就有 `carrier_id`（接球人），直接读它。
            // 回退到 `last_passer_id` 是错的：合球飞行期间传球人已可能被清空，
            // 且发生在同一回合内多次转移时会把责任归给上一个人。
            // 实测（seed 2/7）：一传 + 合球飞行 + 节末死球 → 归因为空。
            | BallTrajectoryKind::ControlTransfer {
                carrier_id,
                ..
            } => Some(carrier_id.clone()),
            BallTrajectoryKind::Pass { .. } | BallTrajectoryKind::LooseBall { .. } => {
                self.ball.last_passer_id.clone()
            }
            // 死球：优先用载荷里的最后触球人（round-9）；
            // 旧流/未携带时回退到旁路字段，保证向后兼容。
            BallTrajectoryKind::Dead {
                last_touch_player, ..
            } => last_touch_player
                .clone()
                .or_else(|| self.ball.last_passer_id.clone()),
            BallTrajectoryKind::Shot { shooter_id, .. } => Some(shooter_id.clone()),
            BallTrajectoryKind::RimRebound { .. } => None,
        }
    }

    pub(crate) fn start_violation_turnover(&mut self, _kind: ViolationKind) {
        // 失误计数已收敛到 `emit_possession_summary` 单一入口
        // （evidence/problem.md §23.10）：此处不再自增，避免重复计数。
        self.emit_possession_summary(
            nba_domain::PossessionEndCause::TurnoverViolation,
            None,
            self.possession_ctx
                .current_possession_turnover_player
                .clone()
                .or_else(|| self.current_turnover_player_id()),
            None,
        );
        self.ball.last_passer_id = None;
        self.ball.pending_loose_ball_terminal = None;
        self.ball.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = false;
        let current_ball_3d = self.ball.ball_pos_3d;
        let baseline =
            Court::nearest_boundary_with_geometry(current_ball_3d.0, self.config.rules.court);
        self.start_inbound_transition(baseline, current_ball_3d);
    }

    pub(crate) fn start_steal_transition(&mut self, stealer_id: String, intercept_pos: Vec2) {
        // 失误计数已收敛到 `emit_possession_summary` 单一入口（§23.10）。
        // The defender is the actor in the STEAL fact; the summary's
        // turnover_player_id is the offensive player who lost the pass.
        self.emit_possession_summary(
            nba_domain::PossessionEndCause::TurnoverSteal,
            None,
            self.possession_ctx
                .current_possession_turnover_player
                .clone()
                .or_else(|| self.current_turnover_player_id()),
            None,
        );
        self.ball.last_passer_id = None;
        self.ball.pending_loose_ball_terminal = None;
        self.ball.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = false;
        self.complete_possession();
        if self.flow.simulation_complete {
            self.settle_scope_ball((intercept_pos, self.config.rules.ball_holder_height_ft));
            return;
        }
        self.flow.possession = opposite(self.flow.possession);
        self.flow.possession_id += 1;
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.clock.shot_clock = self.config.rules.league.shot_clock_seconds;
        self.clock.sub_phase_timer = 0.0;
        self.transition_phase(SubPhase::ActionExecution);
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_ball_state(BallTrajectoryKind::Held {
            carrier_id: stealer_id,
        });
    }

    pub(crate) fn start_rebound_outlet(
        &mut self,
        rebounder_id: String,
        reb_pos: Vec2,
        is_offensive: bool,
    ) {
        if !is_offensive {
            self.complete_possession();
            if self.flow.simulation_complete {
                self.settle_scope_ball((reb_pos, self.config.rules.chest_height_ft));
                return;
            }
        }
        self.flow.possession = if is_offensive {
            self.flow.possession
        } else {
            opposite(self.flow.possession)
        };
        if !is_offensive {
            self.flow.possession_id += 1;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.possession_ctx.current_possession_turnover_player = Some(rebounder_id.clone());
        self.clock.shot_clock = if is_offensive {
            self.config
                .rules
                .league
                .offensive_rebound_shot_clock_seconds
        } else {
            self.config.rules.league.shot_clock_seconds
        };
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::LiveBall);
        self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
        let target_id = if is_offensive {
            rebounder_id.clone()
        } else {
            let candidate = self.new_possession_pg();
            if candidate == rebounder_id {
                let team = match self.flow.possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                self.team_roster_ids(team)
                    .into_iter()
                    .find(|id| id != &rebounder_id)
                    .unwrap_or_else(|| rebounder_id.clone())
            } else {
                candidate
            }
        };
        let rebounder_pos = self
            .systems
            .physics
            .get_player(&rebounder_id)
            .map(|p| p.pos_ft)
            .unwrap_or(reb_pos);
        // The rebound sample already ends at `reb_pos`. Do not move the ball
        // to the player's body center before creating the next trajectory:
        // that would insert an instantaneous, unobserved catch displacement.
        let catch_pos = reb_pos;
        self.ball.ball_pos_3d = (catch_pos, self.config.rules.chest_height_ft);
        if target_id == rebounder_id {
            let dist = (rebounder_pos - catch_pos).length();
            let speed_budget = (self.config.rules.ball_max_speed_ftps
                - self.config.rules.invariant_speed_tolerance_ftps)
                .max(self.config.rules.invariant_speed_tolerance_ftps);
            let transfer_dur = (dist / speed_budget).max(self.config.rules.tick_seconds);
            self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                from_pos: catch_pos,
                from_z: self.config.rules.chest_height_ft,
                target_pos: rebounder_pos,
                target_z: self.config.rules.ball_holder_height_ft,
                carrier_id: rebounder_id,
                start_time: self.clock.current_time,
                duration: transfer_dur,
            });
            return;
        }
        // ## 防守篮板后的快下选择（D27 接线）
        //
        // `effective_transition_leakout_chance` 先前在 crates/ 生产代码
        // 零消费点（仅 `rules_complete_wiring.rs` 能暴露它）。语义上它属于
        // 「抢下防守篮板后，把球推进给已快下的队友而不交给控卫」的决策：
        // 快下意识越强的球队，越少走「交给控卫重新组织」的稳定路线。
        //
        // 判定口径：取本队在场球员中 `effective_transition_leakout_chance`
        // 最大者（能力适配而非名单下标，ADR-005），若其机会值超过
        // `rules.capability.transition_leakout_threshold` 则改为向他出球。
        // 只在防守篮板（`!is_offensive`）生效：进攻篮板后的快下没有语义。
        let target_id = if !is_offensive {
            let team = match self.flow.possession {
                Possession::Home => "home",
                Possession::Away => "away",
            };
            let mut best: Option<(f32, String)> = None;
            for id in self.team_roster_ids(team) {
                if id == rebounder_id {
                    continue;
                }
                let Some(player) = self.systems.physics.get_player(&id) else {
                    continue;
                };
                if !player.on_court {
                    continue;
                }
                let chance = nba_domain::capability::effective_transition_leakout_chance(
                    &self.config.rules,
                    &player.attributes,
                );
                // 数值相同时按 id 排序，保持确定性。
                let better = match &best {
                    None => true,
                    Some((best_chance, best_id)) => {
                        chance > *best_chance || (chance == *best_chance && id < *best_id)
                    }
                };
                if better {
                    best = Some((chance, id));
                }
            }
            match best {
                Some((chance, id))
                    if chance >= self.config.rules.capability.transition_leakout_threshold =>
                {
                    id
                }
                _ => target_id,
            }
        } else {
            target_id
        };
        let target_pos = self
            .systems
            .physics
            .get_player(&target_id)
            .map(|p| p.pos_ft)
            .unwrap_or_else(|| {
                rebounder_pos
                    + Vec2::new(self.config.rules.rebound_outlet_fallback_distance_ft, 0.0)
            });
        let pass_dist = (target_pos - catch_pos).length();
        // 一传（outlet）也必须**领传**（round-7 审计修复）。
        //
        // 此前 `to_pos` 直接冻结接球人**释放时刻**的当前位置，但接球人在
        // 飞行期间仍向战术目标奔跑，于是永远不可能恰好停在冻结点——实测
        // 越位 5–12 ft，全部出现在 `(97.0, 25.0)` 基准发球点的 outlet 上
        // （`PASS_CORRIDOR_REACHABLE` 的最大单一来源）。
        //
        // 决策侧传球早就有领传（`execute_pass` 的 `target_lead_pos` 来自决策层），
        // 唯独 outlet 这条路径漏了。修法与决策侧一致：按接球人的当前速度
        // 外推一个飞行期内的可达点。
        let target_pos = self.lead_receiver_position(&target_id, target_pos, pass_dist, false);
        let pass_dist = (target_pos - catch_pos).length();
        let receive_success =
            self.resolve_pass_success(&rebounder_id, &target_id, catch_pos, target_pos);
        // 一传（outlet pass）同样在释放时一次性裁定拦截。
        let intercept =
            self.resolve_pass_interception(&rebounder_id, &target_id, catch_pos, target_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos: catch_pos,
            to_pos: target_pos,
            target_id: target_id.clone(),
            start_time: self.clock.current_time,
            duration: self.config.rules.pass_duration(pass_dist, false),
            peak_z: self.config.rules.pass_peak_ft,
            inbound: false,
            receive_success,
            intercept,
        });
        self.possession_ctx.current_possession_passes += 1;
        self.journal.pending_events.push(GameEvent::PassRelease {
            passer_id: rebounder_id.clone(),
            receiver_id: target_id.clone(),
            from_pos: (catch_pos.x, catch_pos.y),
            to_pos: (target_pos.x, target_pos.y),
        });
        // 记录传球人（round-9 架构修复）：一传同样是一次传球，必须设置
        // `last_passer_id`，否则该回合若以死球/违例终结，责任球员无法从
        // 权威球态派生（`Dead` 载荷、`Pass` 分支均回退到该字段）。
        //
        // 实测（seed 0/2/7）：防守篮板 → outlet 一传 → 传球失败 + 节末，
        // 因本行缺失使 `turnover_player_id` 为空，报 3 条
        // `TURNOVER_ACTOR_CONSISTENCY` Hard。
        self.ball.last_passer_id = Some(rebounder_id.clone());
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::LiveBall);
        self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
        self.journal.current_event = Some("OUTLET_PASS".to_string());
        self.journal.current_callout = Some(format!(
            "篮板球转入{}，发动战术：{}",
            if is_offensive {
                "二次进攻"
            } else {
                "转换进攻"
            },
            self.config.tactical_set.name_zh()
        ));
    }

    pub(crate) fn start_loose_ball_transition(&mut self, player_id: String, position: Vec2) {
        let player_team = self
            .systems
            .physics
            .get_player(&player_id)
            .map(|player| player.team.as_str());
        let secured_possession = match player_team {
            Some("home") => Possession::Home,
            Some("away") => Possession::Away,
            _ => self.flow.possession,
        };
        let possession_changed = secured_possession != self.flow.possession;
        let loose_terminal = self
            .ball
            .pending_loose_ball_terminal
            .take()
            .unwrap_or(nba_domain::PossessionEndCause::TurnoverLooseBall);
        // ## 篮板源地板球的事实补记（round-17）
        //
        // 投/罚不中弹出的地板球被**进攻方**收下时，语义上前场篮板
        // （ORB）：回合继续 + 进攻钟重置 14s。若不发 `REBOUND` 事实，
        // 评判器的 ORB 窗口记账（RHYTHM_DURATION / DURATION_BOUNDS 的
        // 自变量）会漏计这个 14s 窗口，把合法的 ~37s 回合误判超带。
        let was_rebound_origin = loose_terminal == nba_domain::PossessionEndCause::DefensiveRebound;
        if was_rebound_origin && !possession_changed {
            self.journal.pending_events.push(GameEvent::ReboundContest {
                rebounder_id: player_id.clone(),
                landing_pos: (position.x, position.y),
                is_offensive: true,
            });
        }
        if possession_changed {
            // 篮板源 + 球权易主 = 防守篮板：归因到收球人（rebounder），
            // 而不是寻找一个不存在的「失误球员」。
            if loose_terminal == nba_domain::PossessionEndCause::DefensiveRebound {
                self.emit_possession_summary(loose_terminal, Some(player_id.clone()), None, None);
            } else {
                let turnover_player_id = self
                    .possession_ctx
                    .current_possession_turnover_player
                    .clone()
                    .or_else(|| self.current_turnover_player_id());
                self.emit_possession_summary(loose_terminal, None, turnover_player_id, None);
            }
            self.ball.last_passer_id = None;
            self.ball.pending_pass_receiver = None;
            // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
            self.ball.receiver_estimate = None;
            self.ball.pending_pass_inbound = false;
            self.complete_possession();
            if self.flow.simulation_complete {
                self.settle_scope_ball((position, self.config.rules.ball_holder_height_ft));
                return;
            }
            self.flow.possession = secured_possession;
            self.flow.possession_id += 1;
            self.clock.shot_clock = self.config.rules.league.shot_clock_seconds;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.possession_ctx.current_possession_turnover_player = Some(player_id.clone());
        // The secured ball remains in a transfer trajectory until it reaches
        // the receiver's frozen catch point. Do not advertise Held here: that
        // would let the following tactical planner move the player away before
        // the ball is attached.
        self.ball.ball_pos_3d = (position, self.config.rules.ball_holder_height_ft);
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::Initiation);
        self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
        self.ball.last_passer_id = None;
        self.ball.pending_loose_ball_terminal = None;
    }

    /// F1.3b：把一个球员离散放置到指定位置（含界外发球点），并发
    /// `PlacementApplied` 事实。
    ///
    /// 离散 placement 属于生命周期事实，不是连续运动（gap.md §4.3），
    /// 因此不参与逐 tick 速度/越界不变量判定；调用方必须同时标记
    /// `out_of_bounds_placement` 以声明该球员当前享有界外豁免。
    pub(crate) fn place_player_out_of_bounds(&mut self, player_id: &str, to: Vec2) {
        let Some(player) = self.systems.physics.get_player(player_id) else {
            return;
        };
        let from = player.pos_ft;
        if let Some(p) = self.systems.physics.get_player_mut(player_id) {
            p.pos_ft = to;
            p.target_pos_ft = to;
            p.vel_ft = Vec2::ZERO;
            p.accel_ft = Vec2::ZERO;
            p.out_of_bounds_placement = true;
        }
        // Rapier 后端从刚体回写坐标，必须同步刚体否则放置会被覆盖。
        self.systems.physics.teleport_player(player_id, to);
        self.journal
            .pending_events
            .push(GameEvent::PlacementApplied {
                player_id: player_id.to_string(),
                from: (from.x, from.y),
                to: (to.x, to.y),
                reason: "INBOUND_SETUP".to_string(),
                phase: self.phase_type().as_str().to_string(),
            });
    }

    /// 为一个即将从界外 placement 回场的球员选择一个界内且不与他人
    /// 重叠的落点（gap.md §4.3：离散 placement 必须直接给出合法坐标）。
    ///
    /// 优先原地 clamp；若与在场球员距离不足，则沿向内方向逐步搜索。
    /// 搜索不出时退回合法 clamp 位置（至少保证在界内）。
    pub(crate) fn free_in_court_spot(&self, from: Vec2, exempt: &Option<String>) -> Vec2 {
        let margin = self.config.rules.player_radius_ft;
        let base = self.config.rules.court.clamp_playable(from, margin);
        let required = self.config.rules.min_player_separation_ft;
        let occupied: Vec<Vec2> = self
            .systems
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && exempt.as_deref() != Some(p.id.as_str()))
            .map(|p| p.pos_ft)
            .collect();
        let is_free = |candidate: Vec2| {
            occupied
                .iter()
                .all(|pos| pos.distance(candidate) >= required)
        };
        if is_free(base) {
            return base;
        }
        // 从当前位置向场内方向逐步内推，步长取球员半径。
        let center = self.config.rules.court.center();
        let inward = (center - base).normalize_or_zero();
        if inward.length_squared() < f32::EPSILON {
            return base;
        }
        let step = self.config.rules.player_radius_ft.max(f32::EPSILON);
        let attempts = ((self.config.rules.court.width_ft / step).ceil() as usize).max(1);
        for i in 1..=attempts {
            let candidate = self
                .config
                .rules
                .court
                .clamp_playable(base + inward * step * i as f32, margin);
            if is_free(candidate) {
                return candidate;
            }
        }
        base
    }

    /// 松球出界的显式状态转移（第一性原理：出界是事实，不是边界夹取）。
    ///
    /// 球权交给最后触球方的对手，并进入发球程序。此前界外松球被
    /// `clamp_playable` 夹回边界，单 tick 位移数英尺造成 `BALL_SPEED`
    /// 尖峰（实测 122 ft/s > 85 上限）。
    pub(crate) fn start_out_of_bounds_transition(
        &mut self,
        boundary_pos: Vec2,
        ball_3d: (Vec2, f32),
    ) {
        // 归因纪律（round-5 审计修复）。
        //
        // 出界**不产生新的失误原因**——它只是球离开场地的位置事实。失误原因
        // 在球出界之前就已确定（掉球/被点掉/被断/违例/松球易主），保存在
        // `pending_loose_ball_terminal`。此前本函数无条件以 `TurnoverViolation`
        // 结算，**覆盖**了既有传球失误原因：实测 8 seed 共 98 条
        // `TURNOVER_ATTRIBUTION` Hard，32 个 `TURNOVER_VIOLATION` 回合中
        // 有 15 个窗口内没有任何 `VIOLATION` 事实。
        //
        // 责任球员必须可归因（D0.1）：松球出界时球可能既无持球人也无
        // `last_passer`（例如篮板弹出界），逐级回退保证不为空。
        let cause = self
            .ball
            .pending_loose_ball_terminal
            .take()
            .unwrap_or(nba_domain::PossessionEndCause::TurnoverViolation);

        let responsible = self
            .possession_ctx
            .current_possession_turnover_player
            .clone()
            .or_else(|| self.current_turnover_player_id())
            .or_else(|| self.ball.last_passer_id.clone())
            .or_else(|| Some(self.carrier_id()));

        // 把「球出界」发布为事实：此前 `OUT_OF_BOUNDS` 在事件流中出现 0 次，
        // 消费方无法区分「出界导致的失误」与「违例导致的失误」。
        self.journal
            .pending_events
            .push(nba_domain::GameEvent::BallOutOfBounds {
                position: [boundary_pos.x, boundary_pos.y],
                last_touch_team: match self.flow.possession {
                    Possession::Home => "home".to_string(),
                    Possession::Away => "away".to_string(),
                },
                responsible_player_id: responsible.clone(),
            });

        self.emit_possession_summary(cause, None, responsible, None);
        self.start_inbound_transition(boundary_pos, ball_3d);
    }

    pub(crate) fn start_inbound_transition(
        &mut self,
        baseline_pos: Vec2,
        current_ball_3d: (Vec2, f32),
    ) {
        self.complete_possession();
        self.ball.last_passer_id = None;
        self.ball.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = false;
        self.ball.pending_loose_ball_terminal = None;
        if self.flow.simulation_complete {
            self.settle_scope_ball(current_ball_3d);
            return;
        }
        self.flow.possession = opposite(self.flow.possession);
        self.flow.possession_id += 1;
        self.update_coach_strategy();
        self.clock.shot_clock = self.config.rules.league.shot_clock_seconds;
        self.sync_team_tactics();
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::DeadBall);
        self.flow.inbound_baseline = baseline_pos;
        let inbounder_id = self.new_possession_pg();
        let release_pos = Court::inbound_release_pos_with_geometry(
            baseline_pos,
            self.config.rules.inbound_release_depth_ft,
            self.config.rules.court,
        );
        if let Some(player) = self.systems.physics.get_player_mut(&inbounder_id) {
            player.target_pos_ft = release_pos;
            player.action = "InboundPositioning".to_string();
            // F1.3：发球程序期间发球员是显式 placement 角色，允许站到
            // 界外发球点（gap.md §4.3/§8.5），否则物理 clamp 会让
            // `inbounder_arrived` 永不成立，比赛卡死在 DeadBall。
            player.out_of_bounds_placement = true;
        }
        // F1.3b：发球员赴界外发球点是**离散 placement**，不是普通运动
        // （gap.md §4.3：换人入场、节间站位、跳球布置同属此类）。
        // 若要求发球员步行过去，一名被场地 clamp 钉在边线的防守者可永久
        // 堵住路径，`inbounder_arrived` 永不成立（本轮 seed 6/9/11 实测
        // 约 19 万 tick 的 OUT_OF_BOUNDS 活锁）。因此直接放置并发事实。
        self.place_player_out_of_bounds(&inbounder_id, release_pos);
        let duration = self
            .config
            .rules
            .pass_duration((release_pos - current_ball_3d.0).length(), true)
            .max(self.config.rules.inbound_setup_seconds);
        self.transition_ball_state(BallTrajectoryKind::InboundTransfer {
            from_pos: current_ball_3d.0,
            from_z: current_ball_3d.1,
            baseline_pos: release_pos,
            inbounder_id,
            start_time: self.clock.current_time,
            duration,
        });
        self.clock.inbound_elapsed = 0.0;
        self.clock.last_decision_time = -self.config.rules.decision_interval_seconds;
        self.journal.current_event = Some("INBOUND_SETUP".to_string());
        self.journal.current_callout = Some(format!(
            "界外发球准备，呼叫战术：{}",
            self.config.tactical_set.name_zh()
        ));
    }

    pub(crate) fn settle_ball_for_period_break(&mut self) {
        let in_flight = matches!(
            self.ball.ball_state,
            BallTrajectoryKind::Pass { .. }
                | BallTrajectoryKind::Shot { .. }
                | BallTrajectoryKind::LooseBall { .. }
                | BallTrajectoryKind::RimRebound { .. }
                | BallTrajectoryKind::ControlTransfer { .. }
        );
        if !in_flight {
            return;
        }
        let (pos, z) = self.ball.ball_pos_3d;
        // 先取责任球员，再构造 Dead——`dead_state` 内部从权威球态派生，
        // 因此**不得**在构造后清除 `last_passer_id`：那样会把刚写进载荷的
        // 最后触球人也一并抹掉，使后续死球终结的归因链断在 None。
        //
        // 实测（round-9）：seed 0/2/7 的 8 秒违例在节末死球后触发，
        // `Dead.last_touch_player` 恒为 None，正是此处顺序造成的。
        // 载荷已是唯一事实源，不再需要旁路字段存续，故无需立即清空。
        self.transition_ball_state(self.dead_state(pos, z));
        self.ball.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.ball.receiver_estimate = None;
        self.ball.pending_pass_inbound = false;
    }

    pub(crate) fn settle_scope_ball(&mut self, position: (Vec2, f32)) {
        self.ball.ball_pos_3d = position;
        self.transition_ball_state(self.dead_state(position.0, position.1));
        self.set_game_flow(GameFlowState::DeadBall);
        self.transition_phase(SubPhase::DeadBallReset);
    }

    pub(crate) fn finish_period(&mut self) {
        let previous_period = self.clock.period;
        if self.clock.period < self.config.rules.league.regulation_periods {
            self.ledger.team_fouls_home = 0;
            self.ledger.team_fouls_away = 0;
            let is_halftime = self.config.rules.league.regulation_periods > 1
                && self.clock.period == self.config.rules.league.regulation_periods / 2;
            self.set_game_flow(if is_halftime {
                GameFlowState::Halftime
            } else {
                GameFlowState::QuarterEnd
            });
            self.clock.period_break_elapsed = 0.0;
            self.transition_phase(SubPhase::DeadBallReset);
            // 第一性原理：节间 `current_time` 冻结（`step_inner` 在
            // QuarterEnd/Halftime 提前返回且不推进时间）。若此时球仍在
            // 飞行（Pass/Shot/Loose），弹道采样是 `progress = (t - start)/duration`
            // 的纯函数——时间一旦恢复推进，球会「瞬移」数英尺，被 L1 判为
            // `BALL_SPEED`（实测 98.2 ft/s > 85 上限，seed 4）。
            //
            // 节末必须先把在飞的球结算成死球：比赛时钟停表期间球也应是死的。
            self.settle_ball_for_period_break();
            // 跨节回合必须显式结算（round-5 审计修复）。
            //
            // `PossessionEndCause::PeriodEnd` 在 domain 中已定义，但引擎从未
            // emit：节末仍在进行中的回合被归到「上一节名下、下一节结束」，
            // 实测 3 个/场，最长 41.6s，越出回合时长上界并产生 Hard defect。
            // 节末是**规则允许**的回合终结方式，必须如实记录。
            let count = self.flow.completed_possessions as u64;
            if self.possession_ctx.last_possession_summary_index != Some(count) {
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::PeriodEnd,
                    None,
                    self.current_turnover_player_id(),
                    None,
                );
                self.complete_possession();
            }
            self.journal
                .pending_events
                .push(GameEvent::PhaseTransition {
                    from: PhaseType::SetPlay,
                    to: PhaseType::DeadBallReset,
                });
            self.journal.current_event = Some("PERIOD_END".to_string());
            self.journal.current_callout = Some(format!(
                "第{}节结束，进入{}休息",
                previous_period,
                if previous_period == self.config.rules.league.regulation_periods / 2 {
                    "中场"
                } else {
                    "节间"
                }
            ));
        } else if self.ledger.home_score == self.ledger.away_score {
            self.clock.period += 1;
            self.clock.game_clock = self.config.rules.league.overtime_duration_seconds;
            self.clock.shot_clock = self.config.rules.league.shot_clock_seconds;
            self.clock.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::Overtime);
            self.transition_phase(SubPhase::Initiation);
            self.journal.current_event = Some("OVERTIME_START".to_string());
        } else {
            self.set_game_flow(GameFlowState::GameEnd);
            self.transition_phase(SubPhase::DeadBallReset);
            self.journal
                .pending_events
                .push(GameEvent::PhaseTransition {
                    from: self.phase_type(),
                    to: PhaseType::DeadBallReset,
                });
            self.journal.current_event = Some("GAME_END".to_string());
            self.journal.current_callout = Some(format!(
                "比赛结束，{}获胜",
                if self.ledger.home_score > self.ledger.away_score {
                    "主队"
                } else {
                    "客队"
                }
            ));
        }
        self.update_scope_completion();
    }
}
