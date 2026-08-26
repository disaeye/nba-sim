use glam::Vec2;
use rand::Rng;
use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT, HOOP_LEFT_FT, HOOP_RIGHT_FT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Possession {
    Home,
    Away,
}

#[derive(Debug, Clone)]
pub struct PlayerProfile {
    pub name: String,
    pub jersey: String,
    pub shot_range_max_ft: f32,
    pub catch_and_shoot_fg: f32,
    pub vision_fov_degrees: f32,
    pub shoot_tendency: f32,
    pub pass_first_tendency: f32,
    pub drive_tendency: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStageType {
    JumpBall,
    HighPickAndRoll,
    DriveAndKick,
    IsolationDrive,
    FiveOutMotion,
    FastBreakTransition,
    PostBasketInbound,
    SidelineInbound,
    FreeThrow,
    Timeout,
}

pub type PlaySegment = MatchStageType;

#[derive(Debug, Clone)]
pub struct DecisionResult {
    pub action: String,
    pub reason: String,
    pub target_jersey: Option<String>,
    pub shot_openness: f32,
    pub pass_openness: f32,
    pub drive_lane_space: f32,
}
pub struct TacticalPlanner;

impl TacticalPlanner {
    pub fn plan_segment(
        possession: Possession,
        segment: PlaySegment,
        elapsed: f32,
        duration: f32,
        current_positions: &[Vec2; 10],
        _rng: &mut impl Rng,
    ) -> (Vec<(Vec2, f32)>, Vec<(Vec2, f32)>) {
        let is_home_offense = possession == Possession::Home;
        let hoop = if is_home_offense { HOOP_RIGHT_FT } else { HOOP_LEFT_FT };
        let dir = if is_home_offense { -1.0 } else { 1.0 };
        let base_x = hoop.x;

        let (mut off_targets, mut def_targets) = match segment {
            PlaySegment::PostBasketInbound => {
                // 死球进球后：发球人去底线，接应后卫去后场，其余人根据时间快速返位
                let inbounder = Vec2::new(hoop.x - dir * 2.0, 25.0);
                let receiver = Vec2::new(hoop.x + dir * 16.0, 25.0);
                let off_w1 = Vec2::new(hoop.x + dir * 35.0, 10.0);
                let off_w2 = Vec2::new(hoop.x + dir * 35.0, 40.0);
                let off_c = Vec2::new(hoop.x + dir * 42.0, 25.0);

                // 得分方防守落位回防到对方半场
                let def_pg = Vec2::new(hoop.x + dir * 45.0, 25.0);
                let def_sg = Vec2::new(hoop.x + dir * 55.0, 12.0);
                let def_sf = Vec2::new(hoop.x + dir * 55.0, 38.0);
                let def_pf = Vec2::new(hoop.x + dir * 65.0, 15.0);
                let def_c = Vec2::new(hoop.x + dir * 70.0, 25.0);

                (
                    vec![inbounder, receiver, off_w1, off_w2, off_c],
                    vec![def_pg, def_sg, def_sf, def_pf, def_c],
                )
            }
            PlaySegment::HighPickAndRoll => {
                // 高位挡拆：中锋提到弧顶为控卫挂掩护，弱侧射手拉开
                let pg = Vec2::new(base_x + dir * 27.0, 25.0 + (elapsed * 3.0).sin() * 4.0);
                let screen_pos = Vec2::new(base_x + dir * 25.0, 22.0);
                let roll_pos = if elapsed > 1.8 { Vec2::new(base_x + dir * 10.0, 25.0) } else { screen_pos };
                let sg = Vec2::new(base_x + dir * 23.0, 8.0);
                let sf = Vec2::new(base_x + dir * 23.0, 42.0);
                let pf = Vec2::new(base_x + dir * 12.0, 40.0);

                let mut def = Vec::new();
                for off_s in &[pg, sg, sf, pf, roll_pos] {
                    let to_h = (hoop - *off_s).normalize();
                    def.push(*off_s + to_h * 4.0);
                }
                (vec![pg, sg, sf, pf, roll_pos], def)
            }
            PlaySegment::IsolationDrive | PlaySegment::FiveOutMotion | PlaySegment::FastBreakTransition | _ => {
                // 阵地与拉开
                let pg = Vec2::new(base_x + dir * 26.0, 25.0);
                let sg = Vec2::new(base_x + dir * 22.0, 9.0);
                let sf = Vec2::new(base_x + dir * 22.0, 41.0);
                let pf = Vec2::new(base_x + dir * 14.0, 10.0);
                let c = Vec2::new(base_x + dir * 8.0, 32.0);
                let mut def = Vec::new();
                for off_s in &[pg, sg, sf, pf, c] {
                    let to_h = (hoop - *off_s).normalize();
                    def.push(*off_s + to_h * 4.2);
                }
                (vec![pg, sg, sf, pf, c], def)
            }
        };

        let remaining_time = (duration - elapsed).max(0.1);

        // 自适应动态速度估算：距离目标越远、剩余时间越紧，速度自动提升（至冲刺 20 ft/s）
        let compute_speed = |curr: Vec2, tgt: Vec2| -> f32 {
            let dist = curr.distance(tgt);
            if dist < 1.0 {
                return 0.0;
            }
            let required_speed = (dist / remaining_time) * 1.15;
            required_speed.clamp(4.0, 21.0)
        };

        let (off_res, def_res) = if is_home_offense {
            let off = off_targets.into_iter().enumerate().map(|(i, tgt)| {
                let spd = compute_speed(current_positions[i], tgt);
                (tgt, spd)
            }).collect();
            let def = def_targets.into_iter().enumerate().map(|(i, tgt)| {
                let spd = compute_speed(current_positions[5 + i], tgt);
                (tgt, spd)
            }).collect();
            (off, def)
        } else {
            let off = off_targets.into_iter().enumerate().map(|(i, tgt)| {
                let spd = compute_speed(current_positions[5 + i], tgt);
                (tgt, spd)
            }).collect();
            let def = def_targets.into_iter().enumerate().map(|(i, tgt)| {
                let spd = compute_speed(current_positions[i], tgt);
                (tgt, spd)
            }).collect();
            (def, off)
        };

        if is_home_offense {
            (off_res, def_res)
        } else {
            (def_res, off_res)
        }
    }

    /// 核心决策函数 (Decision Function): 根据物理空间计算投/传/突的效用
    pub fn evaluate_ballhandler_decision(
        profile: &PlayerProfile,
        ball_pos_ft: Vec2,
        hoop_ft: Vec2,
        defender_pos_ft: Vec2,
        teammates: &[(String, Vec2)], // (jersey, pos)
        opponents: &[Vec2],
    ) -> DecisionResult {
        let dist_to_hoop = (ball_pos_ft - hoop_ft).length();
        let dist_to_defender = (ball_pos_ft - defender_pos_ft).length();
        
        // 1. 投篮窗口评估 (Shot Openness)
        let in_range = dist_to_hoop <= profile.shot_range_max_ft;
        let shot_openness = if in_range {
            ((dist_to_defender - 2.5) / 5.0).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // 2. 传球窗口评估 (Pass Openness) - 寻找最空位队友
        let mut best_pass_target: Option<String> = None;
        let mut max_pass_openness: f32 = 0.0;
        for (jersey, t_pos) in teammates {
            let min_opp_dist = opponents.iter()
                .map(|opp| (*t_pos - *opp).length())
                .fold(f32::INFINITY, f32::min);
            let openness = ((min_opp_dist - 3.0) / 6.0).clamp(0.0, 1.0);
            if openness > max_pass_openness {
                max_pass_openness = openness;
                best_pass_target = Some(jersey.clone());
            }
        }

        // 3. 突破空间评估 (Drive Lane Space)
        let drive_lane_space = ((dist_to_defender - 3.0) / 4.0).clamp(0.0, 1.0);

        // 决策效用函数比较 (Utility Competition)
        let shot_utility = shot_openness * profile.shoot_tendency * profile.catch_and_shoot_fg * 2.5;
        let pass_utility = max_pass_openness * profile.pass_first_tendency * 2.0;
        let drive_utility = drive_lane_space * profile.drive_tendency * 1.8;

        if shot_utility >= pass_utility && shot_utility >= drive_utility && shot_openness > 0.4 {
            DecisionResult {
                action: "SHOT".to_string(),
                reason: format!("空位窗口 {:.1}ft，出手效用 {:.2} 触发投篮", dist_to_defender, shot_utility),
                target_jersey: None,
                shot_openness,
                pass_openness: max_pass_openness,
                drive_lane_space,
            }
        } else if pass_utility > drive_utility && max_pass_openness > 0.5 {
            DecisionResult {
                action: "PASS".to_string(),
                reason: format!("队友 #{} 处于高价值大空位 (开阔度 {:.0}%)", best_pass_target.as_deref().unwrap_or(""), max_pass_openness * 100.0),
                target_jersey: best_pass_target,
                shot_openness,
                pass_openness: max_pass_openness,
                drive_lane_space,
            }
        } else {
            DecisionResult {
                action: "DRIVE".to_string(),
                reason: format!("寻找内线突破与挡拆缝隙 (突进空间 {:.0}%)", drive_lane_space * 100.0),
                target_jersey: None,
                shot_openness,
                pass_openness: max_pass_openness,
                drive_lane_space,
            }
        }
    }
}
