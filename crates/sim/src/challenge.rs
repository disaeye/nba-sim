//! 录像复核与教练挑战：停表期间的裁决更正程序。
//!
//! 引擎的裁决来自几何与事件记录本身，复核不会「找到新证据」：它只确认
//! 原判。程序因此规定挑战失败要付出暂停，而原判一律保留。

use crate::action::{Fact, TeamSide};
use crate::state::Phase;
use crate::world::World;

/// 一次可复核的裁决：谁得到球、这条裁决叫什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ruling {
    pub possession: TeamSide,
    pub call: &'static str,
}

/// 挑战结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChallengeOutcome {
    /// 挑战成立，原判被推翻。
    Overturned,
    /// 挑战失败，原判维持，挑战方损失一次暂停。
    Upheld,
}

impl World {
    /// 登记一条可复核的裁决。发球类裁决在写阶段时调用。
    pub fn record_ruling(&mut self, possession: TeamSide, call: &'static str) {
        self.last_ruling = Some(Ruling { possession, call });
    }

    /// 主教练申请复核。
    ///
    /// 只有停表期间、并且在联赛规定的复核窗口内、且本方还有挑战额度时才
    /// 成立；复核结论一律维持原判，挑战方按档案付出一次暂停。
    pub fn request_challenge(&mut self, team: TeamSide) -> Result<ChallengeOutcome, &'static str> {
        if !matches!(self.phase(), Phase::DeadBall { .. } | Phase::Inbound { .. }) {
            return Err("challenge requires a stoppage");
        }
        let Some(ruling) = self.last_ruling else {
            return Err("no reviewable ruling on record");
        };
        if ruling.possession == team {
            return Err("only the team that lost the ruling may challenge");
        }
        let index = World::team_index(team);
        if self.challenges_left[index] == 0 {
            return Err("no challenge remaining");
        }
        let window = self.rules.league.challenge_final_seconds;
        let final_period = self.clock.period() >= self.rules.league.regulation_periods;
        // 复核窗口在末节最后两分钟：节次与剩余时间两个条件同时成立。
        let late = final_period && self.clock.game_clock() <= f64::from(window);
        if !late {
            return Err("challenge window closed");
        }
        self.challenges_left[index] -= 1;
        // 复核用的是事件记录本身；记录与裁决一致，所以结论是维持原判。
        let cost_timeout = self.timeouts[index].remaining > 0;
        if cost_timeout {
            self.timeouts[index].remaining -= 1;
        }
        self.emit(Fact::Review {
            team,
            call: ruling.call,
            overturned: false,
            cost_timeout,
            time: self.now(),
        });
        Ok(ChallengeOutcome::Upheld)
    }
}
