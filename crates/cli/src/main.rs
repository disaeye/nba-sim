use std::env;

mod commands;

pub(crate) use commands::review::run_review;
pub(crate) use commands::sim_core::run_sim_core;
pub(crate) use commands::validate_core::run_validate_core;

fn parse_option_value<'a>(
    args: &'a [String],
    index: usize,
    usage: &str,
) -> std::io::Result<&'a str> {
    match args.get(index).map(|s| s.as_str()) {
        Some(value) if !value.starts_with("--") => Ok(value),
        Some(value) => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!(
                "❌ option value cannot look like a flag: got `{value}`\n   usage: nba-sim {usage}"
            ),
        )),
        None => Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("❌ missing value after option\n   usage: nba-sim {usage}"),
        )),
    }
}

fn print_usage() {
    println!(
        "NBA-Sim · 动作链内核\n\
         \n\
         用法：nba-sim <子命令> [选项]\n\
         \n\
         子命令：\n\
           sim-core --seed N [--games M] [--json] [--stream PATH] [--league nba|fiba]\n\
                    跑一场或多场比赛；--json 输出摘要与事件\n\
           validate-core [--scenario opening-control|first-possession|halfcourt-screen] [--seed N] [--league nba|fiba] [--out DIR] [--json]\n\
           validate-core --replay INPUT [--out DIR] [--json]\n\
           validate-core --games M [--seed N] [--out DIR] [--league nba|fiba]\n\
           validate-core --mechanism ATTRIBUTE [--seed N] [--out DIR] [--json]\n\
                    默认验证短片段；退出码 0 通过、1 缺陷、2 执行错误\n\
           review --seed N [--possessions M] [--out DIR] [--league nba|fiba]\n\
                    选出过程清单覆盖的回合，写 review.json 与 review.html\n\
         \n\
         -h, --help  本说明"
    );
}

/// 解析 `--league` 的取值。
fn parse_league(value: &str) -> std::io::Result<nba_domain::league::LeagueId> {
    value
        .parse::<nba_domain::league::LeagueId>()
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        print_usage();
        return Ok(());
    }

    if args.get(1).is_some_and(|arg| arg == "validate-core") {
        return commands::validate_args::run(&args);
    }

    if args.len() > 1 && args[1] == "review" {
        let mut seed = 1u64;
        let mut possessions = 20usize;
        let mut league = nba_domain::league::LeagueId::Nba;
        let mut out: Option<std::path::PathBuf> = None;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--seed" => {
                    seed = parse_option_value(&args, i + 1, "--seed <n>")?
                        .parse()
                        .map_err(|_| std::io::Error::other("seed must be a number"))?;
                    i += 2;
                }
                "--possessions" => {
                    possessions = parse_option_value(&args, i + 1, "--possessions <n>")?
                        .parse()
                        .map_err(|_| std::io::Error::other("possessions must be a number"))?;
                    i += 2;
                }
                "--league" => {
                    league = parse_league(parse_option_value(&args, i + 1, "--league <nba|fiba>")?)?;
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
                        format!("unknown review option `{other}`"),
                    ));
                }
            }
        }
        return run_review(seed, possessions, out, league);
    }

    if args.len() > 1 && args[1] == "sim-core" {
        let mut seed = 42u64;
        let mut games = 1u64;
        let mut json = false;
        let mut league = nba_domain::league::LeagueId::Nba;
        let mut stream: Option<String> = None;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--seed" => {
                    seed = parse_option_value(&args, i + 1, "--seed <n>")?
                        .parse()
                        .map_err(|_| std::io::Error::other("seed must be a number"))?;
                    i += 2;
                }
                "--games" => {
                    games = parse_option_value(&args, i + 1, "--games <n>")?
                        .parse()
                        .map_err(|_| std::io::Error::other("games must be a number"))?;
                    i += 2;
                }
                "--league" => {
                    league = parse_league(parse_option_value(&args, i + 1, "--league <nba|fiba>")?)?;
                    i += 2;
                }
                "--json" => {
                    json = true;
                    i += 1;
                }
                "--stream" => {
                    stream = Some(parse_option_value(&args, i + 1, "--stream <path>")?.to_string());
                    i += 2;
                }
                other => {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        format!("unknown sim-core option `{other}`"),
                    ));
                }
            }
        }
        return run_sim_core(seed, games, json, stream.as_deref(), league);
    }

    print_usage();
    std::process::exit(2);
}
