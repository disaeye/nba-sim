//! 流内事件载荷的松散镜像与出手分区事实辅助（D29 拆分自 `lib.rs`）。
//!
//! 这些类型与函数只取评判所需字段：事件载荷镜像结构体（`*Data`）、
//! 四区分区索引与按生产 `CourtGeometry` 的分区判定、第四节晚段/早段
//! 三分出手计数、出手结果的因果归属消解。`lib.rs` 的回合级与比赛级
//! 评判从这里读取，不再各自内联。

use nba_domain::court::ShotZone;
use nba_protocol::StreamTick;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use crate::fixture::ReferenceDistributions;

// ---- 流内事件载荷的松散镜像（只取评判所需字段）----
#[derive(Deserialize)]
pub(crate) struct PassReleaseData {
    pub passer_id: String,
    pub receiver_id: String,
    pub from_pos: (f32, f32),
    pub to_pos: (f32, f32),
}
#[derive(Deserialize)]
pub(crate) struct PassReceivedData {
    pub receiver_id: String,
    pub position: (f32, f32),
}
#[derive(Deserialize)]
pub(crate) struct LooseBallSecuredData {
    pub player_id: String,
}
#[derive(Deserialize)]
pub(crate) struct PassInterceptedData {
    pub passer_id: String,
    pub receiver_id: String,
    pub defender_id: String,
    pub position: (f32, f32),
}
#[derive(Deserialize)]
pub(crate) struct ShotReleaseData {
    pub shooter_id: String,
    pub is_three: bool,
    pub contest_level: f32,
}
#[derive(Deserialize)]
pub(crate) struct FreeThrowData {
    pub made: bool,
}
#[derive(Deserialize)]
pub(crate) struct PossessionSummaryData {
    pub possession_index: u64,
    pub duration_seconds: f32,
    pub passes_count: u32,
    pub terminal_event: nba_domain::PossessionEndCause,
    #[serde(default)]
    pub shooter_id: Option<String>,
    #[serde(default)]
    pub shot_contest_intensity: Option<f32>,
    #[serde(default)]
    pub turnover_player_id: Option<String>,
}

/// 四区数组下标（Rim/Near/Mid/Three）。
pub(crate) fn zone_index(zone: ShotZone) -> usize {
    match zone {
        ShotZone::Rim => 0,
        ShotZone::Near => 1,
        ShotZone::Mid => 2,
        ShotZone::Three => 3,
    }
}

/// 出手分区判定：`is_three` 优先保留事件声明；两分区按生产
/// `CourtGeometry::shot_zone` 几何（nba.v3 存在时用其分区阈值）。
pub(crate) fn shot_zone(
    x: f32,
    y: f32,
    team: &str,
    is_three: bool,
    tick: &StreamTick,
    fixture: &ReferenceDistributions,
) -> ShotZone {
    let attacking_right = team == "home";
    let pos = glam::Vec2::new(x, y);
    if is_three {
        ShotZone::Three
    } else {
        nba_domain::CourtGeometry {
            width_ft: tick.frame.rules.court_width_ft,
            height_ft: tick.frame.rules.court_height_ft,
            hoop_left_x_ft: tick.frame.rules.hoop_left_x_ft,
            hoop_right_x_ft: tick.frame.rules.hoop_right_x_ft,
            hoop_y_ft: tick.frame.rules.hoop_y_ft,
        }
        .shot_zone(
            pos,
            attacking_right,
            fixture
                .joint_situational_bands
                .as_ref()
                .map_or(tick.frame.rules.three_point_distance_ft, |bands| {
                    bands.shot_zone_make.three_point_distance_ft
                }),
            fixture
                .joint_situational_bands
                .as_ref()
                .map_or(22.0, |bands| bands.shot_zone_make.corner_three_distance_ft),
        )
    }
}

/// 第四节晚段/早段出手计数（加时排除；调节钟窗口取 fixture，缺省 300s）。
pub(crate) fn q4_count_shot(
    tick: &StreamTick,
    fixture: &ReferenceDistributions,
    is_three: bool,
    late_fga: &mut usize,
    late_three: &mut usize,
    early_fga: &mut usize,
    early_three: &mut usize,
) {
    if tick.frame.period != 4 {
        return;
    }
    let regulation_window = fixture
        .joint_situational_bands
        .as_ref()
        .map_or(300.0, |bands| {
            bands.q4_late_three_attempt_share.late_window_seconds
        });
    if tick.game_clock <= regulation_window {
        *late_fga += 1;
        *late_three += usize::from(is_three);
    } else {
        *early_fga += 1;
        *early_three += usize::from(is_three);
    }
}

/// 消解一次出手结果：重复结果、缺失记录或二次消解都把证据置为不完整。
pub(crate) fn resolve_shot_outcome(
    release_id: u64,
    shot_events: &mut HashMap<u64, (usize, bool)>,
    duplicates: &HashSet<u64>,
    evidence_complete: &mut bool,
) {
    if duplicates.contains(&release_id) {
        *evidence_complete = false;
        return;
    }
    let Some((_, resolved)) = shot_events.get_mut(&release_id) else {
        *evidence_complete = false;
        return;
    };
    if *resolved {
        *evidence_complete = false;
        return;
    }
    *resolved = true;
}
