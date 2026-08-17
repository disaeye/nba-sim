/**
 * Test fixtures for the state module.Keeps the per-test files small and
 * focused on assertions rather than boilerplate event construction.
 *
 * The lineups store whatever identifying string the caller passes; the
 * state spec calls them "jerseys" but the SUB/PASS/FOUL payloads reference
 * the same namespace of strings (player identifiers used uniformly across
 * events and lineups). See `docs/foundation/identity-roles.md` — both
 * `Player.id` and `Player.jersey` are plain strings, and applyEvent never
 * needs to translate between them.
 */
import type { Event, EventType, GameInput, GameState } from '../../src/state/types.js';
import { createInitialState } from '../../src/state/initial.js';

/**
 * Minimal two-team GameInput: 5 starters per side, all unique ids.
 * Adequate for fold / immutability tests; richer scenarios live in their
 * own describe blocks.
 */
export function minimalGameInput(): GameInput {
  return {
    home: {
      id: 'home',
      starters: ['h1', 'h2', 'h3', 'h4', 'h5'],
    },
    away: {
      id: 'away',
      starters: ['a1', 'a2', 'a3', 'a4', 'a5'],
    },
  };
}

/** Fresh initial state — equivalent to `createInitialState(minimalGameInput())`. */
export function makeState(): GameState {
  return createInitialState(minimalGameInput());
}

/**
 * Build a synthetic Event with sensible defaults. Tests override only the
 * fields they care about. `seq` defaults to 1; `t_game` defaults to 0
 * (the very first event in a fresh state).
 */
export function makeEvent<E extends Event>(overrides: Partial<E> & { type: EventType }): E {
  const base: Event = {
    type: overrides.type,
    t_game: 0,
    t_real: 0,
    seq: 1,
    actors: [],
    payload: {},
    clocks: { game: 720.0, shot: 24.0 },
    score: { home: 0, away: 0 },
  };
  // Merge top-level fields; allow payload + nested objects to be overridden
  // wholesale if the caller specifies them.
  const merged: Event = { ...base, ...overrides } as Event;
  return merged as E;
}
