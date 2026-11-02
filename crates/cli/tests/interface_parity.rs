use nba_sim_core::CompletionSummary;
use nba_wasm::WasmMatchService;
use std::process::Command;

fn assert_json_equivalent(left: &serde_json::Value, right: &serde_json::Value, path: &str) {
    match (left, right) {
        (serde_json::Value::Number(left), serde_json::Value::Number(right)) => {
            if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
                assert_eq!(left, right, "{path}");
            } else if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
                assert_eq!(left, right, "{path}");
            } else {
                let left = left.as_f64().expect("left event number");
                let right = right.as_f64().expect("right event number");
                let tolerance = f32::EPSILON as f64 * left.abs().max(right.abs()).max(1.0) * 2.0;
                assert!(
                    (left - right).abs() <= tolerance,
                    "{path}: {left} != {right} (tolerance {tolerance})"
                );
            }
        }
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            assert_eq!(left.len(), right.len(), "{path} length");
            for (index, (left, right)) in left.iter().zip(right).enumerate() {
                assert_json_equivalent(left, right, &format!("{path}[{index}]"));
            }
        }
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            assert_eq!(left.keys().collect::<Vec<_>>(), right.keys().collect::<Vec<_>>(), "{path} keys");
            for (key, left) in left {
                assert_json_equivalent(left, &right[key], &format!("{path}.{key}"));
            }
        }
        _ => assert_eq!(left, right, "{path}"),
    }
}

#[test]
fn full_game_cli_and_wasm_summaries_match() {
    let seed = 211u64;
    let mut wasm = WasmMatchService::with_seed(seed);
    wasm.setup_match_json("").expect("WASM match setup");
    wasm.start().expect("WASM match starts");
    wasm.fast_forward_ms(8_000_000.0)
        .expect("WASM match advances to completion");
    assert_eq!(wasm.session_state(), "Finished");
    let wasm_summary: CompletionSummary = serde_json::from_str(
        &wasm
            .completion_summary_json()
            .expect("WASM completion summary"),
    )
    .expect("WASM summary JSON");
    let wasm_events: serde_json::Value = serde_json::from_str(
        &wasm.full_event_history_json().expect("WASM full event history"),
    )
    .expect("WASM events JSON");

    let output = Command::new(env!("CARGO_BIN_EXE_nba-sim"))
        .args(["sim-core", "--seed", "211", "--json"])
        .output()
        .expect("CLI full-game command starts");
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let cli_output: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("CLI JSON");
    let cli_summaries: Vec<CompletionSummary> =
        serde_json::from_value(cli_output["games"].clone()).expect("CLI game summaries");
    let cli_events = &cli_output["events"][0];
    assert_eq!(cli_summaries.as_slice(), &[wasm_summary]);
    assert_json_equivalent(&wasm_events, cli_events, "event_history");
}
