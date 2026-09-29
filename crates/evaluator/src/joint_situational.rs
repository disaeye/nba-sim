use crate::fixture::{ReferenceDistributions, ZoneMakeBands};
use crate::report::Judgment;

pub(crate) const SHOT_ZONE_MAKE_JOINT: &str = "SHOT_ZONE_MAKE_JOINT";
pub(crate) const LATE_Q4_SHOT_PROFILE: &str = "LATE_Q4_SHOT_PROFILE";

pub(crate) struct JointSituationalEvidence {
    pub(crate) zone_attempts: [usize; 4],
    pub(crate) zone_makes: [usize; 4],
    pub(crate) late_q4_attempts: usize,
    pub(crate) late_q4_three_attempts: usize,
    pub(crate) early_q4_attempts: usize,
    pub(crate) early_q4_three_attempts: usize,
    pub(crate) shot_facts_complete: bool,
    pub(crate) q4_attempt_facts_complete: bool,
}

pub(crate) fn evaluate(
    out: &mut Vec<Judgment>,
    fixture: &ReferenceDistributions,
    index: u64,
    evidence: JointSituationalEvidence,
) {
    let Some(reference) = fixture.joint_situational_bands.as_ref() else {
        out.push(Judgment::not_applicable(
            SHOT_ZONE_MAKE_JOINT,
            "decision",
            index,
        ));
        out.push(Judgment::not_applicable(
            LATE_Q4_SHOT_PROFILE,
            "decision",
            index,
        ));
        return;
    };

    if !evidence.shot_facts_complete || evidence.zone_attempts.contains(&0) {
        out.push(Judgment::insufficient(
            SHOT_ZONE_MAKE_JOINT,
            "decision",
            index,
        ));
    } else if evidence
        .zone_attempts
        .iter()
        .zip(evidence.zone_makes.iter())
        .any(|(attempts, makes)| makes > attempts)
    {
        out.push(Judgment::defect(
            SHOT_ZONE_MAKE_JOINT,
            "hard",
            "made field goals exceed attempts in a shot zone".to_string(),
            "engine",
            index,
        ));
    } else if evidence
        .zone_attempts
        .iter()
        .all(|attempts| *attempts >= reference.shot_zone_make.minimum_attempts_per_zone)
        && zone_rate_bands_contain(
            &evidence.zone_attempts,
            &evidence.zone_makes,
            &reference.shot_zone_make,
        )
    {
        out.push(Judgment::pass(SHOT_ZONE_MAKE_JOINT, "decision", index));
    } else if evidence
        .zone_attempts
        .iter()
        .any(|attempts| *attempts < reference.shot_zone_make.minimum_attempts_per_zone)
    {
        out.push(Judgment::insufficient(
            SHOT_ZONE_MAKE_JOINT,
            "decision",
            index,
        ));
    } else if zone_rate_bands_contain(
        &evidence.zone_attempts,
        &evidence.zone_makes,
        &reference.shot_zone_make,
    ) {
        out.push(Judgment::pass(SHOT_ZONE_MAKE_JOINT, "decision", index));
    } else {
        out.push(Judgment::defect(
            SHOT_ZONE_MAKE_JOINT,
            "soft",
            "joint shot-zone conversion profile outside the sourced NBA reference bands"
                .to_string(),
            "decision",
            index,
        ));
    }

    let minimum = reference
        .q4_late_three_attempt_share
        .minimum_attempts_per_window;
    if !evidence.q4_attempt_facts_complete
        || evidence.late_q4_attempts < minimum
        || evidence.early_q4_attempts < minimum
    {
        out.push(Judgment::insufficient(
            LATE_Q4_SHOT_PROFILE,
            "decision",
            index,
        ));
        return;
    }
    if evidence.late_q4_three_attempts > evidence.late_q4_attempts
        || evidence.early_q4_three_attempts > evidence.early_q4_attempts
    {
        out.push(Judgment::defect(
            LATE_Q4_SHOT_PROFILE,
            "hard",
            "three-point attempts exceed field-goal attempts in a fourth-quarter window"
                .to_string(),
            "engine",
            index,
        ));
        return;
    }
    let late_rate = evidence.late_q4_three_attempts as f32 / evidence.late_q4_attempts as f32;
    let early_rate = evidence.early_q4_three_attempts as f32 / evidence.early_q4_attempts as f32;
    let delta = late_rate - early_rate;
    if reference.q4_late_three_attempt_share.delta.contains(&delta) {
        out.push(Judgment::pass(LATE_Q4_SHOT_PROFILE, "decision", index));
    } else {
        out.push(Judgment::defect(
            LATE_Q4_SHOT_PROFILE,
            "soft",
            format!("Q4 final-window 3PA/FGA difference {delta:.3} outside sourced band"),
            "decision",
            index,
        ));
    }
}

fn zone_rate_bands_contain(
    attempts: &[usize; 4],
    makes: &[usize; 4],
    bands: &ZoneMakeBands,
) -> bool {
    attempts
        .iter()
        .zip(makes.iter())
        .zip(zone_bands(bands))
        .all(|((attempts, makes), band)| band.contains(&(*makes as f32 / *attempts as f32)))
}

fn zone_bands(bands: &ZoneMakeBands) -> [&crate::fixture::Band; 4] {
    [&bands.rim, &bands.near, &bands.mid, &bands.three]
}
