/**
 * selectPlay — uniform random pick among the plays matching `mode`.
 *
 * Phase 1 weights are uniform per `docs/foundation/possession-plays.md`
 * (equal share per play in the mode). The selection consumes exactly one
 * `rng.next()` call via `pick`, keeping single-stream draw accounting
 * auditable per the foundation RNG contract (M3 / docs/foundation/rng.md).
 *
 * Pure: does not mutate `config`, `rng`, or any shared state. The first
 * draw after `modeSelect` (zero draws) is consumed here; the next draw
 * The duration sample happens in DecisionKernel's commit (src/decision).
 */
import type { Rng } from '../rng/types.js';
import { RngInputError } from '../rng/types.js';
import type { Play, PlaysConfig, PossessionMode } from './types.js';

/**
 * Pick a play from `config.plays` whose `mode` matches. Throws
 * `RngInputError('RNG_EMPTY_ARRAY')` when no plays match — that is a
 * configuration error (the playbook is empty for the requested mode), not
 * a runtime condition the caller can recover from.
 */
export function selectPlay(mode: PossessionMode, rng: Rng, config: PlaysConfig): Play {
  const eligible = config.plays.filter((p) => p.mode === mode);
  if (eligible.length === 0) {
    throw new RngInputError('RNG_EMPTY_ARRAY', 'selectPlay: no plays match the requested mode');
  }
  // Weighted pick: each play contributes its weight (default 1) to the
  // single-stream RNG contract while letting plays.json shape the mix
  // toward NBA tactic-frequency reality (PNR-heavy, ISO/POST as changeup).
  const totalWeight = eligible.reduce((sum, p) => sum + (p.weight ?? 1), 0);
  let roll = rng.next() * totalWeight;
  for (const play of eligible) {
    roll -= play.weight ?? 1;
    if (roll <= 0) return play;
  }
  return eligible[eligible.length - 1]!;
}
