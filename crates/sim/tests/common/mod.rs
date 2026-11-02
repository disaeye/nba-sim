// 整场行为测试的公共构造：缩短节时长的规则档案。
// 与完整 48 分钟走同一条生产路径，tick 数按比例下降，单个测试保持在秒级。
use nba_domain::rules::GameRules;
use nba_sim_core::params::workspace_calibration_dir;
use nba_sim_core::{MatchSetup, Params, World};

/// NBA 短场：每节 120 秒、加时 60 秒。
pub fn nba_short() -> GameRules {
    GameRules::default().shortened(120.0, 60.0)
}

/// FIBA 短场：每节 100 秒、加时 50 秒。
pub fn fiba_short() -> GameRules {
    GameRules::with_league(nba_domain::LeagueProfile::fiba()).shortened(100.0, 50.0)
}

pub fn short_setup() -> MatchSetup {
    MatchSetup::builtin(nba_short())
}

pub fn short_world(seed: u64) -> World {
    let params = Params::load(&workspace_calibration_dir()).expect("calibration tables load");
    World::new(short_setup(), params, seed)
}
