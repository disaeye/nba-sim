/**
 * Pins the §1.6-1.8 observation model: σ formulas, sample cap, half-life
 * decay, L5 presentation halving, and the deterministic normal draw.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/mulberry32.js';
import { sigmaFor, presentationSigma, effectiveSamples, decayedSamples, normal, observe } from '../../src/playerdata/observation.js';
import { OBSERVATION } from '../../src/playerdata/tables.js';

describe('sigmaFor (§1.6/1.8)', () => {
  it('computes σ = base × k_scout × √(200/n)', () => {
    // L3, n=200 → attribute σ = 12
    expect(sigmaFor('attribute', 200, 3)).toBeCloseTo(12, 9);
    // L1, n=200 → 12 × 1.5 = 18
    expect(sigmaFor('attribute', 200, 1)).toBeCloseTo(18, 9);
    // L1, n=800 → 18 × 0.5 = 9
    expect(sigmaFor('attribute', 800, 1)).toBeCloseTo(9, 9);
  });

  it('uses the documented k_scout ladder', () => {
    expect(OBSERVATION.kScout[1]).toBe(1.5);
    expect(OBSERVATION.kScout[2]).toBe(1.2);
    expect(OBSERVATION.kScout[3]).toBe(1.0);
    expect(OBSERVATION.kScout[4]).toBe(0.8);
    expect(OBSERVATION.kScout[5]).toBe(0.6);
  });

  it('tendency is harder to observe than awareness than attributes (15 > 10 > 12)', () => {
    expect(OBSERVATION.sigmaBase.tendency).toBe(15);
    expect(OBSERVATION.sigmaBase.awareness).toBe(10);
    expect(OBSERVATION.sigmaBase.attribute).toBe(12);
    expect(sigmaFor('tendency', 200, 3)).toBeCloseTo(15, 9);
    expect(sigmaFor('awareness', 200, 3)).toBeCloseTo(10, 9);
  });

  it('caps the sample at 2000 — σ never reaches zero', () => {
    expect(effectiveSamples(5000)).toBe(2000);
    expect(sigmaFor('attribute', 100000, 1)).toBeCloseTo(sigmaFor('attribute', 2000, 1), 12);
    expect(sigmaFor('attribute', 2000, 1)).toBeGreaterThan(0);
    expect(sigmaFor('attribute', 2000, 1)).toBeCloseTo(18 * Math.sqrt(0.1), 9);
  });
});

describe('decayedSamples (§1.6 half-life 500)', () => {
  it('halves every 500 rounds', () => {
    expect(decayedSamples(2000, 500)).toBeCloseTo(1000, 9);
    expect(decayedSamples(2000, 1000)).toBeCloseTo(500, 9);
    expect(decayedSamples(800, 0)).toBeCloseTo(800, 9);
  });
});

describe('presentationSigma (§1.7 L5 unlock)', () => {
  it('L5 presents with σ halved (单档呈现)', () => {
    const base = sigmaFor('attribute', 500, 5);
    expect(presentationSigma('attribute', 500, 5)).toBeCloseTo(base / 2, 9);
    expect(presentationSigma('attribute', 500, 4)).toBeCloseTo(sigmaFor('attribute', 500, 4), 9);
  });
});

describe('normal draw (deterministic Box-Muller)', () => {
  it('is deterministic for the same rng stream and consumes exactly 2 draws', () => {
    const a = mulberry32(99);
    const b = mulberry32(99);
    expect(normal(a, 5, 2)).toBe(normal(b, 5, 2));
  });

  it('differs across seeds', () => {
    const a = mulberry32(1);
    const b = mulberry32(2);
    expect(normal(a)).not.toBe(normal(b));
  });

  it('lands near the mean over many draws', () => {
    const rng = mulberry32(1234);
    let sum = 0;
    const N = 2000;
    for (let i = 0; i < N; i++) sum += normal(rng, 10, 3);
    const mean = sum / N;
    expect(Math.abs(mean - 10)).toBeLessThan(0.3);
  });
});

describe('observe', () => {
  it('adds the sigma-scaled error to truth', () => {
    const rng = mulberry32(7);
    const truth = 80;
    const n = 2000; // σ ≈ 3.79 at L1
    const sigma = sigmaFor('attribute', n, 1);
    const observed = observe('attribute', truth, n, 1, rng);
    expect(Math.abs(observed - truth)).toBeLessThan(4 * sigma);
  });
});
