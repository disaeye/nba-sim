/**
 * Per-event-type mutation handlers + payload readers.
 *
 * Each handler has signature `(state, event) => state'` and is pure: it
 * reads `event.payload` (typed by the catalog's `required_payload_fields`)
 * and returns a NEW `GameState` with the affected fields replaced. The
 * dispatch table that calls these lives in `apply.ts`.
 *
 * Payload readers (`str`, `num`, `bool`, `team`) are the parse-don't-
 * validate boundary: one `as <literal-type>` cast per read, justified by
 * the catalog's required-field contract. Interior code receives typed
 * values.
 */
import type { Clocks } from '../clocks/types.js';
import type { Event, GameState, Phase, TeamId } from './types.js';
import { StateError } from './types.js';

// ─── payload readers (boundary casts) ───────────────────────────────────────

export function str(e: Event, key: string): string {
  return e.payload[key] as string;
}

export function num(e: Event, key: string): number {
  return e.payload[key] as number;
}

export function bool(e: Event, key: string): boolean {
  return e.payload[key] as boolean;
}

export function team(e: Event, key: string): TeamId {
  return e.payload[key] as TeamId;
}

export function otherTeam(t: TeamId): TeamId {
  return t === 'home' ? 'away' : 'home';
}

/**
 * Best-effort team lookup by player id from the current lineups. Used by
 * STEAL where the catalog payload lacks the winning team directly.
 * Returns `null` when the player is not on court (caller supplies a
 * default).
 */
export function teamFromActor(state: GameState, playerId: string): TeamId | null {
  if (state.lineups.home.includes(playerId)) return 'home';
  if (state.lineups.away.includes(playerId)) return 'away';
  return null;
}

// ─── scoring ────────────────────────────────────────────────────────────────

/** SHOT_RESULT: on make, increment the shooting team's score by shot_value. */
export function applyShotResult(state: GameState, event: Event): GameState {
  if (!bool(event, 'made')) return state;
  const value = num(event, 'shot_value');
  const shooting = shootingTeam(state, str(event, 'shooter_id'));
  if (shooting === null) return state;
  const score = shooting === 'home'
    ? { ...state.score, home: state.score.home + value }
    : { ...state.score, away: state.score.away + value };
  return { ...state, score };
}

/** FT_RESULT: on make, increment the shooting team's score by 1. */
export function applyFtResult(state: GameState, event: Event): GameState {
  if (!bool(event, 'made')) return state;
  const shooting = shootingTeam(state, str(event, 'shooter_id'));
  if (shooting === null) return state;
  const score = shooting === 'home'
    ? { ...state.score, home: state.score.home + 1 }
    : { ...state.score, away: state.score.away + 1 };
  return { ...state, score };
}

/**
 * Resolve the shooting team. Catalog payloads carry only `shooter_id`,
 * never a team field; we infer from the live lineup first (the shooter is
 * on their own team's court) and fall back to live possession if the
 * shooter id is somehow absent from the lineups. Returns `null` only
 * when neither signal resolves — a kernel invariant break upstream.
 */
function shootingTeam(state: GameState, shooterId: string): TeamId | null {
  return teamFromActor(state, shooterId) ?? state.possession.team;
}

// ─── possession / ball movement ─────────────────────────────────────────────

/** POSSESSION_GAINED: set team, holder, and reset the shot clock to 24. */
export function applyPossessionGained(state: GameState, event: Event): GameState {
  const t = team(event, 'team');
  const holderId = str(event, 'player_id');
  return {
    ...state,
    possession: { team: t },
    ball: { ...state.ball, holderId },
    clocks: { ...state.clocks, shot: 24.0 },
  };
}

/**
 * REBOUND: holder becomes the rebounder. Defensive board flips
 * possession to the opposite team; offensive board keeps it.
 */
export function applyRebound(state: GameState, event: Event): GameState {
  const rebounder = str(event, 'rebounder_id');
  const t = team(event, 'team');
  const offensive = bool(event, 'offensive');
  return {
    ...state,
    ball: { ...state.ball, holderId: rebounder },
    possession: { team: offensive ? t : otherTeam(t) },
  };
}

/** STEAL: stealer becomes holder; possession flips to the stealer's team. */
export function applySteal(state: GameState, event: Event): GameState {
  const stealer = str(event, 'stealer_id');
  const victim = str(event, 'victim_id');
  const winner = teamFromActor(state, stealer) ?? otherTeam(teamFromActor(state, victim) ?? 'home');
  return {
    ...state,
    ball: { ...state.ball, holderId: stealer },
    possession: { team: winner },
  };
}

/** TURNOVER: clear the failed holder, except a STEAL chain already transferred control. */
export function applyTurnover(state: GameState, event: Event): GameState {
  const t = team(event, 'team');
  const stealer = event.payload['stealer_id'];
  if (typeof stealer === 'string') {
    return {
      ...state,
      ball: { ...state.ball, holderId: stealer, status: 'held' },
      possession: { team: otherTeam(t) },
    };
  }
  return {
    ...state,
    ball: { ...state.ball, holderId: null, status: 'loose' },
    possession: { team: otherTeam(t) },
  };
}

/** JUMP_BALL_TAP: tapping_team gains possession; tapper becomes holder if known. */
export function applyJumpBallTap(state: GameState, event: Event): GameState {
  const tappingTeam = team(event, 'tapping_team');
  // tapper_id is optional per catalog; only set holder when present.
  const tapper = event.payload['tapper_id'];
  const holderId = typeof tapper === 'string' ? tapper : state.ball.holderId;
  return {
    ...state,
    ball: { ...state.ball, holderId },
    possession: { team: tappingTeam },
  };
}

// ─── fouls, lineups, period, timeouts, clocks ──────────────────────────────

/** FOUL: increment per-player and per-team counters; set bonus at 5+. */
export function applyFoul(state: GameState, event: Event): GameState {
  const offender = str(event, 'offender_id');
  const t = team(event, 'offender_team');
  const prevPlayerCount = state.fouls.players[offender] ?? 0;
  const prevTeamCount = t === 'home' ? state.fouls.team.home : state.fouls.team.away;
  const newTeamCount = prevTeamCount + 1;
  return {
    ...state,
    fouls: {
      team:
        t === 'home'
          ? { ...state.fouls.team, home: newTeamCount }
          : { ...state.fouls.team, away: newTeamCount },
      players: { ...state.fouls.players, [offender]: prevPlayerCount + 1 },
      // NBA: 5th team foul in a period triggers the bonus.
      bonus:
        t === 'home'
          ? { ...state.fouls.bonus, home: newTeamCount >= 5 }
          : { ...state.fouls.bonus, away: newTeamCount >= 5 },
    },
  };
}

/** PERIOD_END: advance period, zero out game clock, mark DEAD_PERIOD_END. */
export function applyPeriodEnd(state: GameState, event: Event): GameState {
  const endedPeriod = num(event, 'period');
  const nextPeriod = endedPeriod + 1;
  const clocks: Clocks = {
    period: nextPeriod,
    game: 0,
    shot: state.clocks.shot,
  };
  return {
    ...state,
    period: nextPeriod,
    clocks,
    phase: 'DEAD_PERIOD_END',
  };
}

/** SUB: swap player_out for player_in in the named team's lineup. */
export function applySub(state: GameState, event: Event): GameState {
  const out = str(event, 'player_out_id');
  const inn = str(event, 'player_in_id');
  const t = team(event, 'team');
  const current = t === 'home' ? state.lineups.home : state.lineups.away;
  const idx = current.indexOf(out);
  if (idx === -1) {
    throw new StateError(
      'STATE_SUB_TARGET_NOT_IN_LINEUP',
      'SUB',
      `SUB: player_out_id "${out}" is not in ${t} lineup [${current.join(', ')}]`,
    );
  }
  // Rebuild the array so callers observing the prior lineup array
  // reference are unaffected.
  const rebuilt = [...current];
  rebuilt[idx] = inn;
  const lineups =
    t === 'home'
      ? { ...state.lineups, home: rebuilt }
      : { ...state.lineups, away: rebuilt };
  // P5.x: keep the pose map in sync — the outgoing player's pose must
  // not linger (11+ player frames), and the incoming player needs a
  // placeholder pose immediately (missing poses drop them from LIVE
  // frames). The next dead-ball alignment walks them into formation.
  const poses = { ...state.poses };
  delete poses[out];
  if (!poses[inn]) {
    const lineup = t === 'home' ? rebuilt : rebuilt;
    const posIdx = Math.max(0, lineup.indexOf(inn));
    const x = t === 'home' ? 0.04 : 0.96;
    poses[inn] = {
      jersey: inn,
      team: t,
      x,
      y: 0.25 + 0.1 * (posIdx % 5),
      targetX: x,
      targetY: 0.25 + 0.1 * (posIdx % 5),
      action: 'idle',
      hasBall: false,
      arrived: false,
      vx: 0,
      vy: 0,
    };
  }
  // If the outgoing player held the ball, the ball must not point at a
  // player who is no longer on court (I2: held ⇒ exactly one hasBall).
  // In a live tick the incoming player takes possession; in a dead ball
  // the holder is cleared (the inbound/FT flow re-owns it).
  let ballMotion = state.ballMotion;
  let ball = state.ball;
  if (state.ballMotion.holderId === out || state.ball.holderId === out) {
    if (state.phase === 'LIVE') {
      ballMotion = { ...ballMotion, holderId: inn };
      ball = { ...ball, holderId: inn, status: 'held' };
      poses[inn] = { ...poses[inn]!, hasBall: true };
    } else {
      ballMotion = { ...ballMotion, holderId: null };
      ball = { ...ball, holderId: null, status: 'dead' };
    }
  }
  return { ...state, lineups, poses, ballMotion, ball };
}

/** TIMEOUT_START: enter TIMEOUT, decrement the calling team's remaining. */
export function applyTimeoutStart(state: GameState, event: Event): GameState {
  const t = team(event, 'team');
  const prevRemaining = t === 'home' ? state.timeouts.remaining.home : state.timeouts.remaining.away;
  const remaining =
    t === 'home'
      ? { ...state.timeouts.remaining, home: Math.max(0, prevRemaining - 1) }
      : { ...state.timeouts.remaining, away: Math.max(0, prevRemaining - 1) };
  return {
    ...state,
    phase: 'TIMEOUT',
    timeouts: { remaining },
  };
}

/** FT_SEQUENCE_END: optional payload.next_phase steers; default to LIVE. */
export function applyFtSequenceEnd(state: GameState, event: Event): GameState {
  return { ...state, phase: (event.payload['next_phase'] as Phase) ?? 'LIVE' };
}

export function applyAlignment(state: GameState, event: Event): GameState {
  const raw = event.payload['players'];
  if (!Array.isArray(raw)) return state;
  const players: import('./types.js').AlignedPlayer[] = [];
  for (const row of raw) {
    if (typeof row !== 'object' || row === null) continue;
    const r = row as Record<string, unknown>;
    if (typeof r.jersey !== 'string') continue;
    if (r.team !== 'home' && r.team !== 'away') continue;
    if (typeof r.x !== 'number' || typeof r.y !== 'number') continue;
    const teamId: TeamId = r.team;
    players.push({
      jersey: r.jersey,
      team: teamId,
      x: r.x,
      y: r.y,
      zone: (typeof r.zone === 'string' ? r.zone : 'frontcourt_center') as import('./types.js').CourtZone,
      task: typeof r.task === 'string' ? r.task : 'idle',
      hasBall: r.hasBall === true,
    });
  }
  if (players.length === 0) return state;
  const offense: TeamId | null =
    event.payload['offense'] === 'home' || event.payload['offense'] === 'away'
      ? event.payload['offense']
      : null;
  const ballHolder = players.find((p) => p.hasBall);
  return {
    ...state,
    alignment: {
      context: typeof event.payload['context'] === 'string' ? event.payload['context'] : event.type,
      offense,
      players,
    },
    ball: ballHolder
      ? { ...state.ball, holderId: ballHolder.jersey, zone: ballHolder.zone }
      : state.ball,
  };
}

/** CLOCK_EXPIRY_ADJUDICATION: zero out the named clock (or residual). */
export function applyClockExpiry(state: GameState, event: Event): GameState {
  const which = str(event, 'clock');
  const residualRaw = event.payload['residual_seconds'];
  const residual = typeof residualRaw === 'number' ? residualRaw : 0;
  const clocks: Clocks =
    which === 'shot'
      ? { ...state.clocks, shot: residual }
      : which === 'game'
        ? { ...state.clocks, game: residual }
        : state.clocks;
  return { ...state, clocks };
}
