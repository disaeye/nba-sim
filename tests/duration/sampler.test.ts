import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/index.js';
import type { Rng } from '../../src/rng/types.js';
import { sampleDuration, loadDurationConfig } from '../../src/duration/index.js';
import { DurationNotFoundError } from '../../src/duration/types.js';

describe('duration', () => {
  describe('loadDurationConfig', () => {
    it('loads config/duration.json with all 15 catalog entries', () => {
      const cfg = loadDurationConfig();
      expect(cfg.durations).toHaveLength(15);
    });

    it('exposes the align_halfcourt entry with the configured range', () => {
      const cfg = loadDurationConfig();
      const entry = cfg.durations.find((d) => d.id === 'align_halfcourt');
      expect(entry).toBeDefined();
      // Read the configured range so rate/tuning shifts don't break the test.
      const min = entry?.min ?? 0;
      const max = entry?.max ?? 0;
      expect(max).toBeGreaterThan(min);
    });
  });

  describe('sampleDuration', () => {
    it('returns a value inside the configured range for align_halfcourt', () => {
      const rng: Rng = mulberry32(42);
      const cfg = loadDurationConfig();
      const v = sampleDuration('align_halfcourt', rng, cfg);
      const entry = cfg.durations.find((d) => d.id === 'align_halfcourt');
      const min = entry?.min ?? 0;
      const max = entry?.max ?? 0;
      expect(v).toBeGreaterThanOrEqual(min);
      expect(v).toBeLessThanOrEqual(max);
    });

    it('throws DurationNotFoundError for an unknown id', () => {
      const rng: Rng = mulberry32(1);
      const cfg = loadDurationConfig();
      try {
        sampleDuration('nonexistent', rng, cfg);
        throw new Error('expected sampleDuration to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(DurationNotFoundError);
        const err = e as DurationNotFoundError;
        expect(err.code).toBe('DURATION_NOT_FOUND');
        expect(err.id).toBe('nonexistent');
      }
    });

    it('throws DurationNotFoundError for the empty-string id', () => {
      const rng: Rng = mulberry32(1);
      const cfg = loadDurationConfig();
      expect(() => sampleDuration('', rng, cfg)).toThrow(DurationNotFoundError);
    });

    it('consumes exactly one rng.next() per call (single-stream draw accounting)', () => {
      const rngSample = mulberry32(1);
      const rngRaw = mulberry32(1);
      const cfg = loadDurationConfig();
      void sampleDuration('pass', rngSample, cfg); // consumes draw #1
      const afterSample = rngSample.next(); // draw #2
      void rngRaw.next(); // draw #1
      const afterRaw = rngRaw.next(); // draw #2
      expect(afterSample).toBe(afterRaw);
    });

    // Property test — 10000 samples of align_halfcourt (uniform[1.0, 2.5]).
    // Asserts three invariants:
    //   (a) every sample is inside the configured range [1.0, 2.5]
    //   (b) every sample is snapped to the 0.1 grid (re-uses `quantize`)
    //   (c) the sample mean is inside the configured range (trivially true
    //       for any in-range sample but pinned per the task spec)
    // Plus one tighter statistical check the spec leaves implicit:
    //   (d) the sample mean is within ±0.2 of the theoretical uniform mean
    //       1.75 — a meaningful correctness check that the formula is
    //       `min + next() * (max - min)` and not, say, biased toward min.
    it('10000 samples of align_halfcourt: all in range, all on 0.1 grid, mean in band', () => {
      const rng: Rng = mulberry32(7);
      const cfg = loadDurationConfig();
      const entry = cfg.durations.find((d) => d.id === 'align_halfcourt');
      const min = entry?.min ?? 0;
      const max = entry?.max ?? 0;
      const samples = Array.from({ length: 10000 }, () =>
        sampleDuration('align_halfcourt', rng, cfg),
      );

      // (a) range — read from the entry instead of hard-coded literals
      for (const v of samples) {
        expect(v).toBeGreaterThanOrEqual(min);
        expect(v).toBeLessThanOrEqual(max);
      }

      // (b) quantization to 0.1 — a value already on the grid is a fixed
      // point of quantize. Floating-point safety: use Math.abs instead of ===.
      for (const v of samples) {
        const tenths = v * 10;
        expect(Math.abs(tenths - Math.round(tenths))).toBeLessThan(1e-9);
      }

      // (c) + (d) mean — both the configured range (loose) and a tighter
      // ±0.2 around the theoretical uniform mean (min + max) / 2.
      const mean = samples.reduce((a, b) => a + b, 0) / samples.length;
      const theoretical = (min + max) / 2;
      expect(mean).toBeGreaterThanOrEqual(min);
      expect(mean).toBeLessThanOrEqual(max);
      expect(mean).toBeGreaterThan(theoretical - 0.2);
      expect(mean).toBeLessThan(theoretical + 0.2);
    });
  });
});
