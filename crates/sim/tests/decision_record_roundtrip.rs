use glam::Vec2;
use nba_sim_core::action::{ActionKind, ShotKind};
use nba_sim_core::state::{CandidateTrace, DecisionRecord, Revalidation};
use nba_sim_core::utility::{EpvTerms, Utility};

#[test]
fn decision_record_roundtrips_with_candidates_and_revalidation() {
    let candidate = CandidateTrace {
        kind: ActionKind::Shoot(ShotKind::CatchShoot),
        target: Some(3),
        to_ft: Vec2::new(12.0, 4.0),
        utility: Utility {
            epv: EpvTerms {
                ability: 0.5,
                contest: -0.2,
                lane: 0.0,
                help: -0.1,
                tendency: 0.05,
                context: 0.2,
                turnover: -0.03,
            },
            variance: 0.1,
            clock_cost: 0.2,
            foul_cost: 0.0,
            tactical_bonus: 0.3,
        },
        total: 0.72,
        probability: 1.0,
    };
    let record = DecisionRecord {
        decision_id: 7,
        tick: 42,
        actor: 3,
        observation_digest: 0xdead_beef,
        candidates: vec![candidate],
        chosen: 0,
        revalidation: Revalidation::Cancelled("recheck".to_string()),
    };
    let text = serde_json::to_string(&record).expect("decision record encodes");
    let back: DecisionRecord = serde_json::from_str(&text).expect("decision record roundtrips");
    assert_eq!(back, record);
}
