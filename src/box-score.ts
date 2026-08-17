/**
 * Box-score aggregation — folds the event timeline into per-player
 * traditional counting stats.
 *
 * Walks the events array once, accumulating stats keyed by jersey string
 * (the identifier every event actor field carries). The roster maps
 * provide the playerId ↔ jersey ↔ teamId translation the PlayerBox
 * output requires.
 *
 * Stats aggregated (per config/event-catalog.json required_payload_fields):
 *   SHOT_RESULT → fga (+1 always); fgm, points (+shot_value on make);
 *                 tpa (+1 when shot_value=3); tpm (+1 on 3pt make)
 *   FT_RESULT   → fta (+1 always); ftm, points (+1 on make)
 *   REBOUND     → oreb (+1 offensive) or dreb (+1 defensive)
 *   STEAL       → stl (+1)
 *   TURNOVER    → tov (+1)
 *   FOUL        → pf  (+1)
 *
 * Not emitted by v0.1.0 plays (always 0): ast, blk. The event catalog
 * has no AST or BLK event types in the runner's action set; these
 * fields are carried for schema completeness and future expansion.
 *
 * Minutes: derived by walking the event timeline when `initialLineups`
 * is provided. Each interval between consecutive events is credited to
 * the 5 players per team who were on court during that interval; SUB
 * events swap players in/out from the following interval onward.
 * Without `initialLineups`, minutes defaults to 0 (the caller didn't
 * supply the data needed for an honest computation).
 */
import type { Event } from './state/types.js';

// ─── public types ───────────────────────────────────────────────────────────

/** Player identity used for roster lookups. */
export interface Player {
  readonly id: string;
  readonly jersey: string;
  readonly teamId: string;
}

/**
 * The canonical 18-field traditional NBA box score entry.
 * Schema-pinned by config/schemas/game-result.schema.json#$defs/playerBox.
 */
export interface PlayerBox {
  playerId: string;
  jersey: string;
  teamId: string;
  points: number;
  fgm: number;
  fga: number;
  tpm: number;
  tpa: number;
  ftm: number;
  fta: number;
  oreb: number;
  dreb: number;
  ast: number;
  tov: number;
  pf: number;
  stl: number;
  blk: number;
  minutes: number;
}

/** Per-team player box arrays. */
export interface BoxScore {
  home: PlayerBox[];
  away: PlayerBox[];
}

/** Roster maps: jersey → Player, one per team. */
export interface RosterMaps {
  home: ReadonlyMap<string, Player>;
  away: ReadonlyMap<string, Player>;
}

/** Optional context for minutes computation. */
export interface BoxScoreOptions {
  readonly initialLineups?: { readonly home: readonly string[]; readonly away: readonly string[] };
  readonly totalSeconds?: number;
}

// ─── accumulator ────────────────────────────────────────────────────────────

/** Mutable per-player accumulator. Built up during the event walk. */
interface Acc {
  points: number;
  fgm: number;
  fga: number;
  tpm: number;
  tpa: number;
  ftm: number;
  fta: number;
  oreb: number;
  dreb: number;
  ast: number;
  tov: number;
  pf: number;
  stl: number;
  blk: number;
  minutes: number;
}

function emptyAcc(): Acc {
  return { points: 0, fgm: 0, fga: 0, tpm: 0, tpa: 0, ftm: 0, fta: 0, oreb: 0, dreb: 0, ast: 0, tov: 0, pf: 0, stl: 0, blk: 0, minutes: 0 };
}

// ─── main entry point ───────────────────────────────────────────────────────

/**
 * Aggregate events into per-player box-score stats.
 *
 * @param events  the delta-encoded event timeline (in order).
 * @param rosters jersey → Player maps for both teams.
 * @param options optional initialLineups for minutes computation.
 */
export function computeBoxScore(
  events: readonly Event[],
  rosters: RosterMaps,
  options?: BoxScoreOptions,
): BoxScore {
  // One accumulator per jersey, keyed by the jersey string the events carry.
  const accs = new Map<string, Acc>();

  function acc(jersey: string): Acc {
    let a = accs.get(jersey);
    if (a === undefined) {
      a = emptyAcc();
      accs.set(jersey, a);
    }
    return a;
  }

  for (const e of events) {
    walkEvent(e, acc);
  }

  if (options?.initialLineups) {
    const minutes = computeMinutes(events, options.initialLineups, options.totalSeconds);
    for (const [jersey, sec] of minutes) {
      const a = accs.get(jersey) ?? emptyAcc();
      accs.set(jersey, a);
      a.minutes = Math.round(sec);
    }
  }

  return buildBoxScore(accs, rosters);
}

// ─── per-event aggregation ──────────────────────────────────────────────────

function walkEvent(e: Event, acc: (jersey: string) => Acc): void {
  switch (e.type) {
    case 'SHOT_RESULT': {
      const shooter = e.payload['shooter_id'];
      if (typeof shooter !== 'string') break;
      const a = acc(shooter);
      a.fga += 1;
      const made = e.payload['made'] === true;
      const value = e.payload['shot_value'];
      if (value === 3) a.tpa += 1;
      if (made) {
        a.fgm += 1;
        if (value === 3) a.tpm += 1;
        a.points += typeof value === 'number' ? value : 0;
        const assister = e.payload['assister_id'];
        if (typeof assister === 'string' && assister !== shooter) acc(assister).ast += 1;
      }
      break;
    }
    case 'FT_RESULT': {
      const shooter = e.payload['shooter_id'];
      if (typeof shooter !== 'string') break;
      const a = acc(shooter);
      a.fta += 1;
      if (e.payload['made'] === true) {
        a.ftm += 1;
        a.points += 1;
      }
      break;
    }
    case 'REBOUND': {
      const rebounder = e.payload['rebounder_id'];
      if (typeof rebounder !== 'string') break;
      const a = acc(rebounder);
      if (e.payload['offensive'] === true) a.oreb += 1;
      else a.dreb += 1;
      break;
    }
    case 'STEAL': {
      const stealer = e.payload['stealer_id'];
      if (typeof stealer !== 'string') break;
      acc(stealer).stl += 1;
      break;
    }
    case 'TURNOVER': {
      const player = e.payload['player_id'];
      if (typeof player !== 'string') break;
      acc(player).tov += 1;
      break;
    }
    case 'FOUL': {
      const offender = e.payload['offender_id'];
      if (typeof offender !== 'string') break;
      acc(offender).pf += 1;
      break;
    }
    default:
      // All other event types (PASS, HANDOFF, DRIVE, cosmetic markers,
      // phase transitions, etc.) carry no box-score counting stats.
      break;
  }
}

// ─── minutes ────────────────────────────────────────────────────────────────

/**
 * Walk the event timeline once, crediting each interval between
 * consecutive events to the 5-on-court jerseys per team. SUB events
 * swap players in/out from the following interval onward. The period
 * number is tracked from PERIOD_START/PERIOD_END payloads (period 1 by
 * default for events before the first PERIOD_START).
 *
 * Honesty invariants (I6):
 *   - sum(minutes) per team == 5 * totalSeconds  (5 on court at all times)
 *   - every jersey that appeared on court has minutes > 0
 *   - jersey that never appeared has minutes == 0
 */
function periodLength(period: number): number {
  return period >= 5 ? 300 : 720;
}

function gameElapsed(period: number, tGame: number): number {
  let elapsed = 0;
  for (let p = 1; p < period; p++) elapsed += periodLength(p);
  return elapsed + (periodLength(period) - tGame);
}

function computeMinutes(
  events: readonly Event[],
  initialLineups: { readonly home: readonly string[]; readonly away: readonly string[] },
  totalSeconds?: number,
): Map<string, number> {
  const earned = new Map<string, number>();
  const credit = (jersey: string, seconds: number): void => {
    if (seconds <= 0) return;
    earned.set(jersey, (earned.get(jersey) ?? 0) + seconds);
  };

  const homeCourt: string[] = [...initialLineups.home];
  const awayCourt: string[] = [...initialLineups.away];

  let prevPeriod = 1;
  let elapsedBase: number | null = null;
  let maxElapsed = 0;

  for (const e of events) {
    let currPeriod = prevPeriod;
    if (e.type === 'PERIOD_START' || e.type === 'PERIOD_END') {
      const p = e.payload['period'];
      if (typeof p === 'number') currPeriod = p;
    }

    const currElapsed = Math.max(0, gameElapsed(currPeriod, e.t_game));
    if (elapsedBase !== null) {
      const dt = currElapsed - elapsedBase;
      if (dt > 0) {
        for (const j of homeCourt) credit(j, dt);
        for (const j of awayCourt) credit(j, dt);
      }
    }

    if (e.type === 'SUB') {
      const out = e.payload['player_out_id'];
      const inn = e.payload['player_in_id'];
      const team = e.payload['team'];
      if (
        typeof out === 'string' &&
        typeof inn === 'string' &&
        (team === 'home' || team === 'away')
      ) {
        const arr = team === 'home' ? homeCourt : awayCourt;
        const idx = arr.indexOf(out);
        if (idx !== -1) arr[idx] = inn;
      }
    }

    prevPeriod = currPeriod;
    elapsedBase = currElapsed;
    maxElapsed = Math.max(maxElapsed, currElapsed);
  }

  const end = totalSeconds ?? maxElapsed;
  if (elapsedBase !== null && end > elapsedBase) {
    const tail = end - elapsedBase;
    for (const j of homeCourt) credit(j, tail);
    for (const j of awayCourt) credit(j, tail);
  } else if (elapsedBase === null && end > 0) {
    for (const j of [...homeCourt, ...awayCourt]) credit(j, end);
  }

  return earned;
}

// ─── build output ───────────────────────────────────────────────────────────

function buildBoxScore(accs: Map<string, Acc>, rosters: RosterMaps): BoxScore {
  const home: PlayerBox[] = [];
  const away: PlayerBox[] = [];

  for (const player of rosters.home.values()) {
    home.push(toPlayerBox(player, accs.get(player.jersey)));
  }
  for (const player of rosters.away.values()) {
    away.push(toPlayerBox(player, accs.get(player.jersey)));
  }

  return { home, away };
}

function toPlayerBox(player: Player, raw: Acc | undefined): PlayerBox {
  const a = raw ?? emptyAcc();
  return {
    playerId: player.id,
    jersey: player.jersey,
    teamId: player.teamId,
    points: a.points,
    fgm: a.fgm,
    fga: a.fga,
    tpm: a.tpm,
    tpa: a.tpa,
    ftm: a.ftm,
    fta: a.fta,
    oreb: a.oreb,
    dreb: a.dreb,
    ast: a.ast,
    tov: a.tov,
    pf: a.pf,
    stl: a.stl,
    blk: a.blk,
    minutes: a.minutes,
  };
}
