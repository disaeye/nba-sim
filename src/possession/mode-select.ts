/**
 * modeSelect — see `docs/foundation/possession-plays.md` §Mode Select.
 *
 * Routes a possession to TRANSITION or HALFCOURT based on its start reason.
 * Turnovers and steals are transition-biased; defensive rebounds and makes
 * use calibrated probabilities; all other reasons default to HALFCOURT.
 *
 * Probabilistic triggers consume exactly one rng.next():
 *   AFTER_DEFENSIVE_REBOUND → P(TRANSITION) = TRANSITION_FROM_DREB
 *   AFTER_MAKE              → P(TRANSITION) = TRANSITION_FROM_MAKE
 *   LIVE_BALL_TURNOVER_RECOVER / AFTER_TURNOVER → 0.30 transition rate
 *
 * STEAL is pinned to TRANSITION without consuming a draw. Safe defaults also
 * consume no draw. The runner always passes the single stream RNG; this
 * function owns whether the mode decision needs a draw.
 *
 * Draw count: one rng.next() for each probabilistic trigger; zero for
 * the pinned trigger and safe defaults. The caller always passes the RNG;
 * this function decides internally whether to draw.
 */
import type { Rng } from '../rng/types.js';
import type { PossessionMode, StartReason } from './types.js';

/** P(TRANSITION | start_reason = AFTER_DEFENSIVE_REBOUND) — NBA teams
 *  push ~30% of defensive rebounds into transition looks. The old 0.15
 *  starved transition share below the acceptance band (0.093 vs
 *  [0.12, 0.18]). */
export const TRANSITION_FROM_DREB = 0.24;
/** P(TRANSITION | start_reason = AFTER_MAKE) — ~8% of makes are attacked
 *  quickly (grabbable inbound → early offense). The old 0.02 under-fed
 *  transition. */
export const TRANSITION_FROM_MAKE = 0.08;

/**
 * Pinned TRANSITION trigger. The other live-ball turnover reasons below are
 * transition-biased, but probabilistic, so they consume one RNG draw.
 */
const TRANSITION_PINNED: ReadonlySet<string> = new Set<string>([
  'STEAL',
]);

/**
 * Resolve a start_reason + rng into TRANSITION or HALFCOURT.
 *
 * Determinism: same start_reason + same rng draw sequence + same
 * probability tables always returns the same mode. Tests pin both
 * pinned-mode outcomes (no rng needed) and probabilistic outcomes
 * (specific rng draw values).
 *
 * @param startReason why the possession is starting (FSM trigger — the
 *                    simulate loop uses UPPER_SNAKE: AFTER_JUMP_BALL,
 *                    AFTER_DEFENSIVE_REBOUND, AFTER_MAKE, ...).
 * @param rng         the single-stream rng (T3 / M3). Only consumed
 *                    for the probabilistic start reasons; pinned and
 *                    default triggers do not advance the stream.
 */
export function modeSelect(startReason: StartReason, rng: Rng): PossessionMode {
  if (TRANSITION_PINNED.has(startReason)) return 'TRANSITION';
  if (startReason === 'AFTER_DEFENSIVE_REBOUND') {
    return rng.next() < TRANSITION_FROM_DREB ? 'TRANSITION' : 'HALFCOURT';
  }
  if (startReason === 'AFTER_MAKE') {
    return rng.next() < TRANSITION_FROM_MAKE ? 'TRANSITION' : 'HALFCOURT';
  }
  if (
    startReason === 'LIVE_BALL_TURNOVER_RECOVER' ||
    startReason === 'AFTER_TURNOVER' ||
    startReason === 'after_turnover'
  ) {
    return rng.next() < 0.30 ? 'TRANSITION' : 'HALFCOURT';
  }
  return 'HALFCOURT';
}

/**
 * Pace-identity variant: the coach's paceBias (0 = grind-it-out halfcourt,
 * 1 = run-first) modulates the transition probabilities around the league
 * baseline. A 0.78 pace coach pushes ~45% of defensive rebounds (vs the
 * 0.24 league base); a 0.22 coach walks it up (~12%). This is what makes
 * "run-and-gun vs slow-grind" express as visibly different possession
 * streams — without it paceBias was validated on input and never read.
 */
export function modeSelectForTeam(
  startReason: StartReason,
  rng: Rng,
  paceBias: number | undefined,
): PossessionMode {
  if (paceBias === undefined) return modeSelect(startReason, rng);
  if (TRANSITION_PINNED.has(startReason)) return 'TRANSITION';
  // paceBias 0.5 → league baseline; slope ±0.6 clamps the extremes to
  // [0.03, 0.55] for DREB and [0.01, 0.20] for after-make pushes.
  const paceShift = (paceBias - 0.5) * 0.6;
  if (startReason === 'AFTER_DEFENSIVE_REBOUND') {
    return rng.next() < Math.max(0.03, Math.min(0.55, TRANSITION_FROM_DREB + paceShift * 0.5)) ? 'TRANSITION' : 'HALFCOURT';
  }
  if (startReason === 'AFTER_MAKE') {
    return rng.next() < Math.max(0.01, Math.min(0.20, TRANSITION_FROM_MAKE + paceShift * 0.15)) ? 'TRANSITION' : 'HALFCOURT';
  }
  if (
    startReason === 'LIVE_BALL_TURNOVER_RECOVER' ||
    startReason === 'AFTER_TURNOVER' ||
    startReason === 'after_turnover'
  ) {
    return rng.next() < Math.max(0.1, Math.min(0.6, 0.30 + paceShift * 0.5)) ? 'TRANSITION' : 'HALFCOURT';
  }
  return 'HALFCOURT';
}
