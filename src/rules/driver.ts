/**
 * FSM driver — the pure rules layer.
 *
 * Every function here is pure: it reads `state` and/or the loaded
 * `config/fsm.json` and returns a value. No function mutates `state`;
 * `transition` returns a fresh `TransitionResult` object (or null), not
 * a mutated state. The caller (todo 19's `step` loop) is responsible for
 * applying the result to produce a new `GameState`.
 *
 * The config is loaded once at module init via a static JSON import
 * (same pattern as `src/duration/loader.ts`); the module-level `FSM_CONFIG`
 * const is a stable reference for the lifetime of the process.
 */
import type { GameState, Phase, TeamId } from '../state/types.js';
import { loadFsmConfig } from './loader.js';
import type { PeriodRouterResult, TransitionResult } from './types.js';

const FSM_CONFIG = loadFsmConfig();

/**
 * The set of phases where the ball is dead. TIMEOUT_START is honored
 * only from these phases in Phase 1 (per `docs/foundation/fsm.md`
 * §Timeout Rules). FT_SEQUENCE and TIMEOUT itself are deliberately
 * excluded — a timeout called mid-FT or nested inside another timeout
 * is not modeled in v0.1.0.
 */
const DEAD_PHASES: ReadonlySet<Phase> = new Set<Phase>([
  'DEAD_OOB',
  'DEAD_FOUL',
  'DEAD_VIOLATION',
  'DEAD_MAKE',
  'DEAD_HELD',
  'DEAD_PERIOD_END',
]);

/** Pre-computed membership set for substitution legality checks. */
const LEGAL_SUB_PHASES: ReadonlySet<Phase> = new Set<Phase>(
  FSM_CONFIG.substitution_legality.legal_phases,
);

// ─── transition ─────────────────────────────────────────────────────────────

/**
 * Look up `(state.phase, trigger)` in the transition table and return
 * the matching row's `{ phase, emit_events, notes }`, or `null` if no
 * legal transition exists for that pair.
 *
 * Pure: does not mutate `state`. The caller applies the result.
 */
export function transition(state: GameState, trigger: string): TransitionResult | null {
  const row = FSM_CONFIG.transitions.find(
    (t) => t.from === state.phase && t.trigger === trigger,
  );
  if (row === undefined) return null;
  return {
    phase: row.to,
    emit_events: row.emit_events,
    notes: row.notes,
  };
}

// ─── periodRouter ───────────────────────────────────────────────────────────

/**
 * Deterministic routing for end-of-period transitions.
 *
 * Given the period that just ended (1-indexed: 1=Q1 … 4=Q4, 5+=OT) and
 * the score difference (home − away) at the moment `DEAD_PERIOD_END` was
 * entered, returns the next phase the FSM should transition to.
 *
 * Rules (see `docs/foundation/fsm.md` §Period Router):
 *   Q1 → PERIOD_BREAK; Q2 → HALFTIME; Q3 → PERIOD_BREAK.
 *   Q4 / OT tied (diff 0) → OVERTIME_SETUP (unlimited OT per decision M6).
 *   Q4 / OT not tied → POST_GAME.
 */
export function periodRouter(periodJustEnded: number, scoreDiff: number): PeriodRouterResult {
  if (periodJustEnded === 1) return 'PERIOD_BREAK';
  if (periodJustEnded === 2) return 'HALFTIME';
  if (periodJustEnded === 3) return 'PERIOD_BREAK';
  return scoreDiff === 0 ? 'OVERTIME_SETUP' : 'POST_GAME';
}

// ─── canCallTimeout ─────────────────────────────────────────────────────────

/**
 * Whether `team` may legally call a timeout right now. True only when
 * the phase is a dead-ball phase (any `DEAD_*`) and the team has at
 * least one timeout remaining.
 *
 * Phase-1 pin: offensive-live-ball timeouts and mid-FT timeouts are
 * NOT modeled (per `docs/foundation/fsm.md` §Timeout Rules). The
 * LIVE-ball allowance for the offensive team is documented but not
 * wired in v0.1.0; this function returns false for LIVE.
 */
export function canCallTimeout(state: GameState, team: TeamId): boolean {
  if (!DEAD_PHASES.has(state.phase)) return false;
  const remaining =
    team === 'home' ? state.timeouts.remaining.home : state.timeouts.remaining.away;
  return remaining > 0;
}

// ─── canSubstitute ──────────────────────────────────────────────────────────

/**
 * Whether a SUB event may legally be emitted right now. True only when
 * the current phase is in `config/fsm.json#substitution_legality.legal_phases`.
 *
 * FT_SEQUENCE is in the illegal list (per the Phase-1 simplification
 * documented in `docs/foundation/fsm.md` §FT_SEQUENCE sub rule): subs
 * are allowed only before the FT_AWARDED transition fires (while still
 * in DEAD_FOUL) or after FT_DONE_SHOOTING_FOUL completes (back in
 * DEAD_FOUL), never mid-sequence.
 */
export function canSubstitute(state: GameState): boolean {
  return LEGAL_SUB_PHASES.has(state.phase);
}

// ─── timeoutAllotment ───────────────────────────────────────────────────────

/**
 * The mandatory timeout cap for the given period. Returns the per-team
 * maximum from `config/fsm.json#timeout_allotments`:
 *   periods 1–3 → regulation_total (7)
 *   period    4 → max_in_q4 (4)
 *   period   5+ → overtime_total (2)
 *
 * The cap is the larger of "what the team carried in" and "what this
 * period allows"; the kernel applies it at period-start. This function
 * only reports the cap; the enforcement lives in the step loop (T19).
 */
export function timeoutAllotment(period: number): number {
  if (period >= 5) return FSM_CONFIG.timeout_allotments.overtime_total;
  if (period === 4) return FSM_CONFIG.timeout_allotments.max_in_q4;
  return FSM_CONFIG.timeout_allotments.regulation_total;
}

// ─── isBonus ────────────────────────────────────────────────────────────────

/**
 * Whether the team is in the penalty (bonus) for the current period.
 * NBA rule: 5th team foul in a period triggers the bonus. The caller
 * passes the team's current-period foul count (e.g.
 * `state.fouls.team.home`); this function checks the threshold.
 */
export function isBonus(teamFouls: number): boolean {
  return teamFouls >= 5;
}
