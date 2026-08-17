import type { Rng } from './types.js';

/**
 * mulberry32 PRNG — the kernel's sole entropy source.
 *
 * Contract: docs/foundation/rng.md. For a fixed 32-bit seed the produced
 * sequence is fully reproducible across implementations, machines, and
 * runs. Internal state is one 32-bit word advanced in place by each
 * `next()` call. Returns a `number` in `[0, 1)` per call.
 *
 * The implementation below is the canonical mulberry32 (bryan cutt's
 * original) and is intentionally byte-identical to reference deployments;
 * the seed=1 first-5-values are pinned in `tests/rng/mulberry32.test.ts`
 * to catch any silent drift.
 *
 * @param seed uint32 seed (any number is coerced via `| 0` on first call).
 */
export function mulberry32(seed: number): Rng {
  let a = seed;
  return {
    next(): number {
      a |= 0;
      a = (a + 0x6d2b79f5) | 0;
      let t = Math.imul(a ^ (a >>> 15), 1 | a);
      t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    },
  };
}
