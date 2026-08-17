/**
 * Player data types — the layered data system defined by
 * `docs/playerdata-design.md` (v1.3a, sole authority).
 *
 * Five layers, locked terminology:
 *   体测 Physical   (8 items, layer 0 — public except DUR)
 *   能力 Ability    (24 items, layer 1 — hidden)
 *   倾向 Tendency   (9 items, layer 1 — hidden)
 *   判断 Awareness  (4 items, layer 1 — hidden)
 *   属性 Attribute  (7 items, layer 2 — visible, grade bands)
 *
 * Data flows one-way: Physical → Ability/Tendency/Awareness → game events →
 * observation model → Attribute. Player operations only ever touch the lower
 * layers; the display layer is read-only.
 */
import type { Rng } from '../rng/types.js';

// ─── closed-set unions ──────────────────────────────────────────────────────

/** 体测 items — symbols locked by §1.2. */
export type PhysicalKey = 'H' | 'WT' | 'WS' | 'VJ' | 'SPD' | 'LAT' | 'AGE' | 'DUR';

/** 能力 items — 24, symbols locked by §1.3. */
export type AbilityKey =
  | 'CS3' | 'OD3' | 'CSM' | 'PUM' | 'FLT' | 'POST' | 'FT'
  | 'FINS' | 'FINW' | 'FINC' | 'DUNK' | 'HND' | 'PNR' | 'PASS'
  | 'PASM' | 'POCK' | 'SCRN' | 'POBD' | 'NAVS' | 'SWCH' | 'RIM'
  | 'STL' | 'BOX' | 'ORB';

/** 倾向 items — 9, symbols locked by §1.4. */
export type TendencyKey =
  | 'T3' | 'TMID' | 'TDRIVE' | 'TPOST' | 'PASS1ST' | 'GAMBLE' | 'FOUL' | 'TAKEOVER' | 'PUSH';

/** 判断 items — 4, symbols locked by §1.5. */
export type AwarenessKey = 'OFFR' | 'DEFR' | 'SPC' | 'PLY';

/** 属性 (display layer) — 7, aggregated from layer-1 truths via §1.6. */
export type AttributeKey =
  | 'SHOOTING'      // 投射
  | 'FINISHING'     // 终结
  | 'PLAYMAKING'    // 组织
  | 'PERIMETER_D'   // 外线防守
  | 'INTERIOR_D'    // 内线防守
  | 'REBOUNDING'    // 篮板
  | 'ATHLETICISM';  // 运动能力

/** Grade bands — S=90+, A=85~89, A-=82~84, B+=78~81, B=74~77, B-=70~73, C+=66~69, C=62~65, D=55~61, F<55 (§1.6). */
export type Grade = 'S' | 'A' | 'A-' | 'B+' | 'B' | 'B-' | 'C+' | 'C' | 'D' | 'F';

// ─── role catalogs (§2) ─────────────────────────────────────────────────────

/** 进攻角色 pool — 10, §2.2. */
export type OffenseRoleId =
  | 'primary_creator'    // 持球核心
  | 'iso_scorer'         // 单打得分手
  | 'secondary_creator'  // 副攻手
  | 'hub'                // 组织枢纽
  | 'post_scorer'        // 背身轴心
  | 'interior_finisher'  // 内线终结
  | 'floor_spacer'       // 无球射手
  | 'cutter'             // 空切终结
  | 'movement_shooter'   // 跑动射手
  | 'transition_runner'; // 转换箭头

/** 防守角色 pool — 7, §2.3. */
export type DefenseRoleId =
  | 'poa_stopper'    // 盯人箭头
  | 'pest'           // 纠缠者
  | 'switch_wing'    // 换防摇摆
  | 'rotator'        // 纪律轮转者
  | 'roamer'         // 协防扫荡
  | 'rim_anchor'     // 护筐锚
  | 'post_defender'; // 顶防工兵

/** Any role id (offense or defense). */
export type RoleId = OffenseRoleId | DefenseRoleId;

// ─── layer-1 value sets ─────────────────────────────────────────────────────

/** 能力层 base values, 20..99 (before physical correction of §1.3). */
export type AbilitySet = Readonly<Record<AbilityKey, number>>;

/** 倾向层, 0..99. Four shot tendencies always sum to 100 after validation (§1.4). */
export type TendencySet = Readonly<Record<TendencyKey, number>>;

/** 判断层, 20..99. */
export type AwarenessSet = Readonly<Record<AwarenessKey, number>>;

/** POT (potential ceiling, hidden): one per ability, 40..99, gen-time fixed (§7.3). */
export type PotSet = Readonly<Record<AbilityKey, number>>;

// ─── layer-0 physical ───────────────────────────────────────────────────────

/** 体测, §1.2. Ranges: H 175..230 cm, WT 70..140 kg, VJ 50..110 cm, SPD/LAT 20..99, AGE 18..40, DUR 20..99. */
export interface Physical {
  readonly H: number;   // cm
  readonly WT: number;  // kg
  readonly WS: number;  // cm
  readonly VJ: number;  // cm
  readonly SPD: number;
  readonly LAT: number;
  readonly AGE: number;
  readonly DUR: number; // hidden
}

/** Normalized physical quantities used inside formulas (§1.2). */
export interface NormalizedPhysical {
  readonly H_n: number;
  readonly WT_n: number;
  readonly VJ_n: number;
  readonly WS_n: number;
}

// ─── the full player record ─────────────────────────────────────────────────

/** Complete layered data for one player (真值 — simulation truth, not observed). */
export interface PlayerData {
  readonly physical: Physical;
  /** Base ability values; effective values add the §1.3 physical correction at read time. */
  readonly ability: AbilitySet;
  readonly pot: PotSet;
  readonly tendency: TendencySet;
  readonly awareness: AwarenessSet;
}

// ─── scouting / observation (§1.6-1.8) ──────────────────────────────────────

/** Scout tier, L1..L5. k_scout = 1.5 / 1.2 / 1.0 / 0.8 / 0.6. */
export type ScoutLevel = 1 | 2 | 3 | 4 | 5;

/** Observation-sample record: how many rounds a player was watched. */
export interface ObservationState {
  /** n = observed rounds, capped at 2000 for σ purposes. */
  readonly n: number;
  readonly scoutLevel: ScoutLevel;
}

/** Which hidden quantity an observation error attaches to. */
export type ObservedKind = 'attribute' | 'tendency' | 'awareness';

/** Observed view of a player — every value carries measurement error. */
export interface ObservedPlayer {
  readonly physical: Physical;
  readonly attribute: Readonly<Record<AttributeKey, number>>;
  readonly tendency: TendencySet;
  readonly awareness: AwarenessSet;
}

// ─── scout report (§1.7) ────────────────────────────────────────────────────

export interface AttributeReportRow {
  readonly key: AttributeKey;
  readonly trueValue: number;
  readonly shownValue: number;
  readonly grade: Grade;
  /** Present when σ ≥ 8 (or L5 halved σ ≥ 8): display as a range. */
  readonly range: { readonly low: Grade; readonly high: Grade } | null;
  /** 5-cell confidence bar fill count, 1..5. */
  readonly confidence: number;
}

export interface FitReportRow {
  readonly role: RoleId;
  readonly observedFit: number;
  /** 球探报告角色文案 per §5.3: ≥80 能胜任 / 65~79 可摇摆至 / <50 不出现. */
  readonly label: string;
}

export interface ScoutReport {
  /** 体测区 — exact values. */
  readonly physical: Physical;
  /** 能力档位区 — grades + confidence bars (L1: wide range presentation). */
  readonly attributes: readonly AttributeReportRow[];
  /** 风格描述区 — 3..5 lines from Tendency_obs (unlocked at L3). */
  readonly styles: readonly string[];
  /** 稀有组合文案. */
  readonly rareStyles: readonly string[];
  /** 角色适配区 — observed Fit (unlocked at L3). */
  readonly roleFit: readonly FitReportRow[];
  /** 样本区 — "已观察 N 回合". */
  readonly sampleRounds: number;
  /** Effective presentation σ used for the grade display (L5 halves it). */
  readonly sigma: number;
}

// ─── fit (§5) ───────────────────────────────────────────────────────────────

/** Fit bands (§5.1): ≥80 天然契合 / 65~79 可胜任 / 50~64 勉强 / <50 错位. */
export type FitBand = 'NATURAL' | 'CAPABLE' | 'MARGINAL' | 'MISMATCH';

export interface FitResult {
  readonly role: RoleId;
  /** 0..100. */
  readonly value: number;
  /** Ability-shortfall aggregate f (0..1 scale). */
  readonly f: number;
  /** Tendency-gate product g (0..1). */
  readonly g: number;
  readonly band: FitBand;
}

/**
 * One gate in a role's Fit config. `min` is a ≥ gate, `max` a ≤ gate;
 * both present = band gate. Exactly one of `tendency` / `physical` is set.
 * `pairedWith` (换防摇摆) makes the gate read min(physical, pairedWith) ≥ min.
 */
export interface FitGate {
  readonly tendency?: TendencyKey;
  /** Physical gate (LAT / H_n / WT_n — see §5.2 防守角色). */
  readonly physical?: FitWeightSource;
  readonly pairedWith?: FitWeightSource;
  readonly min?: number;
  readonly max?: number;
}

/**
 * f-weight entries may reference abilities, awareness, or physical
 * quantities (SPD/LAT raw, *_n normalized).
 */
export type FitWeightSource = AbilityKey | 'OFFR' | 'DEFR' | 'SPC' | 'SPD' | 'LAT' | 'H_n' | 'WT_n' | 'VJ_n' | 'WS_n';

export interface FitConfig {
  readonly fWeights: ReadonlyArray<{ readonly source: FitWeightSource; readonly weight: number }>;
  readonly gates: readonly FitGate[];
}

// ─── chemistry (§2.4) ───────────────────────────────────────────────────────

/** Stable channel names for chemistry modifiers — decision-tree branch channels (§2.4). */
export type ChemistryChannel =
  | 'help_recovery'      // 协防到位概率
  | 'dime_cut'           // 喂饼分支
  | 'relief_target'      // 接应权重
  | 'cut_target'         // 切入目标权重
  | 'move_catch'         // 移动接球分支频率
  | 'iso_space'          // ISO 空间判定
  | 'spacer_open'        // 射手空位概率
  | 'recovery_penalty'   // 失位惩罚
  | 'initiation'         // 发起权重
  | 'paint_clog'         // 油漆区拥堵（空间判定）
  | 'vacuum_penalty'     // 防线真空
  | 'initiation_eff';    // 发起效率折扣

export interface ChemistryEffect {
  /** Stable id, e.g. 'PC×FS'. */
  readonly id: string;
  /** The roles involved (one id when the rule is count-based). */
  readonly roles: readonly RoleId[];
  readonly channel: ChemistryChannel;
  /** Multiplicative modifier: 1.1 / 0.9 / 0.85 etc. */
  readonly modifier: number;
  readonly note: string;
}

// ─── possession budget (§2.2) ───────────────────────────────────────────────

export interface PossessionShare {
  /** Index into the input roles array. */
  readonly index: number;
  readonly role: OffenseRoleId;
  readonly fit: number;
  /** Stage 1: demand / Σdemand. */
  readonly nominal: number;
  /** Stage 2: after conflict compression, renormalized to 0.92 total. */
  readonly actual: number;
  /** True when a conflict compression reduced this share below nominal. */
  readonly compressed: boolean;
}

// ─── prototype catalog (§7.2) ───────────────────────────────────────────────

export type PrototypeClass = 'guard' | 'wing' | 'big';

export type PrototypeId =
  | 'ball_dominant'   // 持球大核
  | 'iso_scorer_p'    // 单打得分手
  | 'playmaker'       // 组织后卫
  | 'three_d'         // 3D 侧翼
  | 'pure_shooter'    // 纯射手
  | 'sixth_man'       // 乱战第六人
  | 'defensive_stopper' // 防守尖兵
  | 'roll_big_p'      // 吃饼中锋
  | 'rim_protector_p' // 护筐蓝领
  | 'stretch_big'     // 空间型内线
  | 'post_big'        // 背身传统中锋
  | 'jack_of_all';    // 全能工具人

export interface PrototypeDef {
  readonly id: PrototypeId;
  readonly zhName: string;
  readonly class: PrototypeClass;
  /**
   * Key ability means — sampled with N(0,5) noise; also get POT +5 (§7.3).
   * The doc's 关键能力均值 column also lists awareness items (OFFR/DEFR/SPC)
   * and physical items (WT, LAT) — those drive their own sampling paths.
   */
  readonly keyAbilities: Readonly<Partial<Record<AbilityKey | 'OFFR' | 'DEFR' | 'SPC' | 'WT' | 'LAT', number>>>;
  /** Tendency feature means — sampled with N(0,10) noise (§7.5). */
  readonly tendencyFeatures: Readonly<Partial<Record<TendencyKey, number>>>;
  /** 联盟占比 (§7.2). */
  readonly share: number;
  /** VJ/SPD/LAT means (§7.4, interpretation ⌛). */
  readonly physical: { readonly VJ: number; readonly SPD: number; readonly LAT: number };
  /** LAT floor (尖兵/大核 LAT≥70, §7.4). */
  readonly latFloor?: number;
  /** PLY mean by prototype (§7.4). */
  readonly plyMean: number;
}

// ─── stamina (§8) ───────────────────────────────────────────────────────────

export interface StaminaEffects {
  /** Execution multiplier from the §8.4 segment function. */
  readonly exec: number;
  /** OFFR/DEFR penalty at 25..39 STM (0 otherwise). */
  readonly awarenessPenalty: number;
  /** Injury probability multiplier (<25 → 1.5, else 1). */
  readonly injuryMult: number;
  /** <25 → coach gets a forced-substitution prompt. */
  readonly forcedSub: boolean;
}

// ─── morale (§9) ────────────────────────────────────────────────────────────

/** Morale input events — full enumeration of §9.2. */
export type MoraleEventId =
  | 'WIN' | 'LOSS'
  | 'HIGHLIGHT' | 'LOWLIGHT'
  | 'DEMOTED_TO_BENCH' | 'PROMOTED_TO_STARTER'
  | 'DNP' | 'BENCHED_IN_CLUTCH' | 'SYSTEM_MISFIT'
  | 'CONTRACT_YEAR' | 'TRADE_RUMOR'
  | 'CHAMPIONSHIP' | 'PLAYOFF_EXIT';

export interface MoraleEffects {
  /** Execution multiplier per §9.3 band. */
  readonly exec: number;
  /** Training-efficiency multiplier per §9.3 band. */
  readonly train: number;
  /** TAKEOVER erosion (−2/month) at 10..24 MOR, 0 otherwise. */
  readonly takeoverErosion: number;
  /** 0..9 MOR: public trade demand + refuses extension. */
  readonly demandsTrade: boolean;
}

// ─── growth & season systems (§3-4) ────────────────────────────────────────

/** Data groups of the trainability / age-curve tables (§3.1-3.2). */
export type DataGroup =
  | 'SHOOTING'   // 投篮组
  | 'TECHNIQUE'  // 技术组
  | 'FINISHING'  // 终结组
  | 'DEFENSE'    // 防守组
  | 'AWARENESS'  // 判断组
  | 'BODY'       // 身体组 (体测)
  | 'TENDENCY'   // 倾向
  | 'PHYSICAL';  // 体测（全部）

/** 伤病类型 (§3.3). */
export type InjuryType = 'KNEE_ANKLE_ACHILLES' | 'BACK' | 'HAND_WRIST' | 'LEG_FOOT';

/** 伤病等级 (§3.3). */
export type InjuryGrade = 'MINOR' | 'MODERATE' | 'SEVERE';

export interface InjuryResult {
  readonly grade: InjuryGrade;
  /** Games missed (SEVERE = season, represented as Infinity). */
  readonly gamesOut: number;
  /** Data groups hit by this injury. */
  readonly groups: readonly DataGroup[];
  /** POT targets (per type table; physical keys carry no POT — noted in tables). */
  readonly potTargets: readonly (AbilityKey | PhysicalKey)[];
  /** Season body penalty (MODERATE: −5; SEVERE: −5..15 random). */
  readonly bodyPenalty: number;
  /** SEVERE only: permanent POT cut −10..20 (random). */
  readonly potCut: number;
}

export { Rng };
