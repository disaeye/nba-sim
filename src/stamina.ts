/**
 * 体力系统内核接线 (§8 of docs/playerdata-design.md).
 *
 * The pure math lives in `playerdata/stamina.ts` (staminaMax, segment
 * effects, fatigue). This module owns the per-game lifecycle:
 *
 *   - STM_max from playerData (DUR/AGE) when present, else league defaults
 *     (DUR 60 / AGE 25 → 88).
 *   - Per-tick consumption while on court (rate by role, §8.2) and bench
 *     rest (+2.0/min, §8.3); quarter breaks +15, halftime +30.
 *   - The §8.4 exec factor feeds resolve and the decision EV.
 *   - STM < 35 → automatic substitution request; < 25 → forced-sub prompt
 *     (§8.6), surfaced as STATE_NOTE events.
 *   - Per-game fatigue F = max(0, consumed − rest) is reported for the
 *     season layer (§8.5 decay/back-to-back live there).
 *
 * Role rates: the doc's table is keyed by the 17-role catalog. Rosters that
 * carry `lineupIdentity` doc roles (generated rosters) use the doc rates
 * directly; the engine's TeamRole fallback maps handler ≈ 持球核心 1.6,
 * screener/spacers ≈ 内线终结/无球射手 0.9 (⌛ interpretation).
 */
import { staminaMax, staminaEffects, consumptionPerMinute } from './playerdata/stamina.js';
import { OFFENSE_ROLES } from './playerdata/tables.js';
import type { LineupPackage } from './identity/types.js';
import type { OffenseRoleId, RoleId } from './playerdata/types.js';
import type { GameInput, Player } from './sim-utils.js';

/** Live per-player stamina within one game. */
export interface LiveStamina {
  readonly max: number;
  stm: number;
  /** On-court consumption this game (fatigue input). */
  consumed: number;
  /** Bench/break recovery earned this game. */
  rest: number;
  /** STM < 35 request already emitted. */
  subRequested: boolean;
  /** STM < 25 forced-sub prompt already emitted. */
  forcedRequested: boolean;
}

export type StaminaMap = Readonly<Record<string, LiveStamina>>;

/** League defaults for rosters without playerData (⌛). */
export const DEFAULT_DUR = 60;
export const DEFAULT_AGE = 25;

/**
 * Playtest scale on the §8.2 consumption rates (⌛ in the doc). The doc's
 * rates combined with the §8.3 recovery (2.0/min bench + 15/30 breaks)
 * mathematically dominate — a 48-min game can never drive any player below
 * the 60-STM penalty line (net = (rate+2)·M − 171 > 0 needs M > 47.5 at
 * rate 1.6). ×1.6 makes long stints land in the 40-59 band and low-DUR
 * players reach sub-50 late in the game, while a normal rotation still
 * recovers between stints.
 */
export const STAMINA_PLAYTEST_SCALE = 1.68;

/** Engine TeamRole → §8.2 rate (⌛ interpretation; doc roles override). */
const TEAM_ROLE_RATE: Readonly<Record<string, number>> = {
  handler: 1.6, // 持球核心 tier
  screener: 0.9, // 内线终结 tier
  strong_corner: 0.9, // 无球射手 tier
  weak_corner: 0.9,
  slot: 0.9,
};

/** §8.2 transition-round multiplier. */
export const TRANSITION_MULT = 1.2;

/** Build the per-game stamina map from the rosters. */
export function initStamina(input: GameInput): StaminaMap {
  const out: Record<string, LiveStamina> = {};
  const add = (p: Player): void => {
    const dur = p.playerData?.physical.DUR ?? DEFAULT_DUR;
    const age = p.playerData?.physical.AGE ?? DEFAULT_AGE;
    const max = staminaMax(dur, age);
    out[p.jersey] = { max, stm: max, consumed: 0, rest: 0, subRequested: false, forcedRequested: false };
  };
  for (const p of input.home.roster) add(p);
  for (const p of input.away.roster) add(p);
  return out;
}

/** Doc role carried by a jersey's lineupIdentity assignment, if any. */
export function docRoleFor(jersey: string, pkg: LineupPackage | undefined): OffenseRoleId | null {
  const assignment = pkg?.lineupIdentity?.assignments.find((a) => a.jersey === jersey);
  const role = assignment?.roles[0];
  if (role !== undefined && role in OFFENSE_ROLES) return role as OffenseRoleId;
  return null;
}

/** §8.2 per-minute rate for one jersey, from doc role or engine TeamRole. */
export function consumptionRateFor(
  jersey: string,
  teamRole: string | undefined,
  pkg: LineupPackage | undefined,
  playerData?: Readonly<Record<string, import('./playerdata/types.js').PlayerData>>,
): number {
  const docRole = docRoleFor(jersey, pkg);
  const base = docRole !== null
    ? consumptionPerMinute(docRole as RoleId)
    : teamRole !== undefined && teamRole in TEAM_ROLE_RATE
      ? TEAM_ROLE_RATE[teamRole]!
      : 0.9;
  // P4.3: DUR (durability) modulates the base rate — a 90-DUR ironman
  // burns ~15% slower than a 50-DUR glass cannon; the staminaMax already
  // scales with DUR/AGE, this is the per-minute slope.
  const dur = playerData?.[jersey]?.physical.DUR;
  const durFactor = dur !== undefined ? Math.max(0.8, Math.min(1.2, 1 + (60 - dur) / 200)) : 1;
  return base * STAMINA_PLAYTEST_SCALE * durFactor;
}

export interface StaminaTickInput {
  readonly stamina: StaminaMap;
  readonly dt: number;
  /** jersey → per-minute rate for every on-court player. */
  readonly rates: Readonly<Record<string, number>>;
  /** Handler jersey during a TRANSITION-mode possession (×1.2). */
  readonly transitionHandler: string | null;
  /** On-court jerseys (bench players rest). */
  readonly onCourt: readonly string[];
}

/** One tick: on-court consumption + bench rest. Returns a new map. */
export function tickStamina(input: StaminaTickInput): StaminaMap {
  const out: Record<string, LiveStamina> = {};
  const minutes = input.dt / 60;
  for (const [jersey, st] of Object.entries(input.stamina)) {
    let stm = st.stm;
    let consumed = st.consumed;
    let rest = st.rest;
    if (input.onCourt.includes(jersey)) {
      let rate = input.rates[jersey] ?? 0.9;
      if (input.transitionHandler !== null && jersey === input.transitionHandler) rate *= TRANSITION_MULT;
      const spend = rate * minutes;
      stm = Math.max(0, stm - spend);
      consumed += spend;
    } else {
      const gain = 2.0 * minutes;
      stm = Math.min(st.max, stm + gain);
      rest += gain;
    }
    out[jersey] = { ...st, stm, consumed, rest };
  }
  return out;
}

/**
 * §8.3 quarter break +5 / halftime +15 (⌛ recalibration). The doc's +15/+30
 * combined with the 2.0/min bench rest mathematically dominate any single
 * game's consumption (a 48-minute ironman spends ~69 at spacer rates but
 * banks 75 from breaks alone), which made the §8.6 sub thresholds (<35/<25)
 * unreachable — dead UI. The recalibrated breaks keep normal rotations
 * fresh while a full-game player crosses 35 late in Q4.
 */
export function applyPeriodRest(stamina: StaminaMap, halftime: boolean): StaminaMap {
  const bonus = halftime ? 15 : 5;
  const out: Record<string, LiveStamina> = {};
  for (const [jersey, st] of Object.entries(stamina)) {
    out[jersey] = { ...st, stm: Math.min(st.max, st.stm + bonus), rest: st.rest + bonus };
  }
  return out;
}

/** §8.4 exec factor for resolve/EV pricing. */
export function staminaFactor(stm: number): number {
  return staminaEffects(stm).exec;
}

export interface SubRequest {
  readonly jersey: string;
  readonly team: 'home' | 'away';
  readonly stm: number;
  /** 'stamina_sub_request' (<35, §8.6) or 'stamina_forced_sub' (<25, §8.4). */
  readonly reason: 'stamina_sub_request' | 'stamina_forced_sub';
}

/** Newly crossed sub thresholds since the last tick (each fires once). */
export function detectSubRequests(
  stamina: StaminaMap,
  onCourt: readonly string[],
  teamOf: Readonly<Record<string, 'home' | 'away'>>,
): readonly SubRequest[] {
  const requests: SubRequest[] = [];
  for (const jersey of onCourt) {
    const st = stamina[jersey];
    if (!st) continue;
    if (st.stm < 35 && !st.subRequested) {
      requests.push({ jersey, team: teamOf[jersey] ?? 'home', stm: st.stm, reason: 'stamina_sub_request' });
    }
    if (st.stm < 25 && !st.forcedRequested) {
      requests.push({ jersey, team: teamOf[jersey] ?? 'home', stm: st.stm, reason: 'stamina_forced_sub' });
    }
  }
  return requests;
}

/** §8.5 in-game fatigue accumulation F = max(0, consumed − rest). */
export function fatigueReport(stamina: StaminaMap): Readonly<Record<string, { max: number; end: number; consumed: number; rest: number; fatigue: number }>> {
  const out: Record<string, { max: number; end: number; consumed: number; rest: number; fatigue: number }> = {};
  for (const [jersey, st] of Object.entries(stamina)) {
    out[jersey] = {
      max: st.max,
      end: Math.round(st.stm * 10) / 10,
      consumed: Math.round(st.consumed * 10) / 10,
      rest: Math.round(st.rest * 10) / 10,
      fatigue: Math.max(0, Math.round((st.consumed - st.rest) * 10) / 10),
    };
  }
  return out;
}
