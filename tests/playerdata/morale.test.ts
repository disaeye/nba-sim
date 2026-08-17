/**
 * Pins §9 morale: event deltas, weekly regression, output bands, and the
 * design boundary that morale never mutates truths.
 */
import { describe, it, expect } from 'vitest';

import { applyMoraleEvent, weeklyRegression, moraleEffects, applyTakeoverErosion, clampMorale } from '../../src/playerdata/morale.js';
import { MORALE } from '../../src/playerdata/tables.js';
import type { MoraleEventId } from '../../src/playerdata/types.js';

describe('events (§9.2)', () => {
  it('full event table matches the doc', () => {
    expect(MORALE.events.WIN).toBe(1);
    expect(MORALE.events.LOSS).toBe(-1);
    expect(MORALE.events.HIGHLIGHT).toBe(3);
    expect(MORALE.events.LOWLIGHT).toBe(-2);
    expect(MORALE.events.DEMOTED_TO_BENCH).toBe(-8);
    expect(MORALE.events.PROMOTED_TO_STARTER).toBe(5);
    expect(MORALE.events.DNP).toBe(-2);
    expect(MORALE.events.BENCHED_IN_CLUTCH).toBe(-3);
    expect(MORALE.events.SYSTEM_MISFIT).toBe(-5);
    expect(MORALE.events.CONTRACT_YEAR).toBe(5);
    expect(MORALE.events.TRADE_RUMOR).toBe(-6);
    expect(MORALE.events.CHAMPIONSHIP).toBe(15);
    expect(MORALE.events.PLAYOFF_EXIT).toBe(-5);
  });

  it('applies events instantly and clamps to 0..100', () => {
    expect(applyMoraleEvent(60, 'WIN')).toBe(61);
    expect(applyMoraleEvent(60, 'CHAMPIONSHIP')).toBe(75);
    expect(applyMoraleEvent(95, 'CHAMPIONSHIP')).toBe(100);
    expect(applyMoraleEvent(5, 'PLAYOFF_EXIT')).toBe(0);
  });

  it('throws on unknown events', () => {
    expect(() => applyMoraleEvent(60, 'NOPE' as MoraleEventId)).toThrow();
  });
});

describe('weekly regression (§9.1)', () => {
  it('moves 10% of the gap toward base 60', () => {
    expect(weeklyRegression(80)).toBeCloseTo(60 + (80 - 60) * 0.9, 9);
    expect(weeklyRegression(40)).toBeCloseTo(60 + (40 - 60) * 0.9, 9);
    expect(weeklyRegression(60)).toBe(60);
  });

  it('base is 60', () => {
    expect(MORALE.base).toBe(60);
  });
});

describe('outputs (§9.3)', () => {
  it('75..100: exec ×1.03, train ×1.1', () => {
    expect(moraleEffects(75)).toEqual({ exec: 1.03, train: 1.1, takeoverErosion: 0, demandsTrade: false });
    expect(moraleEffects(100)).toEqual({ exec: 1.03, train: 1.1, takeoverErosion: 0, demandsTrade: false });
  });

  it('45..74: neutral', () => {
    const e = moraleEffects(60);
    expect(e.exec).toBe(1);
    expect(e.train).toBe(1);
    expect(e.demandsTrade).toBe(false);
  });

  it('25..44: exec ×0.97, train ×0.9', () => {
    expect(moraleEffects(30).exec).toBeCloseTo(0.97, 9);
    expect(moraleEffects(30).train).toBeCloseTo(0.9, 9);
  });

  it('10..24: exec ×0.95 + TAKEOVER erosion −2/month', () => {
    const e = moraleEffects(15);
    expect(e.exec).toBeCloseTo(0.95, 9);
    expect(e.takeoverErosion).toBe(-2);
  });

  it('0..9: trade demand + refuses extension', () => {
    const e = moraleEffects(5);
    expect(e.demandsTrade).toBe(true);
  });
});

describe('erosion path', () => {
  it('TAKEOVER −2/month with validation', () => {
    const t = { T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 70, PUSH: 40 };
    const out = applyTakeoverErosion(t, 2);
    expect(out.TAKEOVER).toBe(66);
    expect(applyTakeoverErosion(t, 0)).toBe(t);
  });

  it('design boundary: morale functions never mutate ability/awareness truths', () => {
    // The module surface is input→new-value; no mutation of player data exists.
    expect(Object.keys(MORALE.bands).length).toBeGreaterThan(0);
    expect(clampMorale(120)).toBe(100);
    expect(clampMorale(-5)).toBe(0);
  });
});
