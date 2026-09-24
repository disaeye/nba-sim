//! 比赛级构成准则簇（dev 方案 §5.1）。
//!
//! 每条准则遵循 D1 证据模型：证据不足 = `InsufficientEvidence`（不得满分），
//! fixture 缺参考带 = `NotApplicable`（v1 fixture 向后兼容路径）。
//! 定位：必要条件的回归网——进带不庆祝，出带必报警。
//!
//! 与 `lib.rs` 的逐回合准则分开：本模块裁决的是**比赛级分布形态**
//! （出手构成、节奏、命中率剖面），输入是一次汇总的证据结构，
//! 而不是逐回合事件窗口。

use crate::fixture::ReferenceDistributions;
use crate::report::Judgment;
use crate::{bands_turnover, REGULATION_SECONDS_48MIN};

/// D2 构成准则的输入证据（dev 方案 §5.1）：从事件流采集的比赛级统计量。
/// 出手按四区（attributes.md §2.3a）：`fga_rim` < 5ft、`fga_near` 5–14ft、
/// `fga_mid` ≥ 14ft 且三分线内、`fga_three` 三分线外。`fga_two = rim+near+mid`。
pub(crate) struct CompositionEvidence {
    pub(crate) possessions: usize,
    pub(crate) turnovers: usize,
    pub(crate) fga_two: usize,
    pub(crate) fga_three: usize,
    pub(crate) fgm_two: usize,
    pub(crate) fgm_three: usize,
    pub(crate) fga_mid: usize,
    pub(crate) fga_near: usize,
    pub(crate) fga_rim: usize,
    pub(crate) fta: usize,
    pub(crate) wall_seconds: f32,
}

pub(crate) fn evaluate_composition_criteria(
    out: &mut Vec<Judgment>,
    fixture: &ReferenceDistributions,
    idx: u64,
    ev: CompositionEvidence,
) {
    // v1 fixture 无构成带：准则不适用，而非默认通过。
    let Some(bands) = &fixture.composition_bands else {
        for criterion in [
            "SHOT_PROFILE_3PA_RATE",
            "SHOT_PROFILE_ZONE_MIX",
            "TEAM_TURNOVER_RATE",
            "PACE_POSSESSIONS",
            "FT_RATE",
            "SHOT_MAKE_PROFILE",
            "ASSIST_PROFILE",
        ] {
            out.push(Judgment::not_applicable(criterion, "decision", idx));
        }
        return;
    };

    let fga = ev.fga_two + ev.fga_three;
    // 证据门槛：出手/回合样本过少时判 InsufficientEvidence，不给假结论。
    const MIN_SHOTS_FOR_PROFILE: usize = 10;
    const MIN_POSSESSIONS_FOR_PACE: usize = 10;

    // SHOT_PROFILE_3PA_RATE：三分出手占比。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient(
            "SHOT_PROFILE_3PA_RATE",
            "decision",
            idx,
        ));
    } else {
        let rate = ev.fga_three as f32 / fga as f32;
        if bands.three_attempt_rate.contains(&rate) {
            out.push(Judgment::pass("SHOT_PROFILE_3PA_RATE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_PROFILE_3PA_RATE",
                "soft",
                format!(
                    "3PA rate {:.3} outside {:?} ({} 3PA / {} FGA)",
                    rate, bands.three_attempt_rate, ev.fga_three, fga
                ),
                "decision",
                idx,
            ));
        }
    }

    // SHOT_PROFILE_ZONE_MIX：篮下/近筐/中投构成（attributes.md §2.3a 四区，
    // 中距离回归的直接证据）。三带全入带才通过；fixture 缺带 = NotApplicable。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient(
            "SHOT_PROFILE_ZONE_MIX",
            "decision",
            idx,
        ));
    } else {
        let mid_share = ev.fga_mid as f32 / fga as f32;
        let near_share = ev.fga_near as f32 / fga as f32;
        let rim_share = ev.fga_rim as f32 / fga as f32;
        let mid_ok = bands
            .mid_range_share_of_fga
            .as_ref()
            .map(|band| band.contains(&mid_share))
            .unwrap_or(true);
        let near_ok = bands
            .near_range_share_of_fga
            .as_ref()
            .map(|band| band.contains(&near_share))
            .unwrap_or(true);
        let rim_ok = bands
            .rim_share_of_fga
            .as_ref()
            .map(|band| band.contains(&rim_share))
            .unwrap_or(true);
        if mid_ok && near_ok && rim_ok {
            out.push(Judgment::pass("SHOT_PROFILE_ZONE_MIX", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_PROFILE_ZONE_MIX",
                "soft",
                format!(
                    "zone mix off: rim {:.3} (band {:?}), near {:.3} (band {:?}), mid {:.3} (band {:?}) of {} FGA",
                    rim_share,
                    bands.rim_share_of_fga,
                    near_share,
                    bands.near_range_share_of_fga,
                    mid_share,
                    bands.mid_range_share_of_fga,
                    fga
                ),
                "decision",
                idx,
            ));
        }
    }

    // TEAM_TURNOVER_RATE：比赛级固定分母失误率（现有 TURNOVER_RATE 的升格版）。
    if ev.possessions < MIN_POSSESSIONS_FOR_PACE {
        out.push(Judgment::insufficient(
            "TEAM_TURNOVER_RATE",
            "decision",
            idx,
        ));
    } else {
        let rate = ev.turnovers as f32 / ev.possessions as f32;
        if bands_turnover(fixture).contains(&rate) {
            out.push(Judgment::pass("TEAM_TURNOVER_RATE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "TEAM_TURNOVER_RATE",
                "soft",
                format!(
                    "turnover rate {:.3} outside {:?} ({}/{})",
                    rate,
                    bands_turnover(fixture),
                    ev.turnovers,
                    ev.possessions
                ),
                "decision",
                idx,
            ));
        }
    }

    // PACE_POSSESSIONS：48 分钟等效回合数。
    if ev.possessions < MIN_POSSESSIONS_FOR_PACE || ev.wall_seconds <= 0.0 {
        out.push(Judgment::insufficient("PACE_POSSESSIONS", "decision", idx));
    } else {
        let pace = ev.possessions as f32 * REGULATION_SECONDS_48MIN / ev.wall_seconds;
        if bands.pace_possessions_per_48min.contains(&pace) {
            out.push(Judgment::pass("PACE_POSSESSIONS", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "PACE_POSSESSIONS",
                "soft",
                format!(
                    "pace {:.1} poss/48min outside {:?} ({} poss in {:.0}s)",
                    pace, bands.pace_possessions_per_48min, ev.possessions, ev.wall_seconds
                ),
                "decision",
                idx,
            ));
        }
    }

    // FT_RATE：罚球率 FTA/FGA。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient("FT_RATE", "officiating", idx));
    } else {
        let rate = ev.fta as f32 / fga as f32;
        if bands.free_throw_rate.contains(&rate) {
            out.push(Judgment::pass("FT_RATE", "officiating", idx));
        } else {
            out.push(Judgment::defect(
                "FT_RATE",
                "soft",
                format!(
                    "FT rate {:.3} outside {:?} ({} FTA / {} FGA)",
                    rate, bands.free_throw_rate, ev.fta, fga
                ),
                "officiating",
                idx,
            ));
        }
    }

    // SHOT_MAKE_PROFILE：两/三分命中率（校准 D3.1 的直接门）。
    if fga < MIN_SHOTS_FOR_PROFILE {
        out.push(Judgment::insufficient("SHOT_MAKE_PROFILE", "decision", idx));
    } else {
        let three_pct = if ev.fga_three > 0 {
            ev.fgm_three as f32 / ev.fga_three as f32
        } else {
            0.0
        };
        let two_pct = if ev.fga_two > 0 {
            ev.fgm_two as f32 / ev.fga_two as f32
        } else {
            0.0
        };
        let three_ok = ev.fga_three == 0 || bands.three_make_pct.contains(&three_pct);
        let two_ok = ev.fga_two == 0 || bands.two_make_pct.contains(&two_pct);
        if three_ok && two_ok {
            out.push(Judgment::pass("SHOT_MAKE_PROFILE", "decision", idx));
        } else {
            out.push(Judgment::defect(
                "SHOT_MAKE_PROFILE",
                "soft",
                format!(
                    "make pct off: 3P {:.3} (band {:?}, {}/{}), 2P {:.3} (band {:?}, {}/{})",
                    three_pct,
                    bands.three_make_pct,
                    ev.fgm_three,
                    ev.fga_three,
                    two_pct,
                    bands.two_make_pct,
                    ev.fgm_two,
                    ev.fga_two
                ),
                "decision",
                idx,
            ));
        }
    }

    // ASSIST_PROFILE：事件流当前没有 AST 载荷——按 dev 方案 §5.1 登记为
    // InsufficientEvidence 缺口，禁止伪造助攻（盲区清单条目 #4 的对应准则）。
    out.push(Judgment::insufficient("ASSIST_PROFILE", "decision", idx));
}
