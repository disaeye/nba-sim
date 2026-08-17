import { describe, it, expect } from 'vitest';

import { mulberry32, unit, int, pick, weighted } from '../../src/rng/index.js';

describe('mulberry32', () => {
  describe('determinism', () => {
    // Reference values were produced by running the canonical mulberry32
    // reference implementation with seed=1. Pinning the first 5 values locks
    // the algorithm byte-for-byte; the first-100-values property is covered
    // by the "two instances deep-equal" test below.
    it('produces these exact first 5 values for seed=1', () => {
      const expected = [
        0.6270739405881613,
        0.002735721180215478,
        0.5274470399599522,
        0.9810509674716741,
        0.9683778982143849,
      ];
      const rng = mulberry32(1);
      const actual = [unit(rng), unit(rng), unit(rng), unit(rng), unit(rng)];
      expect(actual).toEqual(expected);
    });

    it('produces the same first 100 values across two instances with the same seed', () => {
      const a = mulberry32(1);
      const b = mulberry32(1);
      const sa = Array.from({ length: 100 }, () => a.next());
      const sb = Array.from({ length: 100 }, () => b.next());
      expect(sa).toEqual(sb);
    });

    it('produces different sequences for different seeds', () => {
      const a = mulberry32(1);
      const b = mulberry32(2);
      const sa = Array.from({ length: 5 }, () => a.next());
      const sb = Array.from({ length: 5 }, () => b.next());
      expect(sa).not.toEqual(sb);
    });
  });

  describe('range [0, 1)', () => {
    it('never returns a value < 0 or >= 1 over 10000 draws from seed=1', () => {
      const rng = mulberry32(1);
      for (let i = 0; i < 10000; i++) {
        const v = rng.next();
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThan(1);
        expect(Number.isFinite(v)).toBe(true);
      }
    });
  });

  describe('unit', () => {
    it('returns the same value as rng.next() on an aligned stream', () => {
      const rngUnit = mulberry32(7);
      const rngRaw = mulberry32(7);
      expect(unit(rngUnit)).toBe(rngRaw.next());
    });

    it('consumes exactly one next() per call', () => {
      const rngUnit = mulberry32(1);
      const rngRaw = mulberry32(1);
      void unit(rngUnit); // consumes draw #1
      const afterUnit = rngUnit.next(); // draw #2
      void rngRaw.next(); // draw #1
      const afterRaw = rngRaw.next(); // draw #2
      expect(afterUnit).toBe(afterRaw);
    });
  });

  describe('int', () => {
    it('always returns an inclusive integer in [min, max] over 10000 draws', () => {
      const rng = mulberry32(1);
      const counts = new Map<number, number>();
      for (let i = 0; i < 10000; i++) {
        const v = int(rng, 1, 6);
        expect(Number.isInteger(v)).toBe(true);
        expect(v).toBeGreaterThanOrEqual(1);
        expect(v).toBeLessThanOrEqual(6);
        counts.set(v, (counts.get(v) ?? 0) + 1);
      }
      // 6 buckets, ~1667 expected per face. Allow [1000, 2300] (very loose
      // uniformity band — the determinism + range assertions are the strict
      // contract; distribution is a sanity check).
      for (let face = 1; face <= 6; face++) {
        const c = counts.get(face) ?? 0;
        expect(c).toBeGreaterThan(1000);
        expect(c).toBeLessThan(2300);
      }
    });

    it('returns min when max === min', () => {
      const rng = mulberry32(99);
      expect(int(rng, 5, 5)).toBe(5);
    });

    it('throws when min > max', () => {
      const rng = mulberry32(1);
      expect(() => int(rng, 5, 1)).toThrow();
    });

    it('throws when min or max is not an integer', () => {
      const rng = mulberry32(1);
      expect(() => int(rng, 1.5, 6)).toThrow();
      expect(() => int(rng, 1, 6.5)).toThrow();
    });

    it('consumes exactly one next() per call', () => {
      const rngInt = mulberry32(1);
      const rngRaw = mulberry32(1);
      void int(rngInt, 1, 100); // consumes draw #1
      const afterInt = rngInt.next(); // draw #2
      void rngRaw.next(); // draw #1
      const afterRaw = rngRaw.next(); // draw #2
      expect(afterInt).toBe(afterRaw);
    });
  });

  describe('pick', () => {
    it('returns an element of the array over 100 draws', () => {
      const rng = mulberry32(1);
      const arr = ['a', 'b', 'c'];
      for (let i = 0; i < 100; i++) {
        const v = pick(rng, arr);
        expect(arr).toContain(v);
      }
    });

    it('returns the only element of a singleton array', () => {
      const rng = mulberry32(1);
      expect(pick(rng, ['only'])).toBe('only');
    });

    it('throws on empty array', () => {
      const rng = mulberry32(1);
      expect(() => pick(rng, [])).toThrow();
    });

    it('consumes exactly one next() per call', () => {
      const rngPick = mulberry32(1);
      const rngRaw = mulberry32(1);
      void pick(rngPick, ['a', 'b', 'c', 'd']); // consumes draw #1
      const afterPick = rngPick.next(); // draw #2
      void rngRaw.next(); // draw #1
      const afterRaw = rngRaw.next(); // draw #2
      expect(afterPick).toBe(afterRaw);
    });
  });

  describe('weighted', () => {
    it('selects items with probability proportional to weights over 10000 draws', () => {
      const rng = mulberry32(1);
      const items = ['A', 'B', 'C'];
      const weights = [1, 3, 6]; // total 10 → 10%, 30%, 60%
      const counts = new Map<string, number>([
        ['A', 0],
        ['B', 0],
        ['C', 0],
      ]);
      for (let i = 0; i < 10000; i++) {
        const v = weighted(rng, items, weights);
        const prev = counts.get(v);
        if (prev === undefined) {
          throw new Error(`weighted returned unexpected value: ${v}`);
        }
        counts.set(v, prev + 1);
      }
      const n = 10000;
      // ±5% absolute tolerance. Loose because mulberry32 is well-mixed and
      // 10000 draws give small variance; the strict contract is the formula
      // (single cumulative-table lookup over one unit(rng) draw).
      expect(counts.get('A')! / n).toBeGreaterThan(0.05);
      expect(counts.get('A')! / n).toBeLessThan(0.15);
      expect(counts.get('B')! / n).toBeGreaterThan(0.25);
      expect(counts.get('B')! / n).toBeLessThan(0.35);
      expect(counts.get('C')! / n).toBeGreaterThan(0.55);
      expect(counts.get('C')! / n).toBeLessThan(0.65);
    });

    it('selects the only item when weights has length 1', () => {
      const rng = mulberry32(1);
      expect(weighted(rng, ['solo'], [5])).toBe('solo');
    });

    it('throws when items and weights have different lengths', () => {
      const rng = mulberry32(1);
      expect(() => weighted(rng, ['a', 'b'], [1])).toThrow();
      expect(() => weighted(rng, ['a'], [1, 2])).toThrow();
    });

    it('throws when all weights are zero', () => {
      const rng = mulberry32(1);
      expect(() => weighted(rng, ['a', 'b'], [0, 0])).toThrow();
    });

    it('throws on negative weight', () => {
      const rng = mulberry32(1);
      expect(() => weighted(rng, ['a', 'b'], [1, -1])).toThrow();
    });

    it('throws on NaN weight', () => {
      const rng = mulberry32(1);
      expect(() => weighted(rng, ['a', 'b'], [1, Number.NaN])).toThrow();
    });

    it('throws on empty items', () => {
      const rng = mulberry32(1);
      expect(() => weighted(rng, [], [])).toThrow();
    });

    it('consumes exactly one next() per call', () => {
      const rngW = mulberry32(1);
      const rngRaw = mulberry32(1);
      void weighted(rngW, ['x', 'y', 'z'], [1, 2, 3]); // consumes draw #1
      const afterW = rngW.next(); // draw #2
      void rngRaw.next(); // draw #1
      const afterRaw = rngRaw.next(); // draw #2
      expect(afterW).toBe(afterRaw);
    });
  });
});
