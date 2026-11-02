//! 比赛内核时钟的运行阶段判定与推进。

use crate::state::{Clock, ClockAdvance, Phase};

/// 根据当前比赛阶段推进对应时钟。
pub fn advance(phase: Phase, clock: &mut Clock, seconds: f64) -> ClockAdvance {
    let clocks = phase.clocks();
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
            (Phase::JumpBallFlight, true, false, true),
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
            (
                Phase::FreeThrowFlight { resolve_at: 0.0 },
                false,
                false,
                false,
            ),
            (Phase::Live, true, true, true),
            (Phase::PassFlight { resolve_at: 1.0 }, true, true, true),
            (Phase::ShotFlight { resolve_at: 1.0 }, true, false, true),
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
            let advanced = advance(phase, &mut clock, 0.25);
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
