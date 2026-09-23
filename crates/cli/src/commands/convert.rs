//! `evaluate` 与 `pbp-convert` 子命令：离线评判与外部数据转换。
//!
//! 两者都不跑模拟：`evaluate` 读既有 ndjson 流产评判工件，
//! `pbp-convert` 把外部 PBP 数据转成参考分布 fixture。

use std::fs;

use crate::{enforce_hard_gate, read_stream_text, write_judgment_artifacts};

/// 离线评判（quality 评判规范）：对既有 ndjson 流产出评判工件。
///
/// 参考分布由联赛入口选择（与模拟路径同一映射 `for_league`）：
/// 此前硬编码 `nba_v1()`，而 v1 无构成带，七条构成准则全部
/// `NotApplicable`——同一条流在模拟内评 v2、离线复评 v1，两处结论
/// 无法互相对照。NBA 缺省即 v2，与模拟路径一致；FIBA 经 `--league fiba`。
pub(crate) fn run_evaluate(stream_path: &str, league: &str) -> std::io::Result<()> {
    let stream = read_stream_text(stream_path)?;
    let ticks = nba_evaluator::parse_stream(&stream)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    let fixture = nba_evaluator::ReferenceDistributions::for_league(league);
    let judgments = nba_evaluator::evaluate_stream(&ticks, &fixture);
    let report = write_judgment_artifacts(stream_path, &judgments, &fixture)?;
    println!(
        "🧾 Evaluated {} ticks: {} judgments, {} defects",
        ticks.len(),
        report.total_judgments,
        report.defect_count
    );
    for row in report.defects_by_criterion.iter().take(5) {
        println!(
            "   [{:>3}x] {} ({})",
            row.count, row.criterion, row.attribution
        );
    }
    println!("   Realism index: {:.3}", report.realism_index);
    // gap.md §18.6：evaluate 子命令同样必须反映 Hard 门。
    if enforce_hard_gate(&report, "evaluate") {
        std::process::exit(1);
    }
    Ok(())
}

/// 转换外部 PBP 数据为 ReferenceDistributions fixture（M8 扩展位）。
pub(crate) fn run_pbp_convert(
    input_path: &str,
    out_path: &str,
    league: &str,
    version: &str,
) -> std::io::Result<()> {
    let content = fs::read_to_string(input_path)?;
    let events: Vec<nba_evaluator::PbpEvent> = if content.trim_start().starts_with('[') {
        serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
    } else {
        content
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                serde_json::from_str(l)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
            })
            .collect::<Result<Vec<_>, _>>()?
    };

    let fixture = nba_evaluator::convert_pbp_events_to_fixture(&events, version, league);
    let serialized = serde_json::to_string_pretty(&fixture)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(out_path, serialized)?;
    println!(
        "📊 Converted {} PBP events → {} (version: {}, league: {})",
        events.len(),
        out_path,
        version,
        league
    );
    Ok(())
}
