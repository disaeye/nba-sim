/**
 * Pins §1.7 scout report: five blocks, unlock tiers (L1/L3/L5), style text
 * from observed tendencies, rare combos, confidence bars, and sample counts.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/mulberry32.js';
import { buildScoutReport, confidenceCells } from '../../src/playerdata/scout.js';
import type { PlayerData, TendencySet } from '../../src/playerdata/types.js';

function player(tendency: Partial<TendencySet>, abilityLevel = 88, cs3Override?: number): PlayerData {
  const ability = {} as Record<string, number>;
  for (const k of ['CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT', 'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS', 'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM', 'STL', 'BOX', 'ORB']) {
    ability[k] = k === 'CS3' && cs3Override !== undefined ? cs3Override : abilityLevel;
  }
  const pot = {} as Record<string, number>;
  for (const k of Object.keys(ability)) pot[k] = 90;
  return {
    physical: { H: 200, WT: 105, WS: 210, VJ: 80, SPD: 70, LAT: 70, AGE: 24, DUR: 70 },
    ability: ability as PlayerData['ability'],
    pot: pot as PlayerData['pot'],
    tendency: { T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 40, PUSH: 40, ...tendency },
    awareness: { OFFR: 60, DEFR: 60, SPC: 60, PLY: 55 },
  };
}

describe('report structure (§1.7)', () => {
  it('emits all five blocks with the sample line', () => {
    const r = buildScoutReport(player({}), { n: 300, scoutLevel: 3 }, mulberry32(1));
    expect(r.physical.H).toBe(200);
    expect(r.attributes).toHaveLength(7);
    expect(r.styles).toBeInstanceOf(Array);
    expect(r.roleFit).toBeInstanceOf(Array);
    expect(r.sampleRounds).toBe(300);
    expect(r.sigma).toBeGreaterThan(0);
  });

  it('attribute rows carry grade, confidence 1..5, and range when σ ≥ 8', () => {
    const r = buildScoutReport(player({}), { n: 200, scoutLevel: 1 }, mulberry32(2));
    for (const row of r.attributes) {
      expect(['S', 'A', 'A-', 'B+', 'B', 'B-', 'C+', 'C', 'D', 'F']).toContain(row.grade);
      expect(row.confidence).toBeGreaterThanOrEqual(1);
      expect(row.confidence).toBeLessThanOrEqual(5);
      expect(row.range).not.toBeNull(); // σ at n=200, L1 is 25.5 ≥ 8
    }
  });

  it('large samples collapse the range display (single grade)', () => {
    const r = buildScoutReport(player({}), { n: 2000, scoutLevel: 5 }, mulberry32(3));
    // L5 halved σ: 12×0.6×0.316/2 = 1.14 < 8 → no range rows.
    expect(r.sigma).toBeLessThan(8);
    for (const row of r.attributes) {
      expect(row.range).toBeNull();
    }
  });
});

describe('unlock tiers (§1.7)', () => {
  it('L1: physical + grades only — no styles, no role fit', () => {
    const r = buildScoutReport(player({ T3: 85 }), { n: 2000, scoutLevel: 1 }, mulberry32(4));
    expect(r.styles).toHaveLength(0);
    expect(r.roleFit).toHaveLength(0);
  });

  it('L3: style lines and role-fit section appear', () => {
    const r = buildScoutReport(player({ T3: 85, TAKEOVER: 15 }), { n: 2000, scoutLevel: 3 }, mulberry32(5));
    expect(r.styles.length).toBeGreaterThan(0);
    expect(r.roleFit.length).toBeGreaterThan(0);
  });

  it('styles read observed tendencies — high ≥70 and low ≤40 lines', () => {
    // T3 85 → "痴迷外线出手"; TDRIVE 10 → "很少突破" (σ small at n=2000).
    const r = buildScoutReport(player({ T3: 85, TDRIVE: 10 }), { n: 2000, scoutLevel: 3 }, mulberry32(6));
    expect(r.styles).toContain('痴迷外线出手');
    expect(r.styles).toContain('很少突破');
  });

  it('middle tendencies produce no line', () => {
    const r = buildScoutReport(player({ T3: 55, TDRIVE: 55 }), { n: 2000, scoutLevel: 3 }, mulberry32(7));
    expect(r.styles).not.toContain('痴迷外线出手');
    expect(r.styles).not.toContain('几乎不投三分');
  });
});

describe('rare combos (§1.7)', () => {
  it('TAKEOVER high + ability high → 投篮选择糟糕，但就是能进', () => {
    const r = buildScoutReport(player({ TAKEOVER: 85, T3: 85, TDRIVE: 80, TPOST: 20, PASS1ST: 20 }), { n: 2000, scoutLevel: 3 }, mulberry32(8));
    expect(r.rareStyles).toContain('投篮选择糟糕，但就是能进');
  });

  it('TAKEOVER high + low ability → 自信的普通球员', () => {
    // CS3 40 keeps the average attribute below 60.
    const r = buildScoutReport(player({ TAKEOVER: 85, T3: 85, TDRIVE: 80, TPOST: 20, PASS1ST: 20 }, 40), { n: 2000, scoutLevel: 3 }, mulberry32(9));
    expect(r.rareStyles).toContain('自信的普通球员');
  });
});

describe('role fit section (§5.3 labels)', () => {
  it('sorts by observed fit and only shows ≥65 labels', () => {
    const r = buildScoutReport(player({ TAKEOVER: 85, T3: 85, TDRIVE: 80, TPOST: 20, PASS1ST: 20 }), { n: 2000, scoutLevel: 3 }, mulberry32(10));
    expect(r.roleFit.length).toBeGreaterThan(0);
    for (let i = 1; i < r.roleFit.length; i++) {
      expect(r.roleFit[i - 1]!.observedFit).toBeGreaterThanOrEqual(r.roleFit[i]!.observedFit);
    }
    for (const row of r.roleFit) {
      expect(['能胜任', '可摇摆至']).toContain(row.label);
    }
  });
});

describe('confidenceCells', () => {
  it('shrinks as σ grows and stays within 1..5', () => {
    expect(confidenceCells(0)).toBe(5);
    expect(confidenceCells(16)).toBe(1);
    expect(confidenceCells(100)).toBe(1);
    expect(confidenceCells(8)).toBe(3);
  });
});
