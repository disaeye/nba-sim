/**
 * T21 — pace:check harness smoke (opt-in batch gate).
 *
 * The default `npm run test:fast` skips this; run `npm run test:slow` to
 * execute the multi-seed batch. The full 256-seed gate that actually
 * enforces the M8 bands runs via `npm run pace:check`.
 */
import { describe, it, expect } from 'vitest';
import { RUN_SLOW_TESTS } from '../_slow.js';
import { runPaceCheck, loadDemoConfig } from '../../scripts/pace-check.js';

describe.skipIf(!RUN_SLOW_TESTS)('pace-check harness — subset smoke (seeds 1..16)', () => {
  const cfg = loadDemoConfig('config/demo-game.json');
  const seeds = Array.from({ length: 16 }, (_, i) => i + 1);
  const report = runPaceCheck(seeds, cfg, '2026-01-01T00:00:00.000Z');

  it('runs without throwing and returns a well-formed report', () => {
    expect(report).toBeDefined();
    expect(report.games_simulated).toBe(16);
    expect(report.team_games).toBe(32);
    expect(report.total_possessions).toBeGreaterThan(0);
    expect(report.seeds).toEqual(seeds);
  });

  it('every metric has a numeric value, band, and boolean pass', () => {
    for (const key of Object.keys(report.metrics) as Array<keyof typeof report.metrics>) {
      const m = report.metrics[key];
      expect(Number.isFinite(m.value)).toBe(true);
      expect(m.band.min).toBeLessThan(m.band.max);
      expect(typeof m.pass).toBe('boolean');
    }
  });

  it('overall_pass is a boolean and equals the AND of all metric passes', () => {
    expect(typeof report.overall_pass).toBe('boolean');
    const allPass = (Object.keys(report.metrics) as Array<keyof typeof report.metrics>)
      .every((k) => report.metrics[k].pass);
    expect(report.overall_pass).toBe(allPass);
  });

  it('FG% lands in the wide sanity band even at N=16', () => {
    const fg = report.metrics.fg_pct;
    expect(fg.value).toBeGreaterThan(fg.band.min);
    expect(fg.value).toBeLessThan(fg.band.max);
  });

  it('OREB% lands in the wide sanity band even at N=16', () => {
    const oreb = report.metrics.oreb_pct;
    expect(oreb.value).toBeGreaterThan(oreb.band.min);
    expect(oreb.value).toBeLessThan(oreb.band.max);
  });

  it('computed pace is a positive, finite number (catastrophe guard)', () => {
    const pace = report.metrics.pace_per_team;
    expect(pace.value).toBeGreaterThan(50);
    expect(pace.value).toBeLessThan(250);
  });

  it('computed possession length is positive and finite (catastrophe guard)', () => {
    const len = report.metrics.mean_possession_length_seconds;
    expect(len.value).toBeGreaterThan(5);
    expect(len.value).toBeLessThan(40);
  });

  it('transition share is in [0, 1]', () => {
    const t = report.metrics.transition_share;
    expect(t.value).toBeGreaterThanOrEqual(0);
    expect(t.value).toBeLessThanOrEqual(1);
  });
});
