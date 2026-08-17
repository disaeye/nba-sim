/**
 * 球员生成器 (§7) — five fixed steps:
 *
 *   选原型 → 物理生成 → 能力采样 → POT 生成 → 倾向采样（含校验）
 *
 * Draw order is fixed and documented so generation is fully deterministic
 * for a given rng stream (mulberry32):
 *   age (if not given)        1 draw
 *   H                         1 uniform + 2 normal
 *   WS ratio                  2 normal
 *   WT                        2 normal
 *   VJ / SPD / LAT            2 normal each
 *   DUR                       2 normal
 *   PLY                       2 normal
 *   24 abilities              2 normal each
 *   24 POTs                   2 normal each
 *   9 tendencies              2 normal each
 *
 * `normal` (Box-Muller) consumes exactly two rng.next() calls.
 */
import { int, unit, weighted } from '../rng/helpers.js';
import { normal } from './observation.js';
import { clip } from './normalize.js';
import { priorExperienceBonus, roundInt, validateTendencies } from './growth.js';
import {
  ABILITY_ORDER,
  LEAGUE,
  PHYSICAL_GEN,
  POT,
  PROTOTYPES,
  PROTOTYPE_SHARES,
  TENDENCY_ORDER,
  UTILITY_ABILITY,
} from './tables.js';
import type { Rng } from '../rng/types.js';
import type {
  AbilityKey,
  AbilitySet,
  AwarenessSet,
  PlayerData,
  PrototypeClass,
  PrototypeDef,
  PrototypeId,
  TendencyKey,
  TendencySet,
} from './types.js';

const PROTOTYPE_BY_ID: Readonly<Record<PrototypeId, PrototypeDef>> = Object.fromEntries(
  PROTOTYPES.map((p) => [p.id, p]),
) as Record<PrototypeId, PrototypeDef>;

/** Pick a prototype by 联盟占比 (§7.2) — one rng draw. */
export function pickPrototype(rng: Rng): PrototypeDef {
  return weighted(rng, PROTOTYPES, PROTOTYPE_SHARES);
}

/** Prototype lookup by id. */
export function prototypeById(id: PrototypeId): PrototypeDef {
  const def = PROTOTYPE_BY_ID[id];
  if (def === undefined) throw new Error(`prototypeById: unknown prototype ${id}`);
  return def;
}

export interface GenerateOptions {
  /** Explicit prototype; defaults to a league-share draw. */
  readonly prototype?: PrototypeId;
  /** Explicit age; defaults to a draft-age draw (18..22). */
  readonly age?: number;
  readonly rng: Rng;
}

/** §7.4 physical generation. */
function generatePhysical(def: PrototypeDef, age: number, rng: Rng): PlayerData['physical'] {
  const cls = def.class;
  const heightRange = PHYSICAL_GEN.height[cls];
  const H = roundInt(clip(
    heightRange.min + unit(rng) * (heightRange.max - heightRange.min) + normal(rng, 0, PHYSICAL_GEN.heightNoiseSigma),
    PHYSICAL_RANGES_H_MIN,
    PHYSICAL_RANGES_H_MAX,
  ));
  const ratio = clip(
    normal(rng, PHYSICAL_GEN.wsRatio.mean, PHYSICAL_GEN.wsRatio.sigma),
    PHYSICAL_GEN.wsRatio.min,
    PHYSICAL_GEN.wsRatio.max,
  );
  const WS = roundInt(H * ratio);
  const bigBonus = cls === 'big' ? PHYSICAL_GEN.wt.bigBonus : 0;
  // 背身传统中锋 carries an explicit WT 120 in its key abilities (§7.2).
  const wtMean = typeof def.keyAbilities.WT === 'number' ? def.keyAbilities.WT : H + PHYSICAL_GEN.wt.offset + bigBonus;
  const WT = roundInt(clip(normal(rng, wtMean, PHYSICAL_GEN.wt.sigma), PHYSICAL_RANGES_WT_MIN, PHYSICAL_RANGES_WT_MAX));
  const VJ = roundInt(clip(normal(rng, def.physical.VJ, 8), PHYSICAL_GEN.vjClip.min, PHYSICAL_GEN.vjClip.max));
  let LAT = roundInt(clip(normal(rng, def.physical.LAT, 8), PHYSICAL_GEN.spdLatClip.min, PHYSICAL_GEN.spdLatClip.max));
  if (def.latFloor !== undefined && LAT < def.latFloor) LAT = def.latFloor;
  const SPD = roundInt(clip(normal(rng, def.physical.SPD, 8), PHYSICAL_GEN.spdLatClip.min, PHYSICAL_GEN.spdLatClip.max));
  const DUR = roundInt(clip(normal(rng, PHYSICAL_GEN.dur.mean, PHYSICAL_GEN.dur.sigma), PHYSICAL_RANGES_DUR_MIN, PHYSICAL_RANGES_DUR_MAX));
  return { H, WT, WS, VJ, SPD, LAT, AGE: age, DUR };
}

const PHYSICAL_RANGES_H_MIN = 175;
const PHYSICAL_RANGES_H_MAX = 230;
const PHYSICAL_RANGES_WT_MIN = 70;
const PHYSICAL_RANGES_WT_MAX = 140;
const PHYSICAL_RANGES_DUR_MIN = 20;
const PHYSICAL_RANGES_DUR_MAX = 99;

/** §7.2 ability sampling: key abilities N(mean,5), others N(55,10), clip [20,99]. */
function generateAbilities(def: PrototypeDef, rng: Rng): AbilitySet {
  const out = {} as Record<AbilityKey, number>;
  for (const key of ABILITY_ORDER) {
    const mean = def.keyAbilities[key];
    let value: number;
    if (mean !== undefined) {
      value = normal(rng, mean, PHYSICAL_GEN.abilitySigma);
    } else if (def.id === 'jack_of_all') {
      // 全能工具人: 全能力 62~68 均值, 方差最小.
      value = UTILITY_ABILITY.min + unit(rng) * (UTILITY_ABILITY.max - UTILITY_ABILITY.min) + normal(rng, 0, 1);
    } else {
      value = normal(rng, PHYSICAL_GEN.abilityBase, PHYSICAL_GEN.abilityBaseSigma);
    }
    out[key] = roundInt(clip(value, PHYSICAL_GEN.abilityClip.min, PHYSICAL_GEN.abilityClip.max));
  }
  return out;
}

/** §7.3 POT: clip(gen + N(μ_age, σ_age), max(gen+2, 40), 99); key abilities +5. */
function generatePot(def: PrototypeDef, age: number, ability: AbilitySet, rng: Rng): Record<AbilityKey, number> {
  let band = POT.ageBands[POT.ageBands.length - 1]!;
  for (const b of POT.ageBands) {
    if (age >= b.min && age <= b.max) {
      band = b;
      break;
    }
  }
  const out = {} as Record<AbilityKey, number>;
  for (const key of ABILITY_ORDER) {
    const boost = def.keyAbilities[key] !== undefined ? POT.keyBonus : 0;
    const pot = clip(
      ability[key] + normal(rng, band.mu, band.sigma) + boost,
      Math.max(ability[key] + POT.minGap, POT.floor),
      POT.cap,
    );
    out[key] = roundInt(pot);
  }
  return out;
}

/** §7.5 tendency sampling: N(feature,10) clip [5,95]; unlisted features default 50 (⌛). */
function generateTendencies(def: PrototypeDef, rng: Rng): TendencySet {
  const out = {} as Record<TendencyKey, number>;
  for (const key of TENDENCY_ORDER) {
    const mean = def.tendencyFeatures[key] ?? 50;
    out[key] = roundInt(clip(
      normal(rng, mean, PHYSICAL_GEN.tendencySigma),
      PHYSICAL_GEN.tendencyClip.min,
      PHYSICAL_GEN.tendencyClip.max,
    ));
  }
  return validateTendencies(out);
}

/**
 * 判断层 OFFR/DEFR/SPC — prototypes list them in 关键能力均值 (e.g. OFFR 82),
 * so they sample like abilities: N(mean, 5), else N(55, 10), clip [20,99].
 */
function generateAwareness(def: PrototypeDef, rng: Rng): Pick<AwarenessSet, 'OFFR' | 'DEFR' | 'SPC'> {
  const sample = (key: 'OFFR' | 'DEFR' | 'SPC'): number => {
    const mean = def.keyAbilities[key];
    const value = mean !== undefined
      ? normal(rng, mean, PHYSICAL_GEN.abilitySigma)
      : normal(rng, PHYSICAL_GEN.abilityBase, PHYSICAL_GEN.abilityBaseSigma);
    return roundInt(clip(value, PHYSICAL_GEN.abilityClip.min, PHYSICAL_GEN.abilityClip.max));
  };
  return { OFFR: sample('OFFR'), DEFR: sample('DEFR'), SPC: sample('SPC') };
}

/** §7.4 PLY distribution by prototype + veteran experience bonus. */
function generatePly(def: PrototypeDef, age: number, rng: Rng): number {
  const base = normal(rng, def.plyMean, PHYSICAL_GEN.plySigma);
  const bonus = age >= 22 ? priorExperienceBonus(age) : 0;
  return roundInt(clip(base + bonus, 20, 99));
}

/**
 * Generate one full player record. Fixed draw order (see header).
 * Age defaults to the draft band 18..22 when not given.
 */
export function generatePlayer(opts: GenerateOptions): PlayerData {
  const { rng } = opts;
  const def = opts.prototype !== undefined ? prototypeById(opts.prototype) : pickPrototype(rng);
  const age = opts.age ?? int(rng, LEAGUE.draftAges.min, LEAGUE.draftAges.max);
  const physical = generatePhysical(def, age, rng);
  const ability = generateAbilities(def, rng);
  const pot = generatePot(def, age, ability, rng);
  const tendency = generateTendencies(def, rng);
  const awareness = {
    ...generateAwareness(def, rng),
    PLY: generatePly(def, age, rng),
  };
  return { physical, ability, pot, tendency, awareness };
}

export interface RosterOptions {
  readonly count: number;
  /** Age band for every player (draft: 18..22). */
  readonly ageMin?: number;
  readonly ageMax?: number;
  readonly rng: Rng;
}

/** Generate a roster; one extra draw per player for age. */
export function generateRoster(opts: RosterOptions): PlayerData[] {
  const min = opts.ageMin ?? LEAGUE.draftAges.min;
  const max = opts.ageMax ?? LEAGUE.draftAges.max;
  const players: PlayerData[] = [];
  for (let i = 0; i < opts.count; i++) {
    players.push(generatePlayer({ rng: opts.rng, age: int(opts.rng, min, max) }));
  }
  return players;
}

/**
 * §7.6 draft pool — prototype draws by league share with the two ecosystem
 * guarantees: prototype 1 (持球大核) ≥1, prototype 10 (空间型内线) ≥2.
 */
export function generateDraftPool(count: number, rng: Rng): PrototypeDef[] {
  const pool: PrototypeDef[] = [];
  for (let i = 0; i < count; i++) pool.push(pickPrototype(rng));
  const replaceIndex = (pred: (p: PrototypeDef) => boolean, def: PrototypeDef): void => {
    const candidates = pool.map((p, i) => (pred(p) ? i : -1)).filter((i) => i !== -1);
    if (candidates.length === 0) {
      // Fallback: replace a random non-matching slot (keeps the guarantee).
      const idx = int(rng, 0, pool.length - 1);
      pool[idx] = def;
      return;
    }
    const idx = candidates[int(rng, 0, candidates.length - 1)]!;
    pool[idx] = def;
  };
  const countOf = (id: PrototypeId): number => pool.filter((p) => p.id === id).length;
  while (countOf('ball_dominant') < LEAGUE.guarantee.ballDominant) {
    replaceIndex((p) => p.id !== 'ball_dominant' && p.id !== 'stretch_big', prototypeById('ball_dominant'));
  }
  while (countOf('stretch_big') < LEAGUE.guarantee.stretchBig) {
    replaceIndex((p) => p.id !== 'ball_dominant' && p.id !== 'stretch_big', prototypeById('stretch_big'));
  }
  return pool;
}

/** Prototype class helper for callers that need position-ish grouping. */
export function prototypeClass(id: PrototypeId): PrototypeClass {
  return prototypeById(id).class;
}
