/**
 * 外界因素 (§3) + 角色承担 (§4) — training, age curves, injuries, experience,
 * role immersion, performance feedback, mismatch. All season-level math;
 * the kernel is game-level, so these are pure functions over PlayerData.
 */
import { ABILITY_ORDER, AGE_CURVES, DATA_GROUPS, EXPERIENCE, IMMERSION, IMMERSION_RULES, INJURY, INJURY_TYPES, MISMATCH, PERFORMANCE_FEEDBACK, PHYSICAL_GEN, POSSESSION, SHOT_TENDENCIES, SHOT_TENDENCY_SUM, TAKEOVER_PASS1ST_CLASH, TENDENCY_ORDER, TRAINING } from './tables.js';
import { normal } from './observation.js';
import { clip } from './normalize.js';
import { immersionEligible as fitImmersionEligible } from './fit.js';
import type { Rng } from '../rng/types.js';
import type {
  AbilityKey,
  AwarenessKey,
  AwarenessSet,
  DataGroup,
  InjuryGrade,
  InjuryResult,
  InjuryType,
  PlayerData,
  RoleId,
  TendencySet,
} from './types.js';

// ─── §1.4 tendency validation (shared with the generator) ───────────────────

/**
 * Tendency validation rules (§1.4):
 * 1. The four shot tendencies are force-normalized to sum 100
 *    (largest-remainder rounding for exact integer sums).
 * 2. TAKEOVER≥70 ∧ PASS1ST≥70 → PASS1ST clamped to 69.
 */
export function validateTendencies(t: TendencySet): TendencySet {
  const out = { ...t };
  // Step 1: normalize the four shot tendencies to sum exactly 100.
  const sum = SHOT_TENDENCIES.reduce((acc, k) => acc + Math.max(0, out[k]), 0);
  if (sum <= 0) {
    for (const k of SHOT_TENDENCIES) out[k] = SHOT_TENDENCY_SUM / SHOT_TENDENCIES.length;
  } else {
    const scaled = SHOT_TENDENCIES.map((k) => (Math.max(0, out[k]) / sum) * SHOT_TENDENCY_SUM);
    const floored = scaled.map((v) => Math.floor(v));
    let remainder = SHOT_TENDENCY_SUM - floored.reduce((a, b) => a + b, 0);
    // Distribute the remainder to the largest fractional parts (deterministic order).
    const order = SHOT_TENDENCIES.map((k, i) => ({ i, frac: scaled[i]! - floored[i]! }))
      .sort((a, b) => b.frac - a.frac);
    for (let j = 0; j < order.length && remainder > 0; j++) {
      floored[order[j]!.i]! += 1;
      remainder -= 1;
    }
    for (let i = 0; i < SHOT_TENDENCIES.length; i++) {
      out[SHOT_TENDENCIES[i]!] = Math.min(99, floored[i]!);
    }
  }
  // Step 2: TAKEOVER/PASS1ST clash rule.
  if (out.TAKEOVER >= TAKEOVER_PASS1ST_CLASH.takeoverMin && out.PASS1ST >= TAKEOVER_PASS1ST_CLASH.takeoverMin) {
    out.PASS1ST = TAKEOVER_PASS1ST_CLASH.pass1stClamped;
  }
  return out;
}

// ─── §3.1 training ──────────────────────────────────────────────────────────

/** 年龄效率: 18~21=1.0 / 22~25=0.8 / 26~28=0.5 / 29~32=0.25 / 33+=0.1. */
export function ageEfficiency(age: number): number {
  for (const band of TRAINING.ageEfficiency) {
    if (age >= band.min && age <= band.max) return band.eff;
  }
  return TRAINING.ageEfficiency[TRAINING.ageEfficiency.length - 1]!.eff;
}

/**
 * Δa = 点数 × 0.14 × 年龄效率 × (1 − a/POT).
 * Returns the raw delta; caller applies coach/performance multipliers.
 */
export function trainingDelta(points: number, ability: number, pot: number, age: number): number {
  if (pot <= ability) return 0; // 到达即停
  return points * TRAINING.coefficient * ageEfficiency(age) * (1 - ability / pot);
}

/** Training efficiency multiplier stack (§3.1, §4.3, §4.4, §6.6). */
export interface TrainingContext {
  readonly coachLevel?: 1 | 2 | 3 | 4 | 5;
  /** 高光: ≥55 场且效率队内前 20% → ×1.2. */
  readonly highlight?: boolean;
  /** 板凳/DNP: <20 场 → ×0.8. */
  readonly bench?: boolean;
  /** 重伤复出季 → ×0.9. */
  readonly recovery?: boolean;
  /** 角色错配 (Fit<50 满赛季) → ×0.9. */
  readonly mismatch?: boolean;
}

/** Product of all active training modifiers. */
export function trainingMultiplier(ctx: TrainingContext): number {
  let mult = 1;
  if (ctx.coachLevel !== undefined) mult *= TRAINING.coachTrain[ctx.coachLevel];
  if (ctx.highlight) mult *= PERFORMANCE_FEEDBACK.highlightTrain;
  if (ctx.bench) mult *= PERFORMANCE_FEEDBACK.benchTrain;
  if (ctx.recovery) mult *= PERFORMANCE_FEEDBACK.recoveryTrain;
  if (ctx.mismatch) mult *= MISMATCH.trainMult;
  return mult;
}

/** Training delta for one ability with the full context (awareness trains ×0.3). */
export function trainingDeltaFor(
  data: PlayerData,
  key: AbilityKey,
  points: number,
  ctx: TrainingContext = {},
): number {
  const eff = trainingDelta(points, data.ability[key], data.pot[key], data.physical.AGE);
  return eff * trainingMultiplier(ctx);
}

/** Natural growth = 训练公式点数=30 等效量, inside the growth window only (§3.2). */
export function naturalGrowthPoints(): number {
  return TRAINING.naturalPoints;
}

/** Is this ability inside its group's growth window? (§3.2 成长期). */
export function inGrowthWindow(key: AbilityKey | AwarenessKey, age: number): boolean {
  if (key in DATA_GROUPS) {
    const group = DATA_GROUPS[key as AbilityKey] ?? DATA_GROUPS[key as AwarenessKey];
    const curve = AGE_CURVES.find((c) => c.group === group);
    return curve !== undefined && age <= curve.growthEnd;
  }
  return false;
}

// ─── §3.2 age decline ───────────────────────────────────────────────────────

/** Annual decline for a data group at an age (0 outside the decline phase). */
export function annualDecline(group: DataGroup, age: number): number {
  if (group === 'TENDENCY' || group === 'PHYSICAL') return 0;
  const curve = AGE_CURVES.find((c) => c.group === group);
  if (curve === undefined || age < curve.declineStart) return 0;
  let rate = 0;
  for (const band of curve.declineRates) {
    if (age >= band.min) rate = band.rate;
  }
  return rate;
}

/** Body-group (SPD/LAT/VJ) decline — applies to 体测, untrainable. */
export function bodyAnnualDecline(age: number): number {
  return annualDecline('BODY', age);
}

/** Which group an ability/awareness item belongs to. */
export function groupOf(key: AbilityKey | AwarenessKey): DataGroup {
  if (key in DATA_GROUPS) return DATA_GROUPS[key as AbilityKey] ?? DATA_GROUPS[key as AwarenessKey];
  return 'PHYSICAL';
}

// ─── §3.3 injuries ──────────────────────────────────────────────────────────

export interface InjuryContext {
  readonly dur: number;
  readonly age: number;
  /** 场均分钟 (avg minutes per game). */
  readonly avgMinutes: number;
  /** 疲劳积累 F (§8.5). */
  readonly fatigue: number;
  /** STM <25 at the moment (injury ×1.5). */
  readonly lowStamina?: boolean;
}

/** P = 2% × (80/DUR) × 年龄系数 × 负荷系数. */
export function injuryProbability(ctx: InjuryContext): number {
  let ageCoef = 1.0;
  for (const band of INJURY.ageCoef) {
    if (ctx.age <= band.max) {
      ageCoef = band.coef;
      break;
    }
  }
  let load = 1;
  if (ctx.avgMinutes > INJURY.loadMinutes) load *= INJURY.loadCoef;
  if (ctx.fatigue > INJURY.fatigueThreshold) load *= INJURY.fatigueLoadCoef;
  if (ctx.lowStamina) load *= 1.5;
  return INJURY.base * (INJURY.durFactor / ctx.dur) * ageCoef * load;
}

/** Inclusive integer draw from one rng.next() call. */
function intUniform(rng: Rng, min: number, max: number): number {
  return Math.floor(min + rng.next() * (max - min + 1));
}

/** Severity + type + gamesOut + data effects for a confirmed injury. */
export function injuryEffects(type: InjuryType, grade: InjuryGrade, rng: Rng): InjuryResult {
  const typeInfo = INJURY_TYPES[type];
  let gamesOut = Infinity;
  if (grade !== 'SEVERE') {
    const entry = INJURY.grades.find((g) => g.grade === grade);
    const range = entry?.games;
    if (range !== undefined && range !== null) gamesOut = intUniform(rng, range.min, range.max);
  }
  const bodyPenalty = grade === 'MODERATE'
    ? INJURY.moderateBodyPenalty
    : grade === 'SEVERE'
      ? intUniform(rng, INJURY.severeBodyPenalty.min, INJURY.severeBodyPenalty.max)
      : 0;
  const potCut = grade === 'SEVERE'
    ? intUniform(rng, INJURY.severePotCut.min, INJURY.severePotCut.max)
    : 0;
  return {
    grade,
    gamesOut,
    groups: typeInfo.groups,
    potTargets: typeInfo.potTargets,
    bodyPenalty,
    potCut,
  };
}

/**
 * Roll injury for one game: returns null when no injury occurs.
 * Draw order: severity (1 unit) → type (1 unit) → games/penalties (1 unit each).
 */
export function rollInjury(ctx: InjuryContext, rng: Rng): InjuryResult | null {
  const p = injuryProbability(ctx);
  if (rng.next() >= p) return null;
  const severity = rng.next();
  let grade: InjuryGrade = 'MINOR';
  let acc = 0;
  for (const g of INJURY.grades) {
    acc += g.share;
    if (severity < acc) {
      grade = g.grade as InjuryGrade;
      break;
    }
  }
  const types = Object.keys(INJURY_TYPES) as InjuryType[];
  const type = types[Math.floor(rng.next() * types.length)]!;
  return injuryEffects(type, grade, rng);
}

// ─── §4.1 experience → awareness ────────────────────────────────────────────

/** ΔAwareness/赛季 = min(出场回合数/1000, 2.0) × 年龄系数. */
export function experienceGrowth(rounds: number, age: number): number {
  let coef = 1.0;
  for (const band of EXPERIENCE.ageCoef) {
    if (age <= band.max) {
      coef = band.coef;
      break;
    }
  }
  return Math.min(rounds / EXPERIENCE.roundsPerPoint, EXPERIENCE.maxPerSeason) * coef;
}

/** Apply one season's experience growth to all four awareness items (cap 95). */
export function applyExperienceGrowth(awareness: AwarenessSet, rounds: number, age: number): AwarenessSet {
  const delta = experienceGrowth(rounds, age);
  const out = { ...awareness };
  for (const key of Object.keys(out) as AwarenessKey[]) {
    out[key] = Math.min(EXPERIENCE.cap, out[key] + delta);
  }
  return out;
}

/**
 * Prior-seasons experience bonus for veterans (§7.4 "老将按 4.1 经验成长
 * 额外提升"). Assumes `veteranRoundsPerSeason` rounds per prior season (⌛),
 * seasons completed at ages 20..age−1.
 */
export function priorExperienceBonus(age: number): number {
  let bonus = 0;
  for (let s = 20; s < age; s++) {
    bonus += experienceGrowth(PHYSICAL_GEN.veteranRoundsPerSeason, s);
  }
  return bonus;
}

// ─── §4.2 role immersion → tendency ─────────────────────────────────────────

/**
 * 浸润条件 (§4.2): role held a full season, role rounds ≥ 40% of team
 * rounds, Fit ≥65, not bench/DNP, not mismatch.
 */
export function roleImmersionEligible(args: { fit: number; roleRounds: number; teamRounds: number; games: number }): boolean {
  if (!fitImmersionEligible(args.fit)) return false;
  if (args.games < IMMERSION_RULES.minGames) return false;
  if (args.teamRounds <= 0) return false;
  return args.roleRounds / args.teamRounds >= IMMERSION_RULES.roleShare;
}

/**
 * Apply one season's immersion offsets for a role, with per-role caps,
 * then re-validate (§1.4 rules run after every offset).
 */
export function applyImmersion(tendency: TendencySet, role: RoleId, seasons = 1): TendencySet {
  const out = { ...tendency };
  for (const offset of IMMERSION[role]) {
    const delta = offset.delta * seasons;
    let value = out[offset.tendency] + delta;
    if (offset.capMax !== undefined) value = Math.min(offset.capMax, value);
    if (offset.capMin !== undefined) value = Math.max(offset.capMin, value);
    out[offset.tendency] = clip(value, 0, 99);
  }
  return validateTendencies(out);
}

// ─── §4.4 mismatch ──────────────────────────────────────────────────────────

export interface MismatchReport {
  /** Fit <50 for a full season. */
  readonly friction: boolean;
  /** 体系不适 morale event fired. */
  readonly systemMisfit: boolean;
  /** 2 consecutive mismatch seasons → confidence erosion. */
  readonly erosion: { takeover: number; push: number };
}

/** 角色错配 (§4.4): friction blocks immersion and cuts training; erosion at 2+ seasons. */
export function mismatchReport(fit: number, consecutiveSeasons: number): MismatchReport {
  const friction = fit < POSSESSION.mismatchFitThreshold;
  return {
    friction,
    systemMisfit: friction,
    erosion: friction && consecutiveSeasons >= MISMATCH.consecutiveSeasons
      ? { takeover: MISMATCH.erosion.takeover, push: MISMATCH.erosion.push }
      : { takeover: 0, push: 0 },
  };
}

// ─── §7.5 shared sampling helper (used by the generator) ────────────────────

/** Round a value to an integer (generation outputs integers). */
export function roundInt(v: number): number {
  return Math.round(v);
}

export { ABILITY_ORDER, TENDENCY_ORDER };
