//! 录像复核与教练挑战：停表期间的裁决更正程序。
//!
//! 引擎的裁决来自几何与事件记录本身，复核不会「找到新证据」：它只确认
//! 原判，或按已登记的事实证据改判。程序规定挑战失败要付出暂停。
//!
//! 哪些裁决可以复核由联赛档案的 `officials.review_scope` 给定：挑战入口
//! 与裁判自动发起的即时回放都读同一张表。

use crate::action::{Fact, TeamSide};
use crate::state::Phase;
use crate::world::World;

/// 裁决名称到档案里的复核范围键。
///
/// 未知名称返回空串，`LeagueProfile::reviews` 对空串为假，因此没登记的
/// 裁决一律不可复核。这张表让裁决名称与复核范围解耦：规则档案只写复核
/// 的问题，事件流只写裁判当时吹了什么。
pub fn review_scope(call: &str) -> &'static str {
    match call {
        "out_of_bounds" => "out_of_bounds_assignment",
        "goaltending" => "goaltending",
        "basket_interference" => "basket_interference",
        "last_touch" => "last_touch",
        "flagrant_upgrade" => "flagrant_upgrade",
        "unsportsmanlike_upgrade" => "unsportsmanlike_upgrade",
        "disqualifying_upgrade" => "disqualifying_upgrade",
        "two_versus_three_points" => "two_versus_three_points",
        "game_clock_at_period_end" => "game_clock_at_period_end",
        "shot_clock_at_period_end" => "shot_clock_at_period_end",
        "clock_malfunction" => "clock_malfunction",
        _ => "",
    }
}

/// 该裁决是否在档案的即时回放范围内，且裁判有权发起复核。
pub fn is_reviewable(world: &World, call: &str) -> bool {
    world.rules.league.authorizes("replay_review")
        && world.rules.league.reviews(review_scope(call))
}

/// 一次可复核的裁决：谁得到球、这条裁决叫什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ruling {
    pub possession: TeamSide,
    pub call: &'static str,
    /// 是否存在可推翻原判的事实证据。
    pub overturnable: bool,
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
        self.last_ruling = Some(Ruling {
            possession,
            call,
            overturnable: false,
        });
    }

    /// 登记一条附带证据状态的可复核裁决。
    pub fn record_ruling_with_evidence(
        &mut self,
        possession: TeamSide,
        call: &'static str,
        overturnable: bool,
    ) {
        self.last_ruling = Some(Ruling {
            possession,
            call,
            overturnable,
        });
    }

    /// 主教练申请复核。
    ///
    /// 停表期间、联赛规定的复核窗口内、本方还有挑战额度、且这条裁决在
    /// 档案的复核范围内，四个条件同时成立才受理。结论维持原判时按档案
    /// 付出一次暂停，有事实证据时改判并翻转球权。
    pub fn request_challenge(&mut self, team: TeamSide) -> Result<ChallengeOutcome, &'static str> {
        if !matches!(self.phase(), Phase::DeadBall { .. } | Phase::Inbound { .. }) {
            return Err("challenge requires a stoppage");
        }
        let Some(ruling) = self.last_ruling else {
            return Err("no reviewable ruling on record");
        };
        if !is_reviewable(self, ruling.call) {
            return Err("ruling is outside the instant-replay scope");
        }
        if ruling.possession == team {
            return Err("only the team that lost the ruling may challenge");
        }
        let index = World::team_index(team);
        if self.challenges_left[index] == 0 {
            return Err("no challenge remaining");
        }
        let window = self.rules.league.challenge_final_seconds;
        let final_period = self.clock.period() >= self.rules.league.periods.regulation_periods;
        // 复核窗口在末节最后两分钟：节次与剩余时间两个条件同时成立。
        let late = final_period && self.clock.game_clock() <= f64::from(window);
        if !late {
            return Err("challenge window closed");
        }
        self.challenges_left[index] -= 1;
        if ruling.overturnable {
            // 挑战成功：原判被推翻，不扣除暂停，球权逆转给挑战方
            let new_possession = team;
            match self.phase() {
                Phase::DeadBall { ready_at, .. } => {
                    self.set_phase(Phase::DeadBall {
                        team: new_possession,
                        ready_at,
                    });
                }
                Phase::Inbound { ready_at, .. } => {
                    self.set_phase(Phase::Inbound {
                        team: new_possession,
                        ready_at,
                    });
                }
                _ => {}
            }
            self.possession.team = new_possession;
            self.emit(Fact::Review {
                team,
                call: ruling.call,
                overturned: true,
                cost_timeout: false,
                time: self.now(),
            });
            Ok(ChallengeOutcome::Overturned)
        } else {
            // 复核维持原判：扣除一次暂停
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
}
