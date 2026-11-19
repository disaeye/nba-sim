use std::io;
use std::path::{Path, PathBuf};

use nba_domain::league::LeagueId;
use nba_sim_core::params::{workspace_calibration_dir, workspace_validation_dir};
use nba_sim_core::RuntimeParams;
use nba_validation::fragment::{self, FragmentInput, Scenario};

pub(crate) fn run(
    seed: u64,
    league: LeagueId,
    scenario: Option<Scenario>,
    replay: Option<PathBuf>,
    out: Option<PathBuf>,
    json: bool,
) -> io::Result<u8> {
    let params = RuntimeParams::load(&workspace_calibration_dir()).map_err(io::Error::other)?;
    let inputs = if let Some(path) = &replay {
        let input = serde_json::from_reader(io::BufReader::new(std::fs::File::open(path)?))?;
        vec![input]
    } else {
        let scenarios: &[Scenario] = match &scenario {
            Some(scenario) => std::slice::from_ref(scenario),
            None => &Scenario::ALL,
        };
        scenarios.iter().map(|scenario| FragmentInput::builtin(league, seed, *scenario, &params)).collect()
    };
    let root = out.unwrap_or_else(workspace_validation_dir);
    let mut reports = Vec::new();
    let mut failed = false;
    for input in inputs {
        let scenario = input.scenario.name();
        let seed = input.manifest.seed;
        let dir = root.join(format!("{}-{seed}-{scenario}", input.manifest.league().as_str()));
        let saved = replay.as_ref().map(|path| path.with_file_name("world.cbor"));
        let checkpoint = match saved {
            Some(path) if path.try_exists()? => fragment::FragmentCheckpoint::load(input.clone(), &path, &params),
            _ => fragment::capture(input.clone(), &params),
        };
        let result = checkpoint.as_ref().map_err(String::clone).and_then(|world| world.clone().resume());
        match result {
            Ok(run) => {
                if !run.report.passed() {
                    failed = true;
                    std::fs::create_dir_all(&dir)?;
                    write(&dir.join("input.json"), &input)?;
                    checkpoint.as_ref().map_err(|error| io::Error::other(error.clone()))?.save(&dir.join("world.cbor")).map_err(io::Error::other)?;
                    write(&dir.join("fragment.json"), &run)?;
                    if !json {
                        eprintln!("复现：nba-sim validate-core --replay {}", dir.join("input.json").display());
                    }
                }
                if !json {
                    println!("{scenario} seed {seed}: {}（tick {}–{}，{} 项问题）", if run.report.passed() { "通过" } else { "缺陷" }, run.report.start_tick, run.report.end_tick, run.report.findings.len());
                    for finding in &run.report.findings {
                        println!("  {} tick {} events {:?}: {}", finding.check, finding.tick, finding.events, finding.detail);
                    }
                }
                reports.push(run.report);
            }
            Err(error) => {
                std::fs::create_dir_all(&dir)?;
                write(&dir.join("input.json"), &input)?;
                if let Ok(checkpoint) = checkpoint {
                    checkpoint.save(&dir.join("world.cbor")).map_err(io::Error::other)?;
                }
                write(&dir.join("execution_error.json"), &error)?;
                return Err(io::Error::other(format!("{error}; 复现：nba-sim validate-core --replay {}", dir.join("input.json").display())));
            }
        }
    }
    if json {
        serde_json::to_writer_pretty(io::stdout().lock(), &reports)?;
        println!();
    }
    Ok(u8::from(failed))
}

fn write(path: &Path, value: &impl serde::Serialize) -> io::Result<()> {
    let mut writer = io::BufWriter::new(std::fs::File::create(path)?);
    serde_json::to_writer(&mut writer, value)?;
    io::Write::flush(&mut writer)
}
