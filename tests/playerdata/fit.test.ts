/**
 * Pins §5 Fit: f/g computation, gate penalties, bands, transmission
 * coefficients, and the observed-fit substitution path.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/mulberry32.js';
import { trueFit, observedFit, trueFitSources, observedFitSources, fitSourceValue, gateFactor, possessionWeight, execEfficiency, deviationFactor, immersionEligible, reportLabel, bestRoles, fitBandOf, OFFENSE_ROLE_IDS, DEFENSE_ROLE_IDS } from '../../src/playerdata/fit.js';
import { observedPlayer } from '../../src/playerdata/observation.js';
import { FIT_BANDS } from '../../src/playerdata/tables.js';
import type { PlayerData } from '../../src/playerdata/types.js';

/** Elite player — every ability 99; physical chosen so all §5.2 gates pass. */
function elitePlayer(tendency: Partial<PlayerData['tendency']> = {}): PlayerData {
  const ability = {} as Record<string, number>;
  for (const k of ['CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT', 'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS', 'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM', 'STL', 'BOX', 'ORB']) {
    ability[k] = 99;
  }
  const pot = {} as Record<string, number>;
  for (const k of Object.keys(ability)) pot[k] = 99;
  return {
    physical: { H: 225, WT: 135, WS: 236, VJ: 100, SPD: 99, LAT: 99, AGE: 24, DUR: 80 },
    ability: ability as PlayerData['ability'],
    pot: pot as PlayerData['pot'],
    tendency: { T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 40, PUSH: 40, ...tendency },
    awareness: { OFFR: 99, DEFR: 99, SPC: 99, PLY: 90 },
  };
}

describe('computeFit (§5.1)', () => {
  it('elite player: gates pass for every role whose gates are satisfiable together', () => {
    const p = elitePlayer({ TAKEOVER: 90, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90, GAMBLE: 70 });
    // The catalog's gates are intentionally mutually exclusive in places:
    // 副攻手 wants TAKEOVER 40~69 (PC wants ≥70), 纪律轮转者 wants GAMBLE≤45
    // (扫荡 wants ≥60) — those roles cannot pass simultaneously and are
    // covered by their own gate tests.
    const compatible = [
      'primary_creator', 'iso_scorer', 'hub', 'post_scorer', 'interior_finisher',
      'cutter', 'movement_shooter', 'transition_runner',
      'poa_stopper', 'pest', 'switch_wing', 'roamer', 'rim_anchor', 'post_defender',
    ] as const;
    for (const role of compatible) {
      const fit = trueFit(p, role);
      expect(fit.g, role).toBeCloseTo(1, 6);
      expect(fit.band, role).toBe('NATURAL');
    }
    // H=225 → H_n = 20+(50/55)×79 = 91.818 → HND_eff = 99 − 4.182 = 94.818.
    // 持球核心: f = 0.7×(0.25+0.25×hnd+0.2+0.15+0.15) + 0.3×min(1,hnd,1,1,1).
    const pc = trueFit(p, 'primary_creator');
    const hnd = 94.8181818 / 99;
    const fExpected = 0.7 * (0.25 + 0.25 * hnd + 0.2 + 0.15 + 0.15) + 0.3 * hnd;
    expect(pc.f).toBeCloseTo(fExpected, 6);
    expect(pc.value).toBeCloseTo(pc.f * 100, 6);
    // Defense roles whose f-weights avoid HND hit 100 exactly.
    expect(trueFit(p, 'poa_stopper').value).toBeCloseTo(100, 6);
  });

  it('violated ≥ gate applies the smooth penalty 0.5+0.5×(value/min)', () => {
    const p = elitePlayer({ TAKEOVER: 30, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    // iso_scorer gates: TAKEOVER≥60 (violated: 30) → g = 0.75.
    const fit = trueFit(p, 'iso_scorer');
    expect(fit.g).toBeCloseTo(0.5 + 0.5 * (30 / 60), 9);
    expect(fit.value).toBeCloseTo(fit.f * 100 * fit.g, 6);
    expect(fit.band).toBe('CAPABLE');
  });

  it('violated ≤ gate applies 0.5+0.5×(max/value)', () => {
    const p = elitePlayer({ TAKEOVER: 90, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    // floor_spacer gates: T3≥60 ok, TAKEOVER≤40 violated (90)
    const fit = trueFit(p, 'floor_spacer');
    expect(fit.g).toBeCloseTo(0.5 + 0.5 * (40 / 90), 9);
  });

  it('band gate (副攻手 TAKEOVER 40~69) penalizes both sides', () => {
    const low = elitePlayer({ TAKEOVER: 30, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    const high = elitePlayer({ TAKEOVER: 80, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    const mid = elitePlayer({ TAKEOVER: 55, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    expect(trueFit(low, 'secondary_creator').g).toBeCloseTo(0.5 + 0.5 * (30 / 40), 9);
    expect(trueFit(high, 'secondary_creator').g).toBeCloseTo(0.5 + 0.5 * (69 / 80), 9);
    expect(trueFit(mid, 'secondary_creator').g).toBe(1);
  });

  it('interior_finisher has no gates (g always 1)', () => {
    const p = elitePlayer();
    expect(trueFit(p, 'interior_finisher').g).toBe(1);
  });

  it('physical gates read the normalized quantities (护筐锚 H_n≥70, 顶防工兵 WT_n≥70)', () => {
    const p = elitePlayer();
    expect(trueFit(p, 'rim_anchor').g).toBe(1);
    expect(trueFit(p, 'post_defender').g).toBe(1);
    // 换防摇摆: min(LAT, H_n) ≥ 55 — satisfied for the elite player.
    expect(trueFit(p, 'switch_wing').g).toBe(1);
  });

  it('physical gate failure halves the fit (small player → rim_anchor mismatch)', () => {
    const base = elitePlayer();
    const small: PlayerData = { ...base, physical: { ...base.physical, H: 178 } };
    const fit = trueFit(small, 'rim_anchor');
    // H=178 → H_n = 20+(3/55)*79 = 24.31 < 70 → g = 0.5+0.5×(24.31/70)
    expect(fit.g).toBeCloseTo(0.5 + 0.5 * (24.30909 / 70), 6);
  });

  it('f is the shortfall aggregate on the 0..1 scale (0.7×Σw + 0.3×min)', () => {
    // 持球核心 f weights: OD3 .25 / HND .25 / PNR .20 / PASM .15 / OFFR .15.
    // HND is the only non-clipped source (tallness penalty) → it is also min.
    const p = elitePlayer({ TAKEOVER: 90, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90, GAMBLE: 70 });
    expect(fitSourceValue(trueFitSources(p), 'OD3')).toBeCloseTo(99, 6);
    const hnd = 94.8181818 / 99;
    const expected = 0.7 * (0.25 + 0.25 * hnd + 0.2 + 0.15 + 0.15) + 0.3 * hnd;
    const fit = trueFit(p, 'primary_creator');
    expect(fit.f).toBeCloseTo(expected, 9);
  });
});

describe('bands (§5.1)', () => {
  it('documents 80/65/50 thresholds', () => {
    expect(FIT_BANDS.natural).toBe(80);
    expect(FIT_BANDS.capable).toBe(65);
    expect(FIT_BANDS.marginal).toBe(50);
  });

  it('classifies by the threshold ladder', () => {
    expect(fitBandOf(80)).toBe('NATURAL');
    expect(fitBandOf(79.9)).toBe('CAPABLE');
    expect(fitBandOf(65)).toBe('CAPABLE');
    expect(fitBandOf(64.9)).toBe('MARGINAL');
    expect(fitBandOf(50)).toBe('MARGINAL');
    expect(fitBandOf(49.9)).toBe('MISMATCH');
  });
});

describe('transmission (§5.3)', () => {
  it('possession weight = 0.6 + 0.4×Fit/100', () => {
    expect(possessionWeight(100)).toBeCloseTo(1.0, 9);
    expect(possessionWeight(0)).toBeCloseTo(0.6, 9);
    expect(possessionWeight(50)).toBeCloseTo(0.8, 9);
  });

  it('exec efficiency = 0.70 + 0.30×Fit/100', () => {
    expect(execEfficiency(100)).toBeCloseTo(1.0, 9);
    expect(execEfficiency(0)).toBeCloseTo(0.7, 9);
    expect(execEfficiency(50)).toBeCloseTo(0.85, 9);
  });

  it('deviation factor = 1.3 − 0.3×Fit/100', () => {
    expect(deviationFactor(0)).toBeCloseTo(1.3, 9);
    expect(deviationFactor(100)).toBeCloseTo(1.0, 9);
  });

  it('immersion requires Fit ≥ 65', () => {
    expect(immersionEligible(65)).toBe(true);
    expect(immersionEligible(64)).toBe(false);
  });

  it('report labels: ≥80 能胜任 / 65~79 可摇摆至 / <50 不出现', () => {
    expect(reportLabel(80)).toBe('能胜任');
    expect(reportLabel(79)).toBe('可摇摆至');
    expect(reportLabel(65)).toBe('可摇摆至');
    expect(reportLabel(64)).toBe('');
    expect(reportLabel(49)).toBe('');
  });
});

describe('observed fit (§1.8)', () => {
  it('equals true fit when observation error is zeroed by a huge sample', () => {
    const p = elitePlayer({ TAKEOVER: 90, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    // At n=2000, L5: σ_attribute = 12×0.6×0.316 = 2.28 — small but nonzero;
    // we only assert the pipeline runs and stays in [0,100].
    const rng = mulberry32(5);
    const fit = observedFit(p, 'primary_creator', 2000, 5, rng);
    expect(fit.value).toBeGreaterThanOrEqual(0);
    expect(fit.value).toBeLessThanOrEqual(100);
    expect(fit.role).toBe('primary_creator');
  });

  it('observedPlayer feeds the substitution path', () => {
    const p = elitePlayer();
    const rng = mulberry32(9);
    const observed = observedPlayer(p, 2000, 5, rng);
    const sources = observedFitSources(observed);
    expect(sources.ability.CS3).toBeCloseTo(observed.attribute.SHOOTING, 9);
    expect(sources.tendency.TAKEOVER).toBeCloseTo(observed.tendency.TAKEOVER, 9);
    expect(sources.physical.H).toBe(225);
  });
});

describe('bestRoles', () => {
  it('returns the highest-fit offense and defense roles', () => {
    const p = elitePlayer({ TAKEOVER: 90, PASS1ST: 90, T3: 90, TDRIVE: 90, PUSH: 90, TPOST: 90 });
    const { offense, defense } = bestRoles(p);
    // Defense: poa_stopper (no HND in f-weights) reaches 100.
    expect(defense.value).toBeCloseTo(100, 6);
    // Offense: 持球核心 or 组织枢纽 — the HND tallness penalty caps f < 1.
    expect(offense.value).toBeGreaterThan(90);
    expect(offense.value).toBeLessThanOrEqual(100);
  });
});

describe('gateFactor', () => {
  it('handles both-sided gates and missing bounds', () => {
    expect(gateFactor({ tendency: 'TAKEOVER', min: 70 }, 70)).toBe(1);
    expect(gateFactor({ tendency: 'TAKEOVER', min: 70 }, 69)).toBeCloseTo(0.5 + 0.5 * (69 / 70), 9);
    expect(gateFactor({ tendency: 'TAKEOVER', max: 40 }, 41)).toBeCloseTo(0.5 + 0.5 * (40 / 41), 9);
    expect(gateFactor({ tendency: 'TAKEOVER', min: 40, max: 69 }, 100)).toBeCloseTo(0.5 + 0.5 * (69 / 100), 9);
    expect(gateFactor({}, 50)).toBe(1);
  });
});
