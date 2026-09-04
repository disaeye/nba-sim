use glam::Vec2;
use serde::{Deserialize, Serialize};

pub const COURT_WIDTH_FT: f32 = 94.0;
pub const COURT_HEIGHT_FT: f32 = 50.0;
pub const HOOP_LEFT_FT: Vec2 = Vec2::new(5.25, 25.0);
pub const HOOP_RIGHT_FT: Vec2 = Vec2::new(88.75, 25.0);

/// Competition court geometry shared by every subsystem that interprets a
/// position.  The constants above are compatibility defaults only; runtime
/// code should use the geometry carried by `GameRules`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default)]
pub struct CourtGeometry {
    pub width_ft: f32,
    pub height_ft: f32,
    pub hoop_left_x_ft: f32,
    pub hoop_right_x_ft: f32,
    pub hoop_y_ft: f32,
}

impl Default for CourtGeometry {
    fn default() -> Self {
        Self {
            width_ft: COURT_WIDTH_FT,
            height_ft: COURT_HEIGHT_FT,
            hoop_left_x_ft: HOOP_LEFT_FT.x,
            hoop_right_x_ft: HOOP_RIGHT_FT.x,
            hoop_y_ft: HOOP_LEFT_FT.y,
        }
    }
}

impl CourtGeometry {
    /// FIBA 场地（28m × 15m ≈ 91.86 × 49.21 ft），篮筐位置按比例内收。
    pub fn fiba() -> Self {
        Self {
            width_ft: 91.86,
            height_ft: 49.21,
            hoop_left_x_ft: 5.25,
            hoop_right_x_ft: 91.86 - 5.25,
            hoop_y_ft: 49.21 / 2.0,
        }
    }

    pub fn hoop_pos(self, is_home_attacking_right: bool) -> Vec2 {
        Vec2::new(
            if is_home_attacking_right {
                self.hoop_right_x_ft
            } else {
                self.hoop_left_x_ft
            },
            self.hoop_y_ft,
        )
    }

    pub fn contains(self, pos: Vec2, margin: f32) -> bool {
        pos.x >= margin
            && pos.x <= self.width_ft - margin
            && pos.y >= margin
            && pos.y <= self.height_ft - margin
    }

    pub fn clamp_playable(self, pos: Vec2, margin: f32) -> Vec2 {
        let margin_x = margin.clamp(0.0, self.width_ft / 2.0);
        let margin_y = margin.clamp(0.0, self.height_ft / 2.0);
        Vec2::new(
            pos.x.clamp(margin_x, self.width_ft - margin_x),
            pos.y.clamp(margin_y, self.height_ft - margin_y),
        )
    }
}

/// Semantic areas used by shot quality, spacing, tactics, and replay views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CourtRegion {
    Rim,
    RestrictedArea,
    Paint,
    LowPost,
    Elbow,
    MidRange,
    CornerThree,
    WingThree,
    TopThree,
    Backcourt,
}

impl CourtGeometry {
    pub fn region(
        self,
        pos: Vec2,
        attacking_right: bool,
        three_point_distance_ft: f32,
    ) -> CourtRegion {
        let hoop = self.hoop_pos(attacking_right);
        let distance = (pos - hoop).length();
        let scale_x = self.width_ft / COURT_WIDTH_FT;
        let scale_y = self.height_ft / COURT_HEIGHT_FT;
        let paint_width = 8.0 * scale_x;
        let paint_depth = 15.0 * scale_y;
        let restricted_radius = 4.0 * scale_x;
        let in_paint =
            (pos.x - hoop.x).abs() <= paint_width && (pos.y - self.hoop_y_ft).abs() <= paint_depth;
        if distance <= restricted_radius {
            CourtRegion::Rim
        } else if in_paint && distance <= paint_width {
            CourtRegion::RestrictedArea
        } else if in_paint {
            CourtRegion::Paint
        } else if distance >= three_point_distance_ft {
            let corner_depth = 8.0 * scale_y;
            let near_sideline = pos.y <= corner_depth || pos.y >= self.height_ft - corner_depth;
            if near_sideline {
                CourtRegion::CornerThree
            } else if (pos.y - self.hoop_y_ft).abs() <= corner_depth {
                CourtRegion::TopThree
            } else {
                CourtRegion::WingThree
            }
        } else {
            CourtRegion::MidRange
        }
    }

    pub fn is_backcourt(self, pos: Vec2, attacking_right: bool) -> bool {
        let midcourt = self.width_ft / 2.0;
        if attacking_right {
            pos.x < midcourt
        } else {
            pos.x > midcourt
        }
    }
}

pub struct Court;

impl Court {
    pub fn norm_to_ft(norm: Vec2) -> Vec2 {
        Self::norm_to_ft_with_geometry(norm, CourtGeometry::default())
    }

    pub fn norm_to_ft_with_geometry(norm: Vec2, geometry: CourtGeometry) -> Vec2 {
        Vec2::new(norm.x * geometry.width_ft, norm.y * geometry.height_ft)
    }

    pub fn ft_to_norm(ft: Vec2) -> Vec2 {
        Self::ft_to_norm_with_geometry(ft, CourtGeometry::default())
    }

    pub fn ft_to_norm_with_geometry(ft: Vec2, geometry: CourtGeometry) -> Vec2 {
        Vec2::new(ft.x / geometry.width_ft, ft.y / geometry.height_ft)
    }

    pub fn hoop_pos(is_home_attacking_right: bool) -> Vec2 {
        CourtGeometry::default().hoop_pos(is_home_attacking_right)
    }

    pub fn hoop_pos_with_geometry(is_home_attacking_right: bool, geometry: CourtGeometry) -> Vec2 {
        geometry.hoop_pos(is_home_attacking_right)
    }

    /// Projects a playable court point onto the nearest boundary line.
    pub fn nearest_boundary(pos: Vec2) -> Vec2 {
        Self::nearest_boundary_with_geometry(pos, CourtGeometry::default())
    }

    pub fn nearest_boundary_with_geometry(pos: Vec2, geometry: CourtGeometry) -> Vec2 {
        let x = pos.x.clamp(0.0, geometry.width_ft);
        let y = pos.y.clamp(0.0, geometry.height_ft);
        let candidates = [
            (x.abs(), Vec2::new(0.0, y)),
            (
                (geometry.width_ft - x).abs(),
                Vec2::new(geometry.width_ft, y),
            ),
            (y.abs(), Vec2::new(x, 0.0)),
            (
                (geometry.height_ft - y).abs(),
                Vec2::new(x, geometry.height_ft),
            ),
        ];
        candidates
            .into_iter()
            .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(_, point)| point)
            .unwrap_or(Vec2::ZERO)
    }

    /// Out-of-bounds release spot for an inbounder, pushed outward from a boundary point.
    pub fn inbound_release_pos(boundary_pos: Vec2, depth_ft: f32) -> Vec2 {
        Self::inbound_release_pos_with_geometry(boundary_pos, depth_ft, CourtGeometry::default())
    }

    pub fn inbound_release_pos_with_geometry(
        boundary_pos: Vec2,
        depth_ft: f32,
        geometry: CourtGeometry,
    ) -> Vec2 {
        let clamped = Vec2::new(
            boundary_pos.x.clamp(0.0, geometry.width_ft),
            boundary_pos.y.clamp(0.0, geometry.height_ft),
        );
        if clamped.x <= 0.0 {
            Vec2::new(-depth_ft, clamped.y)
        } else if clamped.x >= geometry.width_ft {
            Vec2::new(geometry.width_ft + depth_ft, clamped.y)
        } else if clamped.y <= 0.0 {
            Vec2::new(clamped.x, -depth_ft)
        } else {
            Vec2::new(clamped.x, geometry.height_ft + depth_ft)
        }
    }

    /// Whether a point is a legal near-boundary location for an inbound release.
    pub fn is_inbound_release(pos: Vec2, tolerance_ft: f32, geometry: CourtGeometry) -> bool {
        let near_left = (pos.x - 0.0).abs() <= tolerance_ft && pos.y >= -tolerance_ft && pos.y <= geometry.height_ft + tolerance_ft;
        let near_right = (pos.x - geometry.width_ft).abs() <= tolerance_ft && pos.y >= -tolerance_ft && pos.y <= geometry.height_ft + tolerance_ft;
        let near_bottom = (pos.y - 0.0).abs() <= tolerance_ft && pos.x >= -tolerance_ft && pos.x <= geometry.width_ft + tolerance_ft;
        let near_top = (pos.y - geometry.height_ft).abs() <= tolerance_ft && pos.x >= -tolerance_ft && pos.x <= geometry.width_ft + tolerance_ft;
        near_left || near_right || near_bottom || near_top
    }

    /// Free throw line position for the attacking side.
    pub fn free_throw_pos(is_home_attacking_right: bool, rules: &crate::rules::GameRules) -> Vec2 {
        Self::free_throw_pos_with_geometry(
            is_home_attacking_right,
            rules.free_throw_distance_ft,
            rules.court,
        )
    }

    pub fn free_throw_pos_with_geometry(
        is_home_attacking_right: bool,
        distance_ft: f32,
        geometry: CourtGeometry,
    ) -> Vec2 {
        let hoop = geometry.hoop_pos(is_home_attacking_right);
        let outward = if is_home_attacking_right { -1.0 } else { 1.0 };
        Vec2::new(hoop.x + outward * distance_ft, hoop.y)
    }
}
