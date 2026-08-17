/**
 * step — the kernel's per-tick transition function.
 *
 * Signature locked for T15+ (rules FSM, possession, resolve) to fill in.
 * The shell here is a pure no-op: it returns the input state unchanged
 * (by reference) inside a new wrapper object and emits no events.
 *
 * Purity: `step` MUST NOT mutate `state` or consume from `rng`. The
 * `rng` parameter is present so T15+ can thread it into the foundation's
 * ordered draw sites (duration → mode_select → play_select → resolve) —
 * see `docs/foundation/rng.md`.
 */
import type { Rng } from '../rng/types.js';
import type { Event, GameState } from './types.js';

/** One tick's worth of work: zero or more events + the resulting state. */
export interface StepResult {
  readonly state: GameState;
  readonly events: readonly Event[];
}

/**
 * Shell implementation: emits nothing, advances nothing. `rng` is
 * intentionally not read — including it in the signature now means T15+
 * fillers don't have to change call sites.
 */
export function step(state: GameState, _rng: Rng): StepResult {
  return { state, events: [] };
}
