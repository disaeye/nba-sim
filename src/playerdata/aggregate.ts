/**
 * 能力层 aggregation (§1.3, §1.6) — effective abilities (physical-corrected)
 * and the display-layer 属性 truth values.
 *
 *   D_true = 0.7 × Σ(wᵢ·aᵢ) + 0.3 × min(aᵢ)     (§1.6)
 *
 * Ability inputs are the effective values (base + §1.3 physical correction),
 * clipped to 20..99. Physical/awareness sources enter directly (SPD, LAT,
 * VJ_n, OFFR, DEFR, SPC).
 */
import { ABILITY_PHYSICAL_MODIFIERS, ATTRIBUTE_WEIGHTS, ATTRIBUTE_WEAK_WEIGHT } from './tables.js';
import { normalizePhysical } from './normalize.js';
import type {
  AbilityKey,
  AttributeKey,
  FitWeightSource,
  Grade,
  PlayerData,
} from './types.js';

const ABILITY_MIN = 20;
const ABILITY_MAX = 99;

/**
 * Effective ability value for one key: base value + §1.3 physical
 * corrections, clipped to [20, 99]. The base is the generation-time value;
 * corrections are read-time (they move as the body ages/declines).
 */
export function effectiveAbility(data: PlayerData, key: AbilityKey): number {
  const base = data.ability[key];
  const n = normalizePhysical(data.physical);
  const modifiers = ABILITY_PHYSICAL_MODIFIERS[key] ?? [];
  let value = base;
  for (const m of modifiers) {
    // 'LAT' is a raw 20..99 quantity, not a normalized one (POBD/NAVS/SWCH).
    const raw = m.source === 'LAT' ? data.physical.LAT : n[m.source];
    const source = m.pairedMin !== undefined ? Math.min(raw, n[m.pairedMin]) : raw;
    value += m.factor * (source - m.anchor);
  }
  return Math.min(ABILITY_MAX, Math.max(ABILITY_MIN, value));
}

/**
 * Resolve any Fit/attribute weight source to its raw 20..99-scale value:
 * ability → effective ability; awareness → value; physical → raw or
 * normalized physical quantity.
 */
export function sourceValue(data: PlayerData, source: FitWeightSource): number {
  switch (source) {
    case 'SPD':
    case 'LAT':
      return data.physical[source];
    case 'H_n':
    case 'WT_n':
    case 'VJ_n':
    case 'WS_n':
      return normalizePhysical(data.physical)[source];
    default:
      if (source in data.ability) return effectiveAbility(data, source as AbilityKey);
      if (source in data.awareness) return data.awareness[source as keyof typeof data.awareness];
      throw new Error(`sourceValue: unknown source ${String(source)}`);
  }
}

/** True attribute value (display layer, §1.6) — NOT an observation. */
export function attributeValue(data: PlayerData, key: AttributeKey): number {
  const weights = ATTRIBUTE_WEIGHTS[key];
  if (weights === undefined) {
    throw new Error(`attributeValue: no weight table for ${key}`);
  }
  const values = weights.map((w) => sourceValue(data, w.source));
  let weighted = 0;
  let min = Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i]!;
    weighted += weights[i]!.weight * v;
    if (v < min) min = v;
  }
  return ATTRIBUTE_WEAK_WEIGHT.weighted * weighted + ATTRIBUTE_WEAK_WEIGHT.min * min;
}

/** All seven 属性 truths. */
export function allAttributes(data: PlayerData): Readonly<Record<AttributeKey, number>> {
  return {
    SHOOTING: attributeValue(data, 'SHOOTING'),
    FINISHING: attributeValue(data, 'FINISHING'),
    PLAYMAKING: attributeValue(data, 'PLAYMAKING'),
    PERIMETER_D: attributeValue(data, 'PERIMETER_D'),
    INTERIOR_D: attributeValue(data, 'INTERIOR_D'),
    REBOUNDING: attributeValue(data, 'REBOUNDING'),
    ATHLETICISM: attributeValue(data, 'ATHLETICISM'),
  };
}

/** §1.6 grade bands: S=90+ … F<55. */
export function gradeOf(value: number): Grade {
  const clamped = Math.min(99.99, Math.max(0, value));
  if (clamped >= 90) return 'S';
  if (clamped >= 85) return 'A';
  if (clamped >= 82) return 'A-';
  if (clamped >= 78) return 'B+';
  if (clamped >= 74) return 'B';
  if (clamped >= 70) return 'B-';
  if (clamped >= 66) return 'C+';
  if (clamped >= 62) return 'C';
  if (clamped >= 55) return 'D';
  return 'F';
}

/** Normalize a 20..99-scale value to 0..1 for the §5.1 f formula. */
export function toUnit(value: number): number {
  return value / 99;
}
