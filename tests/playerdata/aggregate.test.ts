/**
 * Pins the §1.2 normalization, §1.3 effective abilities (physical
 * corrections), §1.6 attribute aggregation and grade bands.
 */
import { describe, it, expect } from 'vitest';

import { normalizePhysical, clip } from '../../src/playerdata/normalize.js';
import { effectiveAbility, attributeValue, allAttributes, gradeOf } from '../../src/playerdata/aggregate.js';
import { ATTRIBUTE_WEIGHTS, ATTRIBUTE_WEAK_WEIGHT, WEAK_HAND_RULE } from '../../src/playerdata/tables.js';
import type { AttributeKey, PlayerData } from '../../src/playerdata/types.js';

function flatPlayer(ability: number, awareness = 60, physical?: Partial<PlayerData['physical']>): PlayerData {
  const base = {
    H: 200, WT: 105, WS: 210, VJ: 80, SPD: 70, LAT: 70, AGE: 24, DUR: 70,
    ...physical,
  };
  const abilitySet = {} as Record<string, number>;
  for (const k of ['CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT', 'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS', 'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM', 'STL', 'BOX', 'ORB']) {
    abilitySet[k] = ability;
  }
  const pot = {} as Record<string, number>;
  for (const k of Object.keys(abilitySet)) pot[k] = Math.max(ability + 10, 60);
  const tendency = { T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 40, PUSH: 40 };
  return {
    physical: base,
    ability: abilitySet as PlayerData['ability'],
    pot: pot as PlayerData['pot'],
    tendency,
    awareness: { OFFR: awareness, DEFR: awareness, SPC: awareness, PLY: awareness },
  };
}

describe('normalizePhysical (§1.2)', () => {
  it('maps the documented endpoints to 20 and 99', () => {
    expect(normalizePhysical({ H: 175, WT: 70, VJ: 50, WS: 175 * 0.98, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).H_n).toBeCloseTo(20, 6);
    expect(normalizePhysical({ H: 175, WT: 70, VJ: 50, WS: 175 * 0.98, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).WT_n).toBeCloseTo(20, 6);
    expect(normalizePhysical({ H: 175, WT: 70, VJ: 50, WS: 175 * 0.98, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).VJ_n).toBeCloseTo(20, 6);
    expect(normalizePhysical({ H: 175, WT: 70, VJ: 50, WS: 175 * 0.98, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).WS_n).toBeCloseTo(20, 6);
    expect(normalizePhysical({ H: 230, WT: 140, VJ: 110, WS: 230 * 1.12, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).H_n).toBeCloseTo(99, 6);
    expect(normalizePhysical({ H: 230, WT: 140, VJ: 110, WS: 230 * 1.12, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).WT_n).toBeCloseTo(99, 6);
    expect(normalizePhysical({ H: 230, WT: 140, VJ: 110, WS: 230 * 1.12, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).VJ_n).toBeCloseTo(99, 6);
    expect(normalizePhysical({ H: 230, WT: 140, VJ: 110, WS: 230 * 1.12, SPD: 60, LAT: 60, AGE: 24, DUR: 60 }).WS_n).toBeCloseTo(99, 6);
  });

  it('WS_n uses the WS/H ratio, not raw wingspan', () => {
    // H=200, WS=210 → ratio 1.05 → 20 + (0.07/0.14)*79 = 59.5
    const n = normalizePhysical({ H: 200, WT: 100, VJ: 80, WS: 210, SPD: 60, LAT: 60, AGE: 24, DUR: 60 });
    expect(n.WS_n).toBeCloseTo(20 + (0.07 / 0.14) * 79, 6);
  });
});

describe('effectiveAbility (§1.3 physical corrections)', () => {
  it('returns base when the ability has no physical correction', () => {
    const p = flatPlayer(80);
    expect(effectiveAbility(p, 'CS3')).toBe(80);
    expect(effectiveAbility(p, 'FT')).toBe(80);
  });

  it('POST adds +WT_n×0.15', () => {
    // WT=105 → WT_n = 20 + (35/70)*79 = 59.5 → 80 + 8.925 = 88.925
    const p = flatPlayer(80);
    expect(effectiveAbility(p, 'POST')).toBeCloseTo(80 + 59.5 * 0.15, 6);
  });

  it('HND subtracts (H_n−50)×0.1', () => {
    // H=200 → H_n = 20 + (25/55)*79 = 55.909 → 80 − 0.5909 = 79.409
    const p = flatPlayer(80);
    expect(effectiveAbility(p, 'HND')).toBeCloseTo(80 - (55.9090909 - 50) * 0.1, 5);
  });

  it('SWCH reads min(LAT, H_n)', () => {
    // LAT 70 < H_n 55.909? No: min = 55.909 → 80 + 0.1×55.909 = 85.591
    const p = flatPlayer(80);
    expect(effectiveAbility(p, 'SWCH')).toBeCloseTo(80 + 55.9090909 * 0.1, 5);
  });

  it('clips to [20, 99]', () => {
    const p = flatPlayer(99);
    expect(effectiveAbility(p, 'DUNK')).toBeLessThanOrEqual(99);
    const p2 = flatPlayer(20, 60, { WT: 70 });
    expect(effectiveAbility(p2, 'BOX')).toBeGreaterThanOrEqual(20);
  });
});

describe('attributeValue (§1.6 aggregation)', () => {
  it('every weight table sums to 1', () => {
    for (const key of Object.keys(ATTRIBUTE_WEIGHTS) as AttributeKey[]) {
      const sum = ATTRIBUTE_WEIGHTS[key].reduce((a, w) => a + w.weight, 0);
      expect(sum).toBeCloseTo(1, 9);
    }
  });

  it('SHOOTING is flat when all its sources are flat (no physical corrections)', () => {
    const p = flatPlayer(75);
    expect(attributeValue(p, 'SHOOTING')).toBeCloseTo(75, 9);
  });

  it('weakness term drags below the weighted mean (0.7/0.3 min)', () => {
    // SHOOTING: four 80s + one 40 → weighted = 0.35*80+0.25*80+0.2*80+0.1*80+0.1*40 = 76
    const p = flatPlayer(80);
    const weaker = { ...p.ability, FT: 40 } as PlayerData['ability'];
    const p2 = { ...p, ability: weaker };
    const weighted = 0.35 * 80 + 0.25 * 80 + 0.2 * 80 + 0.1 * 80 + 0.1 * 40;
    const expected = ATTRIBUTE_WEAK_WEIGHT.weighted * weighted + ATTRIBUTE_WEAK_WEIGHT.min * 40;
    expect(attributeValue(p2, 'SHOOTING')).toBeCloseTo(expected, 9);
  });

  it('mixed sources: INTERIOR_D blends RIM/BOX abilities with the DEFR awareness item', () => {
    // RIM_eff = 75 + H_n×0.15 + VJ_n×0.1 + WS_n×0.05 (H=200 → H_n 55.909, VJ=80 → VJ_n 59.5, WS=210 → WS_n 59.5)
    // BOX_eff = 75 + WT_n×0.2 (WT=105 → WT_n 59.5)
    const rimEff = 75 + (55.9090909 * 0.15) + (59.5 * 0.1) + (59.5 * 0.05);
    const boxEff = 75 + 59.5 * 0.2;
    const p = flatPlayer(75, 60);
    const weighted = 0.45 * rimEff + 0.3 * 60 + 0.25 * boxEff;
    const expected = 0.7 * weighted + 0.3 * Math.min(rimEff, 60, boxEff);
    expect(attributeValue(p, 'INTERIOR_D')).toBeCloseTo(expected, 9);
  });

  it('ATHLETICISM reads physical SPD/LAT/VJ_n with the min term', () => {
    const p = flatPlayer(75);
    const n = normalizePhysical(p.physical); // SPD 70, LAT 70, VJ_n = 59.5
    const weighted = 0.35 * 70 + 0.3 * 70 + 0.35 * n.VJ_n;
    const expected = 0.7 * weighted + 0.3 * Math.min(70, 70, n.VJ_n);
    expect(attributeValue(p, 'ATHLETICISM')).toBeCloseTo(expected, 9);
  });
});

describe('gradeOf (§1.6 bands)', () => {
  it('maps the documented band edges', () => {
    expect(gradeOf(90)).toBe('S');
    expect(gradeOf(89)).toBe('A');
    expect(gradeOf(85)).toBe('A');
    expect(gradeOf(84)).toBe('A-');
    expect(gradeOf(82)).toBe('A-');
    expect(gradeOf(81)).toBe('B+');
    expect(gradeOf(78)).toBe('B+');
    expect(gradeOf(77)).toBe('B');
    expect(gradeOf(74)).toBe('B');
    expect(gradeOf(73)).toBe('B-');
    expect(gradeOf(70)).toBe('B-');
    expect(gradeOf(69)).toBe('C+');
    expect(gradeOf(66)).toBe('C+');
    expect(gradeOf(65)).toBe('C');
    expect(gradeOf(62)).toBe('C');
    expect(gradeOf(61)).toBe('D');
    expect(gradeOf(55)).toBe('D');
    expect(gradeOf(54)).toBe('F');
  });
});

describe('weak-hand rule (§1.3)', () => {
  it('documents FINW < FINS−20 as the forced weak-side penalty', () => {
    // Data-side constant only — the −8% efficiency penalty is a runtime read.
    expect(WEAK_HAND_RULE.gap).toBe(20);
    expect(WEAK_HAND_RULE.penalty).toBeCloseTo(0.08, 9);
  });
});

describe('clip', () => {
  it('clamps into range', () => {
    expect(clip(5, 20, 99)).toBe(20);
    expect(clip(120, 20, 99)).toBe(99);
    expect(clip(50, 20, 99)).toBe(50);
  });
});
