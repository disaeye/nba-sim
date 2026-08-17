/**
 * T3 — decision cadence contract.
 *
 * The legacy DECISION_WINDOW_TICKS cadence gate was removed when the decision
 * path became continuous (every tick re-plans from perception, gated only by
 * DWELL_MIN_TICKS). This test verifies the current contract: the dwell minimum
 * enforces a realistic catch-to-release floor.
 */
import { describe, it, expect } from 'vitest';
import { TICK_DT, DWELL_MIN_TICKS } from '../../src/sim-tick.js';

describe('T3 decision cadence', () => {
  it('DWELL_MIN_TICKS * TICK_DT is the catch-to-release floor (>= 1.5s)', () => {
    expect(DWELL_MIN_TICKS).toBeGreaterThan(0);
    const floor = DWELL_MIN_TICKS * TICK_DT;
    expect(floor).toBeGreaterThanOrEqual(1.5);
  });
});
