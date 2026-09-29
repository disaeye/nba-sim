//! 外部 Play-by-Play 数据转换器（M8 扩展位：`docs/quality.md` §2.4 / 历史缺口登记见 `docs/dev/cycles/20260901_historical/status_snapshot.md` §8.4）。
//!
//! 将真实或外部比赛 Play-by-Play (PBP) 事件流聚合统计，生成版本化的 ReferenceDistributions fixture。

use crate::fixture::{Band, OutcomeBands, ReferenceDistributions};
use serde::{Deserialize, Serialize};

/// 单条外部 PBP 事件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PbpEvent {
    pub period: u32,
    pub game_clock: f32,
    pub event_type: String,
    pub possession_id: u32,
    pub team: String,
    pub is_three: Option<bool>,
    pub contest_level: Option<f32>,
    pub duration_seconds: Option<f32>,
    pub pass_count: Option<u32>,
}

/// 聚合统计中间态。
#[derive(Debug, Default)]
struct OutcomeStats {
    durations: Vec<f32>,
    passes: Vec<f32>,
}

impl OutcomeStats {
    fn add(&mut self, dur: f32, passes: u32) {
        self.durations.push(dur);
        self.passes.push(passes as f32);
    }

    fn duration_band(&self, default_min: f32, default_max: f32) -> Band {
        compute_percentile_band(&self.durations, default_min, default_max)
    }

    fn passes_band(&self, default_min: f32, default_max: f32) -> Band {
        compute_percentile_band(&self.passes, default_min, default_max)
    }
}

fn compute_percentile_band(values: &[f32], default_min: f32, default_max: f32) -> Band {
    if values.len() < 3 {
        return Band {
            min: default_min,
            max: default_max,
        };
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let p10_idx = (sorted.len() as f32 * 0.10).floor() as usize;
    let p90_idx = ((sorted.len() as f32 * 0.90).ceil() as usize).min(sorted.len() - 1);
    Band {
        min: (sorted[p10_idx] * 0.9).max(0.0),
        max: (sorted[p90_idx] * 1.1).max(sorted[p10_idx] + 0.1),
    }
}

/// 将 PBP 事件列表转换为 ReferenceDistributions。
pub fn convert_pbp_events_to_fixture(
    events: &[PbpEvent],
    version: &str,
    league: &str,
) -> ReferenceDistributions {
    let mut score_stats = OutcomeStats::default();
    let mut rebound_stats = OutcomeStats::default();
    let mut turnover_stats = OutcomeStats::default();

    let mut total_possessions = 0usize;
    let mut total_turnovers = 0usize;
    let mut total_shots = 0usize;
    let mut three_shots = 0usize;

    for ev in events {
        let dur = ev.duration_seconds.unwrap_or(15.0);
        let passes = ev.pass_count.unwrap_or(2);

        let ev_upper = ev.event_type.to_ascii_uppercase();
        if ev_upper == "SCORE" || ev_upper == "MADE_SHOT" {
            total_possessions += 1;
            score_stats.add(dur, passes);
        } else if ev_upper.contains("TURNOVER") {
            total_possessions += 1;
            total_turnovers += 1;
            turnover_stats.add(dur, passes);
        } else if ev_upper.contains("REBOUND") || ev_upper == "DEFENSIVE_REBOUND" {
            total_possessions += 1;
            rebound_stats.add(dur, passes);
        }

        if ev_upper.contains("SHOT") || ev_upper == "SCORE" {
            total_shots += 1;
            if ev.is_three.unwrap_or(false) {
                three_shots += 1;
            }
        }
    }

    let is_fiba = league.eq_ignore_ascii_case("FIBA");
    let (court_w, court_h) = if is_fiba {
        (91.86, 49.21)
    } else {
        (94.0, 50.0)
    };

    let turnover_rate = if total_possessions > 0 {
        total_turnovers as f32 / total_possessions as f32
    } else {
        0.13
    };

    let three_rate = if total_shots > 0 {
        three_shots as f32 / total_shots as f32
    } else {
        0.39
    };

    let duration_bands = OutcomeBands {
        score: score_stats.duration_band(4.0, 24.0),
        rebound: rebound_stats.duration_band(6.0, 24.0),
        turnover: turnover_stats.duration_band(3.0, 24.0),
    };

    let passes_bands = OutcomeBands {
        score: score_stats.passes_band(0.0, 7.0),
        rebound: rebound_stats.passes_band(0.0, 6.0),
        turnover: turnover_stats.passes_band(0.0, 5.0),
    };

    let base_template = if is_fiba {
        ReferenceDistributions::fiba_v1()
    } else {
        ReferenceDistributions::nba_v1()
    };

    ReferenceDistributions {
        version: version.to_string(),
        league: league.to_string(),
        court_width_ft: court_w,
        court_height_ft: court_h,
        pass_corridor_radius_ft: base_template.pass_corridor_radius_ft,
        // 时钟政策与联赛模板一致；PBP 事件不携带时钟规则。
        offensive_rebound_shot_clock_seconds: base_template.offensive_rebound_shot_clock_seconds,
        duration_tolerance_seconds: base_template.duration_tolerance_seconds,
        duration_bands,
        passes_bands,
        turnover_rate_band: Band {
            min: (turnover_rate * 0.7).max(0.05),
            max: (turnover_rate * 1.3).min(0.35),
        },
        three_attempt_rate_band: Band {
            min: (three_rate * 0.75).max(0.20),
            max: (three_rate * 1.25).min(0.60),
        },
        heavy_contest_threshold: base_template.heavy_contest_threshold,
        phase_dwell_max_seconds: base_template.phase_dwell_max_seconds,
        phase_transitions: base_template.phase_transitions,
        // PBP 转换产物当前不携带构成带标定（provenance: pbp 的构成带
        // 标定是 dev 方案 §5.2 的后续项），缺省即判 NotApplicable。
        composition_bands: None,
        joint_situational_bands: None,
    }
}
