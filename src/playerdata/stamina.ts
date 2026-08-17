/**
 * 体力系统 (§8) — per-game stamina gauge plus cross-game fatigue.
 *
 *   STM_max = max(50, 70 + 0.3×DUR − max(0, AGE−30)×2)
 *
 * Consumption is per-minute on court, scaled by role; recovery on the bench,
 * between quarters, at halftime; fatigue accumulates across games and feeds
 * the injury load coefficient (§3.3 / §8.5).
 */
import { STAMINA } from './tables.js';
import type { RoleId, StaminaEffects } from './types.js';

/** §8.1. */
export function staminaMax(dur: number, age: number): number {
  return Math.max(STAMINA.max.floor, STAMINA.max.base + STAMINA.max.durFactor * dur - Math.max(0, age - STAMINA.max.ageFrom) * STAMINA.max.ageRate);
}

/** §8.2 per-minute consumption for one role. */
export function consumptionPerMinute(role: RoleId): number {
  const base = STAMINA.perMinute[role as keyof typeof STAMINA.perMinute];
  if (base === undefined) throw new Error(`consumptionPerMinute: unknown role ${role}`);
  return base;
}

/** §8.2 situational multipliers. */
export function transitionConsumption(stmPerMin: number): number {
  return stmPerMin * STAMINA.transitionMult;
}

export function pressConsumption(stmPerMin: number): number {
  return stmPerMin * STAMINA.pressMult;
}

/** §8.3 bench rest: +2.0 per minute. */
export function restGain(minutes: number): number {
  return minutes * STAMINA.restPerMinute;
}

/** §8.4 segment function. */
export function staminaEffects(stm: number): StaminaEffects {
  const v = Math.max(0, Math.min(100, stm));
  for (const seg of STAMINA.segments) {
    if (v >= seg.stmMin) {
      if ('exec' in seg) {
        return { exec: seg.exec, awarenessPenalty: 0, injuryMult: 1, forcedSub: false };
      }
      return {
        exec: seg.execBase! + seg.execSlope! * (v / 60),
        awarenessPenalty: seg.awarenessPenalty,
        injuryMult: seg.injuryMult,
        forcedSub: seg.forcedSub,
      };
    }
  }
  return { exec: 0.85, awarenessPenalty: -5, injuryMult: 1.5, forcedSub: true };
}

export interface FatigueInput {
  /** Total on-court consumption this game (role × minutes × multipliers). */
  readonly consumed: number;
  /** Bench rest minutes this game. */
  readonly restMinutes: number;
  /** Quarter breaks sat (0..3) + halftime (0 or 1). */
  readonly quarterRests?: number;
  readonly halftimes?: number;
  /** Back-to-back second game: post-game recovery ×0.7. */
  readonly backToBack?: boolean;
  /** avg >36 min for 10 games → accumulation ×1.3. */
  readonly heavyMinutes?: boolean;
}

export interface FatigueStep {
  readonly fatigue: number;
  /** 1.3 when F > 20 (§8.5) — feeds §3.3 injury formula. */
  readonly loadCoef: number;
}

/**
 * §8.5 — F' = max(0, F − 8(自然衰减) + 消耗 − 恢复盈余), with the
 * heavy-minutes accumulation multiplier applied to consumption.
 */
export function fatigueStep(f: number, input: FatigueInput): FatigueStep {
  let consumed = input.consumed;
  if (input.heavyMinutes) consumed *= STAMINA.fatigue.heavyMult;
  let recovery = restGain(input.restMinutes)
    + (input.quarterRests ?? 0) * STAMINA.quarterRest
    + (input.halftimes ?? 0) * STAMINA.halftimeRest;
  if (input.backToBack) recovery *= STAMINA.fatigue.backToBackRecovery;
  const fatigue = Math.max(0, f + STAMINA.fatigue.decayPerGame + consumed - recovery);
  return {
    fatigue,
    loadCoef: fatigue > STAMINA.fatigue.loadThreshold ? STAMINA.fatigue.loadCoef : 1,
  };
}

/** §8.6 default rotation request: STM < 35. */
export function requestsSub(stm: number): boolean {
  return stm < STAMINA.subBelow;
}

/** Post-game recovery: 100 − fatigue (§8.3). */
export function postGameStamina(fatigue: number): number {
  return Math.max(0, 100 - fatigue);
}
