/**
 * Emit a semantic event through applyEvent — Adjudicate-only helper.
 */
import type { Event, EventType, GameState } from '../state/types.js';
import { applyEvent } from '../state/apply.js';

export function emit(
  state: GameState,
  type: EventType,
  actors: readonly string[],
  payload: Record<string, unknown>,
): GameState {
  const event: Event = {
    type,
    t_game: state.clocks.game,
    t_real: state.realClock,
    seq: state.seq + 1,
    actors: [...actors],
    payload,
    clocks: { game: state.clocks.game, shot: state.clocks.shot },
    score: { ...state.score },
  };
  return applyEvent(state, event);
}

export function lastEvent(state: GameState): Event | null {
  return state.events.length > 0 ? (state.events[state.events.length - 1] ?? null) : null;
}
