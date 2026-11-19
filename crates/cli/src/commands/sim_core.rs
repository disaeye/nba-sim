//! `sim-core` 子命令：运行用户指定的比赛并输出统计或帧流。

use nba_sim_core::{
    run_game_for_league, summarize, CompletionSummary, MatchSetup, RuntimeParams, TeamSide, World,
};
use std::io::Write;
use std::path::Path;

pub(crate) fn run_sim_core(
    seed: u64,
    games: u64,
    json: bool,
    stream: Option<&str>,
    league: nba_domain::league::LeagueId,
) -> std::io::Result<()> {
    let calibration = nba_sim_core::params::workspace_calibration_dir();
    let params = RuntimeParams::load(&calibration)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;

    let mut reports = Vec::new();
    let mut event_logs = Vec::new();
    for offset in 0..games.max(1) {
        let report = match stream {
            Some(path) if offset == 0 => {
                export_stream(seed, &calibration, params.clone(), path, league)?
            }
            _ => run_game_for_league(seed + offset, &calibration, league),
        };
        if !json {
            print_game(&report);
        } else {
            let mut projection_world = World::new(
                MatchSetup::builtin(nba_domain::GameRules::for_league(league)),
                params.clone(),
                seed + offset,
            );
            projection_world.decision_traces = report.decision_traces.clone();
            event_logs.push(
                report
                    .events
                    .iter()
                    .map(|event| projection_world.project_frame_event(event))
                    .collect::<Vec<_>>(),
            );
        }
        reports.push(report);
    }

    if json {
        let summaries: Vec<CompletionSummary> = reports
            .iter()
            .map(CompletionSummary::from_report)
            .collect();
        let output = serde_json::json!({ "games": summaries, "events": event_logs });
        println!(
            "{}",
            serde_json::to_string(&output)
                .map_err(|error| std::io::Error::other(error.to_string()))?
        );
        return Ok(());
    }

    if reports.len() > 1 {
        let count = reports.len() as f32;
        let mut points = 0.0;
        let mut possessions = 0.0;
        let mut fga = 0.0;
        let mut fg3a = 0.0;
        let mut fta = 0.0;
        let mut turnovers = 0.0;
        let mut oreb = 0.0;
        for report in &reports {
            let summary = summarize(report);
            points += (summary.home_score + summary.away_score) as f32;
            possessions += summary.possessions as f32;
            fga += summary.fga as f32;
            fg3a += summary.fg3a as f32;
            fta += summary.fta as f32;
            turnovers += summary.turnovers as f32;
            oreb += summary.oreb as f32;
        }
        println!();
        println!(
            "mean  {:.1} pts  {:.1} poss  {:.1} FGA  {:.1} 3PA  {:.1} FTA  {:.1} TOV  {:.1} OREB",
            points / count,
            possessions / count,
            fga / count,
            fg3a / count,
            fta / count,
            turnovers / count,
            oreb / count
        );
    }

    Ok(())
}

fn export_stream(
    seed: u64,
    _calibration: &Path,
    params: RuntimeParams,
    path: &str,
    league: nba_domain::league::LeagueId,
) -> std::io::Result<nba_sim_core::GameReport> {
    let rules = nba_domain::GameRules::for_league(league);
    let setup = MatchSetup::builtin(rules);
    let mut world = World::new(setup, params, seed);
    let file = std::fs::File::create(path)?;
    if path.ends_with(".gz") {
        let encoder = flate2::write::GzEncoder::new(file, flate2::Compression::fast());
        let mut writer = std::io::BufWriter::new(encoder);
        world.run_with(|world| {
            let tick = world.stream_tick();
            let line = serde_json::to_string(&tick).expect("stream tick serializes");
            writeln!(writer, "{line}").expect("stream write must succeed");
        });
        writer.flush()?;
    } else {
        let mut writer = std::io::BufWriter::new(file);
        world.run_with(|world| {
            let tick = world.stream_tick();
            let line = serde_json::to_string(&tick).expect("stream tick serializes");
            writeln!(writer, "{line}").expect("stream write must succeed");
        });
        writer.flush()?;
    }
    let mut report = world.report();
    report.shot_clock_at_release = world.shot_clock_at_release.clone();
    report.contest_samples = world.contest_samples.clone();
    report.shot_zone_counts = world.shot_zone_counts;
    report.shot_zone_makes = world.shot_zone_makes;
    report.creation_counts = world.creation_counts;
    report.violations = world.violations.clone();
    Ok(report)
}

fn print_game(report: &nba_sim_core::GameReport) {
    println!(
        "seed {}  {} {} - {} {}  ({} possessions)",
        report.seed,
        report.home_team,
        report.home_score,
        report.away_score,
        report.away_team,
        report.possessions.len()
    );
    for (team, lines) in [
        (report.home_team.as_str(), &report.box_score.home),
        (report.away_team.as_str(), &report.box_score.away),
    ] {
        println!("  {team}");
        println!("    player            min  pts  fg   3p   ft  orb drb ast stl blk tov pf  +/-");
        for line in lines.iter().filter(|line| line.seconds > 0.0) {
            println!(
                "    {:<16} {:4.1} {:4} {:2}-{:<2} {:2}-{:<2} {:2}-{:<2} {:3} {:3} {:3} {:3} {:3} {:3} {:3} {:+4}",
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
        let totals = report.box_score.team_totals(if team == report.home_team {
            TeamSide::Home
        } else {
            TeamSide::Away
        });
        println!(
            "    team total        {:4} {:3}-{:<3} {:2}-{:<2} {:2}-{:<2} {:3} {:3} {:3} {:3} {:3}",
            totals.points,
            totals.fgm,
            totals.fga,
            totals.tpm,
            totals.tpa,
            totals.ftm,
            totals.fta,
            totals.oreb,
            totals.dreb,
            totals.assists,
            totals.steals,
            totals.blocks
        );
    }
}
