//! 名册与换人：球员身份由**能力**派生，不由名册数组顺序决定。
//!
//! 依据 ADR-005 与 `tactics.md` TA3「角色是槽位不是身份」：换人、发球员选定
//! 与跳球代表选定都不读名册下标，只读在场状态与能力值。名册数组顺序在此模块
//! 中唯一的用处是战术绑定槽位的同步更新，且更新方式是按 id 查找而非按下标替换。

use glam::Vec2;
use nba_domain::{GameEvent, Possession};
use nba_physics::ballistics::BallTrajectoryKind;

use super::MatchEngine;

impl MatchEngine {
    /// 犯满离场的强制换人（tactics.md 2.3.3 优先级 1：强制换人）。
    ///
    /// 替补实体已在物理层注册（on_court=false，不参与运动学）；换人 =
    /// 翻转在场标志并交接场上位置。候选按 id 字典序取最小者（确定性）。
    /// 若替补全部不可用，则保留原球员（保持 5v5 不变量优先）。
    pub fn forced_substitution(&mut self, out_player_id: &str) {
        // 犯满离场不得被死球归因载荷阻塞：哨响时持球人即归因人，若不
        // 先清除，替补在整个死球窗口都无法执行（charter §5.3 要求个人
        // 犯满后必须退出在场名单；实测 seed 2：第 6 犯后 2.3s 才换下，
        // 期间又犯第 7 次）。清除后帧投影不再引用离场者，与 round-18
        // BALL_HOLDER_ON_COURT 防护同源。
        if let BallTrajectoryKind::Dead {
            last_touch_player, ..
        } = &mut self.ball.ball_state
        {
            if last_touch_player.as_deref() == Some(out_player_id) {
                *last_touch_player = None;
            }
        }
        self.substitute(
            out_player_id,
            nba_domain::SubstitutionReason::FoulTrouble,
            None,
        );
        // 犯满离场是强制事实，不允许被持球状态无限期阻塞：换人被拒时
        // 登记待换下，由每 tick 的重试机制在阻塞解除后立即执行
        // （charter §5.3：个人累计达上限后必须退出在场名单）。
        let still_on_court_fouled_out =
            self.systems
                .physics
                .get_player(out_player_id)
                .is_some_and(|p| {
                    p.on_court && p.foul_count >= self.config.rules.league.max_personal_fouls
                });
        if still_on_court_fouled_out {
            // 板凳耗尽是流内事实：候选（同队不在场且未犯满）为空时，
            // 真实规则允许犯满者继续留在场上，评判器据此豁免。
            let max_fouls = self.config.rules.league.max_personal_fouls;
            let team = self
                .systems
                .physics
                .get_player(out_player_id)
                .map(|p| p.team.clone())
                .unwrap_or_default();
            let bench_available = self
                .systems
                .physics
                .get_players()
                .values()
                .any(|p| p.team == team && !p.on_court && p.foul_count < max_fouls);
            if !bench_available
                && self
                    .observations
                    .bench_depleted_reported
                    .insert(out_player_id.to_string())
            {
                self.journal
                    .pending_events
                    .push(GameEvent::EnforcementApplied {
                        constraint_id: "BENCH_DEPLETED".to_string(),
                        action: format!("retain:{out_player_id}"),
                    });
            }
            self.observations.pending_foulout_sub = Some(out_player_id.to_string());
        } else if self.observations.pending_foulout_sub.as_deref() == Some(out_player_id) {
            self.observations.pending_foulout_sub = None;
        }
    }

    /// 一次换人（gap.md G6）：把 `out_player_id` 换下，同队替补登场。
    ///
    /// ## 确定性替补选取
    ///
    /// 候选 = 同队不在场且个人犯规未达上限的球员，按 id 字典序取最小者。
    /// 原因仅影响**谁被换下**（由各轮换评估器决定），不影响替补顺序。
    ///
    /// ## holder 引用必须按权威球态判定（round-18 修复，适用于所有原因）
    //
    // 进攻犯规时球会先进入 `Dead` 态，`Dead.last_touch_player` 仍指向犯规持球人；
    // 把他换下场后，帧投影的 holder 引用一个不在场的人，触发
    // `BALL_HOLDER_ON_COURT` Hard（实测 seed 31337：190 次/场）。
    // 依据球态持球人和 `current_turnover_player_id` 拦截相关换人；飞行中的传球目标
    // 同理（被已下场的人接住）。
    pub(crate) fn substitute(
        &mut self,
        out_player_id: &str,
        reason: nba_domain::SubstitutionReason,
        in_player_id: Option<String>,
    ) {
        let team = match self.systems.physics.get_player(out_player_id) {
            Some(p) => p.team.clone(),
            None => return,
        };
        // 离场者必须**当前在场**：同一 tick 内可能已经发生一次换人
        // （例如死球轮换先把体力枯竭者换下，而同 tick 的犯满强制换人又
        // 指向同一人）。再对已下场者执行换人会多加一名上场球员，使
        // 场上人数变成 6（实测 seed 8 第 3 节 away 6 人上场）。
        if !self
            .systems
            .physics
            .get_player(out_player_id)
            .is_some_and(|p| p.on_court)
        {
            return;
        }
        if self.ball_holder_id() == Some(out_player_id)
            || self.current_turnover_player_id().as_deref() == Some(out_player_id)
            || self.ball.pending_pass_receiver.as_deref() == Some(out_player_id)
            || matches!(&self.ball.ball_state,
                BallTrajectoryKind::Pass { target_id, .. } if target_id == out_player_id)
        {
            return;
        }
        let max_fouls = self.config.rules.league.max_personal_fouls;
        // 犯满强制换人的替补体能地板（规则通道）。
        let entry_floor = self.config.rules.rotation.foul_trouble_entry_stamina;
        // 登场者：调用方指定（evaluate 已按体力/休息时间过滤），否则取
        // 体能达标者优先的确定性首位（犯满路径）。
        let in_player_id = match in_player_id {
            Some(id) => {
                // 指定者必须确实可登场：不在场、未犯满。异常指定按字典序回退。
                let valid = self
                    .systems
                    .physics
                    .get_player(&id)
                    .is_some_and(|p| p.team == team && !p.on_court && p.foul_count < max_fouls);
                if valid {
                    id
                } else {
                    return;
                }
            }
            None => {
                // 犯满强制换人：优先选**体能达标**的替补（避免把已经跑
                // 空的球员再推上场），全部不达标时退回「未犯满即可」——
                // 犯满离场是强制事实，不能因为替补疲劳而拖延（替补池
                // 枯竭时也必须能执行）。
                let mut candidates: Vec<(f32, String)> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .filter(|p| p.team == team && !p.on_court && p.foul_count < max_fouls)
                    .map(|p| {
                        let norm = p.stamina / p.max_stamina.max(1.0);
                        (norm, p.id.clone())
                    })
                    .collect();
                candidates.sort_by(|a, b| {
                    let left = a.0 < entry_floor;
                    let right = b.0 < entry_floor;
                    left.cmp(&right).then_with(|| a.1.cmp(&b.1))
                });
                let Some((_, first)) = candidates.first().cloned() else {
                    return;
                };
                first
            }
        };
        let (out_pos, out_action_slot) = match self.systems.physics.get_player(out_player_id) {
            Some(p) => (p.pos_ft, p.slot.clone()),
            None => return,
        };
        // 离场者退至替补席区（界外，运动学冻结）。
        let bench_spot = Vec2::new(
            self.config.rules.court.width_ft / 2.0 + if team == "home" { -6.0 } else { 6.0 },
            self.config.rules.court.height_ft + 4.0,
        );
        if let Some(p) = self.systems.physics.get_player_mut(out_player_id) {
            p.on_court = false;
            p.action = "Bench".to_string();
            p.pos_ft = bench_spot;
            p.target_pos_ft = bench_spot;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
        }
        // ## 清除对离场者的持球引用（round-18 修复）
        //
        // `last_passer_id` / `pending_pass_receiver` 可能仍指向离场者
        // （如他在界外接球后球出界、引用未被回合边界清除——实测
        // seed 31337：t=3142 H_08 在 y=-4 接球，43s 后被换下，之后
        // 每次 Pass/Loose 飞行的 holder 投影都引用这个已下场的人，
        // 触发 BALL_HOLDER_ON_COURT 190 次）。
        if self.ball.last_passer_id.as_deref() == Some(out_player_id) {
            self.ball.last_passer_id = None;
        }
        if self.ball.pending_pass_receiver.as_deref() == Some(out_player_id) {
            self.ball.pending_pass_receiver = None;
        }
        if let Some(p) = self.systems.physics.get_player_mut(&in_player_id) {
            p.on_court = true;
            p.action = "EnterCourt".to_string();
            p.pos_ft = out_pos;
            p.target_pos_ft = out_pos;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
            p.slot = out_action_slot;
        }
        self.journal.current_callout = Some(reason.callout(out_player_id, &in_player_id));
        // 战术绑定名册同步（slot 绑定按名册顺序索引）。
        for roster in [
            &mut self.config.home_roster_order,
            &mut self.config.away_roster_order,
        ] {
            if let Some(slot_ref) = roster.iter_mut().find(|id| *id == out_player_id) {
                *slot_ref = in_player_id.clone();
            }
        }
        // 轮换钟：双方状态翻转时刻都记为本次换人的比赛时间。
        let now = self.clock.current_time;
        self.observations.rotation_clock.insert(
            out_player_id.to_string(),
            super::state::RotationClock {
                since: now,
                on_court: false,
            },
        );
        self.observations.rotation_clock.insert(
            in_player_id.clone(),
            super::state::RotationClock {
                since: now,
                on_court: true,
            },
        );
        let quota = self
            .observations
            .substitutions_this_window
            .1
            .max(self.observations.substitutions_this_window.0);
        let _ = quota;
        if team == "home" {
            self.observations.substitutions_this_window.0 += 1;
        } else {
            self.observations.substitutions_this_window.1 += 1;
        }
        self.journal.pending_events.push(GameEvent::Substitution {
            team,
            out_player: out_player_id.to_string(),
            in_player: in_player_id,
            reason,
        });
    }

    /// 死球窗口的轮换评估（gap.md G6）。
    ///
    /// 在**发球准备**（`start_inbound_transition`）与**节末**（`finish_period`）
    /// 两个必经死球点调用。同一窗口只评估一次：`rotation_window_done` 在窗口
    /// 进入时置位，活球恢复（进入 `Initiation`）时复位。
    ///
    /// 每队每窗口最多换 `max_substitutions_per_window` 人；被换下者必须满足
    /// `min_rest_seconds` 休息后才可再次登场（由候选过滤保证）。
    pub(crate) fn evaluate_dead_ball_rotation(&mut self) {
        if self.observations.rotation_window_done {
            return;
        }
        self.observations.rotation_window_done = true;
        self.observations.substitutions_this_window = (0, 0);
        let now = self.clock.current_time;
        let rotation = self.config.rules.rotation;
        let score_diff = self.ledger.home_score as i32 - self.ledger.away_score as i32;
        let garbage_time = score_diff.abs() >= rotation.garbage_time_score_margin
            && self.config.rules.league.regulation_periods as f32
                * self.config.rules.league.period_duration_seconds
                - now
                <= rotation.garbage_time_remaining_seconds;
        // 两个原因的出场候选（体力枯竭、垃圾时间轮休），逐队评估。
        let fatigue_threshold = rotation.fatigue_substitution_threshold
            + if garbage_time {
                rotation.garbage_time_fatigue_relief
            } else {
                0.0
            };
        for team in ["home", "away"] {
            let quota = if team == "home" {
                self.observations.substitutions_this_window.0
            } else {
                self.observations.substitutions_this_window.1
            };
            if quota >= rotation.max_substitutions_per_window {
                continue;
            }
            // 候选离场者：在场、体力低于阈值（枯竭）或垃圾时间轮休主力。
            // 按体力升序 + id 升序取确定性首位。
            let mut candidates: Vec<(f32, f32, String)> = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court && p.team == team)
                .filter(|p| {
                    let norm = p.stamina / p.max_stamina.max(1.0);
                    norm < fatigue_threshold
                })
                .map(|p| {
                    (
                        p.stamina / p.max_stamina.max(1.0),
                        p.foul_count as f32,
                        p.id.clone(),
                    )
                })
                .collect();
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then_with(|| a.2.cmp(&b.2)));
            let Some((_, _, out_id)) = candidates.first().cloned() else {
                continue;
            };
            // 替补必须已休息满 `min_rest_seconds`（无记录视为长期休息），
            // 且体力必须高于换下阈值一定裕量：否则刚登场就到线，造成
            // 「换下 B、C 上场两分钟又到线」的高频横跳（实测 138–176 次/场，
            // 真实 NBA 每队 30–40 次）。
            let entry_stamina_floor =
                fatigue_threshold + rotation.garbage_time_fatigue_relief.max(0.15);
            let rested = |id: &str, norm: f32| -> bool {
                if norm < entry_stamina_floor {
                    return false;
                }
                match self.observations.rotation_clock.get(id) {
                    Some(clock) if !clock.on_court => {
                        now - clock.since >= rotation.min_rest_seconds
                    }
                    Some(_) => false,
                    None => true,
                }
            };
            let max_fouls = self.config.rules.league.max_personal_fouls;
            let mut bench: Vec<(f32, String)> = self
                .systems
                .physics
                .get_players()
                .values()
                .filter(|p| p.team == team && !p.on_court && p.foul_count < max_fouls)
                .map(|p| {
                    let norm = p.stamina / p.max_stamina.max(1.0);
                    (norm, p.id.clone())
                })
                .filter(|(norm, id)| rested(id, *norm))
                .collect();
            // 替补中取体力最高者（最接近满状态）。
            bench.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            if bench.is_empty() {
                continue;
            }
            // 换下者不能是当前持球/发球相关者（`substitute` 内部也有守卫，
            // 此处提前过滤避免白耗窗口配额）。
            let is_holder = self.ball_holder_id() == Some(out_id.as_str())
                || self.current_turnover_player_id().as_deref() == Some(out_id.as_str());
            if is_holder {
                continue;
            }
            let Some((_, in_id)) = bench.first().cloned() else {
                continue;
            };
            self.substitute(
                &out_id,
                nba_domain::SubstitutionReason::StaminaExhaustion,
                Some(in_id),
            );
        }
    }

    /// Returns the first configured starter for the team currently in control.
    /// 选出本队当前的**处理球人**（发球员 / 回合起始持球人）。
    ///
    /// ## 为什么不再取 `roster_order` 首位（round-11 Step4b / P-2）
    ///
    /// 旧实现是「取名册数组里第一个在场者」——**顺序即身份**：把名册数组
    /// 轮转一下，处理球人就变了。文档（`tactics.md TA3`「角色是槽位不是身份」、
    /// `attributes.md §2.7`）要求身份由**能力适配**派生。
    ///
    /// 现在按能力排序：`ball_handling × w1 + passing × w2 + decision_iq × w3`，
    /// 权重与维度都来自档案（`TacticalPlanner::handler_score`，与 `fill_slots` 同源）。
    /// 名册数组顺序**完全不参与**。
    ///
    /// 约束：必须是**在场**球员。若把球交给替补（`on_court=false`），他永远
    /// 不会被物理步进，`inbounder_arrived` 永不成立，比赛停滞在 DeadBall
    /// （历史实测：发球员 `action=Bench`、位置停在替补席）。
    /// 权重取当前进攻档案的持球槽位需求（`TacticalPlanner::handler_score`，
    /// 与 `fill_slots` 同源）。
    pub(crate) fn new_possession_pg(&self) -> String {
        let team = match self.flow.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let spec = match self.flow.possession {
            Possession::Home => &self.config.home_offense_spec,
            Possession::Away => &self.config.away_offense_spec,
        };
        let mut candidates: Vec<(f32, String)> = self
            .systems
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == team)
            .map(|p| {
                let score =
                    nba_decision::tactics::TacticalPlanner::handler_score(spec, &p.attributes);
                (score, p.id.clone())
            })
            .collect();
        // 确定性：分数降序，同分按 id 升序（不依赖 HashMap 迭代序，charter C4）。
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .next()
            .map(|(_, id)| id)
            .unwrap_or_default()
    }

    pub(crate) fn team_roster_ids(&self, team: &str) -> Vec<String> {
        match team {
            "home" => self.config.home_roster_order.clone(),
            "away" => self.config.away_roster_order.clone(),
            _ => Vec::new(),
        }
    }

    /// 遵循宪章 Positionless 原则：由场上实际摸高上限最高的球员担当跳球代表
    /// 摸高分数 = 身高 height_cm * 0.5 + 垂直弹跳 vertical * 0.5
    pub fn select_jumper_id(&self, possession: Possession) -> String {
        let team = match possession {
            Possession::Home => &self.config.home_team,
            Possession::Away => &self.config.away_team,
        };
        team.players
            .iter()
            .filter(|p| {
                self.systems
                    .physics
                    .get_player(&p.id)
                    .map(|phys| phys.on_court)
                    .unwrap_or(true)
            })
            .max_by(|a, b| {
                let reach_a = a.height_cm as f32 * 0.5 + a.attributes.vertical * 0.5;
                let reach_b = b.height_cm as f32 * 0.5 + b.attributes.vertical * 0.5;
                reach_a
                    .partial_cmp(&reach_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id.clone())
            // 兜底不得硬编码球员 id（round-10 Step4a：名册已去序号化，
            // "H_1"/"A_1" 不再存在）。退回该队在场球员的字典序首位，
            // 仍无法确定时返回空串（由调用方按非法状态处理）。
            .or_else(|| {
                let team = match possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                let mut ids: Vec<String> = self
                    .systems
                    .physics
                    .get_players()
                    .values()
                    .filter(|p| p.on_court && p.team == team)
                    .map(|p| p.id.clone())
                    .collect();
                ids.sort();
                ids.into_iter().next()
            })
            .unwrap_or_default()
    }
}
