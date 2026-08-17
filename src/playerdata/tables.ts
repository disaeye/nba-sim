/**
 * Deterministic tables for the player data system — every numeric value in
 * `docs/playerdata-design.md` (v1.3a) transcribed 1:1. The doc is the sole
 * authority; values marked ⌛ there are playtest-pending but fixed here.
 *
 * Conventions:
 * - Ability values live on 20..99; Tendency 0..99; Awareness 20..99.
 * - `share` values sum to 1.0 across the 12 prototypes (7.2).
 * - Modifier entries are additive: delta = factor × (sourceValue − anchor).
 */
import type {
  AbilityKey,
  AttributeKey,
  AwarenessKey,
  ChemistryChannel,
  DataGroup,
  DefenseRoleId,
  FitConfig,
  FitWeightSource,
  Grade,
  InjuryType,
  MoraleEventId,
  OffenseRoleId,
  PhysicalKey,
  PrototypeDef,
  PrototypeId,
  RoleId,
  ScoutLevel,
  TendencyKey,
} from './types.js';

// ─── 1.2 physical ranges / normalization parameters ─────────────────────────

/** Ability keys in §1.3 catalog order. */
export const ABILITY_ORDER: readonly AbilityKey[] = [
  'CS3', 'OD3', 'CSM', 'PUM', 'FLT', 'POST', 'FT',
  'FINS', 'FINW', 'FINC', 'DUNK', 'HND', 'PNR', 'PASS',
  'PASM', 'POCK', 'SCRN', 'POBD', 'NAVS', 'SWCH', 'RIM',
  'STL', 'BOX', 'ORB',
];

/** Tendency keys in §1.4 catalog order. */
export const TENDENCY_ORDER: readonly TendencyKey[] = [
  'T3', 'TMID', 'TDRIVE', 'TPOST', 'PASS1ST', 'GAMBLE', 'FOUL', 'TAKEOVER', 'PUSH',
];

/** Awareness keys in §1.5 catalog order. */
export const AWARENESS_ORDER: readonly AwarenessKey[] = ['OFFR', 'DEFR', 'SPC', 'PLY'];

/** Attribute keys (展示层) in §1.6 order. */
export const ATTRIBUTE_ORDER: readonly AttributeKey[] = [
  'SHOOTING', 'FINISHING', 'PLAYMAKING', 'PERIMETER_D', 'INTERIOR_D', 'REBOUNDING', 'ATHLETICISM',
];

export const PHYSICAL_RANGES = {
  H: { min: 175, max: 230 },
  WT: { min: 70, max: 140 },
  VJ: { min: 50, max: 110 },
  SPD: { min: 20, max: 99 },
  LAT: { min: 20, max: 99 },
  AGE: { min: 18, max: 40 },
  DUR: { min: 20, max: 99 },
} as const;

// ─── 1.2 normalization formulas (H_n / WT_n / VJ_n / WS_n) ──────────────────

export const NORMALIZATION = {
  H: { low: 175, span: 55, base: 20, scale: 79 },
  WT: { low: 70, span: 70, base: 20, scale: 79 },
  VJ: { low: 50, span: 60, base: 20, scale: 79 },
  WS: { lowRatio: 0.98, spanRatio: 0.14, base: 20, scale: 79 },
} as const;

// ─── 1.3 ability catalog: physical corrections ──────────────────────────────

/** One additive physical correction: delta = factor × (source − anchor). */
export interface PhysicalModifier {
  readonly source: 'H_n' | 'WT_n' | 'VJ_n' | 'WS_n' | 'LAT';
  readonly factor: number;
  readonly anchor: number;
  /** SWCH only: the correction reads min(source, pairedMin) (§1.3, +min(LAT, H_n)×0.1). */
  readonly pairedMin?: 'H_n';
}

export const ABILITY_PHYSICAL_MODIFIERS: Readonly<Partial<Record<AbilityKey, readonly PhysicalModifier[]>>> = {
  POST: [{ source: 'WT_n', factor: 0.15, anchor: 0 }],
  FINS: [{ source: 'VJ_n', factor: 0.1, anchor: 0 }],
  FINW: [{ source: 'VJ_n', factor: 0.1, anchor: 0 }],
  FINC: [
    { source: 'WT_n', factor: 0.15, anchor: 0 },
    { source: 'VJ_n', factor: 0.1, anchor: 0 },
  ],
  DUNK: [
    { source: 'VJ_n', factor: 0.2, anchor: 0 },
    { source: 'H_n', factor: 0.1, anchor: 0 },
  ],
  HND: [{ source: 'H_n', factor: -0.1, anchor: 50 }],
  SCRN: [{ source: 'WT_n', factor: 0.2, anchor: 0 }],
  POBD: [{ source: 'LAT', factor: 0.15, anchor: 0 }],
  NAVS: [
    { source: 'LAT', factor: 0.15, anchor: 0 },
    { source: 'WT_n', factor: -0.1, anchor: 50 },
  ],
  SWCH: [{ source: 'LAT', factor: 0.1, anchor: 0, pairedMin: 'H_n' }],
  RIM: [
    { source: 'H_n', factor: 0.15, anchor: 0 },
    { source: 'VJ_n', factor: 0.1, anchor: 0 },
    { source: 'WS_n', factor: 0.05, anchor: 0 },
  ],
  STL: [{ source: 'WS_n', factor: 0.1, anchor: 0 }],
  BOX: [{ source: 'WT_n', factor: 0.2, anchor: 0 }],
  ORB: [
    { source: 'VJ_n', factor: 0.15, anchor: 0 },
    { source: 'WT_n', factor: 0.1, anchor: 0 },
  ],
};

/** 弱侧手 rule: FINW < FINS−20 → forced weak-side efficiency −8% (§1.3). */
export const WEAK_HAND_RULE = { gap: 20, penalty: 0.08 } as const;

// ─── 1.4 tendency validation ────────────────────────────────────────────────

/** The four shot tendencies, forcibly normalized to sum 100 (§1.4). */
export const SHOT_TENDENCIES: readonly TendencyKey[] = ['T3', 'TMID', 'TDRIVE', 'TPOST'];
export const SHOT_TENDENCY_SUM = 100;
export const TAKEOVER_PASS1ST_CLASH = { takeoverMin: 70, pass1stClamped: 69 } as const;

// ─── 1.6 attribute aggregation ──────────────────────────────────────────────

export const ATTRIBUTE_WEAK_WEIGHT = { weighted: 0.7, min: 0.3 } as const;

/** Attribute weight tables (§1.6). Sources are abilities, awareness, or physical quantities. */
export const ATTRIBUTE_WEIGHTS: Readonly<Record<AttributeKey, readonly { source: FitWeightSource; weight: number }[]>> = {
  SHOOTING: [
    { source: 'CS3', weight: 0.35 },
    { source: 'OD3', weight: 0.25 },
    { source: 'CSM', weight: 0.2 },
    { source: 'PUM', weight: 0.1 },
    { source: 'FT', weight: 0.1 },
  ],
  FINISHING: [
    { source: 'FINC', weight: 0.25 },
    { source: 'DUNK', weight: 0.25 },
    { source: 'FINS', weight: 0.2 },
    { source: 'FINW', weight: 0.15 },
    { source: 'FLT', weight: 0.15 },
  ],
  PLAYMAKING: [
    { source: 'PASS', weight: 0.3 },
    { source: 'PASM', weight: 0.25 },
    { source: 'OFFR', weight: 0.2 },
    { source: 'POCK', weight: 0.15 },
    { source: 'HND', weight: 0.1 },
  ],
  PERIMETER_D: [
    { source: 'POBD', weight: 0.4 },
    { source: 'NAVS', weight: 0.25 },
    { source: 'SWCH', weight: 0.2 },
    { source: 'STL', weight: 0.15 },
  ],
  INTERIOR_D: [
    { source: 'RIM', weight: 0.45 },
    { source: 'DEFR', weight: 0.3 },
    { source: 'BOX', weight: 0.25 },
  ],
  REBOUNDING: [
    { source: 'ORB', weight: 0.35 },
    { source: 'BOX', weight: 0.35 },
    { source: 'DEFR', weight: 0.15 },
    { source: 'VJ_n', weight: 0.15 },
  ],
  ATHLETICISM: [
    { source: 'SPD', weight: 0.35 },
    { source: 'LAT', weight: 0.3 },
    { source: 'VJ_n', weight: 0.35 },
  ],
};

/** Grade bands (§1.6). Ordered high→low; gradeOf picks the first band with value ≥ min. */
export const GRADE_BANDS: ReadonlyArray<{ grade: Grade; min: number }> = [
  { grade: 'S', min: 90 },
  { grade: 'A', min: 85 },
  { grade: 'A-', min: 82 },
  { grade: 'B+', min: 78 },
  { grade: 'B', min: 74 },
  { grade: 'B-', min: 70 },
  { grade: 'C+', min: 66 },
  { grade: 'C', min: 62 },
  { grade: 'D', min: 55 },
  { grade: 'F', min: 0 },
];

// ─── 1.6-1.8 observation model ──────────────────────────────────────────────

export const OBSERVATION = {
  /** k_scout per scout level (L1..L5). */
  kScout: { 1: 1.5, 2: 1.2, 3: 1.0, 4: 0.8, 5: 0.6 } as Readonly<Record<ScoutLevel, number>>,
  /** n capped at 2000 — σ never reaches zero. */
  sampleCap: 2000,
  /** Old samples decay with half-life 500 rounds after big data changes. */
  sampleHalfLife: 500,
  /** Base σ for each hidden quantity. */
  sigmaBase: { attribute: 12, tendency: 15, awareness: 10 } as const,
  /** Sample-size pivot: σ = base × k × √(200/n). */
  pivotRounds: 200,
  /** σ ≥ 8 → grade shown as a range; below → single grade. */
  rangeSigma: 8,
  /** L5 presents with σ halved (单档呈现 unlock, §1.7). */
  l5SigmaHalving: true,
} as const;

// ─── 1.7 scout report ───────────────────────────────────────────────────────

export const SCOUT = {
  /** Style lines: tendency observed ≥70 → high text; ≤40 → low text. */
  styleText: [
    { tendency: 'T3', high: '痴迷外线出手', low: '几乎不投三分' },
    { tendency: 'TDRIVE', high: '喜欢持球冲击', low: '很少突破' },
    { tendency: 'TPOST', high: '热衷背身要位', low: '不去低位' },
    { tendency: 'PASS1ST', high: '乐于分享球', low: '球到他手里就停了' },
    { tendency: 'TAKEOVER', high: '关键时刻要球', low: '关键时刻隐身' },
    { tendency: 'GAMBLE', high: '防守爱赌博', low: '防守稳健不失位' },
  ] as const,
  highThreshold: 70,
  lowThreshold: 40,
  /** 能力档位区: grade + 5-cell confidence bar. */
  confidenceCells: 5,
  /** Style + role-fit sections unlock at L3 (解锁节奏). */
  styleUnlockLevel: 3,
  /** Rare-combination lines (§1.7); thresholds are ⌛ interpretation. */
  rareCombos: [
    { id: 'HARD_SHOT_MAKER', highTendency: 'TAKEOVER', highTendencyMin: 70, abilityPair: ['OD3', 'PUM'], abilityMin: 80, text: '投篮选择糟糕，但就是能进' },
    { id: 'CONFIDENT_ROLE', highTendency: 'TAKEOVER', highTendencyMin: 70, abilityMax: 60, text: '自信的普通球员' },
    { id: 'OLD_SCHOOL', highTendency: 'TPOST', highTendencyMin: 60, abilityLow: 'POST', abilityLowMax: 55, text: '活在上个时代' },
  ] as const,
} as const;

// ─── 2.2-2.3 role catalogs ──────────────────────────────────────────────────

export const OFFENSE_ROLES: Readonly<Record<OffenseRoleId, { zh: string; demand: number }>> = {
  primary_creator: { zh: '持球核心', demand: 30 },
  iso_scorer: { zh: '单打得分手', demand: 26 },
  secondary_creator: { zh: '副攻手', demand: 22 },
  hub: { zh: '组织枢纽', demand: 20 },
  post_scorer: { zh: '背身轴心', demand: 18 },
  interior_finisher: { zh: '内线终结', demand: 15 },
  floor_spacer: { zh: '无球射手', demand: 13 },
  cutter: { zh: '空切终结', demand: 12 },
  movement_shooter: { zh: '跑动射手', demand: 12 },
  transition_runner: { zh: '转换箭头', demand: 10 },
};

export const DEFENSE_ROLES: Readonly<Record<DefenseRoleId, { zh: string }>> = {
  poa_stopper: { zh: '盯人箭头' },
  pest: { zh: '纠缠者' },
  switch_wing: { zh: '换防摇摆' },
  rotator: { zh: '纪律轮转者' },
  roamer: { zh: '协防扫荡' },
  rim_anchor: { zh: '护筐锚' },
  post_defender: { zh: '顶防工兵' },
};

// ─── 2.2 possession budget ──────────────────────────────────────────────────

export const POSSESSION = {
  /** Final share renormalization target — 8% reserve for random events & vacancy penalties. */
  totalShare: 0.92,
  /** Conflict compression: lower-Fit member's share ×= 0.8 per conflict (⌛ interpretation of §2.2). */
  compressionStep: 0.8,
  /** Vacancy fill-in efficiency (2.5). */
  vacancyEfficiency: 0.85,
  /** Fit <50 forced into a role (2.5). */
  mismatchEfficiency: 0.8,
  mismatchFitThreshold: 50,
} as const;

// ─── 2.4 chemistry ──────────────────────────────────────────────────────────

interface ChemistryPair {
  readonly id: string;
  readonly roles: readonly RoleId[];
  readonly channel: ChemistryChannel;
  readonly modifier: number;
  readonly note: string;
}

/** 相生 — 8 groups (§2.4). */
export const CHEMISTRY_SYNERGY: readonly ChemistryPair[] = [
  { id: 'PC×FS', roles: ['primary_creator', 'floor_spacer'], channel: 'help_recovery', modifier: 0.9, note: '核心突破/单打分支的协防到位概率 −10%' },
  { id: 'PC×CUT', roles: ['primary_creator', 'cutter'], channel: 'dime_cut', modifier: 1.1, note: '喂饼分支激活（核心 PASS1ST≥40 时切入目标权重 +10%）' },
  { id: 'SC×PC', roles: ['secondary_creator', 'primary_creator'], channel: 'relief_target', modifier: 1.1, note: '核心被包夹时副攻手接应权重 +10%' },
  { id: 'HUB×CUT', roles: ['hub', 'cutter'], channel: 'cut_target', modifier: 1.1, note: '传切激活：切入目标权重 +10%' },
  { id: 'HUB×MS', roles: ['hub', 'movement_shooter'], channel: 'move_catch', modifier: 1.1, note: '发牌激活：移动接球分支频率 +10%' },
  { id: 'ISO×FS', roles: ['iso_scorer', 'floor_spacer'], channel: 'iso_space', modifier: 1.1, note: '拉开单打：ISO 分支空间判定 +10%' },
  { id: 'PS×MS', roles: ['post_scorer', 'movement_shooter'], channel: 'spacer_open', modifier: 1.1, note: '内外呼应：包夹轴心时射手空位概率 +10%' },
  { id: 'POA×RA', roles: ['poa_stopper', 'rim_anchor'], channel: 'recovery_penalty', modifier: 0.9, note: '箭头压迫失败后的失位惩罚 −10%' },
];

/** 相克 — 7 groups (§2.4). `kind: 'compress'` feeds the §2.2 conflict compression. */
export const CHEMISTRY_CLASH: readonly (ChemistryPair & { kind: 'compress' | 'modifier' })[] = [
  { id: 'PC×PC', kind: 'compress', roles: ['primary_creator', 'primary_creator'], channel: 'initiation', modifier: 0, note: '球权需求超限，按 Fit 排序衰减（§2.5）' },
  { id: 'PC×ISO', kind: 'compress', roles: ['primary_creator', 'iso_scorer'], channel: 'initiation', modifier: 0, note: '双持球冲突（§2.5）' },
  { id: 'PC×HUB', kind: 'modifier', roles: ['primary_creator', 'hub'], channel: 'initiation', modifier: 0.9, note: '发起权冲突：两人发起权重各 −10%' },
  { id: 'CUT×IF', kind: 'modifier', roles: ['cutter', 'interior_finisher'], channel: 'paint_clog', modifier: 0.9, note: '油漆区拥堵：切入与背身分支空间判定 −10%' },
  { id: 'CUT×PS', kind: 'modifier', roles: ['cutter', 'post_scorer'], channel: 'paint_clog', modifier: 0.9, note: '油漆区拥堵（同上）' },
  { id: 'PEST×ROAM', kind: 'modifier', roles: ['pest', 'roamer'], channel: 'vacuum_penalty', modifier: 1.1, note: '双赌博防线：两人同时失位时惩罚 +10%（防线真空）' },
  { id: 'FS×3', kind: 'modifier', roles: ['floor_spacer'], channel: 'initiation_eff', modifier: 0.85, note: '无球射手 ≥3 人：无人发起，发起效率折扣 0.85' },
];

/** 重复角色通用规则 classes (§2.4). */
export const DUPLICATE_CLASSES = {
  /** 发起类: same-role repeat → possession conflict (compression). */
  initiator: ['primary_creator', 'iso_scorer', 'hub'] as const,
  /** 终结类: same-role repeat → paint clog −10%. */
  finishing: ['cutter', 'interior_finisher', 'post_scorer'] as const,
  /** 防守赌博类: same-role repeat → defense vacuum +10%. */
  gambling: ['pest', 'roamer'] as const,
  /** 无球射手 ≥3 → 发起效率折扣 0.85 (explicit in clash table). */
  spacerCount: 3,
} as const;

// ─── 5.2 Fit configs (17 roles) ─────────────────────────────────────────────

export const FIT_CONFIGS: Readonly<Record<RoleId, FitConfig>> = {
  // 进攻角色
  primary_creator: {
    fWeights: [
      { source: 'OD3', weight: 0.25 },
      { source: 'HND', weight: 0.25 },
      { source: 'PNR', weight: 0.2 },
      { source: 'PASM', weight: 0.15 },
      { source: 'OFFR', weight: 0.15 },
    ],
    gates: [
      { tendency: 'TAKEOVER', min: 70 },
      { tendency: 'PASS1ST', min: 40 },
    ],
  },
  iso_scorer: {
    fWeights: [
      { source: 'PUM', weight: 0.3 },
      { source: 'OD3', weight: 0.2 },
      { source: 'FINS', weight: 0.2 },
      { source: 'POST', weight: 0.15 },
      { source: 'HND', weight: 0.15 },
    ],
    gates: [{ tendency: 'TAKEOVER', min: 60 }],
  },
  secondary_creator: {
    fWeights: [
      { source: 'PUM', weight: 0.3 },
      { source: 'CSM', weight: 0.2 },
      { source: 'FINS', weight: 0.2 },
      { source: 'HND', weight: 0.2 },
      { source: 'OD3', weight: 0.1 },
    ],
    gates: [{ tendency: 'TAKEOVER', min: 40, max: 69 }],
  },
  hub: {
    fWeights: [
      { source: 'PASS', weight: 0.35 },
      { source: 'OFFR', weight: 0.3 },
      { source: 'POST', weight: 0.2 },
      { source: 'POCK', weight: 0.15 },
    ],
    gates: [{ tendency: 'PASS1ST', min: 70 }],
  },
  post_scorer: {
    fWeights: [
      { source: 'POST', weight: 0.4 },
      { source: 'FINC', weight: 0.25 },
      { source: 'PASM', weight: 0.2 },
      { source: 'WT_n', weight: 0.15 },
    ],
    gates: [{ tendency: 'TPOST', min: 60 }],
  },
  interior_finisher: {
    fWeights: [
      { source: 'DUNK', weight: 0.3 },
      { source: 'FINC', weight: 0.25 },
      { source: 'POST', weight: 0.25 },
      { source: 'ORB', weight: 0.2 },
    ],
    gates: [],
  },
  floor_spacer: {
    fWeights: [
      { source: 'CS3', weight: 0.5 },
      { source: 'SPC', weight: 0.3 },
      { source: 'CSM', weight: 0.2 },
    ],
    gates: [
      { tendency: 'T3', min: 60 },
      { tendency: 'TAKEOVER', max: 40 },
    ],
  },
  cutter: {
    fWeights: [
      { source: 'FINS', weight: 0.3 },
      { source: 'SPC', weight: 0.25 },
      { source: 'DUNK', weight: 0.2 },
      { source: 'SPD', weight: 0.25 },
    ],
    gates: [{ tendency: 'TDRIVE', min: 50 }],
  },
  movement_shooter: {
    fWeights: [
      { source: 'CS3', weight: 0.45 },
      { source: 'SPC', weight: 0.3 },
      { source: 'CSM', weight: 0.15 },
      { source: 'SPD', weight: 0.1 },
    ],
    gates: [{ tendency: 'T3', min: 65 }],
  },
  transition_runner: {
    fWeights: [
      { source: 'FINS', weight: 0.3 },
      { source: 'SPD', weight: 0.3 },
      { source: 'DUNK', weight: 0.2 },
      { source: 'HND', weight: 0.2 },
    ],
    gates: [
      { tendency: 'PUSH', min: 65 },
      { tendency: 'TDRIVE', min: 55 },
    ],
  },
  // 防守角色
  poa_stopper: {
    fWeights: [
      { source: 'POBD', weight: 0.45 },
      { source: 'NAVS', weight: 0.3 },
      { source: 'STL', weight: 0.25 },
    ],
    gates: [{ physical: 'LAT', min: 60 }],
  },
  pest: {
    fWeights: [
      { source: 'POBD', weight: 0.4 },
      { source: 'STL', weight: 0.25 },
      { source: 'LAT', weight: 0.2 },
      { source: 'DEFR', weight: 0.15 },
    ],
    gates: [
      { physical: 'LAT', min: 65 },
      { tendency: 'GAMBLE', min: 50 },
    ],
  },
  switch_wing: {
    fWeights: [
      { source: 'SWCH', weight: 0.5 },
      { source: 'POBD', weight: 0.3 },
      { source: 'NAVS', weight: 0.2 },
    ],
    gates: [{ physical: 'LAT', pairedWith: 'H_n', min: 55 }],
  },
  rotator: {
    fWeights: [
      { source: 'DEFR', weight: 0.45 },
      { source: 'POBD', weight: 0.25 },
      { source: 'LAT', weight: 0.15 },
      { source: 'BOX', weight: 0.15 },
    ],
    gates: [{ tendency: 'GAMBLE', max: 45 }],
  },
  roamer: {
    fWeights: [
      { source: 'DEFR', weight: 0.4 },
      { source: 'STL', weight: 0.25 },
      { source: 'LAT', weight: 0.2 },
      { source: 'RIM', weight: 0.15 },
    ],
    gates: [{ tendency: 'GAMBLE', min: 60 }],
  },
  rim_anchor: {
    fWeights: [
      { source: 'RIM', weight: 0.5 },
      { source: 'BOX', weight: 0.25 },
      { source: 'DEFR', weight: 0.25 },
    ],
    gates: [{ physical: 'H_n', min: 70 }],
  },
  post_defender: {
    fWeights: [
      { source: 'BOX', weight: 0.4 },
      { source: 'WT_n', weight: 0.35 },
      { source: 'RIM', weight: 0.25 },
    ],
    gates: [{ physical: 'WT_n', min: 70 }],
  },
};

// ─── 5.3 Fit → performance transmission ─────────────────────────────────────

export const FIT_TRANSMISSION = {
  /** 实际权重 = 基础份额 × (0.6 + 0.4×Fit/100). */
  possession: { base: 0.6, slope: 0.4 },
  /** E = 0.70 + 0.30×Fit/100. */
  execEfficiency: { base: 0.7, slope: 0.3 },
  /** 偏离率 × (1.3 − 0.3×Fit/100). */
  deviationPenalty: { base: 1.3, slope: -0.3 },
  /** Immersion requires Fit ≥65. */
  immersionFit: 65,
  /** Scout report labels per observed Fit. */
  reportLabels: { natural: 80, capable: 65, mismatchBelow: 50 },
} as const;

export const FIT_BANDS = { natural: 80, capable: 65, marginal: 50 } as const;

// ─── 3.1 training ───────────────────────────────────────────────────────────

export const TRAINING = {
  pointsPerSeason: 100,
  /** Δa = 点数 × 0.14 × 年龄效率 × (1 − a/POT). */
  coefficient: 0.14,
  /** Natural growth equivalent points per year inside the growth window. */
  naturalPoints: 30,
  /** Awareness trains at ×0.3. */
  awarenessEfficiency: 0.3,
  ageEfficiency: [
    { min: 18, max: 21, eff: 1.0 },
    { min: 22, max: 25, eff: 0.8 },
    { min: 26, max: 28, eff: 0.5 },
    { min: 29, max: 32, eff: 0.25 },
    { min: 33, max: 40, eff: 0.1 },
  ] as const,
  /** 教练系数 per coach level (§6.6). */
  coachTrain: { 1: 0.9, 2: 0.95, 3: 1.0, 4: 1.05, 5: 1.1 } as const,
  /** 队友质量: OFFR≥80 teammate → awareness natural growth ×1.2. */
  mentorOffr: 80,
  mentorMult: 1.2,
  /** Body trains only WT ±5kg. */
  wtAdjustKg: 5,
} as const;

/** 可训练性表 (§3.1): which group each ability/awareness item belongs to.
 *  PNR (挡拆持球) is absent from the doc's 3.1 table but must live in a
 *  trainable group to cover all 24 abilities — 技术组 is the natural fit. */
export const DATA_GROUPS: Readonly<Record<AbilityKey | AwarenessKey, 'SHOOTING' | 'TECHNIQUE' | 'FINISHING' | 'DEFENSE' | 'AWARENESS'>> = {
  CS3: 'SHOOTING', OD3: 'SHOOTING', CSM: 'SHOOTING', PUM: 'SHOOTING', FLT: 'SHOOTING', FT: 'SHOOTING',
  HND: 'TECHNIQUE', PASS: 'TECHNIQUE', PASM: 'TECHNIQUE', POCK: 'TECHNIQUE', SCRN: 'TECHNIQUE', POST: 'TECHNIQUE', FINW: 'TECHNIQUE', PNR: 'TECHNIQUE',
  FINS: 'FINISHING', FINC: 'FINISHING', DUNK: 'FINISHING',
  POBD: 'DEFENSE', NAVS: 'DEFENSE', SWCH: 'DEFENSE', RIM: 'DEFENSE', STL: 'DEFENSE', BOX: 'DEFENSE', ORB: 'DEFENSE',
  OFFR: 'AWARENESS', DEFR: 'AWARENESS', SPC: 'AWARENESS', PLY: 'AWARENESS',
};

/** 身体组 members (体测 SPD/LAT/VJ — decline hits these, §3.2). */
export const BODY_PHYSICAL_KEYS = ['SPD', 'LAT', 'VJ'] as const;

// ─── 3.2 age curves ─────────────────────────────────────────────────────────

export interface AgeCurve {
  readonly group: 'BODY' | 'SHOOTING' | 'FINISHING' | 'TECHNIQUE' | 'DEFENSE' | 'AWARENESS';
  readonly growthEnd: number;
  readonly declineStart: number;
  /** [age, rate] pairs — rate applies from age onward. */
  readonly declineRates: ReadonlyArray<{ min: number; rate: number }>;
}

export const AGE_CURVES: readonly AgeCurve[] = [
  { group: 'BODY', growthEnd: 23, declineStart: 29, declineRates: [{ min: 29, rate: -2 }, { min: 33, rate: -4 }] },
  { group: 'SHOOTING', growthEnd: 27, declineStart: 34, declineRates: [{ min: 34, rate: -1 }] },
  { group: 'FINISHING', growthEnd: 25, declineStart: 30, declineRates: [{ min: 30, rate: -2 }] },
  { group: 'TECHNIQUE', growthEnd: 28, declineStart: 34, declineRates: [{ min: 34, rate: -1 }] },
  { group: 'DEFENSE', growthEnd: 27, declineStart: 32, declineRates: [{ min: 32, rate: -1.5 }] },
  { group: 'AWARENESS', growthEnd: 30, declineStart: 36, declineRates: [{ min: 36, rate: -1 }] },
];

// ─── 3.3 injuries ───────────────────────────────────────────────────────────

export const INJURY = {
  /** P = 2% × (80/DUR) × ageCoef × loadCoef. */
  base: 0.02,
  durFactor: 80,
  ageCoef: [
    { max: 27, coef: 1.0 },
    { max: 31, coef: 1.2 },
    { max: 40, coef: 1.5 },
  ] as const,
  /** 场均 >36 分钟 → 1.3. */
  loadMinutes: 36,
  loadCoef: 1.3,
  /** F > 20 → load 1.3 (§8.5). */
  fatigueThreshold: 20,
  fatigueLoadCoef: 1.3,
  grades: [
    { grade: 'MINOR', share: 0.7, games: { min: 1, max: 5 } },
    { grade: 'MODERATE', share: 0.25, games: { min: 6, max: 20 } },
    { grade: 'SEVERE', share: 0.05, games: null },
  ] as const,
  /** MODERATE season body penalty. */
  moderateBodyPenalty: -5,
  /** SEVERE random body penalty range. */
  severeBodyPenalty: { min: -15, max: -5 },
  /** SEVERE random POT cut range. */
  severePotCut: { min: -20, max: -10 },
} as const;

/** 伤病类型 → affected groups + POT targets (§3.3). Physical keys carry no POT (documented gap). */
export const INJURY_TYPES: Readonly<Record<InjuryType, { groups: readonly DataGroup[]; potTargets: readonly (AbilityKey | PhysicalKey)[] }>> = {
  KNEE_ANKLE_ACHILLES: { groups: ['BODY'], potTargets: ['SPD', 'LAT'] },
  BACK: { groups: ['BODY', 'FINISHING'], potTargets: ['VJ', 'FINC'] },
  HAND_WRIST: { groups: ['SHOOTING', 'TECHNIQUE'], potTargets: ['CS3', 'HND'] },
  LEG_FOOT: { groups: ['BODY'], potTargets: ['VJ', 'SPD'] },
};

// ─── 4.1 experience → awareness ─────────────────────────────────────────────

export const EXPERIENCE = {
  /** ΔAwareness/season = min(rounds/1000, 2.0) × ageCoef. */
  roundsPerPoint: 1000,
  maxPerSeason: 2.0,
  cap: 95,
  ageCoef: [
    { max: 25, coef: 1.0 },
    { max: 29, coef: 0.6 },
    { max: 40, coef: 0.2 },
  ] as const,
} as const;

// ─── 4.2 role immersion → tendency offsets ──────────────────────────────────

export interface ImmersionOffset {
  readonly tendency: TendencyKey;
  readonly delta: number;
  readonly capMax?: number;
  readonly capMin?: number;
}

export const IMMERSION: Readonly<Record<RoleId, readonly ImmersionOffset[]>> = {
  primary_creator: [
    { tendency: 'TAKEOVER', delta: 4, capMax: 80 },
    { tendency: 'PASS1ST', delta: 2, capMax: 70 },
  ],
  iso_scorer: [
    { tendency: 'TAKEOVER', delta: 3, capMax: 75 },
    { tendency: 'TMID', delta: 2 },
  ],
  secondary_creator: [
    { tendency: 'TAKEOVER', delta: 2, capMax: 65 },
    { tendency: 'TMID', delta: 3 },
  ],
  hub: [
    { tendency: 'PASS1ST', delta: 4, capMax: 80 },
    { tendency: 'TAKEOVER', delta: -2, capMin: 20 },
  ],
  post_scorer: [
    { tendency: 'TPOST', delta: 4 },
    { tendency: 'T3', delta: -4 },
  ],
  interior_finisher: [
    { tendency: 'TPOST', delta: 4 },
    { tendency: 'T3', delta: -4 },
  ],
  floor_spacer: [
    { tendency: 'T3', delta: 4 },
    { tendency: 'TAKEOVER', delta: -3, capMin: 15 },
  ],
  cutter: [
    { tendency: 'TDRIVE', delta: 3 },
    { tendency: 'T3', delta: -2 },
  ],
  movement_shooter: [
    { tendency: 'T3', delta: 4 },
    { tendency: 'TDRIVE', delta: -2 },
  ],
  transition_runner: [
    { tendency: 'PUSH', delta: 4, capMax: 85 },
    { tendency: 'TDRIVE', delta: 2 },
  ],
  poa_stopper: [{ tendency: 'FOUL', delta: -2 }],
  pest: [{ tendency: 'GAMBLE', delta: 2, capMax: 80 }],
  switch_wing: [],
  rotator: [
    { tendency: 'GAMBLE', delta: -3, capMin: 15 },
    { tendency: 'FOUL', delta: -2 },
  ],
  roamer: [{ tendency: 'GAMBLE', delta: 3 }],
  rim_anchor: [{ tendency: 'FOUL', delta: -2 }],
  post_defender: [{ tendency: 'FOUL', delta: 1 }],
};

export const IMMERSION_RULES = {
  /** 条件: role held a full season, role rounds ≥ 40% team rounds, Fit ≥65. */
  roleShare: 0.4,
  /** No immersion while bench (<20 games) or mismatch. */
  minGames: 20,
} as const;

// ─── 4.3 performance feedback ───────────────────────────────────────────────

export const PERFORMANCE_FEEDBACK = {
  /** 高光: ≥55 games and efficiency in team top 20% → training ×1.2. */
  highlightGames: 55,
  highlightTopPct: 0.2,
  highlightTrain: 1.2,
  /** 板凳/DNP: <20 games → training ×0.8, no experience, no immersion. */
  benchGames: 20,
  benchTrain: 0.8,
  /** 重伤复出季 → ×0.9. */
  recoveryTrain: 0.9,
} as const;

// ─── 4.4 mismatch ───────────────────────────────────────────────────────────

export const MISMATCH = {
  /** Fit <50 role held a full season → no immersion + training ×0.9. */
  trainMult: 0.9,
  /** 2 consecutive mismatch seasons → TAKEOVER −5 or PUSH −5 (confidence erosion). */
  consecutiveSeasons: 2,
  erosion: { takeover: -5, push: -5 },
} as const;

// ─── 7.2-7.4 prototypes ─────────────────────────────────────────────────────

export const PROTOTYPES: readonly PrototypeDef[] = [
  {
    id: 'ball_dominant', zhName: '持球大核', class: 'guard',
    keyAbilities: { OD3: 78, HND: 85, PNR: 82, PASM: 80, OFFR: 82 },
    tendencyFeatures: { TAKEOVER: 80, PASS1ST: 55, TDRIVE: 75 },
    share: 0.05,
    physical: { VJ: 75, SPD: 82, LAT: 78 },
    latFloor: 70,
    plyMean: 65,
  },
  {
    id: 'iso_scorer_p', zhName: '单打得分手', class: 'guard',
    keyAbilities: { PUM: 80, OD3: 74, FINS: 76, POST: 68 },
    // Volume pull-up three threats (Dame/Luka archetype): T3 must
    // survive the §1.4 sum-to-100 normalization — with unlisted shot
    // tendencies defaulting to 50, a raw T3 of 85 normalized to 36 and
    // no generated player ever crossed the ICE trigger (T3 > 55) or
    // the league's 3PT volume floor.
    tendencyFeatures: { TAKEOVER: 70, T3: 80, TMID: 45, TDRIVE: 45, TPOST: 10, PASS1ST: 30 },
    share: 0.08,
    physical: { VJ: 80, SPD: 80, LAT: 76 },
    plyMean: 65,
  },
  {
    id: 'playmaker', zhName: '组织后卫', class: 'guard',
    keyAbilities: { PASS: 84, OFFR: 84, POCK: 78, HND: 82 },
    tendencyFeatures: { PASS1ST: 80, TAKEOVER: 35 },
    share: 0.07,
    physical: { VJ: 70, SPD: 84, LAT: 78 },
    plyMean: 70,
  },
  {
    id: 'three_d', zhName: '3D 侧翼', class: 'wing',
    keyAbilities: { CS3: 82, POBD: 76, SWCH: 74, NAVS: 74 },
    // Same normalization fix as pure_shooter: T3 75 with the other shot
    // tendencies at the 50 default normalized to ~27 (75/275), which
    // produced zero volume shooters in generated rosters.
    tendencyFeatures: { T3: 75, TMID: 30, TDRIVE: 25, TPOST: 10, TAKEOVER: 25, GAMBLE: 40 },
    share: 0.12,
    physical: { VJ: 78, SPD: 80, LAT: 78 },
    plyMean: 60,
  },
  {
    id: 'pure_shooter', zhName: '纯射手', class: 'wing',
    keyAbilities: { CS3: 88, CSM: 76, SPC: 80, FT: 84 },
    // Pure shooters live beyond the arc: T3 85 / sum 140 → ~61% of
    // shots from three after normalization, with σ=10 noise carrying
    // some above the T3 > 55 ICE trigger.
    tendencyFeatures: { T3: 85, TMID: 25, TDRIVE: 20, TPOST: 10, TAKEOVER: 20 },
    share: 0.08,
    physical: { VJ: 70, SPD: 76, LAT: 70 },
    plyMean: 55,
  },
  {
    id: 'sixth_man', zhName: '乱战第六人', class: 'guard',
    keyAbilities: { OD3: 72, FINS: 74, HND: 74, OFFR: 50 },
    tendencyFeatures: { TAKEOVER: 65, T3: 70, TMID: 65, TDRIVE: 70, TPOST: 55, PASS1ST: 25 },
    share: 0.1,
    physical: { VJ: 78, SPD: 82, LAT: 76 },
    plyMean: 55,
  },
  {
    id: 'defensive_stopper', zhName: '防守尖兵', class: 'wing',
    keyAbilities: { POBD: 84, NAVS: 78, STL: 76, LAT: 80 },
    tendencyFeatures: { GAMBLE: 55, TAKEOVER: 20 },
    share: 0.08,
    physical: { VJ: 78, SPD: 80, LAT: 80 },
    latFloor: 70,
    plyMean: 55,
  },
  {
    id: 'roll_big_p', zhName: '吃饼中锋', class: 'big',
    keyAbilities: { DUNK: 82, FINC: 76, ORB: 76, SCRN: 74 },
    tendencyFeatures: { TPOST: 40, T3: 5 },
    share: 0.1,
    physical: { VJ: 82, SPD: 68, LAT: 60 },
    plyMean: 55,
  },
  {
    id: 'rim_protector_p', zhName: '护筐蓝领', class: 'big',
    keyAbilities: { RIM: 84, BOX: 78, DEFR: 70, DUNK: 72 },
    tendencyFeatures: { FOUL: 55, TAKEOVER: 15 },
    share: 0.08,
    physical: { VJ: 78, SPD: 66, LAT: 60 },
    plyMean: 55,
  },
  {
    id: 'stretch_big', zhName: '空间型内线', class: 'big',
    keyAbilities: { CS3: 74, RIM: 70, SPC: 68 },
    // T3 55 with unlisted 50s normalized to 30 — a stretch big that
    // rarely shoots threes. Raise volume so the archetype spaces the
    // floor as designed (~45% of shots from three).
    tendencyFeatures: { T3: 70, TPOST: 25, TMID: 35, TDRIVE: 25 },
    share: 0.06,
    physical: { VJ: 72, SPD: 68, LAT: 62 },
    plyMean: 55,
  },
  {
    id: 'post_big', zhName: '背身传统中锋', class: 'big',
    keyAbilities: { POST: 82, FINC: 76, WT: 120, BOX: 74 },
    tendencyFeatures: { TPOST: 75, T3: 3 },
    share: 0.05,
    physical: { VJ: 66, SPD: 60, LAT: 55 },
    plyMean: 55,
  },
  {
    id: 'jack_of_all', zhName: '全能工具人', class: 'wing',
    keyAbilities: {},
    tendencyFeatures: { T3: 50, TMID: 50, TDRIVE: 50, TPOST: 50, PASS1ST: 50, GAMBLE: 50, FOUL: 50, TAKEOVER: 50, PUSH: 50 },
    share: 0.13,
    physical: { VJ: 75, SPD: 75, LAT: 70 },
    plyMean: 60,
  },
];

export const PROTOTYPE_SHARES = PROTOTYPES.map((p) => p.share);

// ─── 7.3 POT generation ─────────────────────────────────────────────────────

export const POT = {
  ageBands: [
    { min: 18, max: 20, mu: 18, sigma: 8 },
    { min: 21, max: 23, mu: 12, sigma: 6 },
    { min: 24, max: 26, mu: 6, sigma: 4 },
    { min: 27, max: 40, mu: 2, sigma: 2 },
  ] as const,
  /** Prototype key abilities get POT +5. */
  keyBonus: 5,
  /** POT lower bound: max(gen+2, 40), clip 99. */
  minGap: 2,
  floor: 40,
  cap: 99,
} as const;

// ─── 7.4 physical generation ────────────────────────────────────────────────

export const PHYSICAL_GEN = {
  height: {
    guard: { min: 183, max: 198 },
    wing: { min: 195, max: 208 },
    big: { min: 205, max: 225 },
  } as const,
  heightNoiseSigma: 2,
  wsRatio: { mean: 1.05, sigma: 0.03, min: 0.98, max: 1.12 },
  wt: { offset: -100, sigma: 8, bigBonus: 10 },
  vjClip: { min: 50, max: 110 },
  spdLatClip: { min: 20, max: 99 },
  dur: { mean: 60, sigma: 15 },
  /** PLY sampled N(plyMean, 8); veterans add prior experience (§7.4 + 4.1). */
  plySigma: 8,
  /** Ability sampling noise N(0,5); unlisted abilities 55 ± 10. */
  abilitySigma: 5,
  abilityBase: 55,
  abilityBaseSigma: 10,
  abilityClip: { min: 20, max: 99 },
  /** Tendency sampling N(feature, 10), clip [5,95] (§7.5). */
  tendencySigma: 10,
  tendencyClip: { min: 5, max: 95 },
  /** Veteran PLY prior: assumed rounds/season for experience growth (⌛). */
  veteranRoundsPerSeason: 1500,
} as const;

/** 全能工具人: 全能力 62~68 均值 — flat high floor, minimal variance. */
export const UTILITY_ABILITY = { min: 62, max: 68 } as const;

// ─── 7.6 league ecosystem ───────────────────────────────────────────────────

export const LEAGUE = {
  teams: 30,
  rosterSize: 12,
  players: 360,
  draftPerYear: 60,
  draftAges: { min: 18, max: 22 },
  /** 生态约束: prototype 1 ≥1, prototype 10 ≥2 per draft class. */
  guarantee: { ballDominant: 1, stretchBig: 2 },
} as const;

// ─── 8 stamina ──────────────────────────────────────────────────────────────

export const STAMINA = {
  /** STM_max = max(50, 70 + 0.3×DUR − max(0, AGE−30)×2). */
  max: { base: 70, durFactor: 0.3, ageFrom: 30, ageRate: 2, floor: 50 },
  /** Per-minute consumption by role (§8.2). */
  perMinute: {
    primary_creator: 1.6, iso_scorer: 1.6, hub: 1.6,
    secondary_creator: 1.3, post_scorer: 1.3, pest: 1.3, poa_stopper: 1.3,
    movement_shooter: 1.1, cutter: 1.1, transition_runner: 1.1, roamer: 1.1,
    floor_spacer: 0.9, interior_finisher: 0.9, rim_anchor: 0.9, post_defender: 0.9, rotator: 0.9, switch_wing: 0.9,
  } as const,
  transitionMult: 1.2,
  pressMult: 1.5,
  /** Recovery. */
  restPerMinute: 2.0,
  quarterRest: 15,
  halftimeRest: 30,
  /** Fatigue accumulation. */
  fatigue: {
    /** F > 20 → injury load 1.3. */
    loadThreshold: 20,
    loadCoef: 1.3,
    /** Natural decay −8 per game. */
    decayPerGame: -8,
    /** Back-to-back second game: post-game recovery ×0.7. */
    backToBackRecovery: 0.7,
    /** avg >36 min for 10 games → F accumulation ×1.3. */
    heavyMinutes: 36,
    heavyGames: 10,
    heavyMult: 1.3,
  },
  /** 默认轮换: STM <35 自动请求换人. */
  subBelow: 35,
  /** §8.4 segment effects. */
  segments: [
    { stmMin: 60, exec: 1, awarenessPenalty: 0, injuryMult: 1, forcedSub: false },
    { stmMin: 40, execBase: 0.85, execSlope: 0.15, awarenessPenalty: 0, injuryMult: 1, forcedSub: false },
    { stmMin: 25, execBase: 0.85, execSlope: 0.15, awarenessPenalty: -5, injuryMult: 1, forcedSub: false },
    { stmMin: 0, execBase: 0.85, execSlope: 0.15, awarenessPenalty: -5, injuryMult: 1.5, forcedSub: true },
  ] as const,
} as const;

// ─── 9 morale ───────────────────────────────────────────────────────────────

export const MORALE = {
  base: 60,
  /** Weekly regression toward base: 10% of the gap. */
  weeklyRegression: 0.1,
  events: {
    WIN: 1, LOSS: -1,
    HIGHLIGHT: 3, LOWLIGHT: -2,
    DEMOTED_TO_BENCH: -8, PROMOTED_TO_STARTER: 5,
    DNP: -2, BENCHED_IN_CLUTCH: -3, SYSTEM_MISFIT: -5,
    CONTRACT_YEAR: 5, TRADE_RUMOR: -6,
    CHAMPIONSHIP: 15, PLAYOFF_EXIT: -5,
  } as Readonly<Record<MoraleEventId, number>>,
  /** 9.3 output bands (top row first). */
  bands: [
    { morMin: 75, exec: 1.03, train: 1.1, takeoverErosion: 0, demandsTrade: false },
    { morMin: 45, exec: 1.0, train: 1.0, takeoverErosion: 0, demandsTrade: false },
    { morMin: 25, exec: 0.97, train: 0.9, takeoverErosion: 0, demandsTrade: false },
    { morMin: 10, exec: 0.95, train: 1.0, takeoverErosion: -2, demandsTrade: false },
    { morMin: 0, exec: 1.0, train: 1.0, takeoverErosion: 0, demandsTrade: true },
  ] as const,
} as const;

// ─── 6.6 coach (consumed by growth/training) ────────────────────────────────

export const COACH_LEVELS = {
  capacity: { 1: 3, 2: 4, 3: 5, 4: 6, 5: 8 } as const,
  train: TRAINING.coachTrain,
} as const;

// ─── 10 event-stream consumption (engine catalog mapping) ───────────────────

/** Doc §10.2 POSSESSION_START maps to the engine's POSSESSION_GAINED event. */
export const CONSUME = {
  possessionEvent: 'POSSESSION_GAINED',
  /** 高光/低谷判定: 低谷 = FG% <30% 且出手 ≥10. */
  lowlightFg: 0.3,
  lowlightFga: 10,
} as const;

export type { PrototypeId };
