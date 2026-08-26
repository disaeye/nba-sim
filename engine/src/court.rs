use glam::Vec2;

pub const COURT_WIDTH_FT: f32 = 94.0;
pub const COURT_HEIGHT_FT: f32 = 50.0;
pub const HOOP_LEFT_FT: Vec2 = Vec2::new(5.25, 25.0);
pub const HOOP_RIGHT_FT: Vec2 = Vec2::new(88.75, 25.0);

pub struct Court;

impl Court {
    /// Convert normalized coords [0.0, 1.0] to court feet [0.0..94.0, 0.0..50.0]
    pub fn norm_to_ft(norm: Vec2) -> Vec2 {
        Vec2::new(norm.x * COURT_WIDTH_FT, norm.y * COURT_HEIGHT_FT)
    }

    /// Convert court feet to normalized coords [0.0, 1.0]
    pub fn ft_to_norm(ft: Vec2) -> Vec2 {
        Vec2::new(
            (ft.x / COURT_WIDTH_FT).clamp(0.0, 1.0),
            (ft.y / COURT_HEIGHT_FT).clamp(0.0, 1.0),
        )
    }

    pub fn hoop_pos(is_home_attacking_right: bool) -> Vec2 {
        if is_home_attacking_right {
            HOOP_RIGHT_FT
        } else {
            HOOP_LEFT_FT
        }
    }
}
