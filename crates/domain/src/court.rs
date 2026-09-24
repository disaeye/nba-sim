use glam::Vec2;
use serde::{Deserialize, Serialize};

pub const COURT_WIDTH_FT: f32 = 94.0;
pub const COURT_HEIGHT_FT: f32 = 50.0;
/// 底角区域深度（距边线，ft）：该区域内三分线是直线而非圆弧。
/// NBA 真实值为 3 ft —— 底角线距边线 3 ft 且平行于边线，
/// 其最近点距篮筐 22 ft（即「底角 22 ft」的来源）。
pub const CORNER_ZONE_DEPTH_FT: f32 = 3.0;
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

    /// 是否为三分出手（含底角特例）。
    ///
    /// NBA/FIBA 的三分线不是等半径圆弧：弧顶与翼位是 `three_point_distance_ft`，
    /// 但**底角区域是一条更近的直线**（NBA 底角 22 ft、弧顶 23.75 ft）。此前
    /// 引擎在 4 处直接用 `dist >= three_point_distance_ft` 判定，导致底角
    /// 三分被误判成两分——这是"出手构成失真"的一个独立成因（本轮实测：
    /// 档案声明的 CornerSpacer 站在 21.2–21.5 ft，被当成 2PT）。
    ///
    /// `corner_three_distance_ft` 由联赛档案给出；若为 0 则退化为等半径圆弧。
    pub fn is_three_point_attempt(
        self,
        pos: Vec2,
        attacking_right: bool,
        three_point_distance_ft: f32,
        corner_three_distance_ft: f32,
    ) -> bool {
        let hoop = self.hoop_pos(attacking_right);
        let distance = (pos - hoop).length();
        if distance >= three_point_distance_ft {
            return true;
        }
        // 底角：靠近边线且 x 位于篮筐与底线之间时才适用更近的直线距离。
        if corner_three_distance_ft <= 0.0 {
            return false;
        }
        let corner_depth = self.corner_zone_depth_ft();
        let near_sideline = pos.y <= corner_depth || pos.y >= self.height_ft - corner_depth;
        // 仅限进攻半场：底角线不延伸到后场。
        let in_attacking_half = if attacking_right {
            pos.x >= self.width_ft / f32::from(2u8)
        } else {
            pos.x <= self.width_ft / f32::from(2u8)
        };
        near_sideline && in_attacking_half && distance >= corner_three_distance_ft
    }

    /// 底角区域深度（距边线），与 [`Self::region`] 使用同一几何定义。
    pub fn corner_zone_depth_ft(self) -> f32 {
        let scale_y = self.height_ft / COURT_HEIGHT_FT;
        CORNER_ZONE_DEPTH_FT * scale_y
    }

    /// 球场几何中心（placement 搜索、站位对称计算使用）。
    pub fn center(self) -> Vec2 {
        let half = f32::from(2u8);
        Vec2::new(self.width_ft / half, self.height_ft / half)
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

/// 统一出手分区（attributes.md §2.3a）：按**出手点**把一次运动战出手归入
/// 恰好一个区域，决策（候选生成/效用选技）、裁决（命中基准与技能选择）、
/// 事件统计与评判共用同一判定，禁止各子系统自定边界。
///
/// 边界（ft，均按出手点到进攻篮筐的距离）:
/// - `Rim`   距篮 < 5
/// - `Near`  5 ≤ 距篮 < 14
/// - `Mid`   距篮 ≥ 14 且在三分线内（含底角特例几何）
/// - `Three` 三分线外；三分判定优先于距离分区
///
/// `pending_shot_release` 释放时刻的实际球位可能与出手意图点不同，
/// 因此引擎与评判器一律使用「释放时刻出手点」为分类事实。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShotZone {
    Rim,
    Near,
    Mid,
    Three,
}

impl ShotZone {
    /// 展示/事件流中的稳定字符串口径。
    pub fn as_str(self) -> &'static str {
        match self {
            ShotZone::Rim => "Rim",
            ShotZone::Near => "Near",
            ShotZone::Mid => "Mid",
            ShotZone::Three => "Three",
        }
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

/// 统一出手分区的距离阈值（ft，attributes.md §2.3a）。
pub const RIM_ZONE_MAX_DIST_FT: f32 = 5.0;
pub const NEAR_ZONE_MAX_DIST_FT: f32 = 14.0;

impl CourtGeometry {
    /// 统一出手分区判定：先判三分（含底角特例），再按距离分 Rim/Near/Mid。
    ///
    /// 所有环节（决策效用、命中裁决、事件统计、评判）必须调用本函数，
    /// 不得各自内联 `dist < 8` 类判定（attributes.md §2.3a 的单一事实源）。
    pub fn shot_zone(
        self,
        pos: Vec2,
        attacking_right: bool,
        three_point_distance_ft: f32,
        corner_three_distance_ft: f32,
    ) -> ShotZone {
        if self.is_three_point_attempt(
            pos,
            attacking_right,
            three_point_distance_ft,
            corner_three_distance_ft,
        ) {
            return ShotZone::Three;
        }
        let distance = (pos - self.hoop_pos(attacking_right)).length();
        if distance < RIM_ZONE_MAX_DIST_FT {
            ShotZone::Rim
        } else if distance < NEAR_ZONE_MAX_DIST_FT {
            ShotZone::Near
        } else {
            ShotZone::Mid
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

    /// Projects a court position to the defensive baseline of the team being awarded possession.
    pub fn defensive_baseline_with_geometry(
        is_home_possession: bool,
        pos: Vec2,
        geometry: CourtGeometry,
    ) -> Vec2 {
        let baseline_x = if is_home_possession {
            0.0
        } else {
            geometry.width_ft
        };
        let y = pos.y.clamp(0.0, geometry.height_ft);
        Vec2::new(baseline_x, y)
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
        let near_left = (pos.x - 0.0).abs() <= tolerance_ft
            && pos.y >= -tolerance_ft
            && pos.y <= geometry.height_ft + tolerance_ft;
        let near_right = (pos.x - geometry.width_ft).abs() <= tolerance_ft
            && pos.y >= -tolerance_ft
            && pos.y <= geometry.height_ft + tolerance_ft;
        let near_bottom = (pos.y - 0.0).abs() <= tolerance_ft
            && pos.x >= -tolerance_ft
            && pos.x <= geometry.width_ft + tolerance_ft;
        let near_top = (pos.y - geometry.height_ft).abs() <= tolerance_ft
            && pos.x >= -tolerance_ft
            && pos.x <= geometry.width_ft + tolerance_ft;
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
