use std::process::Command;

use nba_domain::league::LeagueId;
use nba_sim_core::params::{workspace_calibration_dir, workspace_root};
use nba_sim_core::RuntimeParams;
use nba_validation::fragment::{capture, FragmentInput, Scenario};

#[test]
fn default_validation_runs_only_registered_fragments() {
    let result = Command::new(env!("CARGO_BIN_EXE_nba-sim"))
        .args(["validate-core", "--league", "fiba", "--json"])
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let reports: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reports.len(), Scenario::ALL.len());
    for report in reports {
        assert!(report["findings"].as_array().unwrap().is_empty());
        assert!(report["advanced_ticks"].as_u64().unwrap() <= u64::from(nba_validation::fragment::DEFAULT_FRAGMENT_TICKS));
    }
}

#[test]
fn replay_loads_the_checkpoint_in_another_process() {
    let params = RuntimeParams::load(&workspace_calibration_dir()).unwrap();
    let input = FragmentInput::builtin(LeagueId::Nba, 1, Scenario::FirstPossession, &params);
    let checkpoint = capture(input.clone(), &params).unwrap();
    let start_tick = checkpoint.tick();
    assert!(start_tick > 0);
    let dir = workspace_root().join(".work/fragment-cli").join(std::process::id().to_string());
    std::fs::create_dir_all(&dir).unwrap();
    let input_path = dir.join("input.json");
    std::fs::write(&input_path, serde_json::to_vec(&input).unwrap()).unwrap();
    let world_path = dir.join("world.cbor");
    checkpoint.save(&world_path).unwrap();
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_nba-sim"))
            .args(["validate-core", "--replay"])
            .arg(&input_path)
            .arg("--out")
            .arg(&dir)
            .arg("--json")
            .output()
            .unwrap()
    };
    let result = run();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
    let reports: Vec<serde_json::Value> = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(reports[0]["start_tick"], start_tick);
    let file = std::fs::OpenOptions::new().append(true).open(&world_path).unwrap();
    std::io::Write::write_all(&mut &file, &[0]).unwrap();
    let invalid = run();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("trailing data"));
    std::fs::remove_dir_all(&dir).unwrap();
}
