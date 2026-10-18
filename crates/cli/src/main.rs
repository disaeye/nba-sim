use std::env;

mod commands;

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
           sim-core --seed N [--games M] [--gate] [--stream PATH]\n\
                    跑一场或多场比赛；--gate 打印四层判定，--stream 写事件流\n\
           validate-core --seed N --games M [--mechanism] [--out DIR]\n\
                    跑判定批次并写工件；退出码 0 通过、1 缺陷、2 执行错误、3 已知不足\n\
         \n\
         -h, --help  本说明"
    );
}

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && (args[1] == "--help" || args[1] == "-h") {
        print_usage();
        return Ok(());
    }

    if args.len() > 1 && args[1] == "validate-core" {
        let mut seed_start = 1u64;
        let mut games = 16u64;
        let mut json = false;
        let mut mechanism = false;
        let mut out: Option<std::path::PathBuf> = None;
        let mut i = 2;
        while i < args.len() {
            match args[i].as_str() {
                "--seed" => {
                    seed_start = parse_option_value(&args, i + 1, "--seed <n>")?
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
                "--json" => {
                    json = true;
                    i += 1;
                }
                "--mechanism" => {
                    mechanism = true;
                    i += 1;
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
        return match run_validate_core(seed_start, games, json, mechanism, out) {
            Ok(0) => Ok(()),
            Ok(code) => std::process::exit(code as i32),
            Err(error) => {
                eprintln!("{error}");
                std::process::exit(2);
            }
        };
    }

    if args.len() > 1 && args[1] == "sim-core" {
        let mut seed = 42u64;
        let mut games = 1u64;
        let mut gate = false;
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
                "--gate" => {
                    gate = true;
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
        return run_sim_core(seed, games, gate, stream.as_deref());
    }

    print_usage();
    std::process::exit(2);
}
