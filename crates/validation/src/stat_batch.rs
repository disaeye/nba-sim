//! 批次 STAT 等效判定（validation.md §6.3/§6.4）。
//!
//! 多场批次的每场充分统计量（`GameMetrics`）与登记参照比对：统计量为
//! 「模拟批次均值 - 参照值」，允许区间为 `[-delta, +delta]`；区间用
//! 以比赛为 cluster 的 bootstrap（`stats.rs` 的配对实现）估计。
//! 判定三出口（§6.3）：区间完全在允许带内 Pass；无交集 Defect；
//! 部分重叠 InsufficientEvidence。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::artifacts::GameMetrics;
use crate::composition::{evaluate_composition_groups, BatchGroupVerdict};
use crate::reference::{RegistrationValue, StatDeltaRegistration};

/// 单条指标的批次判定结果。
#[derive(Debug, Clone, Serialize)]
pub struct BatchMetricVerdict {
    pub criterion_id: String,
    pub metric: String,
    /// 模拟批次均值。
    pub simulated_mean: f64,
    /// 登记参照值。
    pub reference_value: f64,
    /// 允许带半宽（模拟侧统计量与参照值之差的允许绝对值）。
    pub allowed_delta: f64,
    /// bootstrap 置信区间（模拟均值 - 参照值 的分布）。
    pub interval_lower: f64,
    pub interval_upper: f64,
    /// Pass / Defect / InsufficientEvidence。
    pub verdict: String,
    pub reason_code: String,
}

/// 单个登记指标的覆盖说明（参与判定 / 跳过及原因）。
#[derive(Debug, Clone, Serialize)]
pub struct MetricCoverage {
    pub criterion_id: String,
    pub metric: String,
    pub data_coverage: String,
    /// judged | skipped_blocked | skipped_no_reference | skipped_no_simulated_data
    pub status: String,
    pub detail: String,
}

/// 整个批次的 STAT 判定汇总。
#[derive(Debug, Clone, Serialize)]
pub struct BatchStatReport {
    pub n_games: u32,
    pub scope: String,
    pub verdicts: Vec<BatchMetricVerdict>,
    /// 构成组级判定（TV 口径；逐指标判定之外的份额向量整体比较）。
    pub group_verdicts: Vec<BatchGroupVerdict>,
    /// full 叶项内全部已判指标的合取结果（叶项级：任一子指标超差即超差）。
    pub criterion_verdicts: BTreeMap<String, String>,
    /// 每条登记条目的覆盖说明（判定透明度：哪些参判、哪些跳过、为什么）。
    pub coverage: Vec<MetricCoverage>,
}

/// 从登记条目抽取标量参照值（RegistrationValue::Number 直取；
/// Map 里取 `pace`/`orb_pct` 等已知键由调用方给键名）。
fn scalar_reference(value: &RegistrationValue, key: Option<&str>) -> Option<f64> {
    match value {
        RegistrationValue::Number(number) => Some(*number),
        RegistrationValue::Map(map) => {
            let key = key?;
            map.get(key).and_then(serde_json::Value::as_f64)
        }
        RegistrationValue::Text(_) => None,
    }
}

/// 失误原因 14 类 → 判定五类的聚合键（与 artifacts.rs 的 to_reason_* 对齐）。
fn aggregated_reason_share(reason_shares: &serde_json::Map<String, serde_json::Value>, agg: &str) -> Option<f64> {
    let get = |name: &str| reason_shares.get(name).and_then(serde_json::Value::as_f64);
    match agg {
        "agg_pass_lost" => Some(get("BadPass")? + get("OutOfBoundsBadPass")?),
        "agg_ball_lost" => Some(get("LostBall")? + get("OutOfBoundsLostBall")?),
        "agg_violation" => Some(
            get("ShotClock")? + get("Travel")? + get("Backcourt")?
                + get("DiscontinueDribble")? + get("LaneViolation")?
                + get("DoubleDribble")? + get("KickBall")? + get("FTViolation")?,
        ),
        "agg_offensive_foul" => get("OffensiveFoul"),
        _ => None,
    }
}

/// 特殊形态参照键的读取：matrix:/agg_*/q4_*/bonus_order_* 前缀。
fn structured_reference(
    value: &RegistrationValue,
    key: Option<&str>,
) -> Option<f64> {
    let map = match value {
        RegistrationValue::Map(map) => map,
        _ => return None,
    };
    let key = key?;
    if let Some(rest) = key.strip_prefix(MATRIX_KEY_PREFIX) {
        // matrix:<row_reason>:<col_reason>
        let mut parts = rest.split(':');
        let row = parts.next()?;
        let col = parts.next()?;
        let matrix = map.get("transition_matrix")?;
        let row_map = matrix.get(row)?;
        return row_map.get(col).and_then(serde_json::Value::as_f64);
    }
    if let Some(agg) = key.strip_prefix("agg_") {
        let shares = map.get("reason_shares")?;
        return aggregated_reason_share(shares.as_object()?, agg);
    }
    if let Some(label) = key.strip_prefix("q4_") {
        let buckets = map.get("league_three_share_by_margin")?;
        let json_label = match label {
            "0_5" => "0-5",
            "6_10" => "6-10",
            "10p" => "10+",
            _ => return None,
        };
        let bucket = buckets.get(json_label)?;
        return bucket.get("league_three_share").and_then(serde_json::Value::as_f64);
    }
    if let Some(order) = key.strip_prefix("bonus_order_") {
        let shares = map.get("bonus_entry_foul_order_shares")?;
        let json_key = match order {
            "1" => "foul_1",
            "2" => "foul_2",
            "3" => "foul_3",
            "4" => "foul_4",
            "5" => "foul_5",
            // 6 次及以后：1 - 前五序份额。
            "6p" => {
                let mut acc = 0.0;
                for i in 1..=5 {
                    acc += shares.get(format!("foul_{i}").as_str()).and_then(serde_json::Value::as_f64)?;
                }
                return Some(1.0 - acc);
            }
            _ => return None,
        };
        return shares.get(json_key).and_then(serde_json::Value::as_f64);
    }
    None
}

/// 模拟批次相对参照的各指标均值序列。
fn simulated_means(metrics: &[GameMetrics]) -> BTreeMap<&'static str, f64> {
    let n = metrics.len() as f64;
    let mut out = BTreeMap::new();
    // STAT-S01：命中率、三分率、三分占比（team-game 均值的联盟聚合）
    let fga: f64 = metrics.iter().map(|m| m.fga_total).sum::<u64>() as f64;
    let fgm: f64 = metrics.iter().map(|m| m.fgm_total).sum::<u64>() as f64;
    let three_pa: f64 = metrics.iter().map(|m| m.three_pa_total).sum::<u64>() as f64;
    let three_pm: f64 = metrics.iter().map(|m| m.three_pm_total).sum::<u64>() as f64;
    let fta: f64 = metrics.iter().map(|m| m.fta_total).sum::<u64>() as f64;
    let ftm: f64 = metrics.iter().map(|m| m.ftm_total).sum::<u64>() as f64;
    out.insert("fg_pct", if fga > 0.0 { fgm / fga } else { 0.0 });
    out.insert("three_pa_share", if fga > 0.0 { three_pa / fga } else { 0.0 });
    out.insert("three_pct", if three_pa > 0.0 { three_pm / three_pa } else { 0.0 });
    // STAT-G01：team-game 投篮得分（2/3 分，不含罚球）
    let mut shot_points = 0.0;
    for m in metrics {
        let two_m = m.fgm_total as i64 - m.three_pm_total as i64;
        shot_points += (two_m * 2 + m.three_pm_total as i64 * 3) as f64;
    }
    out.insert("shot_points_team_game", shot_points / n);
    // STAT-R01
    out.insert("fta_fga", if fga > 0.0 { fta / fga } else { 0.0 });
    out.insert("ft_pct", if fta > 0.0 { ftm / fta } else { 0.0 });
    let poss_total: f64 = metrics.iter().map(|m| m.possessions).sum::<u64>() as f64;
    let fouls: f64 = metrics.iter().map(|m| m.fouls).sum::<u64>() as f64;
    out.insert(
        "fouls_per100",
        if poss_total > 0.0 { 100.0 * fouls / poss_total } else { 0.0 },
    );
    // STAT-T01
    let tov: f64 = metrics.iter().map(|m| m.tov_total).sum::<u64>() as f64;
    out.insert(
        "tov_rate",
        if poss_total > 0.0 { tov / poss_total } else { 0.0 },
    );
    // STAT-B01：ORB%（ORB / (ORB + 对方 DRB)；两队合计口径下
    // ORB_total / (ORB_total + DRB_total) 是联盟聚合的等价形式）
    let orb: f64 = metrics.iter().map(|m| m.orb_total).sum::<u64>() as f64;
    let drb: f64 = metrics.iter().map(|m| m.drb_total).sum::<u64>() as f64;
    out.insert("orb_pct", if orb + drb > 0.0 { orb / (orb + drb) } else { 0.0 });
    // STAT-G02：Pace（team-game 回合数；分母 48 分钟口径，加时场次由
    // periods 字段折算）
    let mut pace_sum = 0.0;
    for m in metrics {
        let minutes = f64::from(m.periods) * 12.0;
        if minutes > 0.0 {
            // m.possessions 是两队合计回合；Pace 公式分母有 2（§6.2）。
            pace_sum += m.possessions as f64 * 48.0 / (2.0 * minutes);
        }
    }
    out.insert("pace", pace_sum / n);
    // 四区份额构成（STAT-S01 TV 条目的模拟侧合并构成）
    let zone_fga: f64 = (metrics.iter().map(|m| m.fga_rim).sum::<u64>()
        + metrics.iter().map(|m| m.fga_near).sum::<u64>()
        + metrics.iter().map(|m| m.fga_mid).sum::<u64>()
        + metrics.iter().map(|m| m.fga_three).sum::<u64>())
        as f64;
    if zone_fga > 0.0 {
        let rim: u64 = metrics.iter().map(|m| m.fga_rim).sum();
        let near: u64 = metrics.iter().map(|m| m.fga_near).sum();
        let mid: u64 = metrics.iter().map(|m| m.fga_mid).sum();
        let three: u64 = metrics.iter().map(|m| m.fga_three).sum();
        out.insert("share_rim", rim as f64 / zone_fga);
        out.insert("share_near", near as f64 / zone_fga);
        out.insert("share_mid", mid as f64 / zone_fga);
        out.insert("share_three", three as f64 / zone_fga);
    }
    // JOINT-Q01：回合开始原因构成（七类份额）
    let starts_total: f64 = metrics
        .iter()
        .map(|m| {
            m.starts_field_goal + m.starts_free_throw + m.starts_steal
                + m.starts_dead_ball + m.starts_timeout + m.starts_blocked_shot
        })
        .sum::<u64>() as f64;
    if starts_total > 0.0 {
        for (key, counter) in [
            ("starts_field_goal_share", m_count(metrics, |m| m.starts_field_goal)),
            ("starts_free_throw_share", m_count(metrics, |m| m.starts_free_throw)),
            ("starts_steal_share", m_count(metrics, |m| m.starts_steal)),
            ("starts_dead_ball_share", m_count(metrics, |m| m.starts_dead_ball)),
            ("starts_timeout_share", m_count(metrics, |m| m.starts_timeout)),
            ("starts_blocked_shot_share", m_count(metrics, |m| m.starts_blocked_shot)),
        ] {
            out.insert(key, counter / starts_total);
        }
    }
    // JOINT-C04：OREB 回合占比与二次出手率
    if poss_total > 0.0 {
        let oreb: f64 = metrics.iter().map(|m| m.orb_total).sum::<u64>() as f64;
        out.insert("oreb_possession_share", oreb / poss_total);
    }
    let oreb_possessions: f64 = metrics
        .iter()
        .map(|m| m.oreb_possessions)
        .sum::<u64>() as f64;
    if oreb_possessions > 0.0 {
        let with_second: f64 = metrics
            .iter()
            .map(|m| m.oreb_possessions_with_second_attempt)
            .sum::<u64>() as f64;
        out.insert(
            "second_attempt_share_after_oreb",
            with_second / oreb_possessions,
        );
    }
    // STAT-T01：失误原因聚合五类份额（登记侧 14 类按同口径聚合）
    let to_total: f64 = metrics
        .iter()
        .map(|m| {
            m.to_reason_pass_lost
                + m.to_reason_ball_lost
                + m.to_reason_violation
                + m.to_reason_offensive_foul
                + m.to_reason_other
        })
        .sum::<u64>() as f64;
    if to_total > 0.0 {
        out.insert(
            "to_reason_pass_lost_share",
            m_count(metrics, |m| m.to_reason_pass_lost) / to_total,
        );
        out.insert(
            "to_reason_ball_lost_share",
            m_count(metrics, |m| m.to_reason_ball_lost) / to_total,
        );
        out.insert(
            "to_reason_violation_share",
            m_count(metrics, |m| m.to_reason_violation) / to_total,
        );
        out.insert(
            "to_reason_offensive_foul_share",
            m_count(metrics, |m| m.to_reason_offensive_foul) / to_total,
        );
    }
    // JOINT-C01：Q4 末段三分占比（|比分差| 三桶）
    let q4_att: [f64; 3] = [
        m_count(metrics, |m| m.q4_late_three_att_by_margin[0]),
        m_count(metrics, |m| m.q4_late_three_att_by_margin[1]),
        m_count(metrics, |m| m.q4_late_three_att_by_margin[2]),
    ];
    let q4_makes: [f64; 3] = [
        m_count(metrics, |m| m.q4_late_three_makes_by_margin[0]),
        m_count(metrics, |m| m.q4_late_three_makes_by_margin[1]),
        m_count(metrics, |m| m.q4_late_three_makes_by_margin[2]),
    ];
    for (bucket, label) in q4_att.iter().enumerate() {
        if *label > 0.0 {
            out.insert(
                match bucket {
                    0 => "q4_late_three_share_0_5",
                    1 => "q4_late_three_share_6_10",
                    _ => "q4_late_three_share_10p",
                },
                q4_makes[bucket] / label,
            );
        }
    }
    // JOINT-C05：bonus 首进犯规序分布（10 桶直方图 → 份额）
    let bonus_total: f64 = metrics
        .iter()
        .map(|m| m.bonus_entry_order_hist.iter().sum::<u64>())
        .sum::<u64>() as f64;
    if bonus_total > 0.0 {
        // 前五序（1..=5）是判定关注区；6..=10 合并进「六次及以后」份额。
        for (slot, key) in [
            "bonus_entry_order_1_share",
            "bonus_entry_order_2_share",
            "bonus_entry_order_3_share",
            "bonus_entry_order_4_share",
            "bonus_entry_order_5_share",
        ]
        .iter()
        .enumerate()
        {
            let count: f64 = metrics
                .iter()
                .map(|m| m.bonus_entry_order_hist[slot])
                .sum::<u64>() as f64;
            out.insert(key, count / bonus_total);
        }
        let late: f64 = metrics
            .iter()
            .map(|m| m.bonus_entry_order_hist[5..].iter().sum::<u64>())
            .sum::<u64>() as f64;
        out.insert("bonus_entry_order_6p_share", late / bonus_total);
    }
    // JOINT-Q01 转移矩阵：行占比（行=前回合原因，7 类；每行内归一）
    for row in 0..7 {
        let row_total: f64 = metrics
            .iter()
            .map(|m| m.transition_counts[row].iter().sum::<u64>())
            .sum::<u64>() as f64;
        if row_total > 0.0 {
            for col in 0..7 {
                let count: f64 = metrics
                    .iter()
                    .map(|m| m.transition_counts[row][col])
                    .sum::<u64>() as f64;
                out.insert(
                    transition_key(row, col),
                    count / row_total,
                );
            }
        }
    }
    // 四区各自 FG%（STAT-S01 第五条登记条目）
    for (key, made, att) in [
        ("zone_fg_rim", m_count(metrics, |m| m.fgm_rim), m_count(metrics, |m| m.fga_rim)),
        ("zone_fg_near", m_count(metrics, |m| m.fgm_near), m_count(metrics, |m| m.fga_near)),
        ("zone_fg_mid", m_count(metrics, |m| m.fgm_mid), m_count(metrics, |m| m.fga_mid)),
        ("zone_fg_three", m_count(metrics, |m| m.fgm_three), m_count(metrics, |m| m.fga_three)),
    ] {
        if att > 0.0 {
            out.insert(key, made / att);
        }
    }
    out
}

/// 字段求和（含数组下标场景的共用小函数）。
fn m_count(metrics: &[GameMetrics], f: impl Fn(&GameMetrics) -> u64) -> f64 {
    metrics.iter().map(f).sum::<u64>() as f64
}

/// 转移矩阵 (row, col) 的指标键名（行=前回合原因，列=本回合原因；
/// 顺序与登记的 transition_matrix_order 一致：field_goal/free_throw/steal/
/// dead_ball/timeout/blocked_shot/other）。
#[rustfmt::skip]
fn transition_key(row: usize, col: usize) -> &'static str {
    const KEYS: [[&str; 7]; 7] = [
        ["trans_fg_fg",       "trans_fg_ft",       "trans_fg_steal",   "trans_fg_dead",   "trans_fg_timeout",   "trans_fg_block",   "trans_fg_other"],
        ["trans_ft_fg",       "trans_ft_ft",       "trans_ft_steal",   "trans_ft_dead",   "trans_ft_timeout",   "trans_ft_block",   "trans_ft_other"],
        ["trans_steal_fg",    "trans_steal_ft",    "trans_steal_steal","trans_steal_dead","trans_steal_timeout","trans_steal_block","trans_steal_other"],
        ["trans_dead_fg",     "trans_dead_ft",     "trans_dead_steal", "trans_dead_dead", "trans_dead_timeout", "trans_dead_block", "trans_dead_other"],
        ["trans_timeout_fg",  "trans_timeout_ft",  "trans_timeout_steal","trans_timeout_dead","trans_timeout_timeout","trans_timeout_block","trans_timeout_other"],
        ["trans_block_fg",    "trans_block_ft",    "trans_block_steal","trans_block_dead","trans_block_timeout","trans_block_block","trans_block_other"],
        ["trans_other_fg",    "trans_other_ft",    "trans_other_steal","trans_other_dead","trans_other_timeout","trans_other_block","trans_other_other"],
    ];
    KEYS[row][col]
}

/// 指标键 → (criterion_id, metric 名, 登记参照键)。
fn metric_registry() -> Vec<(&'static str, &'static str, &'static str, Option<&'static str>)> {
    vec![
        ("fg_pct", "STAT-S01", "team-game FG%（汇总分子分母）", None),
        ("three_pa_share", "STAT-S01", "team-game 3PA/FGA 份额", None),
        ("three_pct", "STAT-S01", "team-game 3P%", None),
        (
            "share_rim",
            "STAT-S01",
            "四区出手份额构成（Rim/Near/Mid/Three）",
            Some("rim"),
        ),
        (
            "share_near",
            "STAT-S01",
            "四区出手份额构成（Rim/Near/Mid/Three）",
            Some("near"),
        ),
        (
            "share_mid",
            "STAT-S01",
            "四区出手份额构成（Rim/Near/Mid/Three）",
            Some("mid"),
        ),
        (
            "share_three",
            "STAT-S01",
            "四区出手份额构成（Rim/Near/Mid/Three）",
            Some("three"),
        ),
        ("shot_points_team_game", "STAT-G01", "team-game 投篮得分（不含罚球）", None),
        ("fta_fga", "STAT-R01", "FTA/FGA、FTM/FTA、每百回合犯规、投篮犯规占比、奖励阶段罚球", Some("FTA/FGA")),
        ("ft_pct", "STAT-R01", "FTA/FGA、FTM/FTA、每百回合犯规、投篮犯规占比、奖励阶段罚球", Some("FTM/FTA")),
        ("fouls_per100", "STAT-R01", "FTA/FGA、FTM/FTA、每百回合犯规、投篮犯规占比、奖励阶段罚球", Some("fouls_per100")),
        ("tov_rate", "STAT-T01", "TOV/possessions 与失误原因份额", Some("tov_rate")),
        ("orb_pct", "STAT-B01", "ORB 率与前场篮板后出手率", Some("orb_pct")),
        ("pace", "STAT-G02", "回合数与 Pace", Some("pace")),
        (
            "oreb_possession_share",
            "JOINT-C04",
            "出手位置×球运动×卡位×前场篮板二次进攻",
            Some("oreb_possession_share"),
        ),
        (
            "starts_field_goal_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("field_goal"),
        ),
        (
            "starts_steal_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("steal"),
        ),
        (
            "starts_dead_ball_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("dead_ball"),
        ),
        (
            "starts_free_throw_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("free_throw"),
        ),
        (
            "starts_timeout_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("timeout"),
        ),
        (
            "starts_blocked_shot_share",
            "JOINT-Q01",
            "连续回合转移矩阵与 lag-1/lag-2 条件统计",
            Some("blocked_shot"),
        ),
        // JOINT-Q01 转移矩阵逐格与四区各自 FG% 等剩余条目由
        // metric_registry_with_matrix 在函数外拼接（vec! 不支持元素展开）。
        // 四区各自 FG%（STAT-S01 第五条）
        ("zone_fg_rim", "STAT-S01", "四区各自 FG%（Rim/Near/Mid/Three）", Some("rim")),
        ("zone_fg_near", "STAT-S01", "四区各自 FG%（Rim/Near/Mid/Three）", Some("near")),
        ("zone_fg_mid", "STAT-S01", "四区各自 FG%（Rim/Near/Mid/Three）", Some("mid")),
        ("zone_fg_three", "STAT-S01", "四区各自 FG%（Rim/Near/Mid/Three）", Some("three")),
        // STAT-T01 失误原因聚合份额由构成组级 TV 判定覆盖
        // （evaluate_composition_groups 的 turnover_reason_composition），
        // 登记无逐份额允许带，不设逐指标判定。
        // JOINT-C04 二次出手率
        (
            "second_attempt_share_after_oreb",
            "JOINT-C04",
            "出手位置×球运动×卡位×前场篮板二次进攻",
            Some("second_attempt_share_after_oreb"),
        ),
        // JOINT-C01 Q4 末段三分占比（|比分差| 三桶；登记允许带为逐桶绝对差）
        ("q4_late_three_share_0_5", "JOINT-C01", "比分差×剩余时间×进攻时钟条件下的节奏与出手", Some("q4_0_5")),
        ("q4_late_three_share_6_10", "JOINT-C01", "比分差×剩余时间×进攻时钟条件下的节奏与出手", Some("q4_6_10")),
        ("q4_late_three_share_10p", "JOINT-C01", "比分差×剩余时间×进攻时钟条件下的节奏与出手", Some("q4_10p")),
        // JOINT-C05 bonus 首进犯规序份额由构成组级 TV 判定覆盖
        // （evaluate_composition_groups 的 bonus_entry_order_composition），
        // 登记无逐序允许带，不设逐指标判定。
    ]
}

/// 完整指标注册：基础表 + 转移矩阵 49 格。
fn full_metric_registry()
-> Vec<(&'static str, &'static str, &'static str, Option<&'static str>)> {
    let mut registry = metric_registry();
    registry.extend(transition_matrix_registry());
    registry
}

/// JOINT-Q01 转移矩阵 49 格的指标注册（键名与登记矩阵格对齐）。
fn transition_matrix_registry()
-> Vec<(&'static str, &'static str, &'static str, Option<&'static str>)> {
    const REASONS: [&str; 7] = [
        "field_goal",
        "free_throw",
        "steal",
        "dead_ball",
        "timeout",
        "blocked_shot",
        "other",
    ];
    (0..7)
        .flat_map(|row| {
            (0..7).map(move |col| {
                // 登记参照键 matrix:<row_reason>:<col_reason>；
                // structured_reference 据此读 transition_matrix[row][col]。
                let key: &'static str = Box::leak(
                    format!(
                        "{MATRIX_KEY_PREFIX}{}:{}",
                        REASONS[row], REASONS[col]
                    )
                    .into_boxed_str(),
                );
                (
                    transition_key(row, col),
                    "JOINT-Q01",
                    "连续回合转移矩阵与 lag-1/lag-2 条件统计",
                    Some(key),
                )
            })
        })
        .collect()
}

/// 转移矩阵参照键的特判前缀（登记里矩阵是嵌套 map，键 = matrix:<row_reason>:<col_reason>）。
const MATRIX_KEY_PREFIX: &str = "matrix:";

/// 从登记读取某条目的允许带（数值或映射中指定键；均为相对参照值的绝对差）。
/// 转移矩阵格（`matrix:` 前缀键）统一用 `transition_matrix_cell_abs` 允许带。
fn allowed_delta_of(entry: &crate::reference::DeltaEntry, key: Option<&str>) -> Option<f64> {
    match entry.allowed_delta.as_ref()? {
        RegistrationValue::Number(number) => Some(*number),
        RegistrationValue::Map(map) => {
            let key = key?;
            if key.starts_with(MATRIX_KEY_PREFIX) {
                return map
                    .get("transition_matrix_cell_abs")
                    .and_then(serde_json::Value::as_f64);
            }
            if let Some(value) = map.get(key).and_then(serde_json::Value::as_f64) {
                return Some(value);
            }
            // C04 键名差异：参照键 second_attempt_share_after_oreb 的允许带
            // 在登记里叫 second_attempt_share。
            if key == "second_attempt_share_after_oreb" {
                return map
                    .get("second_attempt_share")
                    .and_then(serde_json::Value::as_f64);
            }
            // C01 逐桶判定共用逐桶绝对差允许带。
            if key.starts_with("q4_") {
                return map
                    .get("league_three_share_by_margin_abs")
                    .and_then(serde_json::Value::as_f64);
            }
            None
        }
        RegistrationValue::Text(_) => None,
    }
}


/// 出手距离直方图的桶中心（与 `dist_histogram_bucket` 的 8 桶界对应）。
const DIST_BUCKET_CENTERS: [f64; 8] = [3.0, 8.0, 13.0, 18.0, 21.0, 22.875, 25.875, 61.0];

/// 从登记条目顶层的 reference_quantiles 构造参照样本序列（每分位重复 `per_quantile` 次）。
fn distance_reference_samples(entry: &crate::reference::DeltaEntry, per_quantile: usize) -> Vec<f64> {
    let mut out = Vec::new();
    for key in ["p10", "p25", "p50", "p75", "p90"] {
        if let Some(v) = entry
            .reference_quantiles
            .get(key)
            .and_then(serde_json::Value::as_f64)
        {
            out.extend(std::iter::repeat_n(v, per_quantile));
        }
    }
    out
}

/// 执行批次 STAT 判定。
///
/// `bootstrap_replicates` 控制区间精度；种子固定保证可复现
/// （validation.md §6.3：登记 bootstrap 算法与随机种子）。
pub fn evaluate_batch(
    metrics: &[GameMetrics],
    registration: &StatDeltaRegistration,
    bootstrap_replicates: usize,
    seed: u64,
) -> BatchStatReport {
    use crate::stats::{BootstrapConfig, compare_mean_difference, PairedGameValues};

    let means = simulated_means(metrics);
    let scope = metrics.first().map(|m| m.scope.clone()).unwrap_or_default();
    let mut verdicts = Vec::new();
    let mut criterion_verdicts: BTreeMap<String, String> = BTreeMap::new();

    let mut coverage: Vec<MetricCoverage> = Vec::new();
    let mut judged_metrics: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    // (criterion, metric) 级去重：同一登记条目的多个指标键（如矩阵 49 格）只记一次。
    let mut coverage_seen: std::collections::HashSet<(String, String)> =
        std::collections::HashSet::new();
    // 组级判定涉及的叶项（条目级参与判定证据）。
    let mut judged_criterions: std::collections::HashSet<String> =
        std::collections::HashSet::new();
    for (metric_key, criterion_id, metric_name, reference_key) in full_metric_registry() {
        let entry = registration.deltas.iter().find(|entry| {
            entry.criterion_id == criterion_id && entry.metric == metric_name
        });
        let Some(entry) = entry else {
            continue;
        };
        if !entry.data_coverage.starts_with("full") {
            if coverage_seen.insert((criterion_id.to_string(), metric_name.to_string())) {
                coverage.push(MetricCoverage {
                    criterion_id: criterion_id.to_string(),
                    metric: metric_name.to_string(),
                    data_coverage: entry.data_coverage.clone(),
                    status: "skipped_blocked".to_string(),
                    detail: entry.reason_code.clone().unwrap_or_else(|| "blocked".to_string()),
                });
            }
            continue;
        }
        let Some(reference) = entry.reference_value.as_ref().and_then(|value| {
            structured_reference(value, reference_key)
                .or_else(|| scalar_reference(value, reference_key))
        }) else {
            if coverage_seen.insert((criterion_id.to_string(), metric_name.to_string())) {
                coverage.push(MetricCoverage {
                    criterion_id: criterion_id.to_string(),
                    metric: metric_name.to_string(),
                    data_coverage: entry.data_coverage.clone(),
                    status: "skipped_no_reference".to_string(),
                    detail: "参照值形态未对齐判定口径".to_string(),
                });
            }
            continue;
        };
        let Some(allowed) = allowed_delta_of(entry, reference_key) else {
            if coverage_seen.insert((criterion_id.to_string(), metric_name.to_string())) {
                coverage.push(MetricCoverage {
                    criterion_id: criterion_id.to_string(),
                    metric: metric_name.to_string(),
                    data_coverage: entry.data_coverage.clone(),
                    status: "skipped_no_reference".to_string(),
                    detail: "允许带形态未对齐判定口径".to_string(),
                });
            }
            continue;
        };
        let simulated = means.get(metric_key).copied().unwrap_or(f64::NAN);
        if !simulated.is_finite() {
            if coverage_seen.insert((criterion_id.to_string(), metric_name.to_string())) {
                coverage.push(MetricCoverage {
                    criterion_id: criterion_id.to_string(),
                    metric: metric_name.to_string(),
                    data_coverage: entry.data_coverage.clone(),
                    status: "skipped_no_simulated_data".to_string(),
                    detail: "模拟批次无该指标数据（如 scope 无对应阶段）".to_string(),
                });
            }
            continue;
        }
        judged_metrics
            .insert((criterion_id.to_string(), metric_name.to_string()));
        // 该条目已有 skipped 记录（同条目其他键先失败）且本键判定成功：
        // 条目级状态以「有判定」为准，删除 skipped 记录。
        coverage.retain(|c| {
            !(c.criterion_id == criterion_id
                && c.metric == metric_name
                && c.status.starts_with("skipped"))
        });
        // 配对 bootstrap：模拟批次均值的抽樣分布 vs 固定参照值；
        // PairedGameValues 的 reference 侧填参照值（常量），simulated 侧
        // 填各场统计量——差值分布即模拟批次的抽样波动。
        let paired: Vec<PairedGameValues> = metrics
            .iter()
            .map(|m| {
                let per_game = per_game_value(m, metric_key);
                PairedGameValues {
                    game_id: format!("g{}", m.seed),
                    simulated: per_game,
                    reference,
                }
            })
            .collect();
        let config = BootstrapConfig {
            replicates: bootstrap_replicates as u32,
            family_size: 10,
            alpha: 0.05,
            seed,
        };
        let (interval, verdict) = compare_mean_difference(&paired, allowed, &config);
        let verdict_str = match verdict {
            crate::stats::EquivalenceVerdict::Pass => "Pass",
            crate::stats::EquivalenceVerdict::Defect => "Defect",
            crate::stats::EquivalenceVerdict::InsufficientEvidence => "InsufficientEvidence",
        };
        // 叶项级合取：任一子指标非 Pass，叶项不 Pass；优先级 Defect > Insufficient > Pass
        let slot = criterion_verdicts
            .entry(criterion_id.to_string())
            .or_insert("Pass".to_string());
        match verdict_str {
            "Defect" => *slot = "Defect".to_string(),
            "InsufficientEvidence" if *slot != "Defect" => {
                *slot = "InsufficientEvidence".to_string();
            }
            _ => {}
        }
        verdicts.push(BatchMetricVerdict {
            criterion_id: criterion_id.to_string(),
            metric: metric_name.to_string(),
            simulated_mean: simulated,
            reference_value: reference,
            allowed_delta: allowed,
            interval_lower: interval.lower,
            interval_upper: interval.upper,
            verdict: verdict_str.to_string(),
            reason_code: "batch_equivalence".to_string(),
        });
    }

    // 构成组级判定（份额向量整体 TV 比较；登记允许带为 TV 上界）。
    let group_verdicts =
        evaluate_composition_groups(metrics, registration, bootstrap_replicates, seed);
    for group in &group_verdicts {
        judged_metrics.insert((group.criterion_id.clone(), group.group.to_string()));
        judged_criterions.insert(group.criterion_id.clone());
    }
    // full 条目中无任何逐指标判定的（例如 scope 覆盖不了观测窗口）记为未参判。
    for entry in &registration.deltas {
        let not_judged = !judged_metrics
            .contains(&(entry.criterion_id.clone(), entry.metric.clone()))
            && !judged_criterions.contains(&entry.criterion_id);
        let not_covered = !coverage
            .iter()
            .any(|c| c.criterion_id == entry.criterion_id && c.metric == entry.metric);
        if entry.data_coverage.starts_with("full")
            && not_judged
            && not_covered
            && coverage_seen.insert((entry.criterion_id.clone(), entry.metric.clone()))
        {
            coverage.push(MetricCoverage {
                criterion_id: entry.criterion_id.clone(),
                metric: entry.metric.clone(),
                data_coverage: entry.data_coverage.clone(),
                status: "skipped_no_simulated_data".to_string(),
                detail: "模拟批次无该条目任何指标数据".to_string(),
            });
        }
    }

    // STAT-S01 出手距离分布（W1 口径）：模拟直方图 → 桶中心样本。
    if let Some(entry) = registration
        .deltas
        .iter()
        .find(|e| e.criterion_id == "STAT-S01" && e.metric == "出手距离分布")
    {
        let total_hist: u64 = metrics.iter().map(|m| m.shot_dist_hist.iter().sum::<u64>()).sum();
        let reference_samples = distance_reference_samples(entry, 20);
        if total_hist > 0 && !reference_samples.is_empty() {
            let mut simulated_samples: Vec<f64> = Vec::with_capacity(total_hist as usize);
            let mut merged_hist = [0u64; 8];
            for m in metrics {
                for (merged, &count) in merged_hist.iter_mut().zip(&m.shot_dist_hist) {
                    *merged += count;
                }
            }
            for (bucket, &count) in merged_hist.iter().enumerate() {
                simulated_samples
                    .extend(std::iter::repeat_n(DIST_BUCKET_CENTERS[bucket], count as usize));
            }
            let config = BootstrapConfig {
                replicates: bootstrap_replicates as u32,
                family_size: 10,
                alpha: 0.05,
                seed,
            };
            let allowed = entry
                .allowed_delta
                .as_ref()
                .and_then(|delta| match delta {
                    RegistrationValue::Number(number) => Some(*number),
                    _ => None,
                })
                .unwrap_or(2.5);
            // compare_wasserstein 是配对设计（两侧等长、同下标同抽）。
            // 参照侧是分位构造的代表性样本：重复/截断到与模拟侧等长，
            // 经验分布不变，配对前提恢复。
            let mut reference_matched = Vec::with_capacity(simulated_samples.len());
            if !reference_samples.is_empty() {
                for index in 0..simulated_samples.len() {
                    reference_matched.push(reference_samples[index % reference_samples.len()]);
                }
            }
            let (interval, verdict) = crate::stats::compare_wasserstein(
                &simulated_samples,
                &reference_matched,
                allowed,
                &config,
            );
            let verdict_str = match verdict {
                crate::stats::EquivalenceVerdict::Pass => "Pass",
                crate::stats::EquivalenceVerdict::Defect => "Defect",
                crate::stats::EquivalenceVerdict::InsufficientEvidence => "InsufficientEvidence",
            };
            let slot = criterion_verdicts
                .entry("STAT-S01".to_string())
                .or_insert("Pass".to_string());
            match verdict_str {
                "Defect" => *slot = "Defect".to_string(),
                "InsufficientEvidence" if *slot != "Defect" => {
                    *slot = "InsufficientEvidence".to_string();
                }
                _ => {}
            }
            let simulated_mean = simulated_samples.iter().sum::<f64>()
                / simulated_samples.len() as f64;
            let reference_mean =
                reference_samples.iter().sum::<f64>() / reference_samples.len() as f64;
            verdicts.push(BatchMetricVerdict {
                criterion_id: "STAT-S01".to_string(),
                metric: "出手距离分布（W1，桶中心样本）".to_string(),
                simulated_mean,
                reference_value: reference_mean,
                allowed_delta: allowed,
                interval_lower: interval.lower,
                interval_upper: interval.upper,
                verdict: verdict_str.to_string(),
                reason_code: "batch_wasserstein".to_string(),
            });
            coverage.retain(|c| {
                !(c.criterion_id == "STAT-S01" && c.metric == "出手距离分布")
            });
        }
    }

    for group in &group_verdicts {
        let slot = criterion_verdicts
            .entry(group.criterion_id.clone())
            .or_insert("Pass".to_string());
        match group.verdict.as_str() {
            "Defect" => *slot = "Defect".to_string(),
            "InsufficientEvidence" if *slot != "Defect" => {
                *slot = "InsufficientEvidence".to_string();
            }
            _ => {}
        }
    }

    BatchStatReport {
        n_games: metrics.len() as u32,
        scope,
        verdicts,
        group_verdicts,
        criterion_verdicts,
        coverage,
    }
}


/// 单场单指标值（bootstrap 的 cluster 单位是场）。
fn per_game_value(m: &GameMetrics, metric_key: &str) -> f64 {
    match metric_key {
        "fg_pct" => {
            if m.fga_total > 0 {
                m.fgm_total as f64 / m.fga_total as f64
            } else {
                0.0
            }
        }
        "three_pa_share" => {
            if m.fga_total > 0 {
                m.three_pa_total as f64 / m.fga_total as f64
            } else {
                0.0
            }
        }
        "three_pct" => {
            if m.three_pa_total > 0 {
                m.three_pm_total as f64 / m.three_pa_total as f64
            } else {
                0.0
            }
        }
        "shot_points_team_game" => {
            let two_m = m.fgm_total as i64 - m.three_pm_total as i64;
            (two_m * 2 + m.three_pm_total as i64 * 3) as f64
        }
        "fta_fga" => {
            if m.fga_total > 0 {
                m.fta_total as f64 / m.fga_total as f64
            } else {
                0.0
            }
        }
        "ft_pct" => {
            if m.fta_total > 0 {
                m.ftm_total as f64 / m.fta_total as f64
            } else {
                0.0
            }
        }
        "fouls_per100" => {
            if m.possessions > 0 {
                100.0 * m.fouls as f64 / m.possessions as f64
            } else {
                0.0
            }
        }
        "tov_rate" => {
            if m.possessions > 0 {
                m.tov_total as f64 / m.possessions as f64
            } else {
                0.0
            }
        }
        "orb_pct" => {
            let total = m.orb_total + m.drb_total;
            if total > 0 {
                m.orb_total as f64 / total as f64
            } else {
                0.0
            }
        }
        "pace" => {
            let minutes = f64::from(m.periods) * 12.0;
            if minutes > 0.0 {
                m.possessions as f64 * 48.0 / (2.0 * minutes)
            } else {
                0.0
            }
        }
        "oreb_possession_share" => {
            if m.possessions > 0 {
                m.orb_total as f64 / m.possessions as f64
            } else {
                0.0
            }
        }
        key if key.starts_with("share_") => {
            let total = (m.fga_rim + m.fga_near + m.fga_mid + m.fga_three) as f64;
            if total <= 0.0 {
                return 0.0;
            }
            match key {
                "share_rim" => m.fga_rim as f64 / total,
                "share_near" => m.fga_near as f64 / total,
                "share_mid" => m.fga_mid as f64 / total,
                "share_three" => m.fga_three as f64 / total,
                _ => 0.0,
            }
        }
        key if key.starts_with("starts_") => {
            let total = (m.starts_field_goal
                + m.starts_free_throw
                + m.starts_steal
                + m.starts_dead_ball
                + m.starts_timeout
                + m.starts_blocked_shot) as f64;
            if total <= 0.0 {
                return 0.0;
            }
            match key {
                "starts_field_goal_share" => m.starts_field_goal as f64 / total,
                "starts_steal_share" => m.starts_steal as f64 / total,
                "starts_dead_ball_share" => m.starts_dead_ball as f64 / total,
                _ => 0.0,
            }
        }
        _ => 0.0,
    }
}
