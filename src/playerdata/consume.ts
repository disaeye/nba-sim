/**
 * §10 事件流对接 — the four player-side consumption queries over the kernel's
 * event timeline. The doc lists these as the acceptance criteria for the
 * player systems; the engine's event catalog differs slightly, so
 * `POSSESSION_START` (doc) maps to `POSSESSION_GAINED` (engine catalog).
 *
 * All queries are pure over `TimelineEvent`-shaped events and deterministic.
 */
import { CONSUME } from './tables.js';

/** Structural event shape — TimelineEvent satisfies this. */
export interface EventLike {
  readonly type: string;
  readonly actors: readonly string[];
  readonly payload: Readonly<Record<string, unknown>>;
  readonly t_game: number;
}

export interface InitialLineups {
  readonly home: readonly string[];
  readonly away: readonly string[];
}

/** Elapsed seconds at a (period, t_game) pair (regulation 720s, OT 300s). */
export function elapsedSeconds(period: number, tGame: number): number {
  if (period <= 4) return (period - 1) * 720 + (720 - tGame);
  return 2880 + (period - 5) * 300 + (300 - tGame);
}

/**
 * §10.2 query 1 — 被观察回合数 n: count(actor=X 或 defender=X).
 * Feeds the scout σ formula (§1.6).
 */
export function observedRounds(events: readonly EventLike[], jersey: string): number {
  let n = 0;
  for (const e of events) {
    if (e.actors.includes(jersey)) n += 1;
  }
  return n;
}

/**
 * §10.2 query 2 — 赛季出场回合数: count of possessions where X was on court.
 * Feeds experience growth (§4.1).
 */
export function onCourtPossessions(events: readonly EventLike[], jersey: string, initial: InitialLineups): number {
  const home: string[] = [...initial.home];
  const away: string[] = [...initial.away];
  let count = 0;
  for (const e of events) {
    if (e.type === 'SUB') {
      const out = e.payload['player_out_id'];
      const inn = e.payload['player_in_id'];
      const team = e.payload['team'];
      if (typeof out === 'string' && typeof inn === 'string' && (team === 'home' || team === 'away')) {
        const arr = team === 'home' ? home : away;
        const idx = arr.indexOf(out);
        if (idx !== -1) arr[idx] = inn;
      }
      continue;
    }
    if (e.type === CONSUME.possessionEvent && (home.includes(jersey) || away.includes(jersey))) {
      count += 1;
    }
  }
  return count;
}

/**
 * §10.2 query 4 — 分钟负荷: on-court seconds accumulated from the event
 * timeline (mirrors box-score minutes inference). Feeds the injury load
 * coefficient (§3.3 / §8.5).
 */
export function minutesOnCourt(events: readonly EventLike[], jersey: string, initial: InitialLineups): number {
  const home: string[] = [...initial.home];
  const away: string[] = [...initial.away];
  let prevPeriod = 1;
  let base: number | null = null;
  let seconds = 0;
  for (const e of events) {
    let period = prevPeriod;
    if (e.type === 'PERIOD_START' || e.type === 'PERIOD_END') {
      const p = e.payload['period'];
      if (typeof p === 'number') period = p;
    }
    const elapsed = elapsedSeconds(period, e.t_game);
    if (base !== null && (home.includes(jersey) || away.includes(jersey))) {
      seconds += Math.max(0, elapsed - base);
    }
    if (e.type === 'SUB') {
      const out = e.payload['player_out_id'];
      const inn = e.payload['player_in_id'];
      const team = e.payload['team'];
      if (typeof out === 'string' && typeof inn === 'string' && (team === 'home' || team === 'away')) {
        const arr = team === 'home' ? home : away;
        const idx = arr.indexOf(out);
        if (idx !== -1) arr[idx] = inn;
      }
    }
    prevPeriod = period;
    base = elapsed;
  }
  return seconds / 60;
}

/** Per-player shot accumulator (mutable working state). */
interface ShotAcc {
  fgm: number;
  fga: number;
  pts: number;
}

export interface GameEfficiency {
  readonly jersey: string;
  readonly fgm: number;
  readonly fga: number;
  readonly pts: number;
  readonly pct: number;
  /** Rank within the team by points (ties: more attempts first, then jersey). */
  readonly rank: number;
}

/**
 * §10.2 query 3a — 单场效率聚合: FGM/FGA/PTS per player from SHOT_RESULT,
 * ranked within the supplied roster (team-internal ranking). Pass the
 * team's jersey list; `null` ranks everyone who attempted a shot.
 */
export function perGameEfficiency(events: readonly EventLike[], roster: readonly string[] | null): readonly GameEfficiency[] {
  const acc = new Map<string, ShotAcc>();
  const add = (jersey: string): ShotAcc => {
    let a = acc.get(jersey);
    if (a === undefined) {
      a = { fgm: 0, fga: 0, pts: 0 };
      acc.set(jersey, a);
    }
    return a;
  };
  const inRoster = (jersey: string): boolean => roster === null || roster.includes(jersey);
  for (const e of events) {
    if (e.type !== 'SHOT_RESULT') continue;
    const shooter = e.payload['shooter_id'];
    if (typeof shooter !== 'string' || !inRoster(shooter)) continue;
    const made = e.payload['made'] === true;
    const value = typeof e.payload['shot_value'] === 'number' ? (e.payload['shot_value'] as number) : 2;
    const a = add(shooter);
    a.fga += 1;
    if (made) {
      a.fgm += 1;
      a.pts += value;
    }
  }
  const rows: Array<{ jersey: string; fgm: number; fga: number; pts: number; pct: number }> = [];
  for (const [jersey, a] of acc) {
    rows.push({ jersey, fgm: a.fgm, fga: a.fga, pts: a.pts, pct: a.fga > 0 ? a.fgm / a.fga : 0 });
  }
  const sorted = [...rows].sort((a, b) => b.pts - a.pts || b.fga - a.fga || a.jersey.localeCompare(b.jersey));
  return sorted.map((row, i) => ({ ...row, rank: i + 1 }));
}

export interface HighlightJudgement {
  /** 效率队内前二 → HIGHLIGHT (+3). */
  readonly highlight: readonly string[];
  /** FG% <30% 且出手 ≥10 → LOWLIGHT (−2). */
  readonly lowlight: readonly string[];
}

/**
 * §10.2 query 3 — 高光/低谷场次判定, feeding the §9.2 morale events.
 */
export function highlightLowlight(eff: readonly GameEfficiency[]): HighlightJudgement {
  const highlight: string[] = [];
  const lowlight: string[] = [];
  for (const row of eff) {
    if (row.rank <= 2) highlight.push(row.jersey);
    if (row.fga >= CONSUME.lowlightFga && row.pct < CONSUME.lowlightFg) lowlight.push(row.jersey);
  }
  return { highlight, lowlight };
}
