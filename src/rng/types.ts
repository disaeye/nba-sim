/**
 * RNG contract — see docs/foundation/rng.md.
 *
 * The kernel's sole randomness interface. Every stochastic call site takes
 * an `Rng` as an explicit parameter; there is no module-global, no closure
 * capture, and no field on `GameState`.
 */
export interface Rng {
  /**
   * Advance the single stream by exactly one step and return the next
   * value in `[0, 1)`. The upper bound is exclusive.
   */
  next(): number;
}

/**
 * Error codes for invalid helper inputs. Each is thrown at the helper
 * boundary (the trust edge); interior kernel code receives typed values and
 * never re-validates.
 */
export type RngInputErrorCode =
  | 'RNG_INVALID_RANGE'
  | 'RNG_EMPTY_ARRAY'
  | 'RNG_LENGTH_MISMATCH'
  | 'RNG_INVALID_WEIGHTS';

/**
 * Typed error for invalid `Rng` helper inputs. Carries a stable `code` so
 * callers can branch with `instanceof RngInputError` + `err.code` rather
 * than parsing a message string.
 */
export class RngInputError extends Error {
  readonly code: RngInputErrorCode;

  constructor(code: RngInputErrorCode, message: string) {
    super(message);
    this.name = 'RngInputError';
    this.code = code;
  }
}
