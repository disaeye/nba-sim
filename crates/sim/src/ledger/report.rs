//! 比赛报告与聚合统计。

use crate::boxscore::BoxScore;
use crate::state::{DecisionRecord, DecisionSample, DecisionTraceSample, PassSample, ShotSample};
use crate::world::{PossessionRecord, TimedFact};

pub struct GameReport {
    pub seed: u64,
    pub home_team: String,
    pub away_team: String,
    pub home_score: u32,
    pub away_score: u32,
    pub periods: u32,
    pub regulation_periods: u32,
    pub period_duration_seconds: f32,
    pub overtime_duration_seconds: f32,
    pub tick_seconds: f32,
    pub ticks: u64,
    /// 决策调度器的回退间隔，供判据计算「掩护后首步」窗口。
    pub redecide_interval_seconds: f64,
    pub open_business_instances: u64,
    pub rebound_attempts: u64,
    /// 有对抗的篮板归属（队下标）。
    pub contested_rebounds: [u32; 2],
    pub possessions: Vec<PossessionRecord>,
    pub box_score: BoxScore,
    pub events: Vec<TimedFact>,
    pub shot_clock_at_release: Vec<f64>,
    pub contest_samples: Vec<f32>,
    pub shot_samples: Vec<ShotSample>,
    pub pass_samples: Vec<PassSample>,
    pub decision_samples: Vec<DecisionSample>,
    /// 逐次决策的因果快照，供决定审计与回归比对。
    pub decision_traces: Vec<DecisionTraceSample>,
    pub decision_records: Vec<DecisionRecord>,
    pub shot_zone_counts: [[u32; 4]; 2],
    pub shot_zone_makes: [[u32; 4]; 2],
    pub creation_counts: [u32; 7],
    pub violations: Vec<String>,
    pub games: u32,
}
