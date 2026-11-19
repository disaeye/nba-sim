use nba_domain::league::LeagueId;
use nba_sim_core::RuntimeParams;
use nba_validation::fragment::{capture, FragmentCheckpoint, FragmentInput, Scenario};
use nba_validation::halfcourt::screen_sequence;

#[test]
fn production_screen_has_matching_action_and_a_replayable_checkpoint() {
    let params = RuntimeParams::embedded();
    let mut selected = None;
    for seed in 1..=12 {
        let mut input = FragmentInput::builtin(LeagueId::Nba, seed, Scenario::HalfcourtScreen, &params);
        input.max_ticks = 1_800;
        let checkpoint = match capture(input.clone(), &params) {
            Ok(checkpoint) => checkpoint,
            Err(error) if error.contains("did not reach start") => continue,
            Err(error) => panic!("seed {seed}: {error}"),
        };
        let result = checkpoint.clone().resume().expect("screen fragment reaches its terminal event");
        println!("seed {seed}: {:?}", result.report.screen);
        assert!(result.report.passed(), "seed {seed}: {:?}", result.report.findings);
        if result.report.screen.as_ref().and_then(|screen| screen.used) == Some(true) {
            selected = Some((input, checkpoint, result));
            break;
        }
    }
    let (input, checkpoint, result) = selected.expect("a used screen is required in the bounded samples");
    let sequence = result.report.screen.as_ref().expect("screen evidence");
    assert!(sequence.screen_event.is_some());
    assert!(sequence.defender_id.is_some());
    assert_eq!(sequence.disposition, "ACTION_COMPLETED");
    let decision = &sequence.decision;
    assert!(decision.partner_seen, "the screen initiator must have read its partner");
    assert!(decision.revalidated);
    assert_eq!(decision.revalidation, "accepted");
    assert!(decision.chosen_action.starts_with("Screen"));
    assert!(decision.candidate_count > 1);
    let geometry = &sequence.geometry;
    assert!(geometry.handler_defender_gap_ft > 0.0);
    assert!(geometry.screener_arrival_seconds.is_finite());
    assert!(geometry.defender_arrival_seconds.is_finite());
    assert!(geometry.screen_window_margin_seconds.is_finite());
    assert!(geometry.resolved_screener_spot_gap_ft <= geometry.planned_spot_defender_gap_ft + 4.0);
    let terminal = sequence.terminal_event;
    let without_terminal: Vec<_> = result.report.events.iter().filter(|event| event.event_id != terminal).cloned().collect();
    let finding = screen_sequence(&result.frames, &without_terminal, &result.report.decisions).expect_err("missing terminal must be located");
    assert_eq!(finding.check, "screen_terminal");
    assert!(finding.events.contains(&sequence.start_event));

    let directory = nba_sim_core::params::workspace_root().join(".work/halfcourt-fragment").join(std::process::id().to_string());
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("world.cbor");
    checkpoint.save(&path).expect("save screen checkpoint");
    let replay = FragmentCheckpoint::load(input.clone(), &path, &params)
        .expect("load screen checkpoint").resume().expect("resume screen checkpoint");
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(serde_json::to_vec(&result).unwrap(), serde_json::to_vec(&replay).unwrap());
    println!("seed={} start={} end={} action={}", input.manifest.seed, result.report.start_tick, result.report.end_tick, sequence.action_id);
}

/// 有界片段必须把「看见后选择」「没看见」和「选择后执行失败」分成不同证据。
///
/// 这里不要求同一种子同时出现三种情形：生产路径按几何与随机抽样产生分支。
/// 每个被观察到的分支都必须带着自己的观察摘要、候选、重校验和结算几何，
/// 因此三类情形可以逐项核对，不会被合并成同一个结果。
#[test]
fn fragments_distinguish_perception_choice_and_execution() {
    let params = RuntimeParams::embedded();
    let mut used_after_choice = 0usize;
    let mut failed_after_choice = 0usize;
    let mut checked = 0usize;
    for seed in 1..=12 {
        let mut input = FragmentInput::builtin(LeagueId::Nba, seed, Scenario::HalfcourtScreen, &params);
        input.max_ticks = 1_800;
        let checkpoint = match capture(input, &params) {
            Ok(checkpoint) => checkpoint,
            Err(error) if error.contains("did not reach start") => continue,
            Err(error) => panic!("seed {seed}: {error}"),
        };
        let result = checkpoint.resume().expect("screen fragment reaches its terminal event");
        assert!(result.report.passed(), "seed {seed}: {:?}", result.report.findings);
        let Some(sequence) = result.report.screen.as_ref() else {
            continue;
        };
        checked += 1;
        let decision = &sequence.decision;
        assert!(decision.partner_seen, "发起掩护的人必须读到搭档");
        assert_eq!(decision.chosen_action, "Screen", "片段必须记录被选中的掩护候选");
        assert!(decision.candidate_count > 1, "选择必须来自多个候选");
        assert!(decision.revalidated, "执行前必须有重校验结果");
        assert!(sequence.geometry.screen_window_margin_seconds.is_finite());
        if let Some(seen) = decision.defender_seen {
            assert!(
                seen || decision.unknown_players.iter().any(|id| Some(id.as_str()) == sequence.defender_id.as_deref()),
                "未读到对位防守人时必须在未知名单里出现：{decision:?}"
            );
        }
        match (sequence.used, sequence.disposition.as_str()) {
            (Some(true), "ACTION_COMPLETED") => used_after_choice += 1,
            (Some(false), "ACTION_FAILED") => {
                let geometry = &sequence.geometry;
                assert!(
                    !geometry.contact_expected || !geometry.handler_approached_expected,
                    "执行失败必须给出可定位的几何原因：{geometry:?}"
                );
                failed_after_choice += 1;
            }
            (used, disposition) => panic!("意外的掩护终态 used={used:?} disposition={disposition}"),
        }
    }
    assert!(checked > 0, "有界样本没有产生可核对的掩护片段");
    assert!(used_after_choice > 0, "缺少选择后成功执行的掩护");
    assert!(failed_after_choice > 0, "缺少选择后执行失败的掩护");
}
