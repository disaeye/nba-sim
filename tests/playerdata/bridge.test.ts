/**
 * Pins the PlayerData → LineupCapability bridge: weight tables sum to 1,
 * outputs stay in [0,1], and the mapping reacts to the layered data.
 */
import { describe, it, expect } from 'vitest';

import { bridgeCapabilities, BRIDGE_WEIGHTS } from '../../src/playerdata/bridge.js';
import { effectiveAbility } from '../../src/playerdata/aggregate.js';
import type { LineupCapability } from '../../src/identity/types.js';
import type { PlayerData } from '../../src/playerdata/types.js';

function player(hnd: number, scrn: number, spd: number, push: number): PlayerData {
  const ability = {} as Record<string, number>;
  for (const k of ['CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT', 'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS', 'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM', 'STL', 'BOX', 'ORB']) {
    ability[k] = 60;
  }
  ability.HND = hnd;
  ability.SCRN = scrn;
  const pot = {} as Record<string, number>;
  for (const k of Object.keys(ability)) pot[k] = 90;
  return {
    physical: { H: 200, WT: 105, WS: 210, VJ: 80, SPD: spd, LAT: 70, AGE: 24, DUR: 70 },
    ability: ability as PlayerData['ability'],
    pot: pot as PlayerData['pot'],
    tendency: { T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 40, PUSH: push },
    awareness: { OFFR: 60, DEFR: 60, SPC: 60, PLY: 55 },
  };
}

describe('BRIDGE_WEIGHTS', () => {
  it('every dimension row sums to 1', () => {
    for (const dim of Object.keys(BRIDGE_WEIGHTS) as (keyof LineupCapability)[]) {
      const sum = BRIDGE_WEIGHTS[dim].reduce((a, w) => a + w.weight, 0);
      expect(sum).toBeCloseTo(1, 9);
    }
  });

  it('covers every capability dimension', () => {
    const dims = ['creation', 'pullUp', 'catchShoot', 'rimFinishing', 'passing', 'screening', 'rolling', 'popping', 'postPlay', 'cutting', 'handleSecurity', 'transition', 'onBallDefense', 'helpDefense'];
    for (const d of dims) {
      expect(BRIDGE_WEIGHTS[d as keyof LineupCapability]).toBeDefined();
    }
  });
});

describe('bridgeCapabilities', () => {
  it('single-source dimensions equal the normalized effective ability', () => {
    const p = player(80, 70, 70, 40);
    const cap = bridgeCapabilities(p);
    expect(cap.screening).toBeCloseTo(effectiveAbility(p, 'SCRN') / 99, 9);
    expect(cap.handleSecurity).toBeCloseTo(effectiveAbility(p, 'HND') / 99, 9);
  });

  it('transition blends SPD and PUSH with effective FINS', () => {
    const p = player(60, 60, 90, 80);
    const cap = bridgeCapabilities(p);
    const finsEff = effectiveAbility(p, 'FINS') / 99;
    expect(cap.transition).toBeCloseTo(0.5 * (90 / 99) + 0.3 * (80 / 99) + 0.2 * finsEff, 9);
  });

  it('stays in [0, 1] for extreme inputs', () => {
    const elite = player(99, 99, 99, 95);
    const scrub = player(20, 20, 20, 5);
    for (const cap of [bridgeCapabilities(elite), bridgeCapabilities(scrub)]) {
      for (const v of Object.values(cap)) {
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThanOrEqual(1);
      }
    }
  });

  it('higher layered data ⇒ higher capability (weak monotonicity, strict where shared)', () => {
    const weak = bridgeCapabilities(player(40, 40, 40, 20));
    const strong = bridgeCapabilities(player(90, 90, 90, 80));
    for (const dim of Object.keys(BRIDGE_WEIGHTS) as (keyof LineupCapability)[]) {
      expect(strong[dim]).toBeGreaterThanOrEqual(weak[dim]);
    }
    // Dimensions fed by the boosted sources (HND/SCRN/SPD/PUSH) strictly rise.
    for (const dim of ['creation', 'screening', 'handleSecurity', 'transition', 'cutting'] as const) {
      expect(strong[dim]).toBeGreaterThan(weak[dim]);
    }
    // `passing` blends only fixed abilities (PASS/PASM/POCK) — unchanged.
    expect(strong.passing).toBe(weak.passing);
  });

  it('tendency PUSH only affects transition', () => {
    const a = bridgeCapabilities(player(60, 60, 70, 10));
    const b = bridgeCapabilities(player(60, 60, 70, 90));
    expect(b.transition).toBeGreaterThan(a.transition);
    expect(b.creation).toBe(a.creation);
  });
});
