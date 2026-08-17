import type { Rng } from './types.js';
import { RngInputError } from './types.js';

/**
 * Draw helpers — see docs/foundation/rng.md#helper-functions.
 *
 * Every helper in this file consumes exactly ONE `rng.next()` call per
 * invocation, including `weighted`. That invariant is what keeps draw
 * accounting auditable: the count of `next()` calls across any code path
 * equals the count of helper invocations plus direct `unit(rng)` calls.
 */

/**
 * Thin alias for `rng.next()` returning a float in `[0, 1)`. Present so
 * call sites read as "draw a unit variate" rather than a bare method call.
 */
export function unit(rng: Rng): number {
  return rng.next();
}

/**
 * Inclusive integer draw in `[min, max]` from one `unit(rng)` call.
 * Matches the contract formula:
 *   `Math.floor(unit(rng) * (max - min + 1)) + min`.
 */
export function int(rng: Rng, min: number, max: number): number {
  if (!Number.isInteger(min) || !Number.isInteger(max)) {
    throw new RngInputError(
      'RNG_INVALID_RANGE',
      `int: min and max must be integers (got min=${min}, max=${max})`,
    );
  }
  if (min > max) {
    throw new RngInputError(
      'RNG_INVALID_RANGE',
      `int: min must be <= max (got min=${min}, max=${max})`,
    );
  }
  return Math.floor(unit(rng) * (max - min + 1)) + min;
}

/**
 * Return a random element of `arr` from one `unit(rng)` call. `arr` must
 * be non-empty; equivalent to `arr[int(rng, 0, arr.length - 1)]`.
 */
export function pick<T>(rng: Rng, arr: readonly T[]): T {
  if (arr.length === 0) {
    throw new RngInputError('RNG_EMPTY_ARRAY', 'pick: array must be non-empty');
  }
  const idx = int(rng, 0, arr.length - 1);
  // noUncheckedIndexedAccess: idx is provably in [0, arr.length-1] but TS
  // cannot propagate that through `arr[idx]`, so we narrow explicitly.
  const v = arr[idx];
  if (v === undefined) {
    throw new RngInputError('RNG_EMPTY_ARRAY', `pick: index ${idx} out of range (impossible)`);
  }
  return v;
}

/**
 * Weighted random choice from one `unit(rng)` call. Selects `items[i]`
 * with probability `weights[i] / sum(weights)` via a single cumulative-
 * table lookup.
 *
 * `items.length` must equal `weights.length`; weights must be non-negative
 * finite numbers and not all zero.
 */
export function weighted<T>(rng: Rng, items: readonly T[], weights: readonly number[]): T {
  if (items.length === 0) {
    throw new RngInputError('RNG_EMPTY_ARRAY', 'weighted: items must be non-empty');
  }
  if (weights.length !== items.length) {
    throw new RngInputError(
      'RNG_LENGTH_MISMATCH',
      `weighted: items and weights must have the same length (got items=${items.length}, weights=${weights.length})`,
    );
  }

  // Zip items+weights into a typed tuple array at the boundary. This lets
  // the interior loop use `for...of` and avoid indexed access on two
  // correlated arrays (a known sharp edge of `noUncheckedIndexedAccess`:
  // TS cannot track that `items[i]` and `weights[i]` are both defined for
  // the same `i`, even after a length-equality check).
  const pairs: Array<{ weight: number; item: T }> = [];
  for (let i = 0; i < items.length; i++) {
    const w = weights[i];
    const item = items[i];
    if (w === undefined || item === undefined) {
      throw new RngInputError(
        'RNG_LENGTH_MISMATCH',
        `weighted: index ${i} missing (impossible after length check)`,
      );
    }
    if (w < 0 || !Number.isFinite(w)) {
      throw new RngInputError(
        'RNG_INVALID_WEIGHTS',
        `weighted: weights must be non-negative finite numbers (got weights[${i}]=${w})`,
      );
    }
    pairs.push({ weight: w, item });
  }

  let total = 0;
  for (const { weight } of pairs) {
    total += weight;
  }
  if (total === 0) {
    throw new RngInputError('RNG_INVALID_WEIGHTS', 'weighted: weights must not all be zero');
  }

  const r = unit(rng) * total;
  let acc = 0;
  let last: T | undefined;
  for (const { weight, item } of pairs) {
    last = item;
    acc += weight;
    if (r < acc) {
      return item;
    }
  }
  // Floating-point edge: r === total exactly (rounding). Return the last
  // item seen. `last` is set because pairs.length === items.length >= 1
  // (checked above); the runtime narrowing is forced by TS not propagating
  // the length assertion into the loop body.
  if (last === undefined) {
    throw new RngInputError('RNG_EMPTY_ARRAY', 'weighted: no items accumulated (impossible)');
  }
  return last;
}
