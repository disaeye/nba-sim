import { describe, it, expect } from 'vitest';

import {
  createClocks,
  clampDelta,
  quantize,
  tick,
  resetShotClock,
  type Clocks,
} from '../../src/clocks/index.js';

describe('clocks', () => {
  describe('quantize', () => {
    // Decision M7: Math.round(delta_raw * 10) / 10.
    //
    // ES Math.round rounds half-toward-+Infinity (NOT banker's rounding,
    // NOT half-away-from-zero in the symmetric sense). For positive .5
    // values this looks like "half up": Math.round(2.5) === 3,
    // Math.round(1.5) === 2, Math.round(0.5) === 1. For negative .5 values
    // it rounds toward zero: Math.round(-0.5) === 0, Math.round(-1.5) === -1.
    // Since the kernel only ever quantizes non-negative durations, the
    // practical rule is "round halves up to the next tenth". Pinning the
    // canonical expected values here locks the rounding mode against any
    // future refactor toward Math.trunc / Math.floor / banker's rounding —
    // any such change would invalidate the golden seed (todo 20).

    it('rounds 1.24 down to 1.2', () => {
      expect(quantize(1.24)).toBe(1.2);
    });

    it('rounds 1.25 half-up to 1.3 (ES Math.round half-toward-+Inf)', () => {
      expect(quantize(1.25)).toBe(1.3);
    });

    it('passes through an already-quantized 1.0 unchanged', () => {
      expect(quantize(1.0)).toBe(1.0);
    });

    it('rounds 1.26 up to 1.3', () => {
      expect(quantize(1.26)).toBe(1.3);
    });

    it('quantizes 0.05 to 0.1 (smallest non-zero quantum)', () => {
      expect(quantize(0.05)).toBe(0.1);
    });

    it('quantizes 0 to 0', () => {
      expect(quantize(0)).toBe(0);
    });
  });

  describe('createClocks', () => {
    it('returns period 1, game 720.0 (12:00), shot 24.0', () => {
      const c = createClocks();
      expect(c).toEqual({ period: 1, game: 720.0, shot: 24.0 });
    });
  });

  describe('clampDelta', () => {
    it('returns delta when delta <= shot and delta <= game (no clamp)', () => {
      const clocks: Clocks = { period: 1, game: 10.0, shot: 24.0 };
      expect(clampDelta(5.0, clocks)).toBe(5.0);
    });

    it('clamps to game_clock_remaining when game < delta', () => {
      const clocks: Clocks = { period: 1, game: 10.0, shot: 24.0 };
      expect(clampDelta(30.0, clocks)).toBe(10.0);
    });

    it('clamps to shot_clock_remaining when shot < delta', () => {
      const clocks: Clocks = { period: 1, game: 100.0, shot: 3.0 };
      expect(clampDelta(30.0, clocks)).toBe(3.0);
    });

    it('returns 0 when both clocks are at 0', () => {
      const clocks: Clocks = { period: 4, game: 0.0, shot: 0.0 };
      expect(clampDelta(5.0, clocks)).toBe(0);
    });

    it('is a pure function — does not mutate the input clocks', () => {
      const clocks: Clocks = { period: 1, game: 10.0, shot: 24.0 };
      void clampDelta(30.0, clocks);
      expect(clocks).toEqual({ period: 1, game: 10.0, shot: 24.0 });
    });
  });

  describe('tick', () => {
    it('decrements game and shot by delta, preserves period', () => {
      const initial: Clocks = { period: 1, game: 720.0, shot: 24.0 };
      const next = tick(initial, 5.0);
      expect(next).toEqual({ period: 1, game: 715.0, shot: 19.0 });
    });

    it('returns a NEW object — input identity is unchanged', () => {
      const initial: Clocks = { period: 1, game: 720.0, shot: 24.0 };
      const next = tick(initial, 5.0);
      expect(Object.is(next, initial)).toBe(false);
    });

    it('does not mutate the input clocks', () => {
      const initial: Clocks = { period: 1, game: 720.0, shot: 24.0 };
      void tick(initial, 5.0);
      expect(initial).toEqual({ period: 1, game: 720.0, shot: 24.0 });
    });

    it('is chainable — two sequential ticks are equivalent to one bigger tick', () => {
      const c0: Clocks = { period: 2, game: 600.0, shot: 18.0 };
      const oneShot = tick(c0, 7.3);
      const twoShot = tick(tick(c0, 4.1), 3.2);
      expect(oneShot.game).toBeCloseTo(twoShot.game, 10);
      expect(oneShot.shot).toBeCloseTo(twoShot.shot, 10);
    });

    it('preserves period across tick (period advance is owned by the FSM driver, not tick)', () => {
      const initial: Clocks = { period: 3, game: 100.0, shot: 10.0 };
      const next = tick(initial, 2.0);
      expect(next.period).toBe(3);
    });
  });

  describe('resetShotClock', () => {
    it('resets shot to 24 and preserves game and period', () => {
      const initial: Clocks = { period: 1, game: 500.0, shot: 3.2 };
      const next = resetShotClock(initial, 24);
      expect(next).toEqual({ period: 1, game: 500.0, shot: 24 });
    });

    it('resets shot to 14 and preserves game and period', () => {
      const initial: Clocks = { period: 2, game: 400.0, shot: 20.0 };
      const next = resetShotClock(initial, 14);
      expect(next).toEqual({ period: 2, game: 400.0, shot: 14.0 });
    });

    it('returns a NEW object — input identity is unchanged', () => {
      const initial: Clocks = { period: 1, game: 500.0, shot: 3.2 };
      const next = resetShotClock(initial, 14);
      expect(Object.is(next, initial)).toBe(false);
    });

    it('does not mutate the input clocks', () => {
      const initial: Clocks = { period: 1, game: 500.0, shot: 3.2 };
      void resetShotClock(initial, 14);
      expect(initial).toEqual({ period: 1, game: 500.0, shot: 3.2 });
    });
  });
});
