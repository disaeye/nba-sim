/**
 * applyEvent — the canonical state fold.
 *
 * Pure: NEVER mutates the input state. Returns a NEW `GameState` whose
 * `events` array is one longer and whose `seq` is one higher than the
 * input. Field-level changes are computed by the named handlers in
 * `handlers.ts`; events with an empty `mutates` declaration in the
 * catalog (cosmetic markers) leave every gameplay field equal to the
 * input.
 *
 * Algorithm (see docs/foundation/events.md §Fold Algorithm):
 *   1. Resolve the per-event-type handler.
 *   2. Compute the new gameplay state (a NEW object).
 *   3. Append the event to `events` and bump `seq` — always.
 *
 * The dispatch table is closed over the same 38 catalog entries; the
 * type-level parity test in `tests/state/types.test.ts` catches drift.
 */
import type { Event, GameState } from './types.js';
import { StateError } from './types.js';
import {
  applyClockExpiry,
  applyFoul,
  applyFtResult,
  applyFtSequenceEnd,
  applyJumpBallTap,
  applyPeriodEnd,
  applyPossessionGained,
  applyRebound,
  applyShotResult,
  applySteal,
  applySub,
  applyTimeoutStart,
  applyTurnover,
  applyAlignment,
  otherTeam,
  str,
  team,
} from './handlers.js';

/**
 * Apply a single event to `state` and return the new state. Pure; never
 * mutates `state` or `event`.
 */
export function applyEvent(state: GameState, event: Event): GameState {
  const next: GameState = applyMutations(state, event);
  return {
    ...next,
    events: [...next.events, event],
    seq: next.seq + 1,
  };
}

function applyMutations(state: GameState, event: Event): GameState {
  switch (event.type) {
    // ── empty-mutates (cosmetic markers; no gameplay change) ───────────────
    case 'ALIGN_HALFCOURT':
    case 'SCREEN_SET':
    case 'SCREEN_USE':
    case 'SHOT_RELEASE':
    case 'FT_ATTEMPT':
    case 'STATE_NOTE':
      return state;

    // ── ball-holder movement (map a payload field → ball.holderId) ─────────
    case 'PASS': {
      const note = event.payload['note'];
      if (note === 'flight_start') {
        return { ...state, ball: { ...state.ball, holderId: null, status: 'pass' } };
      }
      if (note === 'flight_complete') {
        return { ...state, ball: { ...state.ball, holderId: str(event, 'receiver_id'), status: 'held' } };
      }
      return { ...state, ball: { ...state.ball, holderId: str(event, 'receiver_id') } };
    }
    case 'HANDOFF':
      return { ...state, ball: { ...state.ball, holderId: str(event, 'receiver_id'), status: 'held' } };
    // ── ball-holder movement (map a payload field → ball.holderId) ─────────
    case 'ADVANCE_BACKCOURT':
    case 'CROSS_HALF':
    case 'DRIVE':
      return { ...state, ball: { ...state.ball, holderId: str(event, 'ballHandlerId') } };
    case 'INBOUND_TOUCH':
      return { ...state, ball: { ...state.ball, holderId: str(event, 'receiver_id') } };

    // ── phase-only events (next phase is derivable from event type) ────────
    case 'GAME_START':
      return { ...state, phase: 'JUMP_BALL' };
    case 'HALFTIME':
      return {
        ...state,
        phase: 'HALFTIME',
        baskets: { home: state.baskets.away, away: state.baskets.home },
      };
    case 'GAME_END':
      return { ...state, phase: 'POST_GAME' };
    case 'MADE_BASKET_DEAD':
      return { ...state, phase: 'DEAD_MAKE' };
    case 'HELD_BALL':
      return { ...state, phase: 'DEAD_HELD' };
    case 'FT_START':
      return { ...state, phase: 'FT_SEQUENCE' };
    case 'FT_SEQUENCE_END':
      return applyFtSequenceEnd(state, event);
    case 'INBOUND_START':
      // No dedicated inbound phase in v0.1.0 (see fsm.json). The emitter
      // is expected to follow with POSSESSION_GAINED; here we just mark
      // the ball as inbound-status. Phase is left for the FSM driver.
      return { ...state, ball: { ...state.ball, status: 'inbound' } };

    // ── scoreboard ─────────────────────────────────────────────────────────
    case 'SHOT_RESULT':
      return applyShotResult(state, event);
    case 'FT_RESULT':
      return applyFtResult(state, event);

    // ── possession changes ────────────────────────────────────────────────
    case 'POSSESSION_GAINED':
      return applyPossessionGained(state, event);
    case 'REBOUND':
      return applyRebound(state, event);
    case 'LOOSE_BALL_RECOVER':
      return {
        ...state,
        ball: { ...state.ball, holderId: str(event, 'recoverer_id') },
        possession: { team: team(event, 'team') },
      };
    case 'STEAL':
      return applySteal(state, event);
    case 'TURNOVER':
      return applyTurnover(state, event);
    case 'VIOLATION':
      return { ...state, possession: { team: otherTeam(team(event, 'team')) } };
    case 'SHOT_CLOCK_VIOLATION':
      return {
        ...state,
        clocks: { ...state.clocks, shot: 0 },
        possession: { team: otherTeam(team(event, 'team')) },
      };

    // ── fouls & subs ───────────────────────────────────────────────────────
    case 'FOUL':
      return applyFoul(state, event);
    case 'SUB':
      return applySub(state, event);

    // ── clocks, period, timeouts, dead balls ──────────────────────────────
    case 'PERIOD_END':
      return applyPeriodEnd(state, event);
    case 'PERIOD_START':
      // Emitter carries the new period number; phase transition is owned
      // by the FSM driver in T15 (PERIOD_START is emitted at break-end).
      return { ...state, period: Number(event.payload['period']) };
    case 'TIMEOUT_START':
      return applyTimeoutStart(state, event);
    case 'TIMEOUT_END':
      // Phase restoration needs prior-state context (T15 territory).
      return state;
    case 'OOB':
      return {
        ...state,
        phase: 'DEAD_OOB',
        possession: { team: otherTeam(team(event, 'team_causing')) },
      };
    case 'JUMP_BALL_TAP':
      return applyJumpBallTap(state, event);
    case 'CLOCK_EXPIRY_ADJUDICATION':
      return applyClockExpiry(state, event);
    case 'JUMP_CIRCLE_ALIGN':
    case 'ALIGNMENT':
      return applyAlignment(state, event);

    default: {
      // Exhaustiveness guard. Adding an EventType without a handler
      // fails compilation (the `never` narrowing rejects the unhandled
      // case). The runtime branch is defensive only; the type system is
      // the load-bearing check.
      const _exhaustive: never = event.type;
      void _exhaustive;
      throw new StateError(
        'STATE_UNKNOWN_EVENT_TYPE',
        event.type,
        `applyEvent: no handler for event type "${event.type}"`,
      );
    }
  }
}
