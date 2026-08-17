/**
 * Pins §7 generator: determinism, physical/ability/POT/tendency constraints,
 * prototype key bonuses, the veteran PLY bonus, and the draft-pool
 * ecosystem guarantees.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/mulberry32.js';
import { generatePlayer, generateRoster, generateDraftPool, pickPrototype, prototypeById } from '../../src/playerdata/generate.js';
import { PHYSICAL_GEN, POT, LEAGUE, SHOT_TENDENCIES, SHOT_TENDENCY_SUM, PROTOTYPES } from '../../src/playerdata/tables.js';
import type { PrototypeId } from '../../src/playerdata/types.js';

function player(seed: number, prototype?: PrototypeId, age?: number) {
  return generatePlayer({ rng: mulberry32(seed), prototype, age });
}

describe('determinism', () => {
  it('same seed + options → identical player', () => {
    expect(player(7, 'ball_dominant', 20)).toEqual(player(7, 'ball_dominant', 20));
  });

  it('different seeds → different players', () => {
    expect(player(7, 'ball_dominant', 20)).not.toEqual(player(8, 'ball_dominant', 20));
  });

  it('different prototypes → different players', () => {
    expect(player(7, 'pure_shooter', 20)).not.toEqual(player(7, 'post_big', 20));
  });
});

describe('physical generation (§7.4)', () => {
  it('stays inside the documented ranges', () => {
    for (let seed = 1; seed <= 50; seed++) {
      const p = player(seed);
      expect(p.physical.H).toBeGreaterThanOrEqual(175);
      expect(p.physical.H).toBeLessThanOrEqual(230);
      expect(p.physical.WT).toBeGreaterThanOrEqual(70);
      expect(p.physical.WT).toBeLessThanOrEqual(140);
      const ratio = p.physical.WS / p.physical.H;
      expect(ratio).toBeGreaterThanOrEqual(0.98);
      expect(ratio).toBeLessThanOrEqual(1.12);
      expect(p.physical.VJ).toBeGreaterThanOrEqual(50);
      expect(p.physical.VJ).toBeLessThanOrEqual(110);
      expect(p.physical.SPD).toBeGreaterThanOrEqual(20);
      expect(p.physical.SPD).toBeLessThanOrEqual(99);
      expect(p.physical.LAT).toBeGreaterThanOrEqual(20);
      expect(p.physical.LAT).toBeLessThanOrEqual(99);
      expect(p.physical.DUR).toBeGreaterThanOrEqual(20);
      expect(p.physical.DUR).toBeLessThanOrEqual(99);
    }
  });

  it('尖兵/大核 LAT ≥ 70 floor', () => {
    for (let seed = 1; seed <= 30; seed++) {
      expect(player(seed, 'defensive_stopper').physical.LAT).toBeGreaterThanOrEqual(70);
      expect(player(seed, 'ball_dominant').physical.LAT).toBeGreaterThanOrEqual(70);
    }
  });

  it('height follows the prototype class band', () => {
    for (let seed = 1; seed <= 20; seed++) {
      const guard = player(seed, 'playmaker');
      expect(guard.physical.H).toBeGreaterThanOrEqual(183);
      expect(guard.physical.H).toBeLessThanOrEqual(198);
      const big = player(seed, 'post_big');
      expect(big.physical.H).toBeGreaterThanOrEqual(205);
      expect(big.physical.H).toBeLessThanOrEqual(225);
    }
  });
});

describe('ability sampling (§7.2)', () => {
  it('key abilities cluster near their prototype means', () => {
    let sum = 0;
    const N = 60;
    for (let seed = 1; seed <= N; seed++) {
      sum += player(seed, 'pure_shooter').ability.CS3;
    }
    const mean = sum / N;
    expect(Math.abs(mean - 88)).toBeLessThan(2);
  });

  it('abilities stay in [20, 99]', () => {
    const p = player(42);
    for (const v of Object.values(p.ability)) {
      expect(v).toBeGreaterThanOrEqual(20);
      expect(v).toBeLessThanOrEqual(99);
    }
  });
});

describe('POT generation (§7.3)', () => {
  it('POT ≥ max(gen+2, 40) and ≤ 99, per ability', () => {
    const p = player(42, 'ball_dominant', 19);
    for (const key of Object.keys(p.ability) as (keyof typeof p.ability)[]) {
      expect(p.pot[key]).toBeGreaterThanOrEqual(Math.max(p.ability[key] + POT.minGap, POT.floor));
      expect(p.pot[key]).toBeLessThanOrEqual(POT.cap);
    }
  });

  it('young players have more upside than veterans', () => {
    const young = player(11, 'pure_shooter', 19);
    const old = player(11, 'pure_shooter', 30);
    for (const key of Object.keys(young.pot) as (keyof typeof young.pot)[]) {
      expect(young.pot[key]).toBeGreaterThanOrEqual(old.pot[key]);
    }
  });

  it('prototype key abilities get the +5 POT bonus', () => {
    // At age 32 (μ=2, σ=2) the ceiling does not clip. Non-key abilities
    // below 50 are excluded because the POT floor (max(gen+2, 40)) inflates
    // their gap and would swamp the +5 key bonus.
    let keyGap = 0;
    let otherGap = 0;
    let keyN = 0;
    let otherN = 0;
    const N = 100;
    const keySet = new Set(Object.keys(prototypeById('ball_dominant').keyAbilities));
    for (let seed = 100; seed < 100 + N; seed++) {
      const r = player(seed, 'ball_dominant', 32);
      for (const k of Object.keys(r.ability) as (keyof typeof r.ability)[]) {
        if (keySet.has(k)) {
          keyGap += r.pot[k] - r.ability[k];
          keyN += 1;
        } else if (r.ability[k] >= 50) {
          otherGap += r.pot[k] - r.ability[k];
          otherN += 1;
        }
      }
    }
    const keyAvg = keyGap / keyN;
    const otherAvg = otherGap / otherN;
    expect(keyAvg).toBeGreaterThan(otherAvg + 3);
  });
});

describe('tendency generation (§7.5)', () => {
  it('the four shot tendencies always sum to exactly 100', () => {
    for (let seed = 1; seed <= 100; seed++) {
      const p = player(seed);
      const sum = SHOT_TENDENCIES.reduce((acc, k) => acc + p.tendency[k], 0);
      expect(sum).toBe(SHOT_TENDENCY_SUM);
    }
  });

  it('TAKEOVER≥70 ∧ PASS1ST≥70 → PASS1ST clamped to 69', () => {
    // 持球大核 features TAKEOVER 80 / PASS1ST 55 — construct the clash case
    // by forcing both high via a customized tendency set is not possible
    // through the API, so verify the validator through the 持球大核 path:
    // PASS1ST feature 55 ± 10 can reach ≥70 while TAKEOVER stays ≥70.
    let sawClash = false;
    for (let seed = 1; seed <= 400 && !sawClash; seed++) {
      const p = player(seed, 'ball_dominant');
      if (p.tendency.TAKEOVER >= 70 && p.tendency.PASS1ST >= 70) {
        sawClash = true;
        expect(p.tendency.PASS1ST).toBe(69);
      }
    }
  });

  it('non-shot tendencies stay in [5, 95]', () => {
    const p = player(42);
    // The four shot tendencies are re-scaled by normalization and can leave
    // [5,95] — the §7.5 clip applies BEFORE §1.4 normalization.
    for (const key of ['PASS1ST', 'GAMBLE', 'FOUL', 'TAKEOVER', 'PUSH'] as const) {
      expect(p.tendency[key]).toBeGreaterThanOrEqual(5);
      expect(p.tendency[key]).toBeLessThanOrEqual(95);
    }
  });
});

describe('PLY (§7.4)', () => {
  it('veterans get a PLY bonus from prior experience (4.1)', () => {
    const young = player(21, 'playmaker', 22);
    const old = player(21, 'playmaker', 33);
    // The draw noise is shared seed, so the difference is the veteran bonus.
    expect(old.awareness.PLY).toBeGreaterThan(young.awareness.PLY);
  });

  it('PLY clips into the 20..99 band', () => {
    for (let seed = 1; seed <= 30; seed++) {
      const p = player(seed);
      expect(p.awareness.PLY).toBeGreaterThanOrEqual(20);
      expect(p.awareness.PLY).toBeLessThanOrEqual(99);
    }
  });
});

describe('pickPrototype (§7.2 shares)', () => {
  it('draws according to league share', () => {
    const rng = mulberry32(7);
    const counts = new Map<PrototypeId, number>();
    const N = 4000;
    for (let i = 0; i < N; i++) {
      const p = pickPrototype(rng);
      counts.set(p.id, (counts.get(p.id) ?? 0) + 1);
    }
    for (const proto of PROTOTYPES) {
      const observed = (counts.get(proto.id) ?? 0) / N;
      expect(Math.abs(observed - proto.share)).toBeLessThan(0.02);
    }
  });

  it('shares sum to 1', () => {
    const total = PROTOTYPES.reduce((a, p) => a + p.share, 0);
    expect(total).toBeCloseTo(1, 9);
  });
});

describe('generateDraftPool (§7.6)', () => {
  it('guarantees ≥1 持球大核 and ≥2 空间型内线 in every class', () => {
    for (let seed = 1; seed <= 20; seed++) {
      const pool = generateDraftPool(LEAGUE.draftPerYear, mulberry32(seed));
      expect(pool).toHaveLength(60);
      const ballDominant = pool.filter((p) => p.id === 'ball_dominant').length;
      const stretchBig = pool.filter((p) => p.id === 'stretch_big').length;
      expect(ballDominant).toBeGreaterThanOrEqual(1);
      expect(stretchBig).toBeGreaterThanOrEqual(2);
    }
  });
});

describe('generateRoster', () => {
  it('is deterministic and returns the requested count', () => {
    const a = generateRoster({ count: 10, rng: mulberry32(5), ageMin: 19, ageMax: 35 });
    const b = generateRoster({ count: 10, rng: mulberry32(5), ageMin: 19, ageMax: 35 });
    expect(a).toEqual(b);
    expect(a).toHaveLength(10);
  });

  it('age defaults to the draft band 18..22', () => {
    const roster = generateRoster({ count: 20, rng: mulberry32(9) });
    for (const p of roster) {
      expect(p.physical.AGE).toBeGreaterThanOrEqual(18);
      expect(p.physical.AGE).toBeLessThanOrEqual(22);
    }
  });
});

describe('generation constants', () => {
  it('documents the §7 tuning values', () => {
    expect(PHYSICAL_GEN.abilitySigma).toBe(5);
    expect(PHYSICAL_GEN.abilityBase).toBe(55);
    expect(PHYSICAL_GEN.abilityBaseSigma).toBe(10);
    expect(PHYSICAL_GEN.tendencySigma).toBe(10);
    expect(POT.keyBonus).toBe(5);
    expect(LEAGUE.draftPerYear).toBe(60);
    expect(LEAGUE.players).toBe(360);
  });
});
