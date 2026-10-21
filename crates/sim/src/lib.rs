//! 动作链篮球模拟内核。
//!
//! 结构：世界状态 → 策略选动作 → 结算产出状态变化 → 下一次决策读新状态。
//! 出手位置与命中率都是动作链的后果，参数表来自真实赛季数据。

pub mod action;
pub mod boxscore;
pub mod defense;
pub mod endgame;
pub mod gate;
pub mod params;
pub mod policy;
pub mod replay;
pub mod resolve;
pub mod rotation;
pub mod session;
pub mod state;
pub mod stream;
pub mod tactics;
pub mod world;

pub use action::{ActionKind, Fact, ShotKind, TeamSide};
pub use boxscore::{BoxScore, PlayerLine, TeamLine};
pub use params::{Params, CALIBRATION_DIR};
pub use replay::{replay_possession, PossessionFrames};
pub use session::{MatchInfo, SessionService, SessionState};
pub use world::{GameReport, MatchSetup, PossessionRecord, TimedFact, World};

use std::path::Path;

pub fn run_game(seed: u64, calibration_dir: &Path) -> GameReport {
    let params = Params::load(calibration_dir).expect("calibration tables must load");
    let setup = MatchSetup::builtin(nba_domain::GameRules::default());
    run_game_with_setup(seed, params, setup)
}

/// 用指定阵容与参数跑一场。机制实验通过它注入属性扰动。
pub fn run_game_with_setup(seed: u64, params: Params, setup: MatchSetup) -> GameReport {
    let mut world = World::new(setup, params, seed);
    world.run()
}

pub fn run_game_default(seed: u64) -> GameReport {
    run_game(seed, &params::workspace_calibration_dir())
}

#[derive(Debug, Clone)]
pub struct Summary {
    pub seed: u64,
    pub home_score: u32,
    pub away_score: u32,
    pub possessions: u32,
    pub fga: u32,
    pub fg3a: u32,
    pub fta: u32,
    pub turnovers: u32,
    pub oreb: u32,
    pub fg2_pct: f32,
    pub fg3_pct: f32,
    pub ft_pct: f32,
}

pub fn summarize(report: &GameReport) -> Summary {
    let home = report.box_score.team_totals(TeamSide::Home);
    let away = report.box_score.team_totals(TeamSide::Away);
    let fgm = home.fgm + away.fgm;
    let fga = home.fga + away.fga;
    let tpm = home.tpm + away.tpm;
    let tpa = home.tpa + away.tpa;
    let ftm = home.ftm + away.ftm;
    let fta = home.fta + away.fta;
    let fg2a = fga - tpa;
    let fg2m = fgm - tpm;
    Summary {
        seed: report.seed,
        home_score: report.home_score,
        away_score: report.away_score,
        possessions: report.possessions.len() as u32,
        fga,
        fg3a: tpa,
        fta,
        turnovers: home.turnovers + away.turnovers,
        oreb: home.oreb + away.oreb,
        fg2_pct: if fg2a == 0 {
            0.0
        } else {
            fg2m as f32 / fg2a as f32
        },
        fg3_pct: if tpa == 0 {
            0.0
        } else {
            tpm as f32 / tpa as f32
        },
        ft_pct: if fta == 0 {
            0.0
        } else {
            ftm as f32 / fta as f32
        },
    }
}
