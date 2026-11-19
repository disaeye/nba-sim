use crate::{commands, parse_league, parse_option_value, run_validate_core};

pub(crate) fn run(args: &[String]) -> std::io::Result<()> {
    let mut seed_start = None;
    let mut games = None;
    let mut scenario = None;
    let mut replay = None;
    let mut json = false;
    let mut mechanism = None;
    let mut league = None;
    let mut out: Option<std::path::PathBuf> = None;
    let mut i = 2;
    while i < args.len() {
        match args[i].as_str() {
            "--seed" => {
                seed_start = Some(parse_option_value(args, i + 1, "--seed <n>")?
                    .parse()
                    .map_err(|_| std::io::Error::other("seed must be a number"))?);
                i += 2;
            }
            "--games" => {
                games = Some(
                    parse_option_value(args, i + 1, "--games <n>")?
                        .parse()
                        .map_err(|_| std::io::Error::other("games must be a number"))?,
                );
                i += 2;
            }
            "--league" => {
                league = Some(parse_league(parse_option_value(args, i + 1, "--league <nba|fiba>")?)?);
                i += 2;
            }
            "--scenario" => {
                let name = parse_option_value(args, i + 1, "--scenario <name>")?;
                scenario = Some(nba_validation::fragment::Scenario::parse(name).map_err(std::io::Error::other)?);
                i += 2;
            }
            "--replay" => {
                replay = Some(std::path::PathBuf::from(parse_option_value(args, i + 1, "--replay <input>")?));
                i += 2;
            }
            "--json" => {
                json = true;
                i += 1;
            }
            "--mechanism" => {
                let key = parse_option_value(args, i + 1, "--mechanism <attribute>")?;
                mechanism = Some(nba_validation::mechanism_core::MechanismAttribute::all().into_iter()
                    .find(|attribute| attribute.key() == key)
                    .ok_or_else(|| std::io::Error::other(format!("unknown mechanism attribute `{key}`")))?);
                i += 2;
            }
            "--out" => {
                out = Some(std::path::PathBuf::from(parse_option_value(
                    &args,
                    i + 1,
                    "--out <dir>",
                )?));
                i += 2;
            }
            other => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    format!("unknown validate-core option `{other}`"),
                ));
            }
        }
    }
    let result = if replay.is_some() && (seed_start.is_some() || league.is_some() || scenario.is_some() || games.is_some() || mechanism.is_some()) {
        Err(std::io::Error::other("--replay uses the scenario, seed and league in its input"))
    } else if let Some(attribute) = mechanism {
        if games.is_some() || scenario.is_some() || league.is_some() {
            Err(std::io::Error::other("--mechanism selects an independent NBA experiment"))
        } else {
            commands::validate_core::run_mechanism(attribute, seed_start.unwrap_or(1), out, json)
        }
    } else if let Some(games) = games {
        if scenario.is_some() {
            Err(std::io::Error::other("--games cannot be combined with --scenario"))
        } else {
            run_validate_core(seed_start.unwrap_or(1), games, json, out, league.unwrap_or(nba_domain::league::LeagueId::Nba))
        }
    } else {
        commands::validate_fragment::run(seed_start.unwrap_or(1), league.unwrap_or(nba_domain::league::LeagueId::Nba), scenario, replay, out, json)
    };
    match result {
        Ok(0) => Ok(()),
        Ok(code) => std::process::exit(code as i32),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(2);
        }
    }
}
