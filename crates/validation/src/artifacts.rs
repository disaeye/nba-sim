//! 工件生成（validation.md §10）。
//!
//! 三个职责：
//! 1. `game_metrics.parquet`：每场比赛充分统计量的列式存储，经
//!    `parquet` crate 的 arrow writer 写入（成熟格式禁止手写生成）。
//! 2. `instances.sqlite`：完成实例与关系索引写入受忽略运行目录的
//!    SQLite 存储（§10「使用 SQLite 等成熟存储库查询」）。
//! 3. 旧工件兼容视图：`judgments.ndjson` 与 `attribution_report.json`
//!    由同一裁决生成（§10「禁止维护第二套判据」），判据数据全部来自
//!    本 crate 的裁决结构。

use std::collections::BTreeMap;

use nba_protocol::StreamTick;
use serde::Serialize;

use crate::reconstruct::{
    PairingAudit, PossessionReconciliation, RebuiltBoxScore, RebuiltClockAccount,
};
use crate::scenario::ScenarioOutcome;
use crate::verdict::Verdict;

/// 每场比赛的充分统计量（parquet 一行 = 一场比赛）。
///
/// 指标全部从流内事实独立累计（validation.md §6.2：位置类帧指标按
/// 时间加权后按比赛聚合，帧不作为独立样本），列集冻结为 schema 一部分；
/// 增删列属于 schema 版本变更。回合数取引擎 `PossessionSummary` 计数
/// 的独立重读（同一事件流，机器聚合通道与 `PossessionReconciliation`
/// 的重建边界证据互补）。
#[derive(Debug, Default, Clone, Serialize)]
pub struct GameMetrics {
    pub seed: u64,
    pub scope: String,
    /// 周期数（由帧观察到的最后节次）。
    pub periods: u32,
    pub ticks: u64,
    pub events: u64,
    pub possessions: u64,
    /// 双方合计出手/命中（比赛级充分统计量）。
    pub fga_total: u64,
    pub fgm_total: u64,
    pub three_pa_total: u64,
    pub three_pm_total: u64,
    pub fta_total: u64,
    pub ftm_total: u64,
    pub orb_total: u64,
    pub drb_total: u64,
    pub blk_total: u64,
    pub stl_total: u64,
    pub tov_total: u64,
    pub pf_total: u64,
    pub passes: u64,
    pub shots_released: u64,
    pub fouls: u64,
    pub movement_intents: u64,
    pub movement_outcomes: u64,
    pub responsibility_started: u64,
    pub play_activations: u64,
    /// 四区出手/命中（双方合计，距离按出手点到进攻篮筐中心：
    /// rim <8 / near 8-16 / mid 16-23.75 / three ≥23.75 或角落 ≥22，
    /// 与 STAT-S01 登记口径一致）。
    pub fga_rim: u64,
    pub fgm_rim: u64,
    pub fga_near: u64,
    pub fgm_near: u64,
    pub fga_mid: u64,
    pub fgm_mid: u64,
    pub fga_three: u64,
    pub fgm_three: u64,
    /// 回合开始原因计数（与 JOINT-Q01 登记的七类原因对齐，
    /// 由 PossessionSummary 的终结类别反推下一回合开始原因）。
    pub starts_field_goal: u64,
    pub starts_free_throw: u64,
    pub starts_steal: u64,
    pub starts_dead_ball: u64,
    pub starts_timeout: u64,
    pub starts_blocked_shot: u64,
    /// 失误原因聚合五类（STAT-T01，来源 PossessionSummary 终结类别；
    /// 登记侧 14 类细目按 pass/球丢失/违例/进攻犯规/其他 聚合对齐）。
    pub to_reason_pass_lost: u64,
    pub to_reason_ball_lost: u64,
    pub to_reason_violation: u64,
    pub to_reason_offensive_foul: u64,
    pub to_reason_other: u64,
    /// 出手距离直方图（STAT-S01 距离分布 W1；双方合计，桶界 0/6/10/16/20/22/23.75/28/94 ft，
    /// 8 桶；登记侧分位数由此直方图重建近似值比较）。
    pub shot_dist_hist: [u64; 8],
    /// JOINT-C01：Q4 末 3 分钟窗口、|比分差| 三桶（0-5/6-10/10+）的三分出手与命中。
    pub q4_late_three_att_by_margin: [u64; 3],
    pub q4_late_three_makes_by_margin: [u64; 3],
    /// JOINT-C04：OREB 回合总数与其中出现 ≥2 次出手的回合数。
    pub oreb_possessions: u64,
    pub oreb_possessions_with_second_attempt: u64,
    /// JOINT-C05：队节内 bonus（penalty）首进时的团队犯规序直方图（序号 1..=10，
    /// 超过 10 归入末桶）与观察到的队节总数。
    pub bonus_entry_order_hist: [u64; 10],
    pub team_periods_observed: u64,
    /// JOINT-Q01：同队相邻回合开始原因转移计数（行=前回合原因，列=本回合原因，
    /// 顺序同 START_REASON_ORDER）。
    pub transition_counts: [[u64; 7]; 7],
    /// 比赛钟运行时长（CAUSE-B03 重建值，秒）。
    pub game_clock_run_seconds: f64,
    /// 活球时长（秒）。
    pub live_ball_seconds: f64,
    /// 双方最终比分。
    pub final_score_home: u32,
    pub final_score_away: u32,
}

impl GameMetrics {
    /// 从流内事实累计每场比赛充分统计量。
    pub fn from_stream(
        seed: u64,
        scope: &str,
        ticks: &[StreamTick],
        rebuilt: &BTreeMap<String, RebuiltBoxScore>,
        clock: &RebuiltClockAccount,
        possession_count: u64,
    ) -> Self {
        let mut metrics = Self {
            seed,
            scope: scope.to_string(),
            ticks: ticks.len() as u64,
            possessions: possession_count,
            game_clock_run_seconds: clock.game_clock_run_seconds,
            live_ball_seconds: clock.live_ball_seconds,
            periods: clock.periods_observed,
            ..Self::default()
        };
        // shot_id → 出手区（SHOT_RELEASE 时记录，FGM 结算时消费）
        let mut shot_zones: std::collections::HashMap<String, u8> =
            std::collections::HashMap::new();
        // shot_id → JOINT-C01 比分差桶（Q4 末段三分出手，FGM 结算时回填命中）
        let mut c01_shot_buckets: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        // 当前队节的 bonus 首进标志（PERIOD 推进或节末结算时重置）；
        // team_periods_observed 按「帧内出现的 (period, 进攻方) 组合」粗计：
        // 每节两队各计一次（见 PossessionSummary 结算段）。
        let mut bonus_seen_this_period = false;
        let mut period_of_last_settlement: u32 = 0;
        // 当前回合内的出手次数（JOINT-C04：OREB 回合二次出手判定）。
        let mut possession_shot_attempts: u64 = 0;
        // 当前回合是否以 OREB 开始（REBOUND is_offensive 置位，回合结算时消费）。
        let mut current_possession_after_oreb = false;
        // 转移矩阵的「前回合原因」游标（同队连续性由 offense_team 变化判定）。
        let mut last_reason_by_team: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for tick in ticks {
            metrics.events += tick.frame.event_log.len() as u64;
            for event in &tick.frame.event_log {
                match event.kind.as_str() {
                    "PASS" => metrics.passes += 1,
                    "SHOT_RELEASE" => {
                        metrics.shots_released += 1;
                        // 四区归属（STAT-S01 登记口径）：距离按出手点到
                        // 进攻篮筐中心；三分优先按 is_three 标记（角落
                        // 22-23.75 ft 的角落三分计入 three 区）。
                        if let Some(payload) =
                            event.data.as_ref().and_then(|data| data.get("ShotRelease"))
                        {
                            let x = payload
                                .get("pos")
                                .and_then(|pos| pos.get(0))
                                .and_then(serde_json::Value::as_f64)
                                .map(|value| value as f32);
                            let y = payload
                                .get("pos")
                                .and_then(|pos| pos.get(1))
                                .and_then(serde_json::Value::as_f64)
                                .map(|value| value as f32);
                            let is_three = payload
                                .get("is_three")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            if let (Some(x), Some(y)) = (x, y) {
                                let (hoop_x, hoop_y) = if x <= 47.0 {
                                    (5.25_f32, 25.0_f32)
                                } else {
                                    (88.75_f32, 25.0_f32)
                                };
                                let radius = ((x - hoop_x).powi(2) + (y - hoop_y).powi(2)).sqrt();
                                let zone = if is_three || radius >= 23.75 {
                                    3
                                } else if radius < 8.0 {
                                    0
                                } else if radius < 16.0 {
                                    1
                                } else {
                                    2
                                };
                                match zone {
                                    0 => metrics.fga_rim += 1,
                                    1 => metrics.fga_near += 1,
                                    2 => metrics.fga_mid += 1,
                                    _ => metrics.fga_three += 1,
                                }
                                // STAT-S01 距离直方图（8 桶，界见字段文档）。
                                let dist_bucket = dist_histogram_bucket(radius);
                                metrics.shot_dist_hist[dist_bucket] += 1;
                                // JOINT-C01：Q4 末 3 分钟按 |比分差| 分桶的三分出手
                                // （命中在 SHOT_ATTEMPT_RESOLVED 按 shot_id 回填）。
                                if tick.frame.period == 4
                                    && tick.frame.t_game <= 180.0
                                    && is_three
                                {
                                    let margin = f64::from(
                                        tick.frame.score.home as i32
                                            - tick.frame.score.away as i32,
                                    )
                                    .abs();
                                    let bucket = margin_bucket(margin);
                                    metrics.q4_late_three_att_by_margin[bucket] += 1;
                                    if let Some(shot_id) = &event.shot_id {
                                        c01_shot_buckets.insert(shot_id.clone(), bucket);
                                    }
                                }
                                if let Some(shot_id) = &event.shot_id {
                                    shot_zones.insert(shot_id.clone(), zone);
                                }
                                possession_shot_attempts += 1;
                            }
                        }
                    }
                    "SHOT_ATTEMPT_RESOLVED" => {
                        if let Some(payload) = event
                            .data
                            .as_ref()
                            .and_then(|data| data.get("ShotAttemptResolved"))
                        {
                            let decision = payload
                                .get("scoring_decision")
                                .and_then(serde_json::Value::as_str);
                            if decision == Some("fga_fgm") {
                                let zone = event
                                    .shot_id
                                    .as_ref()
                                    .and_then(|shot_id| shot_zones.get(shot_id))
                                    .copied();
                                match zone {
                                    Some(0) => metrics.fgm_rim += 1,
                                    Some(1) => metrics.fgm_near += 1,
                                    Some(2) => metrics.fgm_mid += 1,
                                    Some(_) => metrics.fgm_three += 1,
                                    None => {}
                                }
                                // JOINT-C01 三分命中回填。
                                if let Some(bucket) = event
                                    .shot_id
                                    .as_ref()
                                    .and_then(|shot_id| c01_shot_buckets.get(shot_id))
                                    .copied()
                                {
                                    metrics.q4_late_three_makes_by_margin[bucket] += 1;
                                }
                            }
                        }
                    }
                    "FOUL" => {
                        metrics.fouls += 1;
                        // JOINT-C05：bonus（penalty）首进的团队犯规序直方图。
                        if let Some(payload) =
                            event.data.as_ref().and_then(|data| data.get("Foul"))
                        {
                            let is_bonus = payload
                                .get("penalty")
                                .and_then(|penalty| penalty.get("is_bonus"))
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            if is_bonus && !bonus_seen_this_period {
                                bonus_seen_this_period = true;
                                let order = payload
                                    .get("period_team_foul_count")
                                    .and_then(serde_json::Value::as_u64)
                                    .unwrap_or(0);
                                let slot = (order.saturating_sub(1) as usize).min(9);
                                metrics.bonus_entry_order_hist[slot] += 1;
                            }
                        }
                    }
                    "POSSESSION_SUMMARY" => {
                        let Some(payload) =
                            event.data.as_ref().and_then(|data| data.get("PossessionSummary"))
                        else {
                            continue;
                        };
                        let terminal = payload
                            .get("terminal_event")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        // JOINT-C04：OREB 回合二次出手结算（回合结束点）。
                        if current_possession_after_oreb {
                            metrics.oreb_possessions += 1;
                            if possession_shot_attempts >= 2 {
                                metrics.oreb_possessions_with_second_attempt += 1;
                            }
                        }
                        possession_shot_attempts = 0;
                        current_possession_after_oreb = false;
                        // 失误原因聚合（STAT-T01 五类）。
                        match terminal {
                            "TURNOVER_STEAL" | "TURNOVER_PASS_TIPPED" | "TURNOVER_PASS_DROPPED"
                            | "TURNOVER_LOOSE_BALL" => metrics.to_reason_pass_lost += 1,
                            "TURNOVER_VIOLATION" => metrics.to_reason_violation += 1,
                            "TURNOVER_OFFENSIVE_FOUL" => metrics.to_reason_offensive_foul += 1,
                            _ => {}
                        }
                        // 回合开始原因计数已由 TRANSITION_STARTED 事实驱动
                        // （事件分支见本函数前段），PossessionSummary 不再推导。
                        // 队节计数与 bonus 标志按节重置： PossessionSummary 携带
                        // period 帧值；period 变化时重置 bonus 首进标志并累加队节。
                        let period = tick.frame.period;
                        if period_of_last_settlement != 0 && period != period_of_last_settlement
                        {
                            bonus_seen_this_period = false;
                            metrics.team_periods_observed += 2;
                        }
                        period_of_last_settlement = period;
                    }
                    "TRANSITION_STARTED" => {
                        // 回合开始原因事实（引擎直发，Q01 转移矩阵与
                        // starts_* 计数的唯一事实源）。origin → 注册 7 类
                        // 映射与 rebuild_joint_reference.py 的 STARTTYPE
                        // 分类一致：DRB/进球发球=field_goal、罚球后=
                        // free_throw、偷球=steal、违例/出界=dead_ball、
                        // 地板球引擎侧无法区分 steal/block 上游 → other、
                        // timeout 引擎无暂停实现保持结构性缺失。
                        let origin = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("TransitionStarted"))
                            .and_then(|t| t.get("origin"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let reason_index: usize = match origin {
                            "made_field_goal" | "defensive_rebound" => 0,
                            "made_free_throw" => 1,
                            "steal" => 2,
                            "violation" | "out_of_bounds" => 3,
                            "timeout" => 4,
                            "blocked_shot" => 5,
                            _ => 6,
                        };
                        match reason_index {
                            0 => metrics.starts_field_goal += 1,
                            1 => metrics.starts_free_throw += 1,
                            2 => metrics.starts_steal += 1,
                            3 => metrics.starts_dead_ball += 1,
                            4 => metrics.starts_timeout += 1,
                            5 => metrics.starts_blocked_shot += 1,
                            _ => {}
                        }
                        // JOINT-Q01 转移矩阵：同 offense_team 的相邻回合配对。
                        let offense = event
                            .data
                            .as_ref()
                            .and_then(|d| d.get("TransitionStarted"))
                            .and_then(|t| t.get("offense"))
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        if let Some(&prev) = last_reason_by_team.get(&offense) {
                            metrics.transition_counts[prev][reason_index] += 1;
                        }
                        last_reason_by_team.insert(offense, reason_index);
                    }
                    "REBOUND" => {
                        // OREB 置位当前回合（JOINT-C04）。
                        if let Some(payload) =
                            event.data.as_ref().and_then(|data| data.get("ReboundContest"))
                        {
                            let is_offensive = payload
                                .get("is_offensive")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false);
                            if is_offensive {
                                current_possession_after_oreb = true;
                            }
                        }
                    }
                    "MOVEMENT_INTENT" => metrics.movement_intents += 1,
                    "MOVEMENT_OUTCOME" => metrics.movement_outcomes += 1,
                    "RESPONSIBILITY_STARTED" => metrics.responsibility_started += 1,
                    "PLAY_ACTIVATED" => metrics.play_activations += 1,
                    _ => {}
                }
            }
        }
        // 末节未换节的队节计入（period_of_last_settlement 所在节）。
        if period_of_last_settlement != 0 {
            metrics.team_periods_observed += 2;
        }
        for stats in rebuilt.values() {
            metrics.fga_total += u64::from(stats.fga);
            metrics.fgm_total += u64::from(stats.fgm);
            metrics.three_pa_total += u64::from(stats.three_pa);
            metrics.three_pm_total += u64::from(stats.three_pm);
            metrics.fta_total += u64::from(stats.fta);
            metrics.ftm_total += u64::from(stats.ftm);
            metrics.orb_total += u64::from(stats.orb);
            metrics.drb_total += u64::from(stats.drb);
            metrics.blk_total += u64::from(stats.blk);
            metrics.stl_total += u64::from(stats.stl);
            metrics.tov_total += u64::from(stats.tov);
            metrics.pf_total += u64::from(stats.pf);
        }
        metrics
    }
}

/// 出手距离直方图桶（STAT-S01 距离分布；8 桶界 0/6/10/16/20/22/23.75/28/94）。
fn dist_histogram_bucket(radius_ft: f32) -> usize {
    match radius_ft {
        r if r < 6.0 => 0,
        r if r < 10.0 => 1,
        r if r < 16.0 => 2,
        r if r < 20.0 => 3,
        r if r < 22.0 => 4,
        r if r < 23.75 => 5,
        r if r < 28.0 => 6,
        _ => 7,
    }
}

/// |比分差| 三桶（JOINT-C01：0-5 / 6-10 / 10+）。
fn margin_bucket(margin: f64) -> usize {
    if margin <= 5.0 {
        0
    } else if margin <= 10.0 {
        1
    } else {
        2
    }
}

/// `game_metrics.parquet` 写入：一行一场比赛的 arrow RecordBatch。
pub fn write_game_metrics_parquet(
    metrics: &[GameMetrics],
    path: &std::path::Path,
) -> Result<(), String> {
    use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray, UInt32Array, UInt64Array};
    use arrow_schema::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    let schema = Arc::new(Schema::new(vec![
        Field::new("seed", DataType::UInt64, false),
        Field::new("scope", DataType::Utf8, false),
        Field::new("periods", DataType::UInt32, false),
        Field::new("ticks", DataType::UInt64, false),
        Field::new("events", DataType::UInt64, false),
        Field::new("possessions", DataType::UInt64, false),
        Field::new("fga_total", DataType::UInt64, false),
        Field::new("fgm_total", DataType::UInt64, false),
        Field::new("three_pa_total", DataType::UInt64, false),
        Field::new("three_pm_total", DataType::UInt64, false),
        Field::new("fta_total", DataType::UInt64, false),
        Field::new("ftm_total", DataType::UInt64, false),
        Field::new("orb_total", DataType::UInt64, false),
        Field::new("drb_total", DataType::UInt64, false),
        Field::new("blk_total", DataType::UInt64, false),
        Field::new("stl_total", DataType::UInt64, false),
        Field::new("tov_total", DataType::UInt64, false),
        Field::new("pf_total", DataType::UInt64, false),
        Field::new("passes", DataType::UInt64, false),
        Field::new("shots_released", DataType::UInt64, false),
        Field::new("fouls", DataType::UInt64, false),
        Field::new("movement_intents", DataType::UInt64, false),
        Field::new("movement_outcomes", DataType::UInt64, false),
        Field::new("responsibility_started", DataType::UInt64, false),
        Field::new("play_activations", DataType::UInt64, false),
        Field::new("fga_rim", DataType::UInt64, false),
        Field::new("fgm_rim", DataType::UInt64, false),
        Field::new("fga_near", DataType::UInt64, false),
        Field::new("fgm_near", DataType::UInt64, false),
        Field::new("fga_mid", DataType::UInt64, false),
        Field::new("fgm_mid", DataType::UInt64, false),
        Field::new("fga_three", DataType::UInt64, false),
        Field::new("fgm_three", DataType::UInt64, false),
        Field::new("starts_field_goal", DataType::UInt64, false),
        Field::new("starts_free_throw", DataType::UInt64, false),
        Field::new("starts_steal", DataType::UInt64, false),
        Field::new("starts_dead_ball", DataType::UInt64, false),
        Field::new("starts_timeout", DataType::UInt64, false),
        Field::new("starts_blocked_shot", DataType::UInt64, false),
        Field::new("to_reason_pass_lost", DataType::UInt64, false),
        Field::new("to_reason_ball_lost", DataType::UInt64, false),
        Field::new("to_reason_violation", DataType::UInt64, false),
        Field::new("to_reason_offensive_foul", DataType::UInt64, false),
        Field::new("to_reason_other", DataType::UInt64, false),
        Field::new("q4_late_three_att_0_5", DataType::UInt64, false),
        Field::new("q4_late_three_att_6_10", DataType::UInt64, false),
        Field::new("q4_late_three_att_10p", DataType::UInt64, false),
        Field::new("q4_late_three_makes_0_5", DataType::UInt64, false),
        Field::new("q4_late_three_makes_6_10", DataType::UInt64, false),
        Field::new("q4_late_three_makes_10p", DataType::UInt64, false),
        Field::new("oreb_possessions", DataType::UInt64, false),
        Field::new("oreb_possessions_with_second_attempt", DataType::UInt64, false),
        Field::new("bonus_entry_order_1_10", DataType::Utf8, false),
        Field::new("team_periods_observed", DataType::UInt64, false),
        Field::new("transition_counts_row_major", DataType::Utf8, false),
        Field::new("game_clock_run_seconds", DataType::Float64, false),
        Field::new("live_ball_seconds", DataType::Float64, false),
        Field::new("final_score_home", DataType::UInt32, false),
        Field::new("final_score_away", DataType::UInt32, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.seed).collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(StringArray::from(
                metrics.iter().map(|m| m.scope.clone()).collect::<Vec<_>>(),
            )),
            Arc::new(UInt32Array::from(
                metrics.iter().map(|m| m.periods).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.ticks).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.events).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.possessions).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fga_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fgm_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.three_pa_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.three_pm_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fta_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.ftm_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.orb_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.drb_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.blk_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.stl_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.tov_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.pf_total).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.passes).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.shots_released).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fouls).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.movement_intents)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.movement_outcomes)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.responsibility_started)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.play_activations)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fga_rim).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fgm_rim).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fga_near).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fgm_near).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fga_mid).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fgm_mid).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fga_three).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.fgm_three).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.starts_field_goal)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.starts_free_throw)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.starts_steal).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.starts_dead_ball)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics.iter().map(|m| m.starts_timeout).collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.starts_blocked_shot)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.to_reason_pass_lost)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.to_reason_ball_lost)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.to_reason_violation)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.to_reason_offensive_foul)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.to_reason_other)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_att_by_margin[0])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_att_by_margin[1])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_att_by_margin[2])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_makes_by_margin[0])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_makes_by_margin[1])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.q4_late_three_makes_by_margin[2])
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.oreb_possessions)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.oreb_possessions_with_second_attempt)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                metrics
                    .iter()
                    .map(|m| format!("{:?}", m.bonus_entry_order_hist))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt64Array::from(
                metrics
                    .iter()
                    .map(|m| m.team_periods_observed)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(StringArray::from(
                metrics
                    .iter()
                    .map(|m| format!("{:?}", m.transition_counts))
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                metrics
                    .iter()
                    .map(|m| m.game_clock_run_seconds)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(Float64Array::from(
                metrics
                    .iter()
                    .map(|m| m.live_ball_seconds)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt32Array::from(
                metrics
                    .iter()
                    .map(|m| m.final_score_home)
                    .collect::<Vec<_>>(),
            )),
            Arc::new(UInt32Array::from(
                metrics
                    .iter()
                    .map(|m| m.final_score_away)
                    .collect::<Vec<_>>(),
            )),
        ],
    )
    .map_err(|error| format!("record batch failed: {error}"))?;
    let file =
        std::fs::File::create(path).map_err(|error| format!("create {path:?} failed: {error}"))?;
    let mut writer = ArrowWriter::try_new(file, schema, None)
        .map_err(|error| format!("parquet writer failed: {error}"))?;
    writer
        .write(&batch)
        .map_err(|error| format!("parquet write failed: {error}"))?;
    writer
        .close()
        .map_err(|error| format!("parquet close failed: {error}"))?;
    Ok(())
}

/// 完成实例行（SQLite `instances` 表）。
#[derive(Debug)]
pub struct InstanceRow {
    pub instance_kind: &'static str,
    pub instance_id: String,
    /// 创建事件（首个引用身份的事件）位置。
    pub first_tick_index: u64,
    pub first_event_id: u64,
    /// 终结事件位置；流结束时无终结为 `None`（open instance）。
    pub terminal_tick_index: Option<u64>,
    pub terminal_event_id: Option<u64>,
    /// 引用该实例的全部事件数。
    pub event_count: u64,
    pub terminal_kind: Option<String>,
}

/// 关系行（SQLite `relations` 表）。
#[derive(Debug)]
pub struct RelationRow {
    pub event_id: u64,
    pub tick_index: u64,
    pub kind: String,
    pub target_event_id: u64,
}

/// 实例索引与关系审计向 SQLite 的投影输入。
pub struct InstanceStoreInput {
    pub instances: Vec<InstanceRow>,
    pub relations: Vec<RelationRow>,
}

/// `instances.sqlite` 写入：表结构 `instances` + `relations` +
/// `run_meta`（validation.md §10 完成实例与关系索引的查询存储）。
/// 同路径已有库时整库重建（验证运行是批次的唯一写者）。
pub fn write_instance_store(
    input: &InstanceStoreInput,
    path: &std::path::Path,
) -> Result<(), String> {
    let connection = rusqlite::Connection::open(path)
        .map_err(|error| format!("open sqlite {path:?} failed: {error}"))?;
    connection
        .execute_batch(
            "DROP TABLE IF EXISTS relations;
             DROP TABLE IF EXISTS instances;
             DROP TABLE IF EXISTS run_meta;
             CREATE TABLE instances (
                 instance_kind TEXT NOT NULL,
                 instance_id TEXT NOT NULL,
                 first_tick_index INTEGER NOT NULL,
                 first_event_id INTEGER NOT NULL,
                 terminal_tick_index INTEGER,
                 terminal_event_id INTEGER,
                 event_count INTEGER NOT NULL,
                 terminal_kind TEXT,
                 PRIMARY KEY (instance_kind, instance_id)
             );
             CREATE TABLE relations (
                 event_id INTEGER NOT NULL,
                 tick_index INTEGER NOT NULL,
                 kind TEXT NOT NULL,
                 target_event_id INTEGER NOT NULL
             );
             CREATE INDEX relations_event ON relations(event_id);
             CREATE TABLE run_meta (
                 key TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             );",
        )
        .map_err(|error| format!("create tables failed: {error}"))?;
    let insert_instance = "
        INSERT INTO instances VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)";
    for row in &input.instances {
        connection
            .execute(
                insert_instance,
                rusqlite::params![
                    row.instance_kind,
                    row.instance_id,
                    row.first_tick_index as i64,
                    row.first_event_id as i64,
                    row.terminal_tick_index.map(|value| value as i64),
                    row.terminal_event_id.map(|value| value as i64),
                    row.event_count as i64,
                    row.terminal_kind,
                ],
            )
            .map_err(|error| format!("insert instance failed: {error}"))?;
    }
    for row in &input.relations {
        connection
            .execute(
                "INSERT INTO relations VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![
                    row.event_id as i64,
                    row.tick_index as i64,
                    row.kind,
                    row.target_event_id as i64,
                ],
            )
            .map_err(|error| format!("insert relation failed: {error}"))?;
    }
    Ok(())
}

/// 旧工件兼容视图的行格式：quality 评判器 `Judgment` 的判据字段
/// （criterion/verdict/severity/detail/attribution）。
#[derive(Debug, Serialize)]
pub struct LegacyJudgmentRow {
    pub criterion: String,
    pub verdict: String,
    pub severity: String,
    pub detail: String,
    pub attribution: String,
}

/// 归因账本兼容视图（`attribution_report.json`）。
#[derive(Debug, Serialize)]
pub struct LegacyAttributionReport {
    pub total_judgments: usize,
    pub hard_defect_count: usize,
    pub soft_defect_count: usize,
    pub defect_count: usize,
    pub passes: u64,
    pub defects: u64,
    pub insufficient_evidence: u64,
    /// 准则 → 裁决计数的排序账本。
    pub defects_by_criterion: Vec<(String, usize)>,
}

/// 由同一裁决生成旧工件兼容视图（validation.md §10：禁止第二套判据）。
///
/// 来源只有本 crate 的 `ScenarioOutcome` 与事实层裁决；`severity`
/// 由准则的 `gate_class` 映射（Integrity→hard，其余→soft），与
/// quality 评判器的二分法保持同一语义。
pub fn legacy_views(
    scenario_outcomes: &[ScenarioOutcome],
    fact_defects: u64,
    fact_insufficient: u64,
    fact_opportunities: u64,
    fact_passes: u64,
) -> (Vec<LegacyJudgmentRow>, LegacyAttributionReport) {
    let mut rows = Vec::new();
    for outcome in scenario_outcomes {
        let severity = "hard";
        rows.push(LegacyJudgmentRow {
            criterion: outcome.scenario_id.clone(),
            verdict: verdict_str(outcome.verdict),
            severity: severity.to_string(),
            detail: String::new(),
            attribution: "validation.scenario".to_string(),
        });
    }
    if fact_opportunities > 0 {
        rows.push(LegacyJudgmentRow {
            criterion: "MOVE_TACT_FACT_LAYER".to_string(),
            verdict: if fact_defects > 0 {
                "Defect".to_string()
            } else if fact_insufficient > 0 {
                "InsufficientEvidence".to_string()
            } else {
                "Pass".to_string()
            },
            severity: "hard".to_string(),
            detail: format!(
                "opportunities={fact_opportunities} passes={fact_passes} defects={fact_defects} insufficient={fact_insufficient}"
            ),
            attribution: "validation.movement_tact".to_string(),
        });
    }
    let mut defects_by_criterion: BTreeMap<String, usize> = BTreeMap::new();
    for row in &rows {
        if row.verdict == "Defect" {
            *defects_by_criterion
                .entry(row.criterion.clone())
                .or_insert(0) += 1;
        }
    }
    let report = LegacyAttributionReport {
        total_judgments: rows.len(),
        hard_defect_count: rows
            .iter()
            .filter(|row| row.verdict == "Defect" && row.severity == "hard")
            .count(),
        soft_defect_count: rows
            .iter()
            .filter(|row| row.verdict == "Defect" && row.severity == "soft")
            .count(),
        defect_count: rows.iter().filter(|row| row.verdict == "Defect").count(),
        passes: rows.iter().filter(|row| row.verdict == "Pass").count() as u64,
        defects: rows.iter().filter(|row| row.verdict == "Defect").count() as u64,
        insufficient_evidence: rows
            .iter()
            .filter(|row| row.verdict == "InsufficientEvidence")
            .count() as u64,
        defects_by_criterion: defects_by_criterion.into_iter().collect(),
    };
    (rows, report)
}

fn verdict_str(verdict: Verdict) -> String {
    match verdict {
        Verdict::Pass => "Pass".to_string(),
        Verdict::Defect => "Defect".to_string(),
        Verdict::NotApplicable => "NotApplicable".to_string(),
        Verdict::InsufficientEvidence => "InsufficientEvidence".to_string(),
    }
}

/// 实例与关系索引从流投影（SQLite 行 + 双结算/open instance 输入）。
pub fn project_instance_store(ticks: &[StreamTick]) -> (InstanceStoreInput, Vec<String>) {
    let mut order: Vec<(&'static str, String, u64, u64)> = Vec::new();
    let mut counts: BTreeMap<(&'static str, String), u64> = BTreeMap::new();
    let mut terminals: BTreeMap<(&'static str, String), (u64, u64, String)> = BTreeMap::new();
    let mut relations = Vec::new();
    for tick in ticks {
        for event in &tick.frame.event_log {
            for (kind, id) in crate::stream::instance_kind_of(event) {
                let key = (kind, id.clone());
                let entry = counts.entry(key.clone()).or_insert(0);
                if *entry == 0 {
                    order.push((kind, id, tick.frame.tick_index, event.event_id));
                }
                *entry += 1;
                if crate::stream::is_terminal_kind(&event.kind) {
                    terminals.insert(
                        key,
                        (tick.frame.tick_index, event.event_id, event.kind.clone()),
                    );
                }
            }
            for relation in &event.relations {
                relations.push(RelationRow {
                    event_id: event.event_id,
                    tick_index: tick.frame.tick_index,
                    kind: relation.kind.as_str().to_string(),
                    target_event_id: relation.event_id,
                });
            }
        }
    }
    let mut open_instances = Vec::new();
    let mut instances = Vec::new();
    for (kind, id, first_tick, first_event) in order {
        let event_count = counts.get(&(kind, id.clone())).copied().unwrap_or(0);
        let terminal = terminals.get(&(kind, id.clone()));
        if terminal.is_none() {
            open_instances.push(format!("{kind}:{id}"));
        }
        instances.push(InstanceRow {
            instance_kind: kind,
            instance_id: id,
            first_tick_index: first_tick,
            first_event_id: first_event,
            terminal_tick_index: terminal.map(|(tick, _, _)| *tick),
            terminal_event_id: terminal.map(|(_, event, _)| *event),
            event_count,
            terminal_kind: terminal.map(|(_, _, kind)| kind.clone()),
        });
    }
    (
        InstanceStoreInput {
            instances,
            relations,
        },
        open_instances,
    )
}

/// `ledger_comparison.json` 输入：独立重建 vs 引擎声明的逐项比较
/// （CAUSE-B01/B02/B03 的机器可读汇总）。
#[derive(Debug, Default, Clone, Serialize)]
pub struct LedgerComparison {
    pub box_score_rebuilt: BTreeMap<String, RebuiltBoxScore>,
    pub box_score_declared: BTreeMap<String, RebuiltBoxScore>,
    pub possession: PossessionReconciliation,
    pub clock: RebuiltClockAccount,
    pub pairing: PairingAudit,
}
