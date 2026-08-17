import type { Rng } from '../rng/types.js';
import { quantize } from '../clocks/types.js';
import type { DurationConfig } from './types.js';
import { DurationNotFoundError } from './types.js';

/**
 * Sample a duration from the catalog and quantize it to the 0.1 s grid.
 *
 * v0.1.0 implements only the `uniform` distribution:
 *   raw = entry.min + rng.next() * (entry.max - entry.min)
 * The foundation linter (scripts/check-foundation.mjs, todo 9) rejects
 * any entry that uses `trunc_normal` at config-load time, so by the time
 * we reach this function the distribution field is provably `uniform`.
 *
 * The returned value is snapped to the foundation's 0.1 s resolution via
 * `quantize` (decision M7) so every `t_game` on the timeline is a
 * multiple of 0.1 s. Consumes exactly one `rng.next()` per call —
 * auditable against single-stream draw accounting (docs/foundation/rng.md).
 *
 * @throws {DurationNotFoundError} if `id` is not present in `config.durations`.
 */
export function sampleDuration(id: string, rng: Rng, config: DurationConfig): number {
  const entry = config.durations.find((d) => d.id === id);
  if (entry === undefined) {
    throw new DurationNotFoundError(id);
  }
  const raw = entry.min + rng.next() * (entry.max - entry.min);
  return quantize(raw);
}
