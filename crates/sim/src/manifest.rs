//! 回放清单：一次运行的完整输入身份。
//!
//! 清单带着比赛设置（两队名单、首发、规则档案）、种子与三个版本号。
//! 同一份清单必然给出同一场比赛：规则版本说清「按哪套规则判」，校准
//! 版本说清「按哪份校准算」，引擎模型版本说清「按哪套行为模型选」，
//! 三者加上名单与种子就是一次运行的全部输入。
//!
//! 清单可以写到磁盘再读回来：磁盘检查点带着它，恢复时用的就是当初
//! 那一份输入。

use std::path::Path;

use nba_protocol::ENGINE_MODEL_VERSION;
use serde::{Deserialize, Serialize};

use crate::params::RuntimeParams;
use crate::session::{MatchInfo, SessionService};
use crate::world::{MatchSetup, World};

/// 回放清单的格式版本。
///
/// 格式变化时这个值随之变化：旧清单读不进来就地报错。
pub const MANIFEST_VERSION: u32 = 1;

/// 一次运行的完整输入。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchManifest {
    pub version: u32,
    pub seed: u64,
    /// 完整比赛设置：两队名单、首发与规则档案。
    pub setup: MatchSetup,
    /// 规则版本：联赛档案记录的规则集版本。
    pub ruleset_version: String,
    /// 校准版本：参与模拟的配置内容摘要。
    pub calibration_version: String,
    /// 引擎模型版本：行为模型的身份。
    pub engine_model_version: String,
}

impl MatchManifest {
    /// 用比赛设置与种子写一份清单。
    pub fn new(setup: MatchSetup, seed: u64, calibration_version: &str) -> Self {
        Self {
            version: MANIFEST_VERSION,
            seed,
            ruleset_version: setup.rules.league.ruleset_version.clone(),
            setup,
            calibration_version: calibration_version.to_string(),
            engine_model_version: ENGINE_MODEL_VERSION.to_string(),
        }
    }

    /// 该清单使用的联赛档案标识。
    pub fn league(&self) -> nba_domain::league::LeagueId {
        self.setup.rules.league.id
    }

    /// 校验清单与给定的校准、引擎模型是否一致。
    ///
    /// 不一致就地失败：用另一份校准或另一套引擎模型重放出来的不是同一
    /// 场比赛，静默继续只会产出一份看起来正常但已经分叉的比赛。
    pub fn check_against(&self, params: &RuntimeParams) -> Result<(), String> {
        if self.version != MANIFEST_VERSION {
            return Err(format!(
                "unsupported manifest version {}",
                self.version
            ));
        }
        if self.calibration_version != params.calibration_version {
            return Err(format!(
                "manifest calibration version {} does not match the loaded calibration {}",
                self.calibration_version, params.calibration_version
            ));
        }
        if self.engine_model_version != ENGINE_MODEL_VERSION {
            return Err(format!(
                "manifest engine model version {} does not match this engine {}",
                self.engine_model_version, ENGINE_MODEL_VERSION
            ));
        }
        if self.setup.rules.league.ruleset_version != self.ruleset_version {
            return Err(format!(
                "manifest ruleset version {} does not match its own league profile {}",
                self.ruleset_version, self.setup.rules.league.ruleset_version
            ));
        }
        Ok(())
    }

    /// 用这份清单开一个会话。
    pub fn into_session<P: Into<RuntimeParams>>(
        self,
        params: P,
    ) -> Result<SessionService, String> {
        let params: RuntimeParams = params.into();
        self.check_against(&params)?;
        let seed = self.seed;
        let mut service = SessionService::new(seed);
        service.setup_with(self.setup, params, seed)?;
        Ok(service)
    }

    /// 用这份清单跑完一整场。
    pub fn run<P: Into<RuntimeParams>>(self, params: P) -> Result<crate::report::GameReport, String> {
        let params: RuntimeParams = params.into();
        self.check_against(&params)?;
        let seed = self.seed;
        let mut world = World::new(self.setup, params, seed);
        Ok(world.run())
    }

    /// 把清单写到磁盘。
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|error| format!("serialize manifest: {error}"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("create manifest dir {}: {error}", parent.display()))?;
        }
        std::fs::write(path, format!("{text}\n"))
            .map_err(|error| format!("write manifest {}: {error}", path.display()))
    }

    /// 从磁盘读回清单。
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("read manifest {}: {error}", path.display()))?;
        serde_json::from_str(&text)
            .map_err(|error| format!("parse manifest {}: {error}", path.display()))
    }
}

/// 内建名册 + 指定联赛档案的清单。
pub fn builtin_manifest(
    league: nba_domain::league::LeagueId,
    seed: u64,
    calibration_version: &str,
) -> MatchManifest {
    let rules = nba_domain::GameRules::for_league(league);
    MatchManifest::new(MatchSetup::builtin(rules), seed, calibration_version)
}

/// 会话开局后返回的清单：会话实际用的那一份设置。
pub fn manifest_of(info: &MatchInfo, setup: &MatchSetup) -> MatchManifest {
    MatchManifest {
        version: MANIFEST_VERSION,
        seed: info.seed,
        setup: setup.clone(),
        ruleset_version: info.ruleset_version.clone(),
        calibration_version: info.calibration_version.clone(),
        engine_model_version: info.engine_model_version.clone(),
    }
}
