//! 到筐结算：事实发布、得分/篮板分流与打铁双通道路由，以及飞行中
//! 哨响犯规（R1.2）对结算路径的接管。

use glam::Vec2;
use nba_domain::court::Court;
use nba_domain::{GameEvent, SubPhase};
use nba_officiating::resolution::{ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};

use super::BallFlightOutcome;
use super::FlightContext;
use super::MatchEngine;

impl MatchEngine {
    /// 到筐结算：事实发布、得分/篮板分流与打铁双通道路由。
    ///
    /// 从 `Shot` 臂搬出（原样保真）；`is_home` 由 [`FlightContext`] 携带。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn shot_arrival_settlement(
        &mut self,
        sid: String,
        h_pos: Vec2,
        s_pos: Vec2,
        aim_pos: Vec2,
        contact_position: Option<Vec2>,
        made: bool,
        three: bool,
        contest_intensity: f32,
        shot_flight_seconds: f32,
        ctx: FlightContext,
        out: &mut BallFlightOutcome,
    ) {
        let is_home = ctx.is_home;
        // 飞行中哨响的投篮犯规（R1.2）：程序已入队，到筐结算后由结算点
        // 启动罚球，不再进入得分发球或篮板流程。
        let foul_ft_pending = !self.ledger.free_throw_queue.is_empty();
        self.journal.pending_events.push(GameEvent::HoopArrival {
            shooter_id: sid.clone(),
            shot_origin: (s_pos.x, s_pos.y),
            is_made: made,
            is_three: three,
            contest_intensity,
        });
        if three {
            if made {
                self.ledger.box_score.fg3_made += 1;
            }
        } else if made {
            self.ledger.box_score.fg2_made += 1;
        }
        // ## 投篮结果回写到士气状态机（D27 修复不可达分支）
        //
        // `PlayerModulationState::record_shot` 先前在生产代码与测试中
        // 零调用，因此 `consecutive_makes` 恒为 0，
        // `update_stamina_with_rules` 的 `HotHand` 分支
        // （阈值 `hot_hand_makes`）永远不可达——`hot_hand_bias` 无效。
        // 每次出手结算（进或不进）都在此立即回写，使连中/连铁
        // 真正累积。
        if let Some(state) = self.observations.modulation.get_mut(&sid) {
            state.record_shot(made);
        }
        match ResolutionLayer::resolve_shot_arrival(made, three, &sid) {
            ResolutionOutcome::Score { points, .. } => {
                if is_home {
                    self.ledger.home_score += points;
                } else {
                    self.ledger.away_score += points;
                }
                self.ball.ball_pos_3d = (h_pos, self.config.rules.rim_height_ft);
                if foul_ft_pending {
                    // 命中 + 犯规（and-1 语境）：得分计入，回合经罚球延续，
                    // 不发 Score 总结、不进入对方发球。
                    if !self.try_start_next_free_throw_program(out) {
                        // 队列在借用间隙被消费的防御分支：保持原得分路径。
                        let baseline =
                            Court::nearest_boundary_with_geometry(h_pos, self.config.rules.court);
                        self.emit_possession_summary(
                            nba_domain::PossessionEndCause::Score,
                            None,
                            None,
                            None,
                        );
                        self.transition_phase(SubPhase::DeadBallReset);
                        self.start_inbound_transition(
                            baseline,
                            (h_pos, self.config.rules.rim_height_ft),
                        );
                    }
                    return;
                }
                let baseline =
                    Court::nearest_boundary_with_geometry(h_pos, self.config.rules.court);
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::Score,
                    None,
                    None,
                    None,
                );
                self.transition_phase(SubPhase::DeadBallReset);
                self.start_inbound_transition(baseline, (h_pos, self.config.rules.rim_height_ft));
            }
            ResolutionOutcome::Miss { .. } => {
                let shooter_name = self
                    .systems
                    .physics
                    .get_player(&sid)
                    .map(|p| p.jersey.clone())
                    .unwrap_or_else(|| sid.clone());
                self.journal.current_event = Some("SHOT_MISSED".to_string());
                if foul_ft_pending {
                    // 不中 + 投篮犯规：无篮板（真实规则哨响后不争抢），
                    // 罚球程序立即接管。
                    self.journal.current_callout = Some(format!(
                        "{} 投篮不中，裁判鸣哨投篮犯规，走上罚球线",
                        shooter_name
                    ));
                    self.try_start_next_free_throw_program(out);
                    return;
                }
                self.journal.current_callout =
                    Some(format!("砸框而出！{} 投篮不中，争抢篮板！", shooter_name));
                let shooter_for_payload = sid.clone();
                self.transition_phase(SubPhase::FlightAndRebound);
                // 双通道路由（ADR-017 第三步）：探针判「力度过大
                // 越过筐」——出手 → 筐延长线穿过板面且弦外推 z
                // 处于板高内 → 走打板路径（瞄准点为筐心，散射
                // 由弦几何承载）；不足量仍是近筐沿反射。
                let landing_spot = if let Some(contact_position) = contact_position {
                    BallisticsEngine::compute_rebound_landing_from_contact(
                        s_pos,
                        h_pos,
                        shot_flight_seconds,
                        contact_position,
                        &mut self.systems.rng,
                        &self.config.rules,
                    )
                } else if BallisticsEngine::compute_backboard_contact_probe(
                    s_pos,
                    aim_pos,
                    &self.config.rules,
                ) {
                    BallisticsEngine::compute_rebound_landing_bank(
                        s_pos,
                        aim_pos,
                        shot_flight_seconds,
                        &mut self.systems.rng,
                        &self.config.rules,
                    )
                } else {
                    BallisticsEngine::compute_rebound_landing(
                        s_pos,
                        h_pos,
                        shot_flight_seconds,
                        &mut self.systems.rng,
                        &self.config.rules,
                    )
                };
                // 反弹起点 = 触点（近筐沿或板面，第三步双通道）。
                let rebound_from = (landing_spot.contact_pos, landing_spot.contact_z);
                out.new_ball_state = Some(BallTrajectoryKind::RimRebound {
                    from_pos: rebound_from.0,
                    from_z: rebound_from.1,
                    hoop_pos: h_pos,
                    target_landing: landing_spot.landing_pos,
                    start_time: ctx.current_t,
                    duration: landing_spot.flight_duration,
                    peak_z: landing_spot.peak_z,
                    last_touch_team: self.flow.possession,
                    // 物理最后触球人是出手人（触筐不改球权归属）。
                    last_touch_player: Some(shooter_for_payload),
                });
                // ## 篮板冲抢指派（evidence/problem.md §23.8）
                //
                // 实测：球在空中时守方朝球靠近的速率是攻方的**约 42 倍**
                // （0.0042 vs 0.0001 ft/tick），且两者绝对值都极小——
                // 即双方都几乎没有抢篮板行为，攻方几乎为零。后果是
                // ORB% 仅 0.07–0.13（真实 0.245），每次不中直接换手，
                // 回合被压成「一次性进攻」并与节奏过快同向。
                //
                // 此处显式指派：双方球员向球的落点邻域移动（守方优先，
                // 攻方按 offensive_rebound 属性加权），使篮板真的被争抢，
                // 而不是靠判定公式凭空产生归属。
                self.assign_rebound_pursuit(landing_spot.landing_pos, ctx.current_t);
            }
            _ => {}
        }
    }
}
