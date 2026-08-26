use glam::Vec2;
use rand::Rng;
use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT, HOOP_LEFT_FT, HOOP_RIGHT_FT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Possession {
    Home,
    Away,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Transition,
    HalfCourtSet,
    Drive,
    ShotRelease,
    Rebound,
    DeadBall,
}

pub struct TacticalPlanner;

impl TacticalPlanner {
    /// Compute target positions (in court feet) for 5 home players and 5 away players
    pub fn plan_targets(
        possession: Possession,
        _phase: Phase,
        _ball_pos_ft: Vec2,
        shot_clock: f32,
        _rng: &mut impl Rng,
    ) -> (Vec<Vec2>, Vec<Vec2>) {
        let is_home_offense = possession == Possession::Home;
        let hoop = if is_home_offense { HOOP_RIGHT_FT } else { HOOP_LEFT_FT };

        // Offense 5-out / motion spots around the attacking half-court
        let base_x = hoop.x;
        let dir = if is_home_offense { -1.0 } else { 1.0 };

        let pg_target = Vec2::new(base_x + dir * 28.0, 25.0);
        let sg_target = Vec2::new(base_x + dir * 24.0, 10.0);
        let sf_target = Vec2::new(base_x + dir * 24.0, 40.0);
        let pf_target = Vec2::new(base_x + dir * 14.0, 8.0);
        let c_target = Vec2::new(base_x + dir * 10.0, 30.0);

        let mut off_targets = vec![pg_target, sg_target, sf_target, pf_target, c_target];

        // Defense tracks corresponding offensive players with realistic gap
        let mut def_targets = Vec::with_capacity(5);
        for off_spot in &off_targets {
            let to_hoop = hoop - *off_spot;
            let def_spot = *off_spot + to_hoop.normalize() * 4.5;
            def_targets.push(def_spot);
        }

        // Add subtle dynamic micro-movement based on shot clock
        for (i, t) in off_targets.iter_mut().enumerate() {
            let offset_x = ((shot_clock * 1.5 + i as f32).sin() * 2.5) * dir;
            let offset_y = (shot_clock * 2.0 + i as f32 * 1.2).cos() * 3.0;
            t.x = (t.x + offset_x).clamp(2.0, COURT_WIDTH_FT - 2.0);
            t.y = (t.y + offset_y).clamp(2.0, COURT_HEIGHT_FT - 2.0);
        }

        if is_home_offense {
            (off_targets, def_targets)
        } else {
            (def_targets, off_targets)
        }
    }
}
