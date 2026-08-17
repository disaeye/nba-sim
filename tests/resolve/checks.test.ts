/**
 * T18 RED → GREEN tests for the outcome resolver.
 *
 * Each resolve check is exercised for three invariants:
 *   (a) Single-stream draw accounting — consumes exactly the documented
 *       number of `rng.next()` calls (1 for the binary checks; 2 for
 *       resolveDrive which has independent success + fouled draws).
 *       Pattern lifted from T12/T13: align two same-seed RNGs, burn the
 *       documented count through one and the same count raw through the
 *       other, then assert the next draw on each is equal.
 *   (b) Micro-statistical gate — 10000 draws land inside the foundation's
 *       tolerance band. The loose gate is `|rate − expected| < 0.05` per
 *       the task spec; the tighter gate re-uses the config.sanity_bands
 *       values where applicable.
 *   (c) Determinism — same seed produces identical result sequences.
 *
 * The homogeneous model (resolve.md §Homogeneous Model) is asserted by
 * absence: no resolve function takes a player id, so jersey swaps cannot
 * affect outcomes. The full jersey-swap property gate lives in T22.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/index.js';
import type { Rng } from '../../src/rng/types.js';
import {
  loadResolveConfig,
  makeResolveContext,
  resolveDrive,
  resolveFt,
  resolveHandoff,
  resolvePass,
  resolveRebound,
  resolveShot,
  resolveSteal,
} from '../../src/resolve/index.js';

const config = loadResolveConfig();
const ctx = makeResolveContext(config);

// ─── helpers ───────────────────────────────────────────────────────────────

/**
 * Assert that `fn` consumes exactly `expectedDraws` rng.next() calls.
 * Aligns two same-seed RNGs, burns `expectedDraws` through `fn` on one
 * and the same count of raw `next()` calls on the other, then verifies
 * the next draw on each is identical. If `fn` consumes a different count,
 * the streams diverge and the assertion fails.
 */
function assertDrawCount(
  fn: (rng: Rng) => unknown,
  expectedDraws: number,
): void {
  const rngFn = mulberry32(1);
  const rngRaw = mulberry32(1);
  void fn(rngFn);
  for (let i = 0; i < expectedDraws; i++) void rngRaw.next();
  expect(rngFn.next()).toBe(rngRaw.next());
}

/** Sample mean of `n` Bernoulli draws via `fn`. */
function sampleRate(fn: (rng: Rng) => boolean, rng: Rng, n: number): number {
  let hits = 0;
  for (let i = 0; i < n; i++) if (fn(rng)) hits++;
  return hits / n;
}

// ─── loader ────────────────────────────────────────────────────────────────

describe('loadResolveConfig', () => {
  // T21: rate assertions read from the loaded config instead of hardcoded
  // literals, so rate-tuning iterations don't require test rewrites. The
  // configured bands all stay inside [0, 1] and below 1 (no resolve pass
  // is deterministic at rate 1, which would be a kernel concern, not cfg).
  it('returns the configured base_rates from config/resolve.json', () => {
    expect(config.base_rates.pass_success).toBeGreaterThan(0);
    expect(config.base_rates.pass_success).toBeLessThan(1);
    expect(config.base_rates.handoff_success).toBeGreaterThan(0);
    expect(config.base_rates.handoff_success).toBeLessThan(1);
    expect(config.base_rates.drive_success).toBeGreaterThan(0);
    expect(config.base_rates.drive_success).toBeLessThan(1);
    expect(config.base_rates.shot_make_2pt).toBeGreaterThan(0);
    expect(config.base_rates.shot_make_2pt).toBeLessThan(1);
    expect(config.base_rates.shot_make_3pt).toBeGreaterThan(0);
    expect(config.base_rates.shot_make_3pt).toBeLessThan(1);
    expect(config.base_rates.ft_make).toBeGreaterThan(0);
    expect(config.base_rates.ft_make).toBeLessThan(1);
    expect(config.base_rates.steal_attempt_success).toBeGreaterThan(0);
    expect(config.base_rates.steal_attempt_success).toBeLessThan(1);
    expect(config.base_rates.foul_on_drive_rate).toBeGreaterThan(0);
    expect(config.base_rates.foul_on_drive_rate).toBeLessThan(1);
    expect(config.base_rates.offensive_rebound_rate).toBeGreaterThan(0);
    expect(config.base_rates.offensive_rebound_rate).toBeLessThan(1);
  });

  it('exposes the sanity_bands used by the pace regression harness', () => {
    expect(config.sanity_bands.fg_pct_min).toBe(0.25);
    expect(config.sanity_bands.fg_pct_max).toBe(0.65);
  });

  it('pins the v0.1.0 homogeneous model identifier', () => {
    expect(config.model).toBe('homogeneous_v0_1');
  });
});

// ─── resolvePass ───────────────────────────────────────────────────────────

describe('resolvePass', () => {
  it('consumes exactly one rng.next() per call', () => {
    assertDrawCount((rng) => resolvePass(rng, ctx), 1);
  });

  it('10000 draws: success rate ≈ config within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolvePass(rng, ctx).success,
      mulberry32(42),
      10000,
    );
    expect(Math.abs(rate - config.base_rates.pass_success)).toBeLessThan(0.05);
  });

  it('determinism: same seed → same sequence', () => {
    const a = Array.from({ length: 100 }, () => resolvePass(mulberry32(7), ctx).success);
    const b = Array.from({ length: 100 }, () => resolvePass(mulberry32(7), ctx).success);
    expect(a).toEqual(b);
  });
});

// ─── resolveHandoff ────────────────────────────────────────────────────────

describe('resolveHandoff', () => {
  it('consumes exactly one rng.next() per call', () => {
    assertDrawCount((rng) => resolveHandoff(rng, ctx), 1);
  });

  it('10000 draws: success rate ≈ config within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveHandoff(rng, ctx).success,
      mulberry32(11),
      10000,
    );
    expect(Math.abs(rate - config.base_rates.handoff_success)).toBeLessThan(0.05);
  });
});

// ─── resolveDrive ──────────────────────────────────────────────────────────

describe('resolveDrive', () => {
  it('consumes exactly TWO rng.next() per call (success + fouled independent)', () => {
    // resolve.md foul-on-drive note: "MAY consult an additional rng.next()"
    // pins the second draw for the foul check.
    assertDrawCount((rng) => resolveDrive(rng, ctx), 2);
  });

  it('10000 draws: success rate ≈ config within ±0.05', () => {
    const rng = mulberry32(3);
    let succ = 0;
    let foul = 0;
    for (let i = 0; i < 10000; i++) {
      const r = resolveDrive(rng, ctx);
      if (r.success) succ++;
      if (r.fouled) foul++;
    }
    expect(Math.abs(succ / 10000 - config.base_rates.drive_success)).toBeLessThan(0.05);
    expect(Math.abs(foul / 10000 - config.base_rates.foul_on_drive_rate)).toBeLessThan(0.05);
  });

  it('success and fouled are independent (a foul can occur on either outcome)', () => {
    // Run 5000 drives; assert we observe BOTH {success,foul} and
    // {failure,foul} combinations. Independence implies both quadrants
    // are populated; a conditional model would zero one of them.
    const rng = mulberry32(99);
    let successWithFoul = 0;
    let failureWithFoul = 0;
    for (let i = 0; i < 5000; i++) {
      const r = resolveDrive(rng, ctx);
      if (r.fouled) {
        if (r.success) successWithFoul++;
        else failureWithFoul++;
      }
    }
    expect(successWithFoul).toBeGreaterThan(50);
    expect(failureWithFoul).toBeGreaterThan(0);
  });
});

// ─── resolveShot ───────────────────────────────────────────────────────────

describe('resolveShot', () => {
  // resolveShot draws the block check (only when a defender is within
  // block range) then the make check. At tight distance both draw.
  it('consumes exactly two rng.next() per call (2pt, tight)', () => {
    assertDrawCount((rng) => resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', defenderDistFt: 1 }), 2);
  });

  it('consumes exactly two rng.next() per call (3pt, tight)', () => {
    assertDrawCount((rng) => resolveShot(rng, ctx, { shotValue: 3, zone: 'wing_L', defenderDistFt: 1 }), 2);
  });

  it('10000 draws of 2pt rim open: make rate ≈ shot-type base within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', shotType: 'drive_finish', defenderDistFt: 10 }).made,
      mulberry32(1),
      10000,
    );
    expect(Math.abs(rate - config.shot_type_rates.drive_finish_2pt)).toBeLessThan(0.05);
  });

  it('10000 draws of 3pt wing open: make rate ≈ shot-type base within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 3, zone: 'wing_L', shotType: 'catch_shoot', defenderDistFt: 10 }).made,
      mulberry32(2),
      10000,
    );
    expect(Math.abs(rate - config.shot_type_rates.catch_shoot_3pt)).toBeLessThan(0.05);
  });

  it('micro-stat gate: 10000 open rim shots seed=1 land inside config.sanity_bands.fg_pct_*', () => {
    const rng = mulberry32(1);
    let made = 0;
    for (let i = 0; i < 10000; i++) if (resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', shotType: 'drive_finish', defenderDistFt: 10 }).made) made++;
    const fgPct = made / 10000;
    expect(fgPct).toBeGreaterThanOrEqual(config.sanity_bands.fg_pct_min);
    expect(fgPct).toBeLessThanOrEqual(config.sanity_bands.fg_pct_max);
  });

  it('determinism: same seed → same make/miss sequence', () => {
    const a = Array.from({ length: 100 }, () => resolveShot(mulberry32(5), ctx, { shotValue: 3, zone: 'wing_L', defenderDistFt: 10 }).made);
    const b = Array.from({ length: 100 }, () => resolveShot(mulberry32(5), ctx, { shotValue: 3, zone: 'wing_L', defenderDistFt: 10 }).made);
    expect(a).toEqual(b);
  });

  it('P1.1 continuous contest: tight (1ft) < open (7ft) make rate', () => {
    const tight = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', defenderDistFt: 1 }).made,
      mulberry32(11), 5000,
    );
    const open = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', defenderDistFt: 7 }).made,
      mulberry32(12), 5000,
    );
    expect(open).toBeGreaterThan(tight + 0.03);
  });

  it('P1.3 blocks: tight drive_finish blocks more than open catch_shoot', () => {
    const tightDrive = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 2, zone: 'rim', shotType: 'drive_finish', defenderDistFt: 1 }).blocked,
      mulberry32(21), 20000,
    );
    const openCatch = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 3, zone: 'wing_L', shotType: 'catch_shoot', defenderDistFt: 7 }).blocked,
      mulberry32(22), 20000,
    );
    expect(tightDrive).toBeGreaterThan(openCatch + 0.005);
  });
});

// ─── resolveRebound ────────────────────────────────────────────────────────

describe('resolveRebound', () => {
  it('consumes exactly one rng.next() per call', () => {
    assertDrawCount((rng) => resolveRebound(rng, ctx), 1);
  });

  it('10000 draws: offensive rate ≈ config within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveRebound(rng, ctx).offensive,
      mulberry32(13),
      10000,
    );
    expect(Math.abs(rate - config.base_rates.offensive_rebound_rate)).toBeLessThan(0.05);
  });

  it('aggregate offensive rate falls inside config.sanity_bands.oreb_pct_*', () => {
    const rng = mulberry32(21);
    const rate = sampleRate((r) => resolveRebound(r, ctx).offensive, rng, 10000);
    expect(rate).toBeGreaterThanOrEqual(config.sanity_bands.oreb_pct_min);
    expect(rate).toBeLessThanOrEqual(config.sanity_bands.oreb_pct_max);
  });
});

// ─── resolveFt ─────────────────────────────────────────────────────────────

describe('resolveFt', () => {
  it('consumes exactly one rng.next() per call', () => {
    assertDrawCount((rng) => resolveFt(rng, ctx), 1);
  });

  it('10000 draws: make rate ≈ config within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveFt(rng, ctx).made,
      mulberry32(33),
      10000,
    );
    expect(Math.abs(rate - config.base_rates.ft_make)).toBeLessThan(0.05);
  });
});

// ─── resolveSteal ──────────────────────────────────────────────────────────

describe('resolveSteal', () => {
  it('consumes exactly one rng.next() per call', () => {
    assertDrawCount((rng) => resolveSteal(rng, ctx), 1);
  });

  it('10000 draws: success rate ≈ config within ±0.05', () => {
    const rate = sampleRate(
      (rng) => resolveSteal(rng, ctx).success,
      mulberry32(44),
      10000,
    );
    expect(Math.abs(rate - config.base_rates.steal_attempt_success)).toBeLessThan(0.05);
  });

  it('determinism: same seed → same sequence', () => {
    const a = Array.from({ length: 100 }, () => resolveSteal(mulberry32(8), ctx).success);
    const b = Array.from({ length: 100 }, () => resolveSteal(mulberry32(8), ctx).success);
    expect(a).toEqual(b);
  });
});

// ─── homogeneous model — jersey-swap invariance ───────────────────────────

describe('shot-type taxonomy (P1.2)', () => {
  it('catch_shoot prices above pull_up at the same zone/distance', () => {
    const catchRate = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 3, zone: 'wing_L', shotType: 'catch_shoot', defenderDistFt: 6 }).made,
      mulberry32(31), 20000,
    );
    const pullRate = sampleRate(
      (rng) => resolveShot(rng, ctx, { shotValue: 3, zone: 'wing_L', shotType: 'pull_up', defenderDistFt: 6 }).made,
      mulberry32(32), 20000,
    );
    expect(catchRate).toBeGreaterThan(pullRate + 0.01);
  });

  it('determinism holds with the input object form', () => {
    const a = Array.from({ length: 100 }, () => resolveShot(mulberry32(5), ctx, { shotValue: 3, zone: 'wing_L', defenderDistFt: 10 }).made);
    const b = Array.from({ length: 100 }, () => resolveShot(mulberry32(5), ctx, { shotValue: 3, zone: 'wing_L', defenderDistFt: 10 }).made);
    expect(a).toEqual(b);
  });
});
