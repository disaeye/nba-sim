/**
 * Clocks contract — see docs/foundation/clocks-duration.md.
 *
 * The kernel models time as three logical clocks (period number, game
 * seconds remaining, shot seconds remaining) at a fixed 0.1 s resolution.
 * All mutation in this file is functional: `tick` and `resetShotClock`
 * return NEW `Clocks` values; inputs are never modified in place.
 */

/**
 * A point-in-time snapshot of the three live clocks. All values are
 * seconds at 0.1 s resolution. `period` is 1-indexed (1..4 for regulation,
 * 5+ for overtime periods — see fsm.md period router).
 */
export interface Clocks {
  readonly period: number;
  readonly game: number;
  readonly shot: number;
}

/**
 * Initial clocks at the start of a regulation game:
 * period 1, game 12:00 (720.0 s), shot clock full (24.0 s).
 */
export function createClocks(): Clocks {
  return { period: 1, game: 720.0, shot: 24.0 };
}

/**
 * Snap a raw sampled duration to the foundation's 0.1 s grid.
 *
 * Decision M7 (docs/foundation/clocks-duration.md#resolution--quantization):
 *   delta = Math.round(delta_raw * 10) / 10
 *
 * ES `Math.round` rounds half-toward-+Infinity, not banker's rounding and
 * not symmetric half-away-from-zero. For positive `.5` inputs this looks
 * like "round half up": `Math.round(1.5) === 2`, `Math.round(2.5) === 3`.
 * For negative `.5` inputs it rounds toward zero: `Math.round(-0.5) === 0`.
 * Since the kernel only ever quantizes non-negative durations, the
 * practical rule is "round halves up to the next tenth". The TDD test
 * locks `quantize(1.25) === 1.3` precisely to catch any future refactor
 * that silently switches the rounding mode — that would invalidate the
 * golden seed (todo 20) and every `t_game` on the timeline.
 */
export function quantize(deltaRaw: number): number {
  return Math.round(deltaRaw * 10) / 10;
}

/**
 * Clamp a sampled delta to the live clocks. The clamped value is
 * `min(delta, shot_clock_remaining, game_clock_remaining)` per
 * docs/foundation/clocks-duration.md#clamp-rule. The caller (FSM driver,
 * todo 15) is responsible for emitting `SHOT_CLOCK_VIOLATION` /
 * `PERIOD_END` based on which clock the clamp hit; this function only
 * computes the clamped value.
 */
export function clampDelta(delta: number, clocks: Clocks): number {
  return Math.min(delta, clocks.shot, clocks.game);
}

/**
 * Advance both running clocks (game, shot) by `delta` seconds. Returns a
 * NEW `Clocks`; the input is never mutated. `period` is preserved — period
 * transitions are owned by the FSM driver (todo 15), not by `tick`.
 *
 * `delta` MUST already be non-negative, quantized, and clamped
 * (`clampDelta(quantize(raw), clocks)`); `tick` does not re-validate.
 */
export function tick(clocks: Clocks, delta: number): Clocks {
  return {
    period: clocks.period,
    game: clocks.game - delta,
    shot: clocks.shot - delta,
  };
}

/**
 * Return a NEW `Clocks` with the shot clock reset to `to` (24 full or 14
 * partial per the v0.1.0 reset rules subset). `period` and `game` are
 * preserved. The input is never mutated.
 */
export function resetShotClock(clocks: Clocks, to: 24 | 14): Clocks {
  return {
    period: clocks.period,
    game: clocks.game,
    shot: to,
  };
}
