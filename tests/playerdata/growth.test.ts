/**
 * Pins §3/§4 season systems: training formula, age efficiency, decline
 * curves, injuries, experience growth, role immersion, performance
 * feedback, and mismatch effects.
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/mulberry32.js';
import {
  ageEfficiency, trainingDelta, trainingMultiplier, trainingDeltaFor, annualDecline, bodyAnnualDecline, groupOf,
  injuryProbability, injuryEffects, rollInjury, experienceGrowth, applyExperienceGrowth, priorExperienceBonus,
  applyImmersion, roleImmersionEligible, mismatchReport, validateTendencies,
} from '../../src/playerdata/growth.js';
import { EXPERIENCE, INJURY, TRAINING, PERFORMANCE_FEEDBACK, IMMERSION } from '../../src/playerdata/tables.js';
import type { PlayerData, TendencySet } from '../../src/playerdata/types.js';

const T = (t: Partial<TendencySet>): TendencySet => ({
  T3: 50, TMID: 25, TDRIVE: 15, TPOST: 10, PASS1ST: 50, GAMBLE: 40, FOUL: 40, TAKEOVER: 40, PUSH: 40, ...t,
});

function player(age: number, ability = 55): PlayerData {
  const a = {} as Record<string, number>;
  for (const k of ['CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT', 'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS', 'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM', 'STL', 'BOX', 'ORB']) a[k] = ability;
  const pot = {} as Record<string, number>;
  for (const k of Object.keys(a)) pot[k] = 85;
  return {
    physical: { H: 200, WT: 105, WS: 210, VJ: 80, SPD: 70, LAT: 70, AGE: age, DUR: 80 },
    ability: a as PlayerData['ability'],
    pot: pot as PlayerData['pot'],
    tendency: T({}),
    awareness: { OFFR: 60, DEFR: 60, SPC: 60, PLY: 55 },
  };
}

describe('training (§3.1)', () => {
  it('Δa = 点数 × 0.14 × 年龄效率 × (1 − a/POT)', () => {
    // 20岁, a=55, POT=85, 100 points: 100×0.14×1.0×(1−55/85) = 4.941
    expect(trainingDelta(100, 55, 85, 20)).toBeCloseTo(100 * 0.14 * 1.0 * (1 - 55 / 85), 9);
  });

  it('age efficiency ladder', () => {
    expect(ageEfficiency(18)).toBe(1.0);
    expect(ageEfficiency(21)).toBe(1.0);
    expect(ageEfficiency(22)).toBe(0.8);
    expect(ageEfficiency(25)).toBe(0.8);
    expect(ageEfficiency(26)).toBe(0.5);
    expect(ageEfficiency(28)).toBe(0.5);
    expect(ageEfficiency(29)).toBe(0.25);
    expect(ageEfficiency(32)).toBe(0.25);
    expect(ageEfficiency(33)).toBe(0.1);
    expect(ageEfficiency(40)).toBe(0.1);
  });

  it('growth stops at the POT ceiling', () => {
    expect(trainingDelta(100, 85, 85, 20)).toBe(0);
  });

  it('natural growth equals the 30-point equivalent', () => {
    expect(TRAINING.naturalPoints).toBe(30);
    expect(trainingDelta(TRAINING.naturalPoints, 55, 85, 20)).toBeCloseTo(30 * 0.14 * (1 - 55 / 85), 9);
  });

  it('coach/highlight/bench/recovery/mismatch multipliers stack multiplicatively', () => {
    expect(trainingMultiplier({})).toBe(1);
    expect(trainingMultiplier({ coachLevel: 5 })).toBeCloseTo(1.1, 9);
    expect(trainingMultiplier({ highlight: true })).toBeCloseTo(1.2, 9);
    expect(trainingMultiplier({ bench: true })).toBeCloseTo(0.8, 9);
    expect(trainingMultiplier({ recovery: true })).toBeCloseTo(0.9, 9);
    expect(trainingMultiplier({ mismatch: true })).toBeCloseTo(0.9, 9);
    expect(trainingMultiplier({ highlight: true, coachLevel: 3 })).toBeCloseTo(1.2, 9);
  });

  it('awareness trains at ×0.3', () => {
    expect(TRAINING.awarenessEfficiency).toBe(0.3);
  });

  it('groupOf places every ability in the §3.1 table', () => {
    expect(groupOf('CS3')).toBe('SHOOTING');
    expect(groupOf('HND')).toBe('TECHNIQUE');
    expect(groupOf('DUNK')).toBe('FINISHING');
    expect(groupOf('RIM')).toBe('DEFENSE');
    expect(groupOf('OFFR')).toBe('AWARENESS');
  });
});

describe('age decline (§3.2)', () => {
  it('body: −2/yr at 29..32, −4/yr at 33+', () => {
    expect(annualDecline('BODY', 28)).toBe(0);
    expect(annualDecline('BODY', 29)).toBe(-2);
    expect(annualDecline('BODY', 32)).toBe(-2);
    expect(annualDecline('BODY', 33)).toBe(-4);
    expect(bodyAnnualDecline(40)).toBe(-4);
  });

  it('shooting −1/yr from 34; finishing −2/yr from 30; defense −1.5/yr from 32; awareness −1/yr from 36', () => {
    expect(annualDecline('SHOOTING', 33)).toBe(0);
    expect(annualDecline('SHOOTING', 34)).toBe(-1);
    expect(annualDecline('FINISHING', 30)).toBe(-2);
    expect(annualDecline('TECHNIQUE', 34)).toBe(-1);
    expect(annualDecline('DEFENSE', 32)).toBe(-1.5);
    expect(annualDecline('AWARENESS', 36)).toBe(-1);
  });
});

describe('injuries (§3.3)', () => {
  it('P = 2% × (80/DUR) × ageCoef × loadCoef', () => {
    expect(injuryProbability({ dur: 80, age: 25, avgMinutes: 30, fatigue: 0 })).toBeCloseTo(0.02, 12);
    expect(injuryProbability({ dur: 40, age: 25, avgMinutes: 30, fatigue: 0 })).toBeCloseTo(0.04, 12);
    expect(injuryProbability({ dur: 80, age: 32, avgMinutes: 30, fatigue: 0 })).toBeCloseTo(0.02 * 1.5, 12);
    expect(injuryProbability({ dur: 80, age: 25, avgMinutes: 40, fatigue: 0 })).toBeCloseTo(0.02 * 1.3, 12);
    expect(injuryProbability({ dur: 80, age: 25, avgMinutes: 30, fatigue: 30 })).toBeCloseTo(0.02 * 1.3, 12);
    expect(injuryProbability({ dur: 80, age: 25, avgMinutes: 30, fatigue: 0, lowStamina: true })).toBeCloseTo(0.02 * 1.5, 12);
  });

  it('severity split is 70/25/5', () => {
    expect(INJURY.grades[0]!.share).toBeCloseTo(0.7, 9);
    expect(INJURY.grades[1]!.share).toBeCloseTo(0.25, 9);
    expect(INJURY.grades[2]!.share).toBeCloseTo(0.05, 9);
  });

  it('MODERATE BACK: −5 body, 6..20 games out, VJ/FINC POT targets', () => {
    const rng = mulberry32(3);
    const inj = injuryEffects('BACK', 'MODERATE', rng);
    expect(inj.bodyPenalty).toBe(-5);
    expect(inj.potCut).toBe(0);
    expect(inj.gamesOut).toBeGreaterThanOrEqual(6);
    expect(inj.gamesOut).toBeLessThanOrEqual(20);
    expect(inj.groups).toEqual(['BODY', 'FINISHING']);
    expect(inj.potTargets).toEqual(['VJ', 'FINC']);
  });

  it('SEVERE: season-ending, permanent body −5..15 and POT −10..20', () => {
    const rng = mulberry32(4);
    const inj = injuryEffects('HAND_WRIST', 'SEVERE', rng);
    expect(inj.gamesOut).toBe(Infinity);
    expect(inj.bodyPenalty).toBeLessThanOrEqual(-5);
    expect(inj.bodyPenalty).toBeGreaterThanOrEqual(-15);
    expect(inj.potCut).toBeLessThanOrEqual(-10);
    expect(inj.potCut).toBeGreaterThanOrEqual(-20);
    expect(inj.potTargets).toEqual(['CS3', 'HND']);
  });

  it('rollInjury returns null when the draw exceeds P, and a graded injury otherwise', () => {
    const durable = { dur: 99, age: 18, avgMinutes: 10, fatigue: 0 };
    let injuries = 0;
    const rng = mulberry32(11);
    for (let i = 0; i < 200; i++) {
      const inj = rollInjury(durable, rng);
      if (inj !== null) {
        injuries += 1;
        expect(['MINOR', 'MODERATE', 'SEVERE']).toContain(inj.grade);
      }
    }
    // P = 0.02×(80/99)×1×1 ≈ 0.0162 → expect ~3 injuries in 200, allow noise.
    expect(injuries).toBeLessThan(12);
    expect(injuries).toBeGreaterThan(0);
  });

  it('fragile high-load veterans get hurt far more often', () => {
    const fragile = { dur: 30, age: 35, avgMinutes: 40, fatigue: 30 };
    const p = injuryProbability(fragile);
    expect(p).toBeGreaterThan(0.1);
  });
});

describe('experience (§4.1)', () => {
  it('Δ = min(rounds/1000, 2.0) × ageCoef', () => {
    expect(experienceGrowth(1500, 22)).toBeCloseTo(1.5, 9);
    expect(experienceGrowth(3000, 22)).toBeCloseTo(2.0, 9);
    expect(experienceGrowth(1000, 27)).toBeCloseTo(1.0 * 0.6, 9);
    expect(experienceGrowth(1000, 30)).toBeCloseTo(1.0 * 0.2, 9);
    expect(EXPERIENCE.cap).toBe(95);
  });

  it('caps awareness at 95', () => {
    const aw = applyExperienceGrowth({ OFFR: 94, DEFR: 90, SPC: 90, PLY: 90 }, 5000, 20);
    expect(aw.OFFR).toBe(95);
    expect(aw.DEFR).toBeCloseTo(92, 9);
  });

  it('priorExperienceBonus grows with age', () => {
    expect(priorExperienceBonus(20)).toBe(0);
    expect(priorExperienceBonus(24)).toBeGreaterThan(priorExperienceBonus(22));
  });
});

describe('immersion (§4.2)', () => {
  it('applies offsets with caps (and re-runs §1.4 validation)', () => {
    const base = T({ TAKEOVER: 75, PASS1ST: 60 });
    const out = applyImmersion(base, 'primary_creator');
    expect(out.TAKEOVER).toBe(79); // +4, cap 80
    expect(out.PASS1ST).toBe(62);  // +2, cap 70
    const capped = applyImmersion(T({ TAKEOVER: 78, PASS1ST: 69 }), 'primary_creator');
    expect(capped.TAKEOVER).toBe(80); // capped at 80
    // PASS1ST 69+2 → 71 → capped at 70, then the §1.4 clash rule
    // (TAKEOVER≥70 ∧ PASS1ST≥70) clamps it back to 69 — "又要球又不投"不存在.
    expect(capped.PASS1ST).toBe(69);
  });

  it('hub pushes PASS1ST up and TAKEOVER down (floored at 20)', () => {
    const out = applyImmersion(T({ TAKEOVER: 21, PASS1ST: 70 }), 'hub');
    expect(out.PASS1ST).toBe(74);
    expect(out.TAKEOVER).toBe(20);
  });

  it('switch_wing has no offsets', () => {
    expect(IMMERSION.switch_wing).toEqual([]);
    expect(applyImmersion(T({}), 'switch_wing')).toEqual(T({}));
  });

  it('re-validates tendencies after offsets', () => {
    // post_scorer: TPOST +4, T3 −4 — the four-shot sum stays 100.
    const out = applyImmersion(T({ TPOST: 10, T3: 50, TMID: 25, TDRIVE: 15 }), 'post_scorer');
    const sum = out.T3 + out.TMID + out.TDRIVE + out.TPOST;
    expect(sum).toBe(100);
  });

  it('eligibility: fit ≥65, role rounds ≥40%, ≥20 games', () => {
    const ok = { fit: 70, roleRounds: 400, teamRounds: 900, games: 60 };
    expect(roleImmersionEligible(ok)).toBe(true);
    expect(roleImmersionEligible({ ...ok, fit: 64 })).toBe(false);
    expect(roleImmersionEligible({ ...ok, roleRounds: 350 })).toBe(false);
    expect(roleImmersionEligible({ ...ok, games: 15 })).toBe(false);
  });
});

describe('mismatch (§4.4)', () => {
  it('Fit<50 for a season → friction + 体系不适; erosion after 2 seasons', () => {
    const r = mismatchReport(49, 1);
    expect(r.friction).toBe(true);
    expect(r.systemMisfit).toBe(true);
    expect(r.erosion.takeover).toBe(0);
    const r2 = mismatchReport(49, 2);
    expect(r2.erosion.takeover).toBe(-5);
    expect(r2.erosion.push).toBe(-5);
    expect(mismatchReport(60, 5).friction).toBe(false);
  });
});

describe('validateTendencies (§1.4)', () => {
  it('normalizes the four shot tendencies to exactly 100', () => {
    const out = validateTendencies(T({ T3: 90, TMID: 5, TDRIVE: 5, TPOST: 0 }));
    const sum = out.T3 + out.TMID + out.TDRIVE + out.TPOST;
    expect(sum).toBe(100);
  });

  it('all-zero shot tendencies fall back to equal quarters', () => {
    const out = validateTendencies(T({ T3: 0, TMID: 0, TDRIVE: 0, TPOST: 0 }));
    expect(out.T3).toBe(25);
    expect(out.TMID).toBe(25);
    expect(out.TDRIVE).toBe(25);
    expect(out.TPOST).toBe(25);
  });

  it('TAKEOVER≥70 ∧ PASS1ST≥70 → PASS1ST = 69', () => {
    const out = validateTendencies(T({ TAKEOVER: 75, PASS1ST: 80 }));
    expect(out.PASS1ST).toBe(69);
    const untouched = validateTendencies(T({ TAKEOVER: 69, PASS1ST: 80 }));
    expect(untouched.PASS1ST).toBe(80);
  });
});

describe('performance feedback (§4.3)', () => {
  it('documents the multipliers', () => {
    expect(PERFORMANCE_FEEDBACK.highlightGames).toBe(55);
    expect(PERFORMANCE_FEEDBACK.highlightTopPct).toBeCloseTo(0.2, 9);
    expect(PERFORMANCE_FEEDBACK.highlightTrain).toBeCloseTo(1.2, 9);
    expect(PERFORMANCE_FEEDBACK.benchGames).toBe(20);
    expect(PERFORMANCE_FEEDBACK.benchTrain).toBeCloseTo(0.8, 9);
    expect(PERFORMANCE_FEEDBACK.recoveryTrain).toBeCloseTo(0.9, 9);
  });
});

describe('trainingDeltaFor integration', () => {
  it('applies the full context to a raw delta', () => {
    const p = player(20, 55);
    const raw = trainingDelta(100, 55, 85, 20);
    expect(trainingDeltaFor(p, 'CS3', 100, { coachLevel: 5 })).toBeCloseTo(raw * 1.1, 9);
    expect(trainingDeltaFor(p, 'CS3', 100, { highlight: true, coachLevel: 1 })).toBeCloseTo(raw * 1.2 * 0.9, 9);
  });
});
