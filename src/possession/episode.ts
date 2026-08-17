/**
 * Possession episode — the lifecycle wrapper around a single possession.
 *
 * The episode tracks the possession's identity across multiple decision
 * windows: which team, why it started, which mode, the advisory play, and
 * whether it has ended. The caller (simulate loop) reads and advances the
 * mutable fields; the live offense engine (DecisionKernel) never touches
 * the episode directly.
 *
 * OREB continuation (Metis G1.8): when a SHOT misses and the rebound is
 * offensive, the simulate loop selects a fresh advisory play from the same
 * mode and leaves the binding UNCHANGED. The possession_id does not
 * increment.
 */
import type { Event, GameState, TeamId } from '../state/types.js';
import type { Rng } from '../rng/types.js';
import { PossessionError } from './types.js';
import type { EndReason, PossessionEpisode, StartReason } from './types.js';
import { modeSelectForTeam } from './mode-select.js';

export function createEpisode(state: GameState, startReason: StartReason, rng: Rng): PossessionEpisode {
  const team = state.possession.team;
  if (team === null) {
    throw new PossessionError(
      'POSSESSION_NO_TEAM',
      'createEpisode: state.possession.team is null — cannot create an episode without an offensive team',
    );
  }
  return {
    team,
    startReason,
    mode: modeSelectForTeam(startReason, rng, state.coach?.[team]?.paceBias),
    play: null,
    ended: false,
    endReason: null,
  };
}

export function inferEndReason(events: readonly Event[]): EndReason {
  for (let i = events.length - 1; i >= 0; i--) {
    const e = events[i];
    if (e === undefined) continue;
    if (e.type === 'FOUL') {
      const shooting = e.payload['shooting'] === true;
      const fts = e.payload['free_throws_awarded'];
      if (shooting && fts === 1) return 'AND_ONE';
      if (shooting) return 'SHOOTING_FOUL';
      return 'NON_SHOOTING_FOUL';
    }
    if (e.type === 'TURNOVER') return 'TURNOVER';
    if (e.type === 'SHOT_CLOCK_VIOLATION') return 'SHOT_CLOCK_VIOLATION';
    if (e.type === 'PERIOD_END') return 'PERIOD_END';
    if (e.type === 'MADE_BASKET_DEAD') return 'MAKE';
    if (e.type === 'REBOUND') {
      const offensive = e.payload['offensive'];
      if (offensive === true) return 'MISS_OREB_CONTINUE';
      if (offensive === false) return 'MISS_DREB';
    }
  }
  for (const e of events) {
    if (e.type === 'SHOT_RESULT' && e.payload['made'] === true) return 'MAKE';
  }
  return 'UNKNOWN';
}

export function defendingTeam(team: TeamId): TeamId {
  return team === 'home' ? 'away' : 'home';
}
