/**
 * Fit 体系 (§5) — the link between player data and roles.
 *
 *   Fit(球员, 角色) = f × g × 100
 *   f = 0.7 × Σ(wᵢ·aᵢ) + 0.3 × min(aᵢ)      (aᵢ on 0..1, values ÷99)
 *   g = Π gᵢ,  gᵢ = 1 if the gate holds, else a smooth penalty:
 *        ≥-gate violated: 0.5 + 0.5×(value/min)
 *        ≤-gate violated: 0.5 + 0.5×(max/value)
 *
 * Bands: ≥80 天然契合 / 65~79 可胜任 / 50~64 勉强 / <50 错位.
 *
 * Two entry points share one core:
 * - `trueFit`  — simulation truth (abilities are effective values).
 * - `observedFit` — player-facing value (§1.8): display-layer 属性 substitute
 *   for abilities (each ability maps to its dominant attribute), observed
 *   Tendency/Awareness for gates; physical stays exact (public).
 */
import { allAttributes, attributeValue, effectiveAbility, sourceValue, toUnit } from './aggregate.js';
import { FIT_BANDS, FIT_CONFIGS, FIT_TRANSMISSION } from './tables.js';
import { normalizePhysical } from './normalize.js';
import { observedPlayer } from './observation.js';
import type { Rng } from '../rng/types.js';
import type {
  AbilityKey,
  AttributeKey,
  AwarenessKey,
  DefenseRoleId,
  FitConfig,
  FitGate,
  FitResult,
  FitWeightSource,
  ObservedPlayer,
  OffenseRoleId,
  PlayerData,
  RoleId,
  ScoutLevel,
} from './types.js';

/** Ability → dominant 属性 mapping for observed-fit substitution (§1.8, ⌛). */
export const ABILITY_TO_ATTRIBUTE: Readonly<Record<AbilityKey, AttributeKey>> = {
  CS3: 'SHOOTING', OD3: 'SHOOTING', CSM: 'SHOOTING', PUM: 'SHOOTING', FT: 'SHOOTING',
  FLT: 'FINISHING', FINS: 'FINISHING', FINW: 'FINISHING', FINC: 'FINISHING', DUNK: 'FINISHING', POST: 'FINISHING', SCRN: 'FINISHING',
  HND: 'PLAYMAKING', PNR: 'PLAYMAKING', PASS: 'PLAYMAKING', PASM: 'PLAYMAKING', POCK: 'PLAYMAKING',
  POBD: 'PERIMETER_D', NAVS: 'PERIMETER_D', SWCH: 'PERIMETER_D', STL: 'PERIMETER_D',
  RIM: 'INTERIOR_D', BOX: 'INTERIOR_D',
  ORB: 'REBOUNDING',
};

/** Raw per-source values fed into the Fit core. */
export interface FitSources {
  /** Effective ability values (true fit) or observed-attribute substitutes (observed fit). */
  readonly ability: Readonly<Record<AbilityKey, number>>;
  readonly awareness: Readonly<Record<AwarenessKey, number>>;
  readonly tendency: Readonly<Record<keyof PlayerData['tendency'], number>>;
  readonly physical: PlayerData['physical'];
}

/** Build the source record for true fit (effective abilities). */
export function trueFitSources(data: PlayerData): FitSources {
  const ability = {} as Record<AbilityKey, number>;
  for (const key of Object.keys(data.ability) as AbilityKey[]) {
    ability[key] = effectiveAbility(data, key);
  }
  return { ability, awareness: data.awareness, tendency: data.tendency, physical: data.physical };
}

/** Build the source record for observed fit (§1.8 substitution). */
export function observedFitSources(observed: ObservedPlayer): FitSources {
  const ability = {} as Record<AbilityKey, number>;
  for (const key of Object.keys(ABILITY_TO_ATTRIBUTE) as AbilityKey[]) {
    ability[key] = observed.attribute[ABILITY_TO_ATTRIBUTE[key]];
  }
  return {
    ability,
    awareness: observed.awareness,
    tendency: observed.tendency,
    physical: observed.physical,
  };
}

/** Resolve a §5.2 f-weight source against a FitSources record. */
export function fitSourceValue(sources: FitSources, source: FitWeightSource): number {
  switch (source) {
    case 'SPD':
    case 'LAT':
      return sources.physical[source];
    case 'H_n':
    case 'WT_n':
    case 'VJ_n':
    case 'WS_n':
      return normalizePhysical(sources.physical)[source];
    default:
      if (source in sources.ability) return sources.ability[source as AbilityKey];
      if (source in sources.awareness) return sources.awareness[source as AwarenessKey];
      throw new Error(`fitSourceValue: unknown source ${String(source)}`);
  }
}

/** The §5.1 ability-shortfall aggregate f, on 0..1 scale. */
export function fAggregate(sources: FitSources, config: FitConfig): number {
  const values = config.fWeights.map((w) => toUnit(fitSourceValue(sources, w.source)));
  let weighted = 0;
  let min = Infinity;
  for (let i = 0; i < values.length; i++) {
    const v = values[i]!;
    weighted += config.fWeights[i]!.weight * v;
    if (v < min) min = v;
  }
  return 0.7 * weighted + 0.3 * min;
}

/** Gate value for one FitGate: tendency or physical (with 换防摇摆 pairing). */
export function gateValue(sources: FitSources, gate: FitGate): number {
  if (gate.tendency !== undefined) return sources.tendency[gate.tendency];
  if (gate.physical !== undefined) {
    const base = fitSourceValue(sources, gate.physical);
    return gate.pairedWith !== undefined ? Math.min(base, fitSourceValue(sources, gate.pairedWith)) : base;
  }
  throw new Error('gateValue: gate with no tendency/physical source');
}

/** §5.1 smooth gate factor gᵢ. */
export function gateFactor(gate: FitGate, value: number): number {
  const min = gate.min;
  const max = gate.max;
  if (min !== undefined && max !== undefined) {
    // Band gate (副攻手 TAKEOVER 40~69): penalize toward the violated bound.
    if (value >= min && value <= max) return 1;
    if (value < min) return 0.5 + 0.5 * (value / min);
    return 0.5 + 0.5 * (max / value);
  }
  if (min !== undefined) {
    return value >= min ? 1 : 0.5 + 0.5 * (value / min);
  }
  if (max !== undefined) {
    return value <= max ? 1 : 0.5 + 0.5 * (max / value);
  }
  return 1;
}

/** Full Fit computation for one role. */
export function computeFit(sources: FitSources, role: RoleId): FitResult {
  const config = FIT_CONFIGS[role];
  if (config === undefined) throw new Error(`computeFit: unknown role ${role}`);
  const f = fAggregate(sources, config);
  let g = 1;
  for (const gate of config.gates) {
    g *= gateFactor(gate, gateValue(sources, gate));
  }
  const value = Math.min(100, Math.max(0, f * g * 100));
  return { role, value, f, g, band: fitBandOf(value) };
}

/** True Fit — simulation-internal. */
export function trueFit(data: PlayerData, role: RoleId): FitResult {
  return computeFit(trueFitSources(data), role);
}

/** Observed Fit — player-facing, with the same error source as the display layer. */
export function observedFit(
  data: PlayerData,
  role: RoleId,
  n: number,
  scoutLevel: ScoutLevel,
  rng: Rng,
): FitResult {
  const observed = observedPlayer(data, n, scoutLevel, rng);
  return computeFit(observedFitSources(observed), role);
}

/** Fit bands: ≥80 / 65~79 / 50~64 / <50. */
export function fitBandOf(fit: number): FitResult['band'] {
  if (fit >= FIT_BANDS.natural) return 'NATURAL';
  if (fit >= FIT_BANDS.capable) return 'CAPABLE';
  if (fit >= FIT_BANDS.marginal) return 'MARGINAL';
  return 'MISMATCH';
}

// ─── §5.3 transmission ──────────────────────────────────────────────────────

/** 实际权重 = 基础份额 × (0.6 + 0.4×Fit/100) — Fit 低者让球权. */
export function possessionWeight(fit: number): number {
  return FIT_TRANSMISSION.possession.base + FIT_TRANSMISSION.possession.slope * (fit / 100);
}

/** E = 0.70 + 0.30×Fit/100 — 角色相关动作的判定乘数. */
export function execEfficiency(fit: number): number {
  return FIT_TRANSMISSION.execEfficiency.base + FIT_TRANSMISSION.execEfficiency.slope * (fit / 100);
}

/** 偏离率 × (1.3 − 0.3×Fit/100) — 战术执行修正. */
export function deviationFactor(fit: number): number {
  return FIT_TRANSMISSION.deviationPenalty.base + FIT_TRANSMISSION.deviationPenalty.slope * (fit / 100);
}

/** Immersion requires Fit ≥65 (§4.2). */
export function immersionEligible(fit: number): boolean {
  return fit >= FIT_TRANSMISSION.immersionFit;
}

/** Scout-report label per observed Fit (§5.3). */
export function reportLabel(fit: number): string {
  if (fit >= FIT_TRANSMISSION.reportLabels.natural) return '能胜任';
  if (fit >= FIT_TRANSMISSION.reportLabels.capable) return '可摇摆至';
  return '';
}

/** Convenience: the highest-fit offense+defense roles for a player (player-side Fit vector). */
export function bestRoles(data: PlayerData): { offense: FitResult; defense: FitResult } {
  let bestO: FitResult | null = null;
  for (const role of OFFENSE_ROLE_IDS) {
    const fit = trueFit(data, role);
    if (bestO === null || fit.value > bestO.value) bestO = fit;
  }
  let bestD: FitResult | null = null;
  for (const role of DEFENSE_ROLE_IDS) {
    const fit = trueFit(data, role);
    if (bestD === null || fit.value > bestD.value) bestD = fit;
  }
  if (bestO === null || bestD === null) throw new Error('bestRoles: empty role catalog');
  return { offense: bestO, defense: bestD };
}

/** All offense role ids in catalog order. */
export const OFFENSE_ROLE_IDS: readonly OffenseRoleId[] = [
  'primary_creator', 'iso_scorer', 'secondary_creator', 'hub', 'post_scorer',
  'interior_finisher', 'floor_spacer', 'cutter', 'movement_shooter', 'transition_runner',
];

/** All defense role ids in catalog order. */
export const DEFENSE_ROLE_IDS: readonly DefenseRoleId[] = [
  'poa_stopper', 'pest', 'switch_wing', 'rotator', 'roamer', 'rim_anchor', 'post_defender',
];

export { allAttributes, attributeValue };
