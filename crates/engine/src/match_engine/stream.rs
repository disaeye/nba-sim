//! 比赛流导出：帧/事实/总结三种投影的写盘管线与资源治理。
//!
//! 依据 `gap.md` §16.4：默认事实流只写因果与审计所需字段，工程上由磁盘
//! 预检、字节预算与 tick 上限三层守卫保证「写满磁盘」与「无限增长」不会
//! 静默发生。

use std::fs::File;
use std::io::{BufWriter, Write};

use flate2::write::GzEncoder;
use flate2::Compression;
use nba_domain::GameRules;
use nba_invariants::{Violation, ViolationTaxonomy};
use nba_protocol::*;

use super::{ExportSummary, MatchEngine};

/// 流导出模式（gap.md §16.4 资源治理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamMode {
    /// 逐 tick 完整帧：展示/回放需要，体积最大。
    Frames,
    /// gzip 压缩的逐 tick 完整帧：保留 Frames 语义，显著降低存储体积。
    FramesGzip,
    /// 因果事实流（默认）：事实、事件日志、阶段/生命周期变化、回合总结。
    /// 引擎每 tick 自检 L1，因此无需输出每 tick 球员投影。
    #[default]
    Facts,
    /// 仅回合总结与比赛级元数据：体积极小，用于批量统计。
    Summary,
}

impl StreamMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frames" | "full" => Ok(Self::Frames),
            "frames-gzip" | "gzip" | "compressed-frames" => Ok(Self::FramesGzip),
            "facts" | "causal" => Ok(Self::Facts),
            "summary" | "summaries" => Ok(Self::Summary),
            other => Err(format!(
                "unknown stream mode `{other}` (expected frames | frames-gzip | facts | summary)"
            )),
        }
    }
}

enum StreamWriter {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

impl Write for StreamWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(writer) => writer.write(bytes),
            Self::Gzip(writer) => writer.write(bytes),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(writer) => writer.flush(),
            Self::Gzip(writer) => writer.flush(),
        }
    }
}

impl StreamWriter {
    fn finish(self) -> std::io::Result<()> {
        match self {
            Self::Plain(mut writer) => writer.flush(),
            Self::Gzip(writer) => {
                let mut writer = writer.finish()?;
                writer.flush()
            }
        }
    }
}

/// `GameRules` → `FrameRules` 的唯一投影（C6.4）。
///
/// 此前引擎在 `build_tick` 内联展开 18 个字段，同时 protocol 的
/// `FrameRules::default()` 手抄同一组数字——两处独立漂移已实际发生
/// （`player_radius_ft` 引擎 1.8 vs 协议默认 1.0）。收敛为单一函数后，
/// `constraint_system` 的全字段一致性测试负责让任何一侧漂移必红。
pub fn frame_rules_from_game_rules(rules: &GameRules) -> FrameRules {
    FrameRules {
        tick_seconds: rules.tick_seconds,
        court_width_ft: rules.court.width_ft,
        court_height_ft: rules.court.height_ft,
        hoop_left_x_ft: rules.court.hoop_left_x_ft,
        hoop_right_x_ft: rules.court.hoop_right_x_ft,
        hoop_y_ft: rules.court.hoop_y_ft,
        player_radius_ft: rules.player_radius_ft,
        min_player_separation_ft: rules.min_player_separation_ft,
        separation_safety_margin_ft: rules.separation_safety_margin_ft,
        max_player_speed_ftps: rules.max_player_speed_ftps,
        max_player_accel_ftps2: rules.max_player_accel_ftps2,
        ball_max_speed_ftps: rules.ball_max_speed_ftps,
        three_point_distance_ft: rules.league.three_point_distance_ft,
        shot_clock_seconds: rules.league.shot_clock_seconds,
        holder_leash_ft: rules.invariant_holder_leash_ft,
        speed_tolerance_ftps: rules.invariant_speed_tolerance_ftps,
        ball_z_max_ft: rules.ball_z_max_ft,
    }
}

/// 事实流的去重游标：只在阶段/生命周期/回合边界变化时记一行。
#[derive(Debug, Clone, Default)]
struct StreamCursor {
    phase: String,
    game_flow: String,
    completed: usize,
}

/// 事实模式记录：保留因果与审计所需字段，去掉逐 tick 球员/球坐标投影。
///
/// `include_context` 只在流的首条记录为 true：`rules` 与 `tactical_set`
/// 对整场恒定，重复写入会占掉一半以上体积。消费者按首条记录继承即可
/// （gap.md §16.4）。
fn compact_fact_record(tick: &StreamTick, include_context: bool) -> serde_json::Value {
    let f = &tick.frame;
    let mut record = serde_json::json!({
        "t": f.t,
        "t_game": f.t_game,
        "shotClock": f.shot_clock,
        "period": f.period,
        "phase": f.phase,
        "game_flow": f.game_flow,
        "possession_id": f.possession_id,
        "possession_team": f.possession_team,
        "score": f.score,
        "event_type": f.event_type,
        "events": f.events,
        "event_log": f.event_log,
        "event_sequence": f.event_sequence,
        "completed_possessions": f.completed_possessions,
        "simulation_complete": f.simulation_complete,
        "team_fouls_home": f.team_fouls_home,
        "team_fouls_away": f.team_fouls_away,
        "free_throws_remaining": f.free_throws_remaining,
        "gameClock": tick.game_clock,
    });
    record["stream_projection"] = serde_json::Value::String("facts".to_string());
    if include_context {
        record["rules"] = serde_json::to_value(&f.rules).unwrap_or(serde_json::Value::Null);
        record["tactical_set"] = serde_json::Value::String(tick.tactical_set.clone());
    }
    record
}

/// 总结模式记录：只保留回合总结与比赛级元数据。
fn compact_summary_record(tick: &StreamTick, include_context: bool) -> serde_json::Value {
    let f = &tick.frame;
    let summaries: Vec<&FrameEvent> = f
        .event_log
        .iter()
        .filter(|e| e.kind == "POSSESSION_SUMMARY")
        .collect();
    let mut record = serde_json::json!({
        "t": f.t,
        "t_game": f.t_game,
        "period": f.period,
        "phase": f.phase,
        "game_flow": f.game_flow,
        "possession_id": f.possession_id,
        "score": f.score,
        "completed_possessions": f.completed_possessions,
        "simulation_complete": f.simulation_complete,
        "event_sequence": f.event_sequence,
        "event_log": summaries,
    });
    record["stream_projection"] = serde_json::Value::String("summary".to_string());
    if include_context {
        record["rules"] = serde_json::to_value(&f.rules).unwrap_or(serde_json::Value::Null);
        record["tactical_set"] = serde_json::Value::String(tick.tactical_set.clone());
    }
    record
}

/// 运行前磁盘预检（gap.md §16.4）：目标目录可用空间不足预算时直接报错。
///
/// 使用 `df -Pk` 获取可用块数（Linux/macOS 通用），不引入额外依赖；
/// 无法获取时不阻断，由写入期字节预算兜底。
fn ensure_disk_headroom(out_path: &str, budget_bytes: u64) -> std::io::Result<()> {
    let dir = std::path::Path::new(out_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let Ok(output) = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()
    else {
        return Ok(());
    };
    if !output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // df 输出：表头 + 一行数据，第 4 列为可用 1K 块。
    let Some(available_kb) = text
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok())
    else {
        return Ok(());
    };
    let available = available_kb.saturating_mul(1024);
    // 至少需要预算的 1.5 倍，并保留 512 MiB 系统余量。
    let required = budget_bytes + 256 * 1024 * 1024;
    if available < required {
        return Err(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            format!(
                "insufficient disk space in {}: {} MiB available, {} MiB required",
                dir.display(),
                available / (1024 * 1024),
                required / (1024 * 1024)
            ),
        ));
    }
    Ok(())
}

/// 单次受限范围的导出任务。
pub(crate) struct StreamPipeline {
    mode: StreamMode,
}

impl StreamPipeline {
    pub(crate) fn new(mode: StreamMode) -> Self {
        Self { mode }
    }

    /// 按指定流模式导出一场比赛（gap.md §16.4 资源治理）。
    ///
    /// - [`StreamMode::Frames`]：逐 tick 完整帧（展示/回放，大）；
    /// - [`StreamMode::FramesGzip`]：同一完整帧协议的 gzip 存储格式；
    /// - [`StreamMode::Facts`]（默认）：只写因果事实、事件日志、阶段/生命周期
    ///   变化、回合总结与周期性检查点。引擎每 tick 自检 L1，流不必携带
    ///   每 tick 的球员投影，因此体积从数百 MB 降到个位数 MB；
    /// - [`StreamMode::Summary`]：只写回合总结与比赛级元数据，体积极小。
    ///
    /// 所有模式都在写入前检查磁盘空间与文件大小预算，超限即报错而不是
    /// 静默写满磁盘。
    pub(crate) fn run(
        &self,
        engine: &mut MatchEngine,
        scope: &str,
        out_path: &str,
    ) -> std::io::Result<ExportSummary> {
        let mode = self.mode;
        let scope_desc = engine
            .set_scope(scope)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;

        let byte_budget = match mode {
            StreamMode::Frames | StreamMode::FramesGzip => {
                engine.config.rules.stream_frames_max_bytes
            }
            StreamMode::Facts | StreamMode::Summary => engine.config.rules.stream_max_bytes,
        };
        // 运行前磁盘预检：不足安全余量则拒绝启动。
        ensure_disk_headroom(out_path, byte_budget)?;

        let file = File::create(out_path)?;
        let mut writer = match mode {
            StreamMode::FramesGzip => {
                StreamWriter::Gzip(GzEncoder::new(BufWriter::new(file), Compression::default()))
            }
            StreamMode::Frames | StreamMode::Facts | StreamMode::Summary => {
                StreamWriter::Plain(BufWriter::new(file))
            }
        };
        let mut ticks_count = 0;
        let mut all_violations: Vec<Violation> = Vec::new();
        let mut written_bytes: u64 = 0;
        // A lifecycle bug must fail closed rather than grow an unbounded NDJSON
        // file forever. The bound is a safety guard, not a basketball rule.
        let max_ticks = engine.config.rules.stream_max_ticks;
        let mut previous = StreamCursor::default();
        let mut meta_written = false;
        while !engine.is_finished() && ticks_count < max_ticks {
            let tick = engine.step();
            all_violations.append(&mut engine.audit.last_tick_violations);
            let should_write = match mode {
                StreamMode::Frames | StreamMode::FramesGzip => true,
                StreamMode::Facts => {
                    written_bytes == 0
                        || !tick.frame.event_log.is_empty()
                        || !tick.frame.events.is_empty()
                        || tick.frame.phase != previous.phase
                        || tick.frame.game_flow != previous.game_flow
                        || tick.frame.completed_possessions != previous.completed
                        || tick.frame.simulation_complete
                }
                StreamMode::Summary => {
                    !meta_written
                        || tick
                            .frame
                            .event_log
                            .iter()
                            .any(|e| e.kind == "POSSESSION_SUMMARY")
                }
            };
            previous.phase = tick.frame.phase.clone();
            previous.game_flow = tick.frame.game_flow.clone();
            previous.completed = tick.frame.completed_possessions;
            meta_written = true;

            if should_write {
                let payload = match mode {
                    StreamMode::Frames | StreamMode::FramesGzip => serde_json::to_string(&tick)?,
                    StreamMode::Facts => {
                        serde_json::to_string(&compact_fact_record(&tick, written_bytes == 0))?
                    }
                    StreamMode::Summary => {
                        serde_json::to_string(&compact_summary_record(&tick, written_bytes == 0))?
                    }
                };
                written_bytes = written_bytes.saturating_add(payload.len() as u64 + 1);
                if written_bytes > byte_budget {
                    return Err(std::io::Error::other(format!(
                        "stream for scope `{scope}` exceeded {}-byte budget (mode={:?})",
                        byte_budget, mode
                    )));
                }
                writer.write_all(payload.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            ticks_count += 1;
        }
        if !engine.is_finished() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("scope `{scope}` did not complete within {max_ticks} ticks"),
            ));
        }
        if written_bytes == 0 {
            let tick = engine.build_tick();
            let payload = serde_json::to_string(&compact_summary_record(&tick, true))?;
            writer.write_all(payload.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        writer.finish()?;
        if !all_violations.is_empty() {
            eprintln!(
                "=== INVARIANT VIOLATIONS ({} total) ===",
                all_violations.len()
            );
            for v in &all_violations {
                eprintln!("  {}", v);
            }
        }
        let taxonomy = ViolationTaxonomy::from_violations(&all_violations);
        Ok(ExportSummary {
            ticks: ticks_count,
            scope_desc,
            violations: all_violations,
            box_score: engine.ledger.box_score.clone(),
            taxonomy,
        })
    }
}
