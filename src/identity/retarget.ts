/**
 * On-ball retarget — the contract that prevents the "steal on off-ball
 * player" bug. See `docs/foundation/identity-roles.md` §Live Assignment
 * & On-Ball Retarget.
 *
 * The structural fix is simple: there is NO cached "on_ball defender id".
 * `getOnBallDefender` derives the defender from the LIVE lineups +
 * LIVE `ball.holderId` every time it is called. Any caller (STEAL,
 * STRIP, CONTEST resolution) that asks "who is the on-ball defender?"
 * receives the defender for the CURRENT holder, never a stale capture.
 *
 * Phase 1 matchup model: 1:1 by index. Each team's lineup is an ordered
 * 5-array; offensive player[i] is guarded by defensive player[i]. This
 * is sufficient for homogeneous abilities — no switching, doubling, or
 * coaching AI (out of scope per plan guardrails).
 */
import type { GameState } from '../state/types.js';
import type { MatchupMap } from './types.js';
import { IdentityError } from './types.js';

// ─── lineup resolution ──────────────────────────────────────────────────────

/** The lineup of the team currently in possession of the ball. */
function offensiveLineup(state: GameState): readonly string[] {
  const team = state.possession.team;
  if (team === null) {
    throw new IdentityError(
      'IDENTITY_NO_POSSESSION',
      'offensiveLineup: state.possession.team is null — no team has the ball',
    );
  }
  return team === 'home' ? state.lineups.home : state.lineups.away;
}

/** The lineup of the defending team (the opposite of the possessor). */
function defensiveLineup(state: GameState): readonly string[] {
  const team = state.possession.team;
  if (team === null) {
    throw new IdentityError(
      'IDENTITY_NO_POSSESSION',
      'defensiveLineup: state.possession.team is null — no team has the ball',
    );
  }
  return team === 'home' ? state.lineups.away : state.lineups.home;
}

// ─── matchup map ────────────────────────────────────────────────────────────

/**
 * Build the 1:1 by-index holderId → defenderId map from two lineups.
 * `offensiveLineup[i]` is guarded by `defensiveLineup[i]`. Both lineups
 * must be the same length (5 for v0.1.0).
 */
export function buildMatchupMap(
  offensive: readonly string[],
  defensive: readonly string[],
): MatchupMap {
  if (offensive.length !== defensive.length) {
    throw new IdentityError(
      'IDENTITY_LINEUP_LENGTH_MISMATCH',
      `buildMatchupMap: offensive lineup has ${offensive.length} players but defensive has ${defensive.length}`,
    );
  }
  const map: Record<string, string> = {};
  for (let i = 0; i < offensive.length; i++) {
    const off = offensive[i];
    const def = defensive[i];
    if (off === undefined || def === undefined) {
      // Unreachable given the length check above, but noUncheckedIndexedAccess
      // types this access as `string | undefined`. The narrow is mandatory,
      // not defensive bloat.
      throw new IdentityError(
        'IDENTITY_LINEUP_LENGTH_MISMATCH',
        `buildMatchupMap: undefined entry at index ${i}`,
      );
    }
    map[off] = def;
  }
  return map;
}

// ─── the on-ball lookup ─────────────────────────────────────────────────────

/**
 * THE function STEAL / STRIP / CONTEST resolution MUST call. Returns the
 * jersey of the defender covering `holderId` under the Phase 1 1:1 model.
 *
 * Derivation is purely structural: the holder's index in the offensive
 * lineup selects the defender at the same index in the defensive lineup.
 * No cached id, no stale capture — the live lineups + live holderId are
 * the only inputs.
 *
 * `opponentLineup` is an optional override for callers that maintain
 * their own defensive lineup reference (e.g. after a SUB that has not
 * yet been folded into `state.lineups`). When omitted, the defensive
 * lineup is derived from `state.possession.team`.
 */
export function getOnBallDefender(
  state: GameState,
  holderId: string,
  opponentLineup?: readonly string[],
): string {
  const offLineup = offensiveLineup(state);
  const defLineup = opponentLineup ?? defensiveLineup(state);
  const idx = offLineup.indexOf(holderId);
  if (idx === -1) {
    throw new IdentityError(
      'IDENTITY_NO_HOLDER',
      `getOnBallDefender: holder "${holderId}" is not in the offensive lineup [${offLineup.join(', ')}]`,
    );
  }
  const defender = defLineup[idx];
  if (defender === undefined) {
    // Possible when the caller passes a short `opponentLineup` override.
    throw new IdentityError(
      'IDENTITY_NO_DEFENDER',
      `getOnBallDefender: no defender at index ${idx} (defensive lineup length ${defLineup.length})`,
    );
  }
  return defender;
}

// ─── retarget ───────────────────────────────────────────────────────────────

/**
 * Recompute the on_ball assignment after the holder changes (PASS,
 * HANDOFF, STEAL, REBOUND, LOOSE_BALL_RECOVER). Returns a fresh
 * `MatchupMap` for the new state.
 *
 * Phase 1 behaviour: the 1:1 by-index map is invariant across the
 * possession — what changes on retarget is which offensive player the
 * `on_ball` slot conceptually covers. The map content equals
 * `buildMatchupMap(offensive, defensive)`; the function exists as the
 * explicit "ball moved, retarget" call site the possession engine (T17)
 * invokes on every ball-moving event, and as the seam future phases
 * extend with switching/doubling logic.
 *
 * Always returns a NEW object so callers can hold the prior map without
 * aliasing surprises.
 */
export function retargetOnBall(state: GameState, newHolderId: string): MatchupMap {
  const offLineup = offensiveLineup(state);
  if (!offLineup.includes(newHolderId)) {
    throw new IdentityError(
      'IDENTITY_NO_HOLDER',
      `retargetOnBall: new holder "${newHolderId}" is not in the offensive lineup [${offLineup.join(', ')}]`,
    );
  }
  return buildMatchupMap(offLineup, defensiveLineup(state));
}
