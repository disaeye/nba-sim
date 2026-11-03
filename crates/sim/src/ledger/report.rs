//! 比赛报告与聚合统计。

use crate::boxscore::{BoxScore, PlayerLine};
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

impl GameReport {
    pub fn empty() -> Self {
        Self {
            seed: 0,
            home_team: String::new(),
            away_team: String::new(),
            home_score: 0,
            away_score: 0,
            periods: 0,
            regulation_periods: 0,
            period_duration_seconds: 0.0,
            overtime_duration_seconds: 0.0,
            tick_seconds: 0.0,
            ticks: 0,
            redecide_interval_seconds: 0.0,
            open_business_instances: 0,
            rebound_attempts: 0,
            contested_rebounds: [0, 0],
            possessions: Vec::new(),
            box_score: BoxScore::default(),
            events: Vec::new(),
            shot_clock_at_release: Vec::new(),
            contest_samples: Vec::new(),
            shot_samples: Vec::new(),
            pass_samples: Vec::new(),
            decision_samples: Vec::new(),
            decision_traces: Vec::new(),
            decision_records: Vec::new(),
            shot_zone_counts: [[0; 4]; 2],
            shot_zone_makes: [[0; 4]; 2],
            creation_counts: [0; 7],
            violations: Vec::new(),
            games: 0,
        }
    }

    /// 把另一场的统计并入本报告。
    ///
    /// `keep_streams` 为 false 时不保留事实流与决策快照：分布层只需要箱体、
    /// 回合与聚合计数，过程判据读的是 `events` 与 `decision_samples`。
    /// 判定批次在合并后会把这两类流放掉，中间过程不留副本。
    pub fn merge(&mut self, other: &GameReport) {
        if other.games > 0 && self.games == 0 {
            self.regulation_periods = other.regulation_periods;
            self.period_duration_seconds = other.period_duration_seconds;
            self.overtime_duration_seconds = other.overtime_duration_seconds;
            self.tick_seconds = other.tick_seconds;
        } else if other.games > 0 {
            assert_eq!(self.regulation_periods, other.regulation_periods);
            assert_eq!(self.period_duration_seconds, other.period_duration_seconds);
            assert_eq!(self.overtime_duration_seconds, other.overtime_duration_seconds);
            assert_eq!(self.tick_seconds, other.tick_seconds);
        }
        self.games += other.games;
        self.ticks += other.ticks;
        self.redecide_interval_seconds = other.redecide_interval_seconds;
        self.rebound_attempts += other.rebound_attempts;
        // 未结算实例是单场比赛的流尾性质，聚合后没有对应物，不累加。
        self.open_business_instances = 0;
        self.home_score += other.home_score;
        self.away_score += other.away_score;
        self.possessions.extend_from_slice(&other.possessions);
        for team in [0usize, 1] {
            for zone in 0..4 {
                self.shot_zone_counts[team][zone] += other.shot_zone_counts[team][zone];
                self.shot_zone_makes[team][zone] += other.shot_zone_makes[team][zone];
            }
        }
        for (mine, theirs) in self
            .creation_counts
            .iter_mut()
            .zip(other.creation_counts.iter())
        {
            *mine += theirs;
        }
        self.contest_samples
            .extend_from_slice(&other.contest_samples);
        self.shot_clock_at_release
            .extend_from_slice(&other.shot_clock_at_release);
        self.shot_samples.extend_from_slice(&other.shot_samples);
        self.pass_samples.extend_from_slice(&other.pass_samples);
        self.decision_samples
            .extend_from_slice(&other.decision_samples);
        self.violations.extend_from_slice(&other.violations);
        // 过程判据从事实流聚合，因此聚合报告必须保留事件。
        // 事件里的 `trace` 是局内下标，并入后要按下标偏移重映射，
        // 否则聚合报告的决策快照会指向另一场比赛的另一手动作。
        let offset = self.decision_traces.len() as u32;
        let start = self.events.len();
        self.events.extend_from_slice(&other.events);
        if offset != 0 {
            for event in &mut self.events[start..] {
                if let Some(index) = event.trace {
                    event.trace = Some(index + offset);
                }
            }
        }
        self.decision_traces
            .extend(other.decision_traces.iter().cloned());
        self.decision_records
            .extend(other.decision_records.iter().cloned());
        merge_lines(&mut self.box_score.home, &other.box_score.home);
        merge_lines(&mut self.box_score.away, &other.box_score.away);
    }

    /// 只并入判定要读的量：箱体、回合、聚合计数与事实流。
    ///
    /// 决策快照（`decision_traces` 与 `decision_records`）是逐次决策的完整
    /// 候选集，过程判据读的是 `decision_samples`，所以聚合时不需要把它们
    /// 整批留在堆上。
    pub fn merge_for_gate(&mut self, other: &GameReport) {
        self.merge(other);
        self.decision_traces.clear();
        self.decision_traces.shrink_to_fit();
        self.decision_records.clear();
        self.decision_records.shrink_to_fit();
    }
}

fn merge_lines(mine: &mut Vec<PlayerLine>, theirs: &[PlayerLine]) {
    for line in theirs {
        match mine.iter_mut().find(|existing| existing.id == line.id) {
            Some(existing) => {
                existing.seconds += line.seconds;
                existing.points += line.points;
                existing.fgm += line.fgm;
                existing.fga += line.fga;
                existing.tpm += line.tpm;
                existing.tpa += line.tpa;
                existing.ftm += line.ftm;
                existing.fta += line.fta;
                existing.oreb += line.oreb;
                existing.dreb += line.dreb;
                existing.assists += line.assists;
                existing.steals += line.steals;
                existing.blocks += line.blocks;
                existing.turnovers += line.turnovers;
                existing.fouls += line.fouls;
                existing.shooting_fouls += line.shooting_fouls;
                existing.and_one += line.and_one;
                existing.plus_minus += line.plus_minus;
            }
            None => mine.push(line.clone()),
        }
    }
}
