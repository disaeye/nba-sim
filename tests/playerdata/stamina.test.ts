/**
 * Pins §8 stamina: STM_max, per-role consumption, the segment function,
 * fatigue accumulation/decay, back-to-back and heavy-minutes rules.
 */
import { describe, it, expect } from 'vitest';

import { staminaMax, consumptionPerMinute, staminaEffects, restGain, fatigueStep, requestsSub, postGameStamina, transitionConsumption, pressConsumption } from '../../src/playerdata/stamina.js';
import { STAMINA } from '../../src/playerdata/tables.js';

describe('staminaMax (§8.1)', () => {
  it('STM_max = max(50, 70 + 0.3×DUR − max(0, AGE−30)×2)', () => {
    expect(staminaMax(60, 25)).toBe(70 + 18);
    expect(staminaMax(60, 30)).toBe(70 + 18);
    expect(staminaMax(60, 35)).toBe(70 + 18 - 10);
    expect(staminaMax(20, 40)).toBe(70 + 6 - 20); // 56 > floor
    expect(staminaMax(20, 40)).toBeGreaterThanOrEqual(50);
  });
});

describe('consumption (§8.2)', () => {
  it('per-role rates match the table', () => {
    expect(consumptionPerMinute('primary_creator')).toBe(1.6);
    expect(consumptionPerMinute('iso_scorer')).toBe(1.6);
    expect(consumptionPerMinute('hub')).toBe(1.6);
    expect(consumptionPerMinute('secondary_creator')).toBe(1.3);
    expect(consumptionPerMinute('post_scorer')).toBe(1.3);
    expect(consumptionPerMinute('pest')).toBe(1.3);
    expect(consumptionPerMinute('poa_stopper')).toBe(1.3);
    expect(consumptionPerMinute('movement_shooter')).toBe(1.1);
    expect(consumptionPerMinute('cutter')).toBe(1.1);
    expect(consumptionPerMinute('transition_runner')).toBe(1.1);
    expect(consumptionPerMinute('roamer')).toBe(1.1);
    expect(consumptionPerMinute('floor_spacer')).toBe(0.9);
    expect(consumptionPerMinute('interior_finisher')).toBe(0.9);
    expect(consumptionPerMinute('rim_anchor')).toBe(0.9);
    expect(consumptionPerMinute('post_defender')).toBe(0.9);
    expect(consumptionPerMinute('rotator')).toBe(0.9);
    expect(consumptionPerMinute('switch_wing')).toBe(0.9);
  });

  it('situational multipliers: transition ×1.2, press ×1.5', () => {
    expect(transitionConsumption(1.0)).toBeCloseTo(1.2, 9);
    expect(pressConsumption(1.0)).toBeCloseTo(1.5, 9);
  });
});

describe('recovery (§8.3)', () => {
  it('bench rest +2.0/min, quarter +15, halftime +30', () => {
    expect(restGain(5)).toBe(10);
    expect(STAMINA.restPerMinute).toBe(2.0);
    expect(STAMINA.quarterRest).toBe(15);
    expect(STAMINA.halftimeRest).toBe(30);
  });

  it('post-game recovery to 100 − fatigue', () => {
    expect(postGameStamina(12)).toBe(88);
    expect(postGameStamina(150)).toBe(0);
  });
});

describe('staminaEffects (§8.4)', () => {
  it('60..100: no modification', () => {
    expect(staminaEffects(80)).toEqual({ exec: 1, awarenessPenalty: 0, injuryMult: 1, forcedSub: false });
    expect(staminaEffects(60)).toEqual({ exec: 1, awarenessPenalty: 0, injuryMult: 1, forcedSub: false });
  });

  it('40..59: exec ×(0.85 + 0.15×STM/60)', () => {
    const e = staminaEffects(50);
    expect(e.exec).toBeCloseTo(0.85 + 0.15 * (50 / 60), 9);
    expect(e.awarenessPenalty).toBe(0);
  });

  it('25..39: same exec + OFFR/DEFR −5', () => {
    const e = staminaEffects(30);
    expect(e.exec).toBeCloseTo(0.85 + 0.15 * (30 / 60), 9);
    expect(e.awarenessPenalty).toBe(-5);
  });

  it('<25: injury ×1.5 + forced-sub prompt', () => {
    const e = staminaEffects(20);
    expect(e.injuryMult).toBe(1.5);
    expect(e.forcedSub).toBe(true);
  });
});

describe('fatigue (§8.5)', () => {
  it('F\' = max(0, F − 8 + consumed − recovery); load 1.3 when F > 20', () => {
    const step = fatigueStep(10, { consumed: 40, restMinutes: 20 });
    expect(step.fatigue).toBe(2); // 10 − 8 + 40 − 40
    expect(step.loadCoef).toBe(1);

    const heavy = fatigueStep(30, { consumed: 40, restMinutes: 20 });
    expect(heavy.fatigue).toBe(22);
    expect(heavy.loadCoef).toBeCloseTo(1.3, 9);
  });

  it('fatigue never goes negative', () => {
    const step = fatigueStep(5, { consumed: 1, restMinutes: 30 });
    expect(step.fatigue).toBe(0);
  });

  it('back-to-back second game: recovery ×0.7', () => {
    const normal = fatigueStep(0, { consumed: 40, restMinutes: 20 });
    const b2b = fatigueStep(0, { consumed: 40, restMinutes: 20, backToBack: true });
    expect(b2b.fatigue).toBeGreaterThan(normal.fatigue);
  });

  it('heavy minutes (avg >36 for 10 games): accumulation ×1.3', () => {
    const normal = fatigueStep(0, { consumed: 40, restMinutes: 20 });
    const heavy = fatigueStep(0, { consumed: 40, restMinutes: 20, heavyMinutes: true });
    expect(heavy.fatigue).toBeCloseTo(0 + 40 * 1.3 - 40 - 8, 9);
    expect(heavy.fatigue).toBeGreaterThan(normal.fatigue);
  });
});

describe('rotation (§8.6)', () => {
  it('auto-sub request below 35', () => {
    expect(requestsSub(34)).toBe(true);
    expect(requestsSub(35)).toBe(false);
    expect(STAMINA.subBelow).toBe(35);
  });
});
