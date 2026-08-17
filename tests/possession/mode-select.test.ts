/**
 * TDD tests for `modeSelect`. Written BEFORE implementation per the
 * Wave 1+ TDD discipline; the RED phase is the import failing because
 * `src/possession/mode-select.ts` does not exist yet. The GREEN phase
 * is the implementation making every assertion pass.
 *
 * Coverage map:
 *   - STEAL → TRANSITION (pinned, no RNG draw)
 *   - LIVE_BALL_TURNOVER_RECOVER / AFTER_TURNOVER → transition-biased
 *   - AFTER_DEFENSIVE_REBOUND → 15% TRANSITION, otherwise HALFCOURT
 *   - AFTER_MAKE → 2% TRANSITION, otherwise HALFCOURT
 *   - after_jump_ball / after_inbound / unknown → HALFCOURT
 *   - OREB continuation does not reselect a mode
 *
 * T21 tuning: modeSelect consumes one draw for ambiguous live starts.
 * Seeded boundary cases cover both sides of the calibrated thresholds.
 * The safe defaults and pinned STEAL route retain their no-draw guarantees.
 */
import { describe, it, expect } from 'vitest';

import { modeSelect, TRANSITION_FROM_DREB, TRANSITION_FROM_MAKE } from '../../src/possession/mode-select.js';
import { mulberry32 } from '../../src/rng/mulberry32.js';

describe('modeSelect', () => {
  describe('transition triggers (→ TRANSITION)', () => {
    it('returns TRANSITION for STEAL', () => {
      expect(modeSelect('STEAL', mulberry32(1))).toBe('TRANSITION');
    });

    it('returns a mode for LIVE_BALL_TURNOVER_RECOVER (probabilistic)', () => {
      const m = modeSelect('LIVE_BALL_TURNOVER_RECOVER', mulberry32(1));
      expect(m === 'TRANSITION' || m === 'HALFCOURT').toBe(true);
    });

    it('returns a mode for after_turnover (probabilistic)', () => {
      const m = modeSelect('after_turnover', mulberry32(1));
      expect(m === 'TRANSITION' || m === 'HALFCOURT').toBe(true);
    });
  });

  describe('halfcourt triggers (→ HALFCOURT)', () => {
    it('returns HALFCOURT for after_jump_ball', () => {
      expect(modeSelect('after_jump_ball', mulberry32(1))).toBe('HALFCOURT');
    });

    it('returns HALFCOURT for after_inbound', () => {
      expect(modeSelect('after_inbound', mulberry32(1))).toBe('HALFCOURT');
    });

    it('returns HALFCOURT for AFTER_OREB_CONTINUE (OREB continues prior mode; modeSelect is not called for OREB)', () => {
      // The OREB continuation rule means modeSelect is never called with
      // AFTER_OREB_CONTINUE in production; but if it were, HALFCOURT is
      // the safe default for an unrecognized reason.
      expect(modeSelect('AFTER_OREB_CONTINUE', mulberry32(1))).toBe('HALFCOURT');
    });

    it('returns HALFCOURT for unknown / unrecognized start reasons', () => {
      expect(modeSelect('whatever', mulberry32(1))).toBe('HALFCOURT');
      expect(modeSelect('UNKNOWN_REASON', mulberry32(1))).toBe('HALFCOURT');
      expect(modeSelect('', mulberry32(1))).toBe('HALFCOURT');
    });
  });

  describe('probabilistic triggers (T21 tuning)', () => {
    it('AFTER_DEFENSIVE_REBOUND (seed 111: high draw) → HALFCOURT', () => {
      expect(modeSelect('AFTER_DEFENSIVE_REBOUND', mulberry32(111))).toBe('HALFCOURT');
    });

    it('AFTER_DEFENSIVE_REBOUND (seed 7: low draw) → TRANSITION', () => {
      expect(modeSelect('AFTER_DEFENSIVE_REBOUND', mulberry32(7))).toBe('TRANSITION');
    });

    it('AFTER_MAKE (seed 18: high draw) → HALFCOURT', () => {
      expect(modeSelect('AFTER_MAKE', mulberry32(18))).toBe('HALFCOURT');
    });

    it('AFTER_MAKE (seed 7: low draw) → TRANSITION', () => {
      expect(modeSelect('AFTER_MAKE', mulberry32(7))).toBe('TRANSITION');
    });

    it('STEAL does NOT consume rng draws (draw-order guard)', () => {
      const rng1 = mulberry32(42);
      const rng2 = mulberry32(42);
      void modeSelect('STEAL', rng1);
      expect(rng1.next()).toBe(rng2.next());
    });

    it('default HALFCOURT triggers do NOT consume rng draws', () => {
      const rng1 = mulberry32(99);
      const rng2 = mulberry32(99);
      void modeSelect('AFTER_JUMP_BALL', rng1);
      void modeSelect('AFTER_INBOUND', rng1);
      void modeSelect('whatever', rng1);
      expect(rng1.next()).toBe(rng2.next());
    });

    it('AFTER_OREB_CONTINUE does NOT consume rng draws (default route)', () => {
      const rng1 = mulberry32(2024);
      const rng2 = mulberry32(2024);
      void modeSelect('AFTER_OREB_CONTINUE', rng1);
      expect(rng1.next()).toBe(rng2.next());
    });

    it('AFTER_DEFENSIVE_REBOUND consumes EXACTLY 1 rng.next()', () => {
      const rng1 = mulberry32(123);
      const rng2 = mulberry32(123);
      modeSelect('AFTER_DEFENSIVE_REBOUND', rng1);
      void rng2.next();
      expect(rng1.next()).toBe(rng2.next());
    });

    it('AFTER_MAKE consumes EXACTLY 1 rng.next()', () => {
      const rng1 = mulberry32(456);
      const rng2 = mulberry32(456);
      modeSelect('AFTER_MAKE', rng1);
      void rng2.next();
      expect(rng1.next()).toBe(rng2.next());
    });

    it('exposes the probability constants as exports (so other modules and tests can reference them)', () => {
      expect(TRANSITION_FROM_DREB).toBeGreaterThan(0);
      expect(TRANSITION_FROM_DREB).toBeLessThan(1);
      expect(TRANSITION_FROM_MAKE).toBeGreaterThan(0);
      expect(TRANSITION_FROM_MAKE).toBeLessThanOrEqual(TRANSITION_FROM_DREB);
    });
  });
});
