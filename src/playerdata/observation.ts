/**
 * 球探观测模型 (§1.6-1.8) — hidden quantities are only ever visible as
 * "observed value + error". The error shrinks with the sample size n:
 *
 *   σ = base × k_scout × √(200/n)        base: attribute 12 / tendency 15 / awareness 10
 *   n capped at 2000 (σ never reaches zero); old samples decay with
 *   half-life 500 rounds after big data changes.
 *
 * L5 scouts present with σ halved (§1.7 unlock: 单档呈现).
 */
import { OBSERVATION } from './tables.js';
import { allAttributes } from './aggregate.js';
import { clip } from './normalize.js';
import type { Rng } from '../rng/types.js';
import type {
  AttributeKey,
  ObservedKind,
  ObservedPlayer,
  PlayerData,
  ScoutLevel,
} from './types.js';

/** Effective sample count — capped at 2000. */
export function effectiveSamples(n: number): number {
  return Math.min(n, OBSERVATION.sampleCap);
}

/**
 * Sample decay (§1.6): after a big data change, old samples count as
 * `oldN × 0.5^(roundsSince/500)`. Combine with new samples via addition.
 */
export function decayedSamples(oldN: number, roundsSince: number): number {
  if (roundsSince <= 0) return oldN;
  return oldN * 0.5 ** (roundsSince / OBSERVATION.sampleHalfLife);
}

/** σ for one hidden quantity — never zero (n is capped). */
export function sigmaFor(kind: ObservedKind, n: number, scoutLevel: ScoutLevel): number {
  const base = OBSERVATION.sigmaBase[kind];
  const k = OBSERVATION.kScout[scoutLevel];
  return base * k * Math.sqrt(OBSERVATION.pivotRounds / effectiveSamples(Math.max(1, n)));
}

/**
 * Presentation σ — what the report displays. L5 halves it (单档呈现 unlock).
 */
export function presentationSigma(kind: ObservedKind, n: number, scoutLevel: ScoutLevel): number {
  const sigma = sigmaFor(kind, n, scoutLevel);
  if (OBSERVATION.l5SigmaHalving && scoutLevel === 5) return sigma / 2;
  return sigma;
}

/**
 * Standard normal draw via Box-Muller from the single deterministic rng
 * stream. Consumes exactly TWO `rng.next()` calls. Deterministic by
 * construction — no wall-clock or ambient-entropy sources (I5 audit).
 */
export function normal(rng: Rng, mean = 0, sd = 1): number {
  let u1 = rng.next();
  // ln(0) = -Inf; guard the tail.
  if (u1 < 1e-12) u1 = 1e-12;
  const u2 = rng.next();
  const z = Math.sqrt(-2 * Math.log(u1)) * Math.cos(2 * Math.PI * u2);
  return mean + sd * z;
}

/** One observed value: truth + ε, ε~N(0, σ²). */
export function observe(kind: ObservedKind, truth: number, n: number, scoutLevel: ScoutLevel, rng: Rng): number {
  const sigma = sigmaFor(kind, n, scoutLevel);
  return truth + normal(rng, 0, sigma);
}

/**
 * Observed view of a player — display-layer 属性, Tendency and Awareness
 * all carry measurement error; 体测 (physical) is public and exact.
 * Callers must NOT use the result as simulation truth.
 */
export function observedPlayer(data: PlayerData, n: number, scoutLevel: ScoutLevel, rng: Rng): ObservedPlayer {
  const attributes = allAttributes(data);
  const observedAttribute = {} as Record<AttributeKey, number>;
  for (const key of Object.keys(attributes) as AttributeKey[]) {
    observedAttribute[key] = observe('attribute', attributes[key], n, scoutLevel, rng);
  }
  const observedTendency = {} as Record<keyof ObservedPlayer['tendency'], number>;
  for (const key of Object.keys(data.tendency) as (keyof typeof data.tendency)[]) {
    observedTendency[key] = observe('tendency', data.tendency[key], n, scoutLevel, rng);
  }
  const observedAwareness = {} as Record<keyof ObservedPlayer['awareness'], number>;
  for (const key of Object.keys(data.awareness) as (keyof typeof data.awareness)[]) {
    observedAwareness[key] = observe('awareness', data.awareness[key], n, scoutLevel, rng);
  }
  return {
    physical: data.physical,
    attribute: observedAttribute,
    tendency: observedTendency,
    awareness: observedAwareness,
  };
}

/** Clamp a morale/stamina-style 0..100 gauge. */
export function clampGauge(value: number, min = 0, max = 100): number {
  return clip(value, min, max);
}
