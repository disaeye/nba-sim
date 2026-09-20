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
        let team = match self.systems.physics.get_player(out_player_id) {
            Some(p) => p.team.clone(),
            None => return,
        };
        // 犯规方不可能持球；若命中持球者（异常调用），拒绝换人以保球权一致。
        //
        // ## holder 引用必须按球态投影判定（round-18 修复）
        //
        // 只查 `has_ball` 旗标不够：进攻犯规时球先进 `Dead` 态（旗标已清），
        // 但 `Dead.last_touch_player` 仍指向犯规的持球人——把他换下场后，
        // 帧投影的 holder 引用一个不在场的人，触发
        // `BALL_HOLDER_ON_COURT` Hard（实测 seed 31337：190 次/场）。
        // 改为：`has_ball` **或** 球态投影（`current_turnover_player_id`）
        // 命中任一即拒绝。
        if self
            .systems
            .physics
            .get_player(out_player_id)
            .is_some_and(|p| p.has_ball)
            || self.current_turnover_player_id().as_deref() == Some(out_player_id)
            // 飞行中的传球目标也不可换下：飞行 ~0.4s 内换人会让球到达时
            // 「被已下场的人接住」（实测 seed 31337：t=3142 H_08 在飞行中
            // 被罚下，到达帧起 Held{H_08} 引用下场者 190 tick）。
            || self.ball.pending_pass_receiver.as_deref() == Some(out_player_id)
            || matches!(&self.ball.ball_state,
                BallTrajectoryKind::Pass { target_id, .. } if target_id == out_player_id)
        {
            return;
        }
        let max_fouls = self.config.rules.league.max_personal_fouls;
        let mut candidates: Vec<String> = self
            .systems
            .physics
            .get_players()
            .values()
            .filter(|p| p.team == team && !p.on_court && p.foul_count < max_fouls)
            .map(|p| p.id.clone())
            .collect();
        candidates.sort();
        let Some(in_player_id) = candidates.first().cloned() else {
            return;
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
            p.has_ball = false;
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
        self.journal.current_callout = Some(format!(
            "球员 {} 犯满离场，替补 {} 死球登场入位！",
            out_player_id, in_player_id
        ));
        // 战术绑定名册同步（slot 绑定按名册顺序索引）。
        for roster in [
            &mut self.config.home_roster_order,
            &mut self.config.away_roster_order,
        ] {
            if let Some(slot_ref) = roster.iter_mut().find(|id| *id == out_player_id) {
                *slot_ref = in_player_id.clone();
            }
        }
        self.journal.pending_events.push(GameEvent::Substitution {
            team,
            out_player: out_player_id.to_string(),
            in_player: in_player_id,
        });
    }

    /// Returns the first configured starter for the team currently in control.
    /// 选出本队当前的**处理球人**（发球员 / 回合起始持球人）。
    ///
    /// ## 为什么不再取 `roster_order` 首位（round-11 Step4b / P-2）
    ///
    /// 旧实现是「取名册数组里第一个在场者」——**顺序即身份**：把名册数组
    /// 轮转一下，处理球人就变了。契约（`tactics.md TA3`「角色是槽位不是身份」、
    /// `attributes.md §2.7`）要求身份由**能力适配**派生。
    ///
    /// 现在按能力排序：`ball_handling × w1 + passing × w2 + decision_iq × w3`，
    /// 权重与维度都来自档案（`TacticalPlanner::handler_score`，与 `fill_slots` 同源）。
    /// 名册数组顺序**完全不参与**。
    ///
    /// 约束：必须是**在场**球员。若把球交给替补（`on_court=false`），他永远
    /// 不会被物理步进，`inbounder_arrived` 永不成立，比赛卡死在 DeadBall
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
