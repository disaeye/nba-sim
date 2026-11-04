use std::collections::HashSet;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Catalog {
    catalog_version: u32,
    scope: Vec<String>,
    profiles: std::collections::BTreeMap<String, Profile>,
    procedures: Vec<Procedure>,
    violations: Vec<Violation>,
    known_current_gaps: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Profile {
    ruleset_version: String,
    source: String,
    source_status: String,
    team_foul_policy: FoulPolicy,
    related_official_pages: Option<Vec<String>>,
    effective: Option<String>,
}

#[derive(Debug, Deserialize)]
struct FoulPolicy {
    regulation_fouls_to_penalty: u32,
    overtime_fouls_to_penalty: Option<u32>,
    overtime_foul_allowance: Option<u32>,
    overtime_first_quota_duration_seconds: Option<u32>,
    overtime_final_window_seconds: Option<u32>,
    overtime_final_fouls_to_penalty: Option<u32>,
    overtime_team_fouls_carry_from_fourth: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct Procedure {
    id: String,
    name: String,
    nba: Vec<String>,
    fiba: Vec<String>,
    scenarios: Vec<String>,
    implementation: String,
    existing_test: Option<String>,
    passing_scenarios: Option<Vec<String>>,
    missing_scenarios: Option<Vec<String>>,
    scenario_tests: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct Violation {
    id: String,
    nba: Vec<String>,
    fiba: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ActionCatalog {
    catalog_version: u32,
    scope: Vec<String>,
    families: Vec<ActionFamily>,
}

#[derive(Debug, Deserialize)]
struct ActionFamily {
    id: String,
    name: String,
    actions: Vec<String>,
    current_status: String,
    existing_types: Vec<String>,
    production_paths: Vec<String>,
    consumer_status: String,
    evidence: Vec<String>,
    rule_refs: Vec<String>,
    scenarios: Vec<String>,
    scenario_tests: Option<std::collections::BTreeMap<String, String>>,
    #[serde(default)]
    missing_evidence: Vec<String>,
}

fn rule_catalog() -> Catalog {
    serde_json::from_str(include_str!("../../../data/rules/rule_catalog.json"))
        .expect("rule catalog parses")
}

fn action_catalog() -> ActionCatalog {
    serde_json::from_str(include_str!("../../../data/rules/action_catalog.json"))
        .expect("action catalog parses")
}

#[test]
fn rule_catalog_has_versioned_sources_and_unique_scenario_ids() {
    let catalog = rule_catalog();
    assert_eq!(catalog.catalog_version, 1);
    assert_eq!(catalog.scope, ["nba", "fiba"]);
    assert_eq!(catalog.profiles.len(), 2);

    for league in ["nba", "fiba"] {
        let profile = catalog.profiles.get(league).expect("profile exists");
        assert!(!profile.ruleset_version.is_empty());
        assert!(profile.source.starts_with("https://"));
        assert!(!profile.source_status.is_empty());
        assert_eq!(profile.team_foul_policy.regulation_fouls_to_penalty, 5);
        if league == "fiba" {
            assert_eq!(profile.effective.as_deref(), Some("2026-10-01"));
        }
    }

    let nba = catalog.profiles.get("nba").expect("NBA profile exists");
    assert_eq!(nba.ruleset_version, "2025-26");
    let nba_pages = nba
        .related_official_pages
        .as_ref()
        .expect("NBA official rule pages recorded");
    assert!(nba_pages
        .iter()
        .all(|page| page.starts_with("https://official.nba.com/")));
    let nba_fouls = &nba.team_foul_policy;
    assert_eq!(nba_fouls.overtime_fouls_to_penalty, Some(4));
    assert_eq!(nba_fouls.overtime_foul_allowance, Some(3));
    assert_eq!(nba_fouls.overtime_first_quota_duration_seconds, Some(180));
    assert_eq!(nba_fouls.overtime_final_window_seconds, Some(120));
    assert_eq!(nba_fouls.overtime_final_fouls_to_penalty, Some(2));

    let opening_jump = catalog
        .procedures
        .iter()
        .find(|procedure| procedure.id == "PROC-OPENING-JUMP")
        .expect("opening jump-ball procedure");
    assert_eq!(opening_jump.implementation, "implemented");
    let period_start = catalog
        .procedures
        .iter()
        .find(|procedure| procedure.id == "PROC-PERIOD-START")
        .expect("period-start procedure");
    assert_eq!(period_start.implementation, "implemented");
    let overtime = catalog
        .procedures
        .iter()
        .find(|procedure| procedure.id == "PROC-OVERTIME")
        .expect("overtime procedure");
    // 加时的 7 个场景全部有直接回归：这个程序不再有缺失场景。
    assert_eq!(overtime.implementation, "implemented");

    let fiba = catalog.profiles.get("fiba").expect("FIBA profile exists");
    assert_eq!(fiba.ruleset_version, "2026-v1.1");
    assert_eq!(fiba.effective.as_deref(), Some("2026-10-01"));
    assert_eq!(
        fiba.team_foul_policy.overtime_team_fouls_carry_from_fourth,
        Some(true)
    );

    let nba_profile = nba_domain::league::LeagueProfile::nba();
    let fiba_profile = nba_domain::league::LeagueProfile::fiba();
    assert!(nba_profile.period_starts_with_jump_ball(1));
    assert!(!nba_profile.period_starts_with_jump_ball(2));
    assert!(nba_profile.period_starts_with_jump_ball(5));
    assert!(fiba_profile.period_starts_with_jump_ball(1));
    assert!(!fiba_profile.period_starts_with_jump_ball(2));
    assert!(!fiba_profile.period_starts_with_jump_ball(3));
    assert!(!fiba_profile.period_starts_with_jump_ball(4));
    assert!(!fiba_profile.period_starts_with_jump_ball(5));
    assert!(!fiba_profile.period_starts_with_jump_ball(6));

    let mut procedure_ids = HashSet::new();
    let mut scenario_ids = HashSet::new();
    for procedure in &catalog.procedures {
        assert!(!procedure.id.is_empty());
        assert!(!procedure.name.is_empty());
        assert!(
            procedure_ids.insert(&procedure.id),
            "duplicate procedure {}",
            procedure.id
        );
        assert!(matches!(
            procedure.implementation.as_str(),
            "planned" | "partial" | "implemented" | "verified"
        ));
        assert!(
            !procedure.nba.is_empty() || !procedure.fiba.is_empty(),
            "{} has no rule references",
            procedure.id
        );
        assert!(
            !procedure.scenarios.is_empty(),
            "{} has no scenarios",
            procedure.id
        );
        for scenario in &procedure.scenarios {
            assert!(
                scenario_ids.insert(scenario),
                "duplicate scenario {scenario}"
            );
        }
        if let Some(test) = &procedure.existing_test {
            assert!(
                test.starts_with("nba_sim_core::"),
                "unexpected test identity {test}"
            );
        }
        if let Some(passing) = &procedure.passing_scenarios {
            for scenario in passing {
                assert!(
                    procedure.scenarios.contains(scenario),
                    "unknown passing scenario {scenario} in {}",
                    procedure.id
                );
            }
        }
        if let Some(missing) = &procedure.missing_scenarios {
            for scenario in missing {
                assert!(
                    procedure.scenarios.contains(scenario),
                    "unknown missing scenario {scenario} in {}",
                    procedure.id
                );
            }
        }
        if let Some(tests) = &procedure.scenario_tests {
            for (scenario, test) in tests {
                assert!(
                    procedure.scenarios.contains(scenario),
                    "unknown tested scenario {scenario} in {}",
                    procedure.id
                );
                assert!(
                    test.starts_with("nba_sim_core::"),
                    "unexpected scenario test identity {test}"
                );
            }
            if let Some(passing) = &procedure.passing_scenarios {
                assert_eq!(
                    tests.keys().cloned().collect::<HashSet<_>>(),
                    passing.iter().cloned().collect::<HashSet<_>>(),
                    "passing scenario tests are incomplete for {}",
                    procedure.id
                );
            }
        }
    }

    let mut violation_ids = HashSet::new();
    for violation in &catalog.violations {
        assert!(!violation.id.is_empty());
        assert!(
            violation_ids.insert(&violation.id),
            "duplicate violation {}",
            violation.id
        );
        assert!(!violation.nba.is_empty() || !violation.fiba.is_empty());
    }
    assert!(!catalog.known_current_gaps.is_empty());
}

#[test]
fn action_catalog_records_production_evidence_and_unique_scenario_ids() {
    let catalog = action_catalog();
    assert_eq!(catalog.catalog_version, 1);
    assert_eq!(
        catalog.scope,
        [
            "ball_handler",
            "off_ball_offense",
            "defense",
            "rebound",
            "transition"
        ]
    );

    let mut family_ids = HashSet::new();
    let mut scenario_ids = HashSet::new();
    for family in &catalog.families {
        assert!(!family.id.is_empty());
        assert!(!family.name.is_empty());
        assert!(
            family_ids.insert(&family.id),
            "duplicate action family {}",
            family.id
        );
        assert!(!family.actions.is_empty(), "{} has no actions", family.id);
        assert!(matches!(
            family.current_status.as_str(),
            "planned" | "partial" | "implemented" | "verified"
        ));
        assert!(
            !family.rule_refs.is_empty(),
            "{} has no rule references",
            family.id
        );
        assert!(
            !family.scenarios.is_empty(),
            "{} has no scenarios",
            family.id
        );
        assert!(!family.consumer_status.is_empty());
        if !family.missing_evidence.is_empty() {
            assert_ne!(
                family.current_status, "verified",
                "{} has unverified required scenarios",
                family.id
            );
        }
        if family.current_status == "planned" {
            assert!(family.existing_types.is_empty());
            assert!(family.production_paths.is_empty());
            assert!(family.evidence.is_empty());
        }
        for evidence in &family.evidence {
            assert!(
                evidence.starts_with("crates/sim/tests/"),
                "unexpected test identity {evidence}"
            );
        }
        for scenario in &family.scenarios {
            assert!(
                scenario_ids.insert(scenario),
                "duplicate action scenario {scenario}"
            );
        }
        let tests = family.scenario_tests.as_ref().cloned().unwrap_or_default();
        let passing: HashSet<_> = tests.keys().cloned().collect();
        let missing: HashSet<_> = family.missing_evidence.iter().cloned().collect();
        assert!(
            family
                .missing_evidence
                .iter()
                .all(|scenario| family.scenarios.contains(scenario)),
            "unknown missing action scenario in {}",
            family.id
        );
        let scenarios: HashSet<_> = family.scenarios.iter().cloned().collect();
        assert!(
            missing.is_disjoint(&passing),
            "scenario is both passing and missing in {}",
            family.id
        );
        if family.current_status == "planned" {
            assert_eq!(
                passing.len(),
                0,
                "planned family {} has a passing scenario",
                family.id
            );
        }
        assert_eq!(
            missing.union(&passing).cloned().collect::<HashSet<_>>(),
            scenarios,
            "scenario evidence is incomplete in {}",
            family.id
        );
        for (scenario, test) in &tests {
            assert!(
                family.scenarios.contains(scenario),
                "unknown tested action scenario {scenario} in {}",
                family.id
            );
            assert!(
                test.starts_with("crates/sim/tests/"),
                "unexpected action test identity {test}"
            );
        }
        let mapped_tests: HashSet<_> = tests.values().collect();
        let cited_tests: HashSet<_> = family.evidence.iter().collect();
        assert_eq!(
            mapped_tests, cited_tests,
            "test evidence must map to scenario identities in {}",
            family.id
        );
        if family.current_status == "planned" {
            assert!(
                tests.is_empty(),
                "planned family {} unexpectedly has passing test evidence",
                family.id
            );
        }
    }

    for required in [
        "ACTION-DRIBBLE",
        "ACTION-PASS",
        "ACTION-SHOT",
        "ACTION-SCREEN",
        "ACTION-CUT",
        "ACTION-DEFENSE-ON-BALL",
        "ACTION-REBOUND",
    ] {
        assert!(
            family_ids
                .iter()
                .any(|family_id| family_id.as_str() == required),
            "missing required family {required}"
        );
    }
}
