//! ndjson 流解析：把 tick 流文本转成 `Vec<StreamTick>`。
//!
//! 严格与软解析的区分是评判可信度的前提：静默保留能解析的部分会让
//! 截断流被当成完整流评判（高分假安全感的来源之一，gap.md §15.4）。
//! 因此默认入口是严格模式，软解析必须显式调用并由调用方自负证据完整性。

use nba_protocol::StreamTick;

/// 从 ndjson 流解析 tick 序列（严格模式，遇到坏行返回错误及行号）。
pub fn try_parse_stream(content: &str) -> Result<Vec<StreamTick>, String> {
    let mut ticks = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let tick: StreamTick = serde_json::from_str(trimmed)
            .map_err(|e| format!("line {}: failed to parse StreamTick: {}", line_no + 1, e))?;
        ticks.push(tick);
    }
    Ok(ticks)
}

/// 从 ndjson 流解析 tick 序列（**默认严格**，gap.md §15.4 / dev 方案 §4 D1.3）：
/// 任何坏行 = 整个流不可信，返回错误。静默保留能解析的部分会让截断流
/// 被当成完整流评判（高分假安全感的另一来源）。
/// 软解析仅在显式 `parse_stream_lenient` 下可用，并由调用方自负证据完整性。
pub fn parse_stream(content: &str) -> Result<Vec<StreamTick>, String> {
    try_parse_stream(content)
}

/// 软解析（仅显式调用）：坏行打 stderr 告警并保留能解析的部分。
/// 仅用于调试/勘探，不得用于任何评判门。
pub fn parse_stream_lenient(content: &str) -> Vec<StreamTick> {
    match try_parse_stream(content) {
        Ok(ticks) => ticks,
        Err(err) => {
            eprintln!("parse_stream_lenient warning: {}", err);
            content
                .lines()
                .filter(|l| !l.trim().is_empty())
                .filter_map(|l| serde_json::from_str(l).ok())
                .collect()
        }
    }
}
