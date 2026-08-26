use glam::Vec2;
use rand::Rng;
use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT, HOOP_LEFT_FT, HOOP_RIGHT_FT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Possession {
    Home,
    Away,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaySegment {
    // 活球阵地战术
    HighPickAndRoll,
    FiveOutMotion,
    IsolationDrive,
    FastBreakTransition,
    // 死球与过渡
    PostBasketInbound,
    SidelineInbound,
    FreeThrowSetup,
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
}
