//! 四层判定：几何合法性、语境合法性、分布比较、反事实。
//!
//! 前两层在结算时逐动作记录，后两层对批次统计做比较。聚合统计只作为投影
//! 呈现，判定单元始终是动作与回合。

use serde::{Deserialize, Serialize};

use crate::action::{DefenderRoute, Fact};
use crate::params::Params;
use crate::world::GameReport;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: String,
    pub criterion: String,
    pub observed: f64,
    pub target: f64,
    /// 相对误差 `observed / target - 1`；分布层的判定量与工件记录同一份。
    #[serde(default)]
    pub relative_error: f64,
    pub passed: bool,
    pub detail: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GateReport {
    pub geometric_violations: usize,
    pub context_violations: usize,
    pub layers: Vec<LayerResult>,
    pub passed: bool,
}

impl GateReport {
    pub fn failed(&self) -> usize {
        self.layers.iter().filter(|layer| !layer.passed).count()
    }
}

/// 设计边界的方向：上限或下限。
enum Bound {
    Ceiling,
    Floor,
}

fn push_process(gate: &mut GateReport, criterion: &str, observed: f64, bound: f64, kind: Bound) {
    let passed = match kind {
        Bound::Ceiling => observed <= bound,
        Bound::Floor => observed >= bound,
    };
    gate.layers.push(LayerResult {
        layer: "process".to_string(),
        criterion: criterion.to_string(),
        observed,
        target: bound,
        relative_error: (observed - bound) / bound.max(f64::MIN_POSITIVE),
        passed,
        detail: match kind {
            Bound::Ceiling => format!("上限 {bound:.4}"),
            Bound::Floor => format!("下限 {bound:.4}"),
        },
    });
}

fn push_process_range(gate: &mut GateReport, criterion: &str, observed: f64, low: f64, high: f64) {
    gate.layers.push(LayerResult {
        layer: "process".to_string(),
        criterion: criterion.to_string(),
        observed,
        target: high,
        relative_error: (observed - high) / high.max(f64::MIN_POSITIVE),
        passed: observed >= low && observed <= high,
        detail: format!("区间 {low:.4}..{high:.4}"),
    });
}

/// 第三层：把批次分布与校准表比较。带宽取真实赛季的采样波动，
/// 相对误差超过 15% 记为不通过。
pub fn evaluate(report: &GameReport, params: &Params) -> GateReport {
    let shots: u32 = report.shot_zone_counts.iter().flatten().sum();
    let mut gate = GateReport {
        geometric_violations: report
            .violations
            .iter()
            .filter(|violation| violation.starts_with("geometry:"))
            .count(),
        context_violations: report
            .violations
            .iter()
            .filter(|violation| violation.starts_with("context:"))
            .count(),
        ..Default::default()
    };

    let bands = &params.policy;
    let mut push = |criterion: &str, observed: f64, target: f64| {
        let band = bands.band(criterion);
        let error = if target > 0.0 {
            observed / target - 1.0
        } else {
            0.0
        };
        let passed = target > 0.0 && error >= band.lower && error <= band.upper;
        gate.layers.push(LayerResult {
            layer: "distribution".to_string(),
            criterion: criterion.to_string(),
            observed,
            target,
            relative_error: error,
            passed,
            detail: format!(
                "relative error {error:.3} (band {:.3}..{:.3})",
                band.lower, band.upper
            ),
        });
    };

    if shots > 0 {
        let zone_names = ["rim", "near", "mid", "three"];
        for (index, name) in zone_names.iter().enumerate() {
            let target = *params.shot_profile.zone_share.get(*name).unwrap_or(&0.0) as f64;
            let observed: u32 = report.shot_zone_counts.iter().map(|team| team[index]).sum();
            push(
                &format!("zone_share_{name}"),
                observed as f64 / shots as f64,
                target,
            );
        }
        let creation_names = [
            "drive",
            "cut",
            "post",
            "pull_up",
            "putback",
            "transition",
            "set",
        ];
        for (index, name) in creation_names.iter().enumerate() {
            let target = *params
                .shot_profile
                .creation_share
                .get(*name)
                .unwrap_or(&0.0) as f64;
            push(
                &format!("creation_share_{name}"),
                report.creation_counts[index] as f64 / shots as f64,
                target,
            );
        }
    }

    let possessions = report.possessions.len().max(1) as f64;
    let games = report.games.max(1) as f64;
    let turnovers = report
        .possessions
        .iter()
        .filter(|record| record.ended_by.starts_with("turnover"))
        .count() as f64;
    push(
        "turnover_per_possession",
        turnovers / possessions,
        params.possession.league.turnover_per_possession as f64,
    );

    let league = &params.possession.league;
    let fga = report.box_score.team_totals(crate::TeamSide::Home).fga
        + report.box_score.team_totals(crate::TeamSide::Away).fga;
    push(
        "fga_per_possession",
        fga as f64 / possessions,
        league.fga_per_possession as f64,
    );
    let fta = report.box_score.team_totals(crate::TeamSide::Home).fta
        + report.box_score.team_totals(crate::TeamSide::Away).fta;
    push(
        "fta_per_game",
        fta as f64 / games,
        league.fta_per_game as f64,
    );
    let points = (report.home_score + report.away_score) as f64;
    push(
        "points_per_game",
        points / games,
        league.points_per_game as f64,
    );
    let oreb = report.box_score.team_totals(crate::TeamSide::Home).oreb
        + report.box_score.team_totals(crate::TeamSide::Away).oreb;
    // 基准按「真正走到篮板的投失」计算：犯规导致的投失不产生篮板，
    // 用 FGA−FGM 做分母会把犯规率的变化误算成篮板能力的变化。
    let reboundable_misses = report.rebound_attempts as f64 / games;
    push(
        "oreb_per_game",
        oreb as f64 / games,
        league.oreb_per_miss as f64 * reboundable_misses,
    );
    push(
        "possessions_per_game",
        possessions / games,
        league.possessions_per_game as f64,
    );
    let home_line = report.box_score.team_totals(crate::TeamSide::Home);
    let away_line = report.box_score.team_totals(crate::TeamSide::Away);
    push(
        "blocks_per_game",
        (home_line.blocks + away_line.blocks) as f64 / games,
        league.blocks_per_game as f64,
    );
    push(
        "steals_per_game",
        (home_line.steals + away_line.steals) as f64 / games,
        league.steals_per_game as f64,
    );
    push(
        "shooting_fouls_per_game",
        (home_line.shooting_fouls + away_line.shooting_fouls) as f64 / games,
        league.shooting_fouls_per_game as f64,
    );
    push(
        "and_one_per_game",
        (home_line.and_one + away_line.and_one) as f64 / games,
        league.and_one_per_game as f64,
    );

    push_process_layers(&mut gate, report, params);

    gate.passed = gate.geometric_violations == 0
        && gate.context_violations == 0
        && gate.layers.iter().all(|layer| layer.passed);
    gate
}

/// 过程级判据。前八条用 `policy.process` 的设计边界判定；最后三条是结构性
/// 检查，分档计数必须非零，份额本身写入工件。等级目标需要外部参考分布。
fn push_process_layers(gate: &mut GateReport, report: &GameReport, params: &Params) {
    let p = |key: &str| params.policy.p(key) as f64;

    let shot_total = report.shot_samples.len().max(1) as f64;
    let contested = p("contested_threshold") as f32;
    let early_contested = report
        .shot_samples
        .iter()
        .filter(|sample| {
            sample.shot_clock > p("early_clock_seconds")
                && sample.contest > contested
                && (sample.zone == 2 || sample.zone == 3)
        })
        .count() as f64
        / shot_total;
    push_process(
        gate,
        "early_contested_rate",
        early_contested,
        p("early_contested_rate_max"),
        Bound::Ceiling,
    );

    let mut usage: Vec<f64> = report
        .box_score
        .home
        .iter()
        .map(|line| line.fga as f64 + p("usage_fta_weight") * line.fta as f64)
        .collect();
    usage.sort_by(|left, right| right.partial_cmp(left).expect("finite usage"));
    let usage_ratio = if usage.len() >= 5 {
        usage[0] / usage[4].max(f64::MIN_POSITIVE)
    } else {
        f64::MIN_POSITIVE
    };
    push_process(
        gate,
        "usage_top1_over_fifth",
        usage_ratio,
        p("usage_top1_over_fifth_min"),
        Bound::Floor,
    );

    let home_lines = report.box_score.home.len();
    let mut made = vec![0u32; home_lines];
    let mut assisted = vec![0u32; home_lines];
    for event in &report.events {
        if let Fact::Shot {
            shooter,
            made: true,
            assisted_by,
            ..
        } = event.fact
        {
            if shooter < home_lines {
                made[shooter] += 1;
                if assisted_by.is_some() {
                    assisted[shooter] += 1;
                }
            }
        }
    }
    let rates: Vec<f64> = made
        .iter()
        .zip(assisted.iter())
        .filter(|(made, _)| **made > 0)
        .map(|(made, assisted)| *assisted as f64 / *made as f64)
        .collect();
    let spread = rates.iter().copied().fold(f64::NEG_INFINITY, f64::max)
        - rates.iter().copied().fold(f64::INFINITY, f64::min);
    push_process(
        gate,
        "assisted_fg_spread",
        spread,
        p("assisted_fg_spread_min"),
        Bound::Floor,
    );

    let mut pairs = 0usize;
    let mut ping_pong = 0usize;
    for window in report.pass_samples.windows(2) {
        let first = window[0];
        let second = window[1];
        if first.possession != second.possession {
            continue;
        }
        pairs += 1;
        if second.from == first.to && second.to == first.from {
            let move_ft = p("ping_pong_move_ft") as f32;
            let first_move = (second.from_pos - first.from_pos).length();
            let second_move = (second.to_pos - first.to_pos).length();
            if first_move < move_ft && second_move < move_ft {
                ping_pong += 1;
            }
        }
    }
    let ping_pong_rate = ping_pong as f64 / pairs.max(1) as f64;
    push_process(
        gate,
        "ping_pong_rate",
        ping_pong_rate,
        p("ping_pong_rate_max"),
        Bound::Ceiling,
    );

    // 路线响应只算掩护后的第一个决策步，且限三分能力达标的持球人。
    let first_step = crate::world::DECISION_INTERVAL + crate::world::TICK_SECONDS;
    let capable = p("screen_read_shooter_min") as f32;
    let under_total = report
        .decision_samples
        .iter()
        .filter(|sample| {
            matches!(sample.route, Some(DefenderRoute::Under))
                && sample.since_screen <= first_step
                && sample.shooter_three >= capable
        })
        .count() as f64;
    let under_shoot = report
        .decision_samples
        .iter()
        .filter(|sample| {
            matches!(sample.route, Some(DefenderRoute::Under))
                && sample.since_screen <= first_step
                && sample.shooter_three >= capable
                && sample.family == 0
        })
        .count() as f64;
    let under_rate = under_shoot / under_total.max(f64::MIN_POSITIVE);
    let under_bound = p("screen_read_under_shoot_min");
    gate.layers.push(LayerResult {
        layer: "process".to_string(),
        criterion: "screen_read_under_shoot".to_string(),
        observed: under_rate,
        target: under_bound,
        relative_error: (under_rate - under_bound) / under_bound.max(f64::MIN_POSITIVE),
        passed: under_rate >= under_bound,
        detail: format!(
            "{under_shoot:.0}/{under_total:.0} 掩护后首步且三分能力达标"
        ),
    });

    let over_total = report
        .decision_samples
        .iter()
        .filter(|sample| {
            matches!(sample.route, Some(DefenderRoute::Over))
                && sample.since_screen <= first_step
                && sample.shooter_three >= capable
        })
        .count() as f64;
    let over_drive = report
        .decision_samples
        .iter()
        .filter(|sample| {
            matches!(sample.route, Some(DefenderRoute::Over))
                && sample.since_screen <= first_step
                && sample.shooter_three >= capable
                && (sample.family == 1 || sample.family == 7)
        })
        .count() as f64;
    let over_rate = over_drive / over_total.max(f64::MIN_POSITIVE);
    let mut over_histogram = [0usize; 9];
    for sample in report.decision_samples.iter().filter(|sample| {
        matches!(sample.route, Some(DefenderRoute::Over))
            && sample.since_screen <= first_step
            && sample.shooter_three >= capable
    }) {
        over_histogram[sample.family as usize] += 1;
    }
    let over_bound = p("screen_read_over_drive_min");
    gate.layers.push(LayerResult {
        layer: "process".to_string(),
        criterion: "screen_read_over_drive".to_string(),
        observed: over_rate,
        target: over_bound,
        relative_error: (over_rate - over_bound) / over_bound.max(f64::MIN_POSITIVE),
        passed: over_rate >= over_bound,
        detail: format!(
            "{over_drive:.0}/{over_total:.0} 掩护后首步且三分能力达标；族分布 {over_histogram:?}"
        ),
    });

    let wall_minutes = report.ticks as f64 * crate::world::TICK_SECONDS / p("seconds_per_minute");
    let max_minutes = report
        .box_score
        .home
        .iter()
        .chain(report.box_score.away.iter())
        .map(|line| line.minutes())
        .fold(f64::NEG_INFINITY, f64::max);
    push_process(
        gate,
        "minutes_max",
        max_minutes,
        wall_minutes,
        Bound::Ceiling,
    );

    let mut clock_buckets = [0u32; 4];
    for sample in &report.shot_samples {
        let index = if sample.shot_clock > p("clock_edge_first") {
            0
        } else if sample.shot_clock > p("clock_edge_second") {
            1
        } else if sample.shot_clock > p("clock_edge_third") {
            2
        } else {
            3
        };
        clock_buckets[index] += 1;
    }
    push_process(
        gate,
        "shot_clock_buckets_complete",
        clock_buckets.iter().copied().min().unwrap_or(0) as f64,
        p("bucket_presence_min"),
        Bound::Floor,
    );

    let mut distance_buckets = [0u32; 3];
    for sample in &report.shot_samples {
        let index = if sample.defender_ft < p("defender_tight_ft") as f32 {
            0
        } else if sample.defender_ft < p("defender_open_ft") as f32 {
            1
        } else {
            2
        };
        distance_buckets[index] += 1;
    }
    push_process(
        gate,
        "defender_distance_buckets_complete",
        distance_buckets.iter().copied().min().unwrap_or(0) as f64,
        p("bucket_presence_min"),
        Bound::Floor,
    );

    let possessions = report.possessions.len().max(1) as f64;
    let mut bucket_points = [f64::default(); 3];
    let mut bucket_shots = [f64::default(); 3];
    for sample in &report.shot_samples {
        let index = if sample.shot_clock > p("clock_edge_first") {
            0
        } else if sample.shot_clock > p("clock_edge_third") {
            1
        } else {
            2
        };
        bucket_points[index] += sample.points as f64;
        bucket_shots[index] += f64::from(1);
    }
    let means: Vec<f64> = (0..3)
        .map(|index| bucket_points[index] / bucket_shots[index].max(f64::MIN_POSITIVE))
        .collect();
    let monotonic = means[0] > means[1] && means[1] > means[2];
    gate.layers.push(LayerResult {
        layer: "process".to_string(),
        criterion: "shot_points_by_clock".to_string(),
        observed: means[0],
        target: means[2],
        relative_error: (means[0] - means[2]) / means[2].max(f64::MIN_POSITIVE),
        passed: monotonic,
        detail: format!(
            "24-18 {:.3} / 17-7 {:.3} / <6 {:.3}",
            means[0], means[1], means[2]
        ),
    });

    let mut post_fga = f64::default();
    let mut post_tpa = f64::default();
    let mut shooter_fga = f64::default();
    let mut shooter_tpa = f64::default();
    let post_ceiling = p("post_scorer_preference_max") as f32;
    let shooter_floor = p("stationary_shooter_preference_min") as f32;
    for line in report
        .box_score
        .home
        .iter()
        .chain(report.box_score.away.iter())
    {
        if line.three_point_preference <= post_ceiling {
            post_fga += line.fga as f64;
            post_tpa += line.tpa as f64;
        }
        if line.three_point_preference >= shooter_floor {
            shooter_fga += line.fga as f64;
            shooter_tpa += line.tpa as f64;
        }
    }
    push_process(
        gate,
        "post_scorer_three_share",
        post_tpa / post_fga.max(f64::MIN_POSITIVE),
        p("post_scorer_three_share_max"),
        Bound::Ceiling,
    );
    push_process(
        gate,
        "stationary_shooter_three_share",
        shooter_tpa / shooter_fga.max(f64::MIN_POSITIVE),
        p("stationary_shooter_three_share_min"),
        Bound::Floor,
    );

    let mut activations = 0u32;
    let mut endings = 0u32;
    for event in &report.events {
        match &event.fact {
            Fact::PlayActivated { .. } => activations += 1,
            Fact::PlayWindowEnded { .. } => endings += 1,
            _ => {}
        }
    }
    // 每个活球回合最多叫一次战术，所以激活数除以活球回合数就是覆盖率。
    // 回合 id 逐场重置，批次合并后不能拿 id 做匹配。
    let halfcourt = report
        .possessions
        .iter()
        .filter(|record| record.kind == crate::state::PossessionKind::Live)
        .count();
    push_process_range(
        gate,
        "play_activation_rate",
        activations as f64 / (halfcourt as f64).max(f64::MIN_POSITIVE),
        p("play_activation_rate_min"),
        p("play_activation_rate_max"),
    );
    push_process(
        gate,
        "play_window_gap",
        (activations as i64 - endings as i64).unsigned_abs() as f64,
        p("play_window_gap_max"),
        Bound::Ceiling,
    );

    let games = (report.games as f64).max(f64::MIN_POSITIVE);
    let substitutions = report
        .events
        .iter()
        .filter(|event| matches!(&event.fact, Fact::Substitution { .. }))
        .count() as f64;
    push_process(
        gate,
        "substitutions_per_game",
        substitutions / games,
        p("substitutions_per_game_min"),
        Bound::Floor,
    );

    let pass_total: u32 = report.possessions.iter().map(|record| record.passes).sum();
    let pass_mean = pass_total as f64 / possessions;
    push_process_range(
        gate,
        "pass_chain_mean",
        pass_mean,
        p("pass_chain_mean_min"),
        p("pass_chain_mean_max"),
    );
}

/// 多场聚合后再判分布，避免单场采样噪声把判定打穿。
pub fn evaluate_batch(reports: &[GameReport], params: &Params) -> GateReport {
    let mut merged = GameReport::empty();
    for report in reports {
        merged.merge(report);
    }
    evaluate(&merged, params)
}
