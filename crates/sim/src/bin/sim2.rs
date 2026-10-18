use std::path::PathBuf;

use nba_sim_core::{run_game, summarize, TeamSide};

fn main() {
    let mut seeds: Vec<u64> = Vec::new();
    let mut calibration = PathBuf::from(nba_sim_core::CALIBRATION_DIR);
    let mut verbose = false;
    let mut json = false;
    let mut gate = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--json" => json = true,
            "--gate" => gate = true,
            "--seeds" => {
                let value = args.next().expect("--seeds requires a range like 1..10");
                let (start, end) = value
                    .split_once("..")
                    .expect("seed range must be `start..end`");
                let start: u64 = start.parse().expect("seed start must be a number");
                let end: u64 = end.parse().expect("seed end must be a number");
                seeds.extend(start..end);
            }
            "--calibration" => {
                calibration = PathBuf::from(args.next().expect("--calibration requires a path"));
            }
            "--verbose" => verbose = true,
            other => {
                seeds.push(other.parse().expect("seed must be a number"));
            }
        }
    }
    if seeds.is_empty() {
        seeds.push(42);
    }

    println!(
        "seed  score      poss  FGA  3PA   FTA  TOV  OREB  2P%    3P%    FT%"
    );
    let mut totals = Vec::new();
    let mut metrics = Metrics::default();
    for seed in seeds {
        let report = run_game(seed, &calibration);
        metrics.add(&report);
        let summary = summarize(&report);
        let home = report.box_score.team_totals(TeamSide::Home);
        let away = report.box_score.team_totals(TeamSide::Away);
        println!(
            "{:5} {:3}-{:<3}  {:5} {:5} {:4} {:5} {:4} {:5}  {:.3}  {:.3}  {:.3}",
            summary.seed,
            summary.home_score,
            summary.away_score,
            summary.possessions,
            summary.fga,
            summary.fg3a,
            summary.fta,
            summary.turnovers,
            summary.oreb,
            summary.fg2_pct,
            summary.fg3_pct,
            summary.ft_pct
        );
        if verbose {
            print_box(&report, home, away);
            report_distributions(&report);
        }
        totals.push(summary);
    }

    if totals.len() > 1 {
        let count = totals.len() as f32;
        let mean = |values: Vec<f32>| values.iter().sum::<f32>() / count;
        println!();
        println!(
            "mean  {:.1} pts  {:.1} poss  {:.1} FGA  {:.1} 3PA  {:.1} FTA  {:.1} TOV  {:.1} OREB  2P% {:.3}  3P% {:.3}  FT% {:.3}",
            mean(totals.iter().map(|s| (s.home_score + s.away_score) as f32).collect()),
            mean(totals.iter().map(|s| s.possessions as f32).collect()),
            mean(totals.iter().map(|s| s.fga as f32).collect()),
            mean(totals.iter().map(|s| s.fg3a as f32).collect()),
            mean(totals.iter().map(|s| s.fta as f32).collect()),
            mean(totals.iter().map(|s| s.turnovers as f32).collect()),
            mean(totals.iter().map(|s| s.oreb as f32).collect()),
            mean(totals.iter().map(|s| s.fg2_pct).collect()),
            mean(totals.iter().map(|s| s.fg3_pct).collect()),
            mean(totals.iter().map(|s| s.ft_pct).collect()),
        );
    }
    if json {
        print_metrics(&metrics);
    }
    if gate {
        let params = nba_sim_core::Params::load(&calibration).expect("calibration tables must load");
        let mut reports = Vec::new();
        for seed in 1..=16u64 {
            reports.push(run_game(seed, &calibration));
        }
        let report = nba_sim_core::gate::evaluate_batch(&reports, &params);
        let mut samples: Vec<String> = Vec::new();
        for game in &reports {
            for violation in &game.violations {
                if samples.len() < 8 && !samples.contains(violation) {
                    samples.push(violation.clone());
                }
            }
        }
        for sample in samples {
            println!("VIOLATION {sample}");
        }
        println!("GATE {}", serde_json::to_string(&report).expect("gate report serializes"));
    }
}

#[derive(Default)]
struct Metrics {
    games: u32,
    zone: [u32; 4],
    creation: [u32; 7],
    contests: Vec<f32>,
    durations: Vec<f64>,
    shot_clocks: Vec<f64>,
    fga: u32,
    possessions: u32,
    turnovers: u32,
    oreb: u32,
    fta: u32,
    points: u32,
}

impl Metrics {
    fn add(&mut self, report: &nba_sim_core::GameReport) {
        self.games += 1;
        for index in 0..4 {
            self.zone[index] += report.shot_zone_counts[index];
        }
        for index in 0..7 {
            self.creation[index] += report.creation_counts[index];
        }
        self.contests.extend_from_slice(&report.contest_samples);
        self.shot_clocks.extend_from_slice(&report.shot_clock_at_release);
        self.durations
            .extend(report.possessions.iter().map(|p| p.duration));
        let summary = summarize(report);
        self.fga += summary.fga;
        self.possessions += summary.possessions;
        self.turnovers += summary.turnovers;
        self.oreb += summary.oreb;
        self.fta += summary.fta;
        self.points += summary.home_score + summary.away_score;
    }
}

fn percentile(values: &[f64], fraction: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("finite values"));
    let index = ((sorted.len() - 1) as f64 * fraction) as usize;
    sorted[index]
}

fn print_metrics(metrics: &Metrics) {
    let shots: u32 = metrics.zone.iter().sum();
    let zone: Vec<f64> = metrics
        .zone
        .iter()
        .map(|value| *value as f64 / shots.max(1) as f64)
        .collect();
    let creation: Vec<f64> = metrics
        .creation
        .iter()
        .map(|value| *value as f64 / shots.max(1) as f64)
        .collect();
    let mean_contest: f64 = metrics.contests.iter().map(|c| *c as f64).sum::<f64>()
        / metrics.contests.len().max(1) as f64;
    let games = metrics.games.max(1) as f64;
    let clocks: Vec<f64> = metrics.shot_clocks.clone();
    let mut sorted_clocks = clocks.clone();
    sorted_clocks.sort_by(|a, b| a.partial_cmp(b).expect("finite clocks"));
    let report = serde_json::json!({
        "games": metrics.games,
        "points_per_game": metrics.points as f64 / games,
        "possessions_per_game": metrics.possessions as f64 / games,
        "fga_per_game": metrics.fga as f64 / games,
        "fga_per_possession": metrics.fga as f64 / metrics.possessions.max(1) as f64,
        "turnover_per_possession": metrics.turnovers as f64 / metrics.possessions.max(1) as f64,
        "oreb_per_game": metrics.oreb as f64 / games,
        "fta_per_game": metrics.fta as f64 / games,
        "zone_share": {"rim": zone[0], "near": zone[1], "mid": zone[2], "three": zone[3]},
        "creation_share": {"drive": creation[0], "cut": creation[1], "post": creation[2], "pull_up": creation[3], "putback": creation[4], "transition": creation[5], "set": creation[6]},
        "mean_contest": mean_contest,
        "shot_clock_p50": percentile(&sorted_clocks, 0.5),
        "possession_seconds_p50": percentile(&metrics.durations, 0.5),
    });
    println!("METRICS {report}");
}

fn print_box(
    report: &nba_sim_core::GameReport,
    home: nba_sim_core::TeamLine,
    away: nba_sim_core::TeamLine,
) {    for (team, lines, total) in [
        (report.home_team.as_str(), &report.box_score.home, home),
        (report.away_team.as_str(), &report.box_score.away, away),
    ] {
        println!();
        println!("{team}");
        println!("  player            min  pts  fg   3p   ft  orb drb ast stl blk tov pf  +/-");
        for line in lines.iter().filter(|line| line.seconds > 0.0) {
            println!(
                "  {:<16} {:4.1} {:4} {:2}-{:<2} {:2}-{:<2} {:2}-{:<2} {:3} {:3} {:3} {:3} {:3} {:3} {:3} {:+4}",
                line.name,
                line.minutes(),
                line.points,
                line.fgm,
                line.fga,
                line.tpm,
                line.tpa,
                line.ftm,
                line.fta,
                line.oreb,
                line.dreb,
                line.assists,
                line.steals,
                line.blocks,
                line.turnovers,
                line.fouls,
                line.plus_minus
            );
        }
        println!(
            "  team total        {:4} {:3}-{:<3} {:2}-{:<2} {:2}-{:<2} {:3} {:3} {:3} {:3} {:3} {:3}",
            total.points,
            total.fgm,
            total.fga,
            total.tpm,
            total.tpa,
            total.ftm,
            total.fta,
            total.oreb,
            total.dreb,
            total.assists,
            total.steals,
            total.blocks,
            total.turnovers
        );
    }
}

fn report_distributions(report: &nba_sim_core::GameReport) {
    let mut durations: Vec<f64> = report.possessions.iter().map(|p| p.duration).collect();
    durations.sort_by(|a, b| a.partial_cmp(b).expect("finite durations"));
    if !durations.is_empty() {
        println!(
            "  possession wall seconds: p10 {:.1}  p50 {:.1}  p90 {:.1}",
            durations[durations.len() / 10],
            durations[durations.len() / 2],
            durations[durations.len() * 9 / 10]
        );
    }
    let mut clocks = report.shot_clock_at_release.clone();
    clocks.sort_by(|a, b| a.partial_cmp(b).expect("finite clocks"));
    if !clocks.is_empty() {
        println!(
            "  shot clock at release: p10 {:.1}  p50 {:.1}  p90 {:.1}",
            clocks[clocks.len() / 10],
            clocks[clocks.len() / 2],
            clocks[clocks.len() * 9 / 10]
        );
    }
    let contest: f32 =
        report.contest_samples.iter().sum::<f32>() / report.contest_samples.len().max(1) as f32;
    println!("  mean contest: {:.3}", contest);
    let total: u32 = report.shot_zone_counts.iter().sum();
    if total > 0 {
        println!(
            "  zone share: rim {:.3} near {:.3} mid {:.3} three {:.3}",
            report.shot_zone_counts[0] as f32 / total as f32,
            report.shot_zone_counts[1] as f32 / total as f32,
            report.shot_zone_counts[2] as f32 / total as f32,
            report.shot_zone_counts[3] as f32 / total as f32
        );
        println!(
            "  creation share: drive {:.3} cut {:.3} post {:.3} pull_up {:.3} putback {:.3} transition {:.3} set {:.3}",
            report.creation_counts[0] as f32 / total as f32,
            report.creation_counts[1] as f32 / total as f32,
            report.creation_counts[2] as f32 / total as f32,
            report.creation_counts[3] as f32 / total as f32,
            report.creation_counts[4] as f32 / total as f32,
            report.creation_counts[5] as f32 / total as f32,
            report.creation_counts[6] as f32 / total as f32
        );
    }
}
