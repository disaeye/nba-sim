//! 比赛内核时钟的运行阶段判定与推进。

use crate::state::{Clock, ClockAdvance, Phase};

/// 时钟快照：期次、比赛钟、进攻钟。账本、协议与验证只依赖这三个数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockState {
    pub period: u32,
    pub game_clock: f64,
    pub shot_clock: f64,
}

/// 对时钟的一次要求：把哪个钟改成多少。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockDirective {
    /// 设定进攻钟（抢到进攻篮板、犯规重置、节开局）。
    ShotClock(f64),
    /// 设定比赛钟（节间重置、复核回表）。
    GameClock(f64),
}

/// 执行时钟要求的入口：所有改钟的代码都经它，不在各处直接写字段。
pub struct ClockController<'a> {
    clock: &'a mut Clock,
}

impl<'a> ClockController<'a> {
    pub fn new(clock: &'a mut Clock) -> Self {
        Self { clock }
    }

    pub fn apply(&mut self, directive: ClockDirective) {
        match directive {
            ClockDirective::ShotClock(seconds) => self.clock.set_shot_clock(seconds),
            ClockDirective::GameClock(seconds) => self.clock.set_game_clock(seconds),
        }
    }

    pub fn state(&self) -> ClockState {
        self.clock.state()
    }
}

/// 根据当前比赛阶段推进对应时钟。
pub fn advance(
    phase: Phase,
    flight: Option<crate::state::BallFlight>,
    clock: &mut Clock,
    seconds: f64,
) -> ClockAdvance {
    // 飞行期间按飞行种类走钟：传球两钟都走，投篮与跳球只走比赛钟，
    // 罚球停钟；没有飞行时由比赛阶段决定。
    let clocks = match flight {
        Some(flight) => flight.kind.clocks(),
        None => phase.clocks(),
    };
    clock.advance(seconds, clocks);
    clocks
}

#[cfg(test)]
mod tests {
    use super::advance;
    use crate::state::{Clock, Phase, TeamSide};

    #[test]
    fn clock_families_follow_phase_policy() {
        let phases = [
            (Phase::Pregame, false, false, false),
            (
                Phase::DeadBall {
                    team: TeamSide::Home,
                    ready_at: 0.0,
                },
                false,
                false,
                false,
            ),
            (
                Phase::Inbound {
                    team: TeamSide::Home,
                    ready_at: 0.0,
                },
                false,
                false,
                false,
            ),
            (
                Phase::JumpBall {
                    home_jumper: 0,
                    away_jumper: 1,
                    resolve_at: 0.0,
                    kind: crate::action::JumpBallKind::Opening,
                },
                false,
                false,
                false,
            ),
            (
                Phase::FreeThrows {
                    shooter: 0,
                    total: 3,
                    remaining: 3,
                    made: 0,
                    next_at: 0.0,
                    and_one: false,
                    retain_possession: false,
                    foul_id: 9,
                    technical_retained_team: None,
                    after_technical: None,
                },
                false,
                false,
                false,
            ),
            (
                Phase::Timeout {
                    team: TeamSide::Home,
                    started_at: 0.0,
                    resume_at: 0.0,
                    resume_phase: crate::state::ResumePhase::Live,
                },
                false,
                false,
                false,
            ),
            (
                Phase::FreeThrows {
                    shooter: 0,
                    total: 2,
                    remaining: 2,
                    made: 0,
                    next_at: 0.0,
                    and_one: false,
                    retain_possession: false,
                    foul_id: 9,
                    technical_retained_team: None,
                    after_technical: None,
                },
                false,
                false,
                false,
            ),
            (Phase::Live, true, true, true),
            (
                Phase::PeriodBreak {
                    resume_at: 1.0,
                    next_stage: crate::state::MatchStage::Intermission,
                },
                false,
                false,
                false,
            ),
            (Phase::Finished, false, false, false),
        ];
        for (phase, game_clock_runs, shot_clock_runs, elapsed_runs) in phases {
            let mut clock = Clock::new(720.0, 24.0);
            let advanced = advance(phase, None, &mut clock, 0.25);
            assert_eq!(advanced.game_clock, game_clock_runs, "{phase:?}");
            assert_eq!(advanced.shot_clock, shot_clock_runs, "{phase:?}");
            assert_eq!(advanced.elapsed, elapsed_runs, "{phase:?}");
            assert_eq!(advanced.player_time, game_clock_runs, "{phase:?}");
            assert_eq!(
                clock.game_clock(),
                if game_clock_runs { 719.75 } else { 720.0 }
            );
            assert_eq!(
                clock.shot_clock(),
                if shot_clock_runs { 23.75 } else { 24.0 }
            );
            assert_eq!(clock.elapsed(), if elapsed_runs { 0.25 } else { 0.0 });
            assert_eq!(clock.simulation_time(), 0.25);
        }
        // 飞行按种类走钟：传球两钟都走，投篮与跳球只走比赛钟，罚球停钟。
        for (kind, game_clock_runs, shot_clock_runs) in [
            (crate::state::FlightKind::Pass, true, true),
            (crate::state::FlightKind::Shot, true, false),
            (crate::state::FlightKind::JumpBall, true, false),
            (crate::state::FlightKind::FreeThrow, false, false),
        ] {
            let mut clock = Clock::new(720.0, 24.0);
            let flight = crate::state::BallFlight {
                kind,
                resolve_at: 1.0,
            };
            let advanced = advance(Phase::Live, Some(flight), &mut clock, 0.25);
            assert_eq!(advanced.game_clock, game_clock_runs, "{kind:?}");
            assert_eq!(advanced.shot_clock, shot_clock_runs, "{kind:?}");
        }
    }

    #[test]
    fn match_clock_and_shot_clock_can_be_set_independently() {
        let mut clock = Clock::new(720.0, 24.0);
        clock.set_shot_clock_at_least(14.0);
        assert_eq!(clock.game_clock(), 720.0);
        assert_eq!(clock.shot_clock(), 24.0);
        clock.set_shot_clock(14.0);
        assert_eq!(clock.shot_clock(), 14.0);
        clock.start_next_period(300.0, 24.0);
        assert_eq!(clock.period(), 2);
        assert_eq!(clock.game_clock(), 300.0);
        assert_eq!(clock.shot_clock(), 24.0);
    }
}
