//! 联赛入口：同一内核跑 NBA 与 FIBA，帧上带着版本身份。
//!
//! CLI、WASM 与 debug-server 都走这条会话接口，因此这里验证的入口就是
//! 三个前端共用的那一道门。

use nba_domain::league::LeagueId;
use nba_sim_core::params::{
    embedded_simulation_config_version, simulation_config_version, workspace_calibration_dir,
    RuntimeParams,
};
use nba_sim_core::{MatchManifest, MatchSetup, SessionService, SessionState};
use nba_protocol::ENGINE_MODEL_VERSION;

fn params() -> RuntimeParams {
    RuntimeParams::load(&workspace_calibration_dir()).expect("runtime tables load")
}

/// 两个联赛各跑完一整场，比赛走完并产生事件。
#[test]
fn both_leagues_run_a_full_match_through_the_shared_session_entry() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        let mut service = SessionService::new(7);
        let info = service
            .setup_for_league(league, params(), 7)
            .expect("match setup");
        assert_eq!(info.league, league.as_str());
        assert!(
            !info.ruleset_version.is_empty(),
            "{}: 会话信息必须带规则版本",
            league.as_str()
        );
        service.start().expect("match starts");
        while service.state() != SessionState::Finished {
            service.fast_forward(60.0).expect("session advances");
        }
        let report = service.report().expect("completed report");
        assert!(
            report.possessions.len() > 50,
            "{}: 一场比赛至少要有五十个回合，实际 {}",
            league.as_str(),
            report.possessions.len()
        );
        assert!(
            report.home_score + report.away_score > 0,
            "{}: 一场比赛必须得分",
            league.as_str()
        );
    }
}

/// 帧规则带着规则版本、校准版本与引擎模型版本。
#[test]
fn stream_frames_carry_the_three_versions() {
    for league in [LeagueId::Nba, LeagueId::Fiba] {
        let mut service = SessionService::new(9);
        service
            .setup_for_league(league, params(), 9)
            .expect("match setup");
        service.start().expect("match starts");
        service.fast_forward(1.0).expect("session advances");
        let tick = service.snapshot().expect("snapshot after advancing");
        assert_eq!(
            tick.frame.rules.ruleset_version,
            match league {
                LeagueId::Nba => "2025-26",
                LeagueId::Fiba => "2026-v1.1",
            },
            "{}: 帧上的规则版本必须与档案一致",
            league.as_str()
        );
        assert_eq!(
            tick.frame.rules.calibration_version,
            params().calibration_version,
            "{}: 帧上的校准版本必须与加载的校准一致",
            league.as_str()
        );
        assert_eq!(
            tick.frame.rules.engine_model_version, ENGINE_MODEL_VERSION,
            "{}: 帧上的引擎模型版本必须与引擎一致",
            league.as_str()
        );
    }
}

/// 帧上的计时器运行标志：比赛钟与进攻钟在活球时走，死球时停。
#[test]
fn stream_frames_carry_the_timer_running_flags() {
    let mut service = SessionService::new(13);
    service
        .setup_for_league(LeagueId::Nba, params(), 13)
        .expect("match setup");
    service.start().expect("match starts");
    service.fast_forward(2.0).expect("session advances");
    let tick = service.snapshot().expect("snapshot after advancing");
    let flags = &tick.frame.timers;
    let game_clock = flags
        .timers
        .iter()
        .find(|flag| flag.timer == nba_protocol::TimerKind::GameClock)
        .expect("帧必须带比赛钟的运行标志");
    assert!(
        game_clock.running,
        "活球时比赛钟必须在走：{:?}",
        flags.timers
    );
    assert_eq!(
        flags.player_timers.len(),
        16,
        "名单上每名球员各有计时器标志"
    );
    assert!(
        flags
            .player_timers
            .iter()
            .all(|flag| flag.running),
        "活球时球员计时器都在走"
    );
}

/// 校准版本覆盖参与模拟的配置文件：磁盘版与内嵌版读到同一份仓库内容时
/// 给出同一个版本号。
#[test]
fn the_calibration_version_covers_the_simulation_configuration() {
    let from_disk = simulation_config_version(&workspace_calibration_dir())
        .expect("disk configuration is readable");
    let from_embedded = embedded_simulation_config_version();
    assert_eq!(
        from_disk, from_embedded,
        "同一份仓库内容在两种加载方式下必须给出同一个校准版本"
    );
    assert_eq!(
        from_disk,
        params().calibration_version,
        "加载后的参数必须带着同一个校准版本"
    );
    assert_eq!(from_disk.len(), 16, "校准版本是十六位十六进制摘要");
}

/// 回放清单带着完整的比赛设置：自定义名单与规则档案原样恢复。
#[test]
fn a_manifest_restores_the_custom_roster_and_league_profile() {
    let mut setup = MatchSetup::builtin(nba_domain::GameRules::for_league(LeagueId::Fiba));
    setup.home.name = "Harbor Kings".to_string();
    setup.away.name = "Ridge Riders".to_string();
    setup.home.players[0].name = "Custom Starter".to_string();
    let manifest = MatchManifest::new(setup, 31, &params().calibration_version);

    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join(".work")
        .join("checkpoints")
        .join("manifest.json");
    manifest.save(&path).expect("manifest writes");

    let loaded = MatchManifest::load(&path).expect("manifest loads");
    assert_eq!(loaded.league(), LeagueId::Fiba);
    assert_eq!(loaded.setup.home.name, "Harbor Kings");
    assert_eq!(loaded.setup.away.name, "Ridge Riders");
    assert_eq!(loaded.setup.home.players[0].name, "Custom Starter");

    let service = loaded
        .into_session(params())
        .expect("manifest drives a fresh session");
    let info = service.snapshot().expect("snapshot after setup");
    assert_eq!(info.frame.home_team.name, "Harbor Kings");
    assert_eq!(info.frame.away_team.name, "Ridge Riders");
    assert_eq!(info.frame.rules.ruleset_version, "2026-v1.1");
}

/// 校准版本不匹配的清单读不进来：那是另一场比赛。
#[test]
fn a_manifest_with_another_calibration_is_rejected() {
    let manifest = MatchManifest::new(
        MatchSetup::builtin(nba_domain::GameRules::default()),
        41,
        "0000000000000000",
    );
    let error = manifest
        .check_against(&params())
        .expect_err("校准版本不一致的清单必须被拒绝");
    assert!(
        error.contains("calibration version"),
        "错误必须指出校准版本不一致：{error}"
    );
}
