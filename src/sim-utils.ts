/**
 * Utilities and types for the simulate module — extracted to keep
 * simulate.ts under the 250 pure-LOC ceiling. Contains the public
 * type surface, config constants, event-construction helpers,
 * period-transition utilities, and the GameResult builder.
 */
import type { Event, EventType, GameState, TeamId } from './state/types.js';
import { applyEvent } from './state/apply.js';

import type { LineupCapability, LineupPackage } from './identity/types.js';
import type { PossessionEpisode, PlaysConfig } from './possession/types.js';

import { computeBoxScore } from './box-score.js';
import type { Player as BsPlayer, RosterMaps, BoxScoreOptions, BoxScore } from './box-score.js';
import type { WorldSnapshot } from './world/snapshot.js';


import playsJson from '../config/plays.json' with { type: 'json' };
import foundationJson from '../config/foundation.json' with { type: 'json' };

// ─── config constants ───────────────────────────────────────────────────────

export const PLAYS_CONFIG = playsJson as PlaysConfig;
export const FOUNDATION_VERSION = (foundationJson as { foundation_version: string }).foundation_version;
export const MAX_ITERATIONS = 50000;

// ─── public types ───────────────────────────────────────────────────────────

export interface Player {
  readonly id: string;
  readonly jersey: string;
  readonly teamId: string;
  /**
   * Individual talent overlay, 0..1 per LineupCapability dimension.
   * Missing dimensions fall back to the lineup-package-derived default.
   */
  readonly abilities?: Partial<LineupCapability>;
  /**
   * Layered player data (docs/playerdata-design.md v1.3a). When present,
   * the roster's capability overlay is derived from it via the playerdata
   * bridge; an explicit `abilities` field still wins per-dimension.
   * Playerdata 0.1.0: optional overlay, engine behavior unchanged without it.
   */
  readonly playerData?: import('./playerdata/types.js').PlayerData;
}

export interface TeamGameInput {
  readonly teamId: string;
  readonly roster: readonly Player[];
  readonly lineupPackages: readonly LineupPackage[];
}

export interface SubInstruction {
  readonly period: number;
  readonly clockMin: number;
  readonly team: TeamId;
  readonly playerOut: string;
  readonly playerIn: string;
}

export interface GameInput {
  readonly home: TeamGameInput;
  readonly away: TeamGameInput;
  readonly seed: number;
  readonly scriptedSubs?: readonly SubInstruction[];
  /**
   * P5.5 coach identity per team — defensive scheme bias and pace
   * preference. Absent → neutral defaults (no behavior change).
   */
  readonly coach?: {
    readonly home?: CoachProfile;
    readonly away?: CoachProfile;
  };
  /**
   * P6.2 per-team morale (0..100) from the season layer — folds the
   * §9.3 exec multiplier into resolve/decision via the stamina-factor
   * channel. Absent → neutral 60 (exec 1.0).
   */
  readonly morale?: {
    readonly home?: number;
    readonly away?: number;
  };
}

/** P5.5 coach profile: scheme/pace biases, 0..1 normalized. */
export interface CoachProfile {
  /** 0 = pure man defense, 1 = zone-heavy (zone trigger threshold shift). */
  readonly zoneBias?: number;
  /** 0 = slow halfcourt, 1 = run-first transition. */
  readonly paceBias?: number;
  /** 0 = never blitz, 1 = blitz-happy. */
  readonly blitzBias?: number;
}

export interface TimelineEvent extends Event {
  readonly period: number;
}

export interface PossessionEntry {
  readonly possession_id: number;
  readonly period: number;
  readonly start_t_game: number;
  readonly end_t_game: number;
  readonly offensive_team_id: string;
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly start_reason: string;
  readonly end_reason: string;
}

export interface GameResult {
  readonly meta: {
    readonly foundation_version: string;
    readonly seed: number;
    readonly home_team_id: string;
    readonly away_team_id: string;
  };
  readonly events: readonly TimelineEvent[];
  readonly box_score: BoxScore;
  readonly possession_log: readonly PossessionEntry[];
  /** Continuous world state every tick (position truth). */
  readonly snapshots?: readonly import('./world/snapshot.js').WorldSnapshot[];
  /**
   * §8 stamina report per jersey: ceiling, end-of-game STM, consumption,
   * recovery earned, and fatigue F = max(0, consumed − rest) for the
   * season layer (decay/back-to-back live there).
   */
  readonly stamina_report?: Readonly<Record<string, { max: number; end: number; consumed: number; rest: number; fatigue: number }>>;
}
// ─── replay checkpoints ────────────────────────────────────────────────────

/** A stable, read-only handle for one authoritative replay snapshot. */
export interface Checkpoint {
  readonly seq: number;
  readonly t_game: number;
  readonly t_real: number;
  readonly snapshot: WorldSnapshot;
  readonly snapshotIndex: number;
}

function checkpointFromSnapshot(snapshot: WorldSnapshot, snapshotIndex: number): Checkpoint {
  return {
    seq: snapshot.lastEventSeq ?? -1,
    t_game: snapshot.t_game,
    t_real: snapshot.t_real,
    snapshot,
    snapshotIndex,
  };
}

/** Return the first event-aligned snapshot for an exact event sequence. */
export function getCheckpoint(result: GameResult, seq: number): Checkpoint | null {
  const snapshots = result.snapshots ?? [];
  for (let index = 0; index < snapshots.length; index += 1) {
    const snapshot = snapshots[index];
    if (snapshot?.lastEventSeq === seq) return checkpointFromSnapshot(snapshot, index);
  }
  return null;
}

/** Return the first snapshot at an exact logical game-clock value. */
export function getCheckpointByTime(result: GameResult, t_game: number): Checkpoint | null {
  const snapshots = result.snapshots ?? [];
  for (let index = 0; index < snapshots.length; index += 1) {
    const snapshot = snapshots[index];
    if (snapshot?.t_game === t_game) return checkpointFromSnapshot(snapshot, index);
  }
  return null;
}

// ─── helpers ────────────────────────────────────────────────────────────────

const OTHER_TEAM: Record<TeamId, TeamId> = { home: 'away', away: 'home' };

export function otherTeam(t: TeamId): TeamId {
  return OTHER_TEAM[t];
}

export function emit(
  state: GameState,
  type: EventType,
  actors: readonly string[],
  payload: Record<string, unknown>,
  clockOverride?: { readonly game?: number; readonly shot?: number },
): GameState {
  const event: Event = {
    type,
    t_game: state.clocks.game,
    t_real: state.realClock,
    seq: state.seq + 1,
    actors: [...actors],
    payload,
    clocks: {
      game: clockOverride?.game ?? state.clocks.game,
      shot: clockOverride?.shot ?? state.clocks.shot,
    },
    score: { ...state.score },
  };
  return applyEvent(state, event);
}

export function firstInLineup(state: GameState, team: TeamId): string {
  const lineup = team === 'home' ? state.lineups.home : state.lineups.away;
  const first = lineup[0];
  if (first === undefined) throw new Error(`firstInLineup: ${team} lineup is empty`);
  return first;
}

export function secondInLineup(state: GameState, team: TeamId): string {
  const lineup = team === 'home' ? state.lineups.home : state.lineups.away;
  const second = lineup[1];
  if (second === undefined) throw new Error(`secondInLineup: ${team} lineup has < 2 players`);
  return second;
}

export function periodRouterTrigger(endedPeriod: number, scoreDiff: number): string {
  if (endedPeriod === 1) return 'PERIOD_ROUTER_Q1';
  if (endedPeriod === 2) return 'PERIOD_ROUTER_Q2';
  if (endedPeriod === 3) return 'PERIOD_ROUTER_Q3';
  return scoreDiff === 0
    ? (endedPeriod === 4 ? 'PERIOD_ROUTER_Q4_TIED' : 'PERIOD_ROUTER_OT_TIED')
    : (endedPeriod === 4 ? 'PERIOD_ROUTER_Q4_NOT_TIED' : 'PERIOD_ROUTER_OT_NOT_TIED');
}

export function resetForNewPeriod(state: GameState): GameState {
  const isOt = state.period >= 5;
  const gameSeconds = isOt ? 300 : 720;
  return {
    ...state,
    clocks: { period: state.period, game: gameSeconds, shot: 24 },
    fouls: {
      team: { home: 0, away: 0 },
      players: state.fouls.players,
      bonus: { home: false, away: false },
    },
  };
}

// ─── sub application ────────────────────────────────────────────────────────

export function applyDueSubs(state: GameState, pendingSubs: SubInstruction[]): GameState {
  const remaining: SubInstruction[] = [];
  for (const sub of pendingSubs) {
    const isDue = state.clocks.period === sub.period && state.clocks.game <= sub.clockMin * 60;
    if (!isDue) {
      remaining.push(sub);
      continue;
    }
    const lineup = sub.team === 'home' ? state.lineups.home : state.lineups.away;
    // Idempotence guards: the out player must be on court AND the in
    // player must not already be on court. A rotation that fires twice
    // (same-period stage overlap) must not double-apply or corrupt the
    // five.
    if (!lineup.includes(sub.playerOut)) continue;
    if (lineup.includes(sub.playerIn)) continue;
    // A disqualified player (6 personal fouls) can never return — not
    // via the scripted rotation, not via a foul-trouble swap. Real NBA
    // rule; without this the fouled-out star gets subbed back in by the
    // next rotation stage (measured: #1 fouled out in Q3, returned in
    // Q4 and played 6 more minutes).
    if ((state.fouls.players[sub.playerIn] ?? 0) >= 6) continue;

    state = emit(state, 'SUB', [sub.playerOut, sub.playerIn], {
      player_out_id: sub.playerOut,
      player_in_id: sub.playerIn,
      team: sub.team,
    });
  }
  pendingSubs.length = 0;
  pendingSubs.push(...remaining);
  return state;
}

// ─── possession log ─────────────────────────────────────────────────────────

export function pushPossessionLog(
  log: PossessionEntry[],
  id: number,
  episode: PossessionEpisode,
  startT: number,
  startPeriod: number,
  endT: number,
  endReason: string,
): void {
  log.push({
    possession_id: id,
    period: startPeriod,
    start_t_game: startT,
    end_t_game: endT,
    offensive_team_id: episode.team,
    mode: episode.mode,
    start_reason: episode.startReason,
    end_reason: endReason,
  });
}

// ─── input validation ───────────────────────────────────────────────────────

const CAPABILITY_KEYS = [
  'creation', 'pullUp', 'catchShoot', 'rimFinishing', 'passing', 'screening',
  'rolling', 'popping', 'postPlay', 'cutting', 'handleSecurity', 'transition',
  'onBallDefense', 'helpDefense',
] as const;

function assertTeamInput(team: GameInput['home'], label: 'home' | 'away'): void {
  if (team.teamId.trim().length === 0) throw new Error(`simulateGame: ${label} teamId must be non-empty`);
  if (team.roster.length === 0) throw new Error(`simulateGame: ${label} roster must not be empty`);
  const jerseys = new Set<string>();
  for (const player of team.roster) {
    if (player.id.trim().length === 0 || player.jersey.trim().length === 0) {
      throw new Error(`simulateGame: ${label} roster players need non-empty id and jersey`);
    }
    if (player.teamId !== team.teamId) {
      throw new Error(`simulateGame: ${label} roster player ${player.jersey} has teamId ${player.teamId}, expected ${team.teamId}`);
    }
    if (jerseys.has(player.jersey)) throw new Error(`simulateGame: ${label} roster has duplicate jersey ${player.jersey}`);
    jerseys.add(player.jersey);
    for (const [key, value] of Object.entries(player.abilities ?? {})) {
      if (!(CAPABILITY_KEYS as readonly string[]).includes(key)) throw new Error(`simulateGame: unknown ability ${key} on ${label} jersey ${player.jersey}`);
      if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
        throw new Error(`simulateGame: ability ${key} on ${label} jersey ${player.jersey} must be a finite number in [0,1]`);
      }
    }
  }
  const pkg = team.lineupPackages[0];
  if (pkg === undefined) throw new Error(`simulateGame: ${label} team has no lineup packages`);
  if (pkg.players.length !== 5) throw new Error(`simulateGame: ${label} starters must have exactly 5 players`);
  if (new Set(pkg.players).size !== 5) throw new Error(`simulateGame: ${label} starters must have unique players`);
  for (const jersey of pkg.players) {
    if (!jerseys.has(jersey)) throw new Error(`simulateGame: ${label} starter ${jersey} is not in roster`);
  }
  const profile = pkg.usageProfile;
  const profileJerseys = [...profile.creator, ...profile.screener, ...profile.spacer];
  if (new Set(profileJerseys).size !== profileJerseys.length) throw new Error(`simulateGame: ${label} usageProfile roles must not overlap`);
  for (const jersey of profileJerseys) {
    if (!pkg.players.includes(jersey)) throw new Error(`simulateGame: ${label} usageProfile jersey ${jersey} is not in starters`);
  }
  if (profile.creator.length < 2) throw new Error(`simulateGame: ${label} usageProfile needs at least two creators`);
}

export function validateInput(input: GameInput): void {
  if (!Number.isInteger(input.seed) || input.seed < 0 || input.seed > 0xffffffff) {
    throw new Error('simulateGame: seed must be a non-negative uint32 integer');
  }
  if (input.home.teamId === input.away.teamId) throw new Error('simulateGame: home and away teamId must differ');
  assertTeamInput(input.home, 'home');
  assertTeamInput(input.away, 'away');
  const allJerseys = new Set<string>();
  for (const team of [input.home, input.away]) {
    for (const player of team.roster) {
      if (allJerseys.has(player.jersey)) throw new Error(`simulateGame: jersey ${player.jersey} must be unique across teams`);
      allJerseys.add(player.jersey);
    }
  }
  for (const [label, profile] of Object.entries(input.coach ?? {})) {
    if (profile === undefined) continue;
    for (const [key, value] of Object.entries(profile)) {
      if (!['zoneBias', 'paceBias', 'blitzBias'].includes(key)) throw new Error(`simulateGame: unknown ${label} coach field ${key}`);
      if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) throw new Error(`simulateGame: ${label} coach ${key} must be a finite number in [0,1]`);
    }
  }
  for (const [label, morale] of Object.entries(input.morale ?? {})) {
    if (morale === undefined) continue;
    if (typeof morale !== 'number' || !Number.isFinite(morale) || morale < 0 || morale > 100) throw new Error(`simulateGame: ${label} morale must be a finite number in [0,100]`);
  }
  for (const sub of input.scriptedSubs ?? []) {
    if (!Number.isInteger(sub.period) || sub.period < 1 || !Number.isFinite(sub.clockMin) || sub.clockMin < 0 || sub.clockMin > 12) {
      throw new Error('simulateGame: scripted substitution has invalid period or clockMin');
    }
    if (sub.playerOut === sub.playerIn || sub.playerOut.trim().length === 0 || sub.playerIn.trim().length === 0) {
      throw new Error('simulateGame: scripted substitution needs distinct non-empty player ids');
    }
    const team = sub.team === 'home' ? input.home : sub.team === 'away' ? input.away : undefined;
    if (team === undefined) throw new Error(`simulateGame: invalid substitution team ${String(sub.team)}`);
    if (!team.roster.some((p) => p.jersey === sub.playerOut) || !team.roster.some((p) => p.jersey === sub.playerIn)) {
      throw new Error(`simulateGame: scripted substitution references unknown ${sub.team} jersey`);
    }
  }
}

// ─── build result ───────────────────────────────────────────────────────────

export function buildGameResult(
  state: GameState,
  input: GameInput,
  possessionLog: PossessionEntry[],
  rosterMaps: RosterMaps,
): GameResult {
  const timelineEvents = stampPeriods(state.events);
  // Use the starters package (lineupPackages[0]) for minutes — that is the
  // 5-on-court at GAME_START. The `packages` record is mutated by the
  // simulate loop at halftime (bench swap), so reading packages.*.players
  // here would credit the wrong unit.
  const homeStarters = input.home.lineupPackages[0]?.players;
  const awayStarters = input.away.lineupPackages[0]?.players;
  const periods = new Set(timelineEvents.map((e) => e.period));
  let totalSeconds = 0;
  for (const p of periods) {
    totalSeconds += p >= 5 ? 300 : 720;
  }
  const boxOptions: BoxScoreOptions | undefined =
    homeStarters && awayStarters
      ? {
          initialLineups: { home: homeStarters, away: awayStarters },
          totalSeconds,
        }
      : undefined;
  return {
    meta: {
      foundation_version: FOUNDATION_VERSION,
      seed: input.seed,
      home_team_id: input.home.teamId,
      away_team_id: input.away.teamId,
    },
    events: timelineEvents,
    box_score: computeBoxScore(state.events, rosterMaps, boxOptions),
    possession_log: possessionLog,
  };
}

export function stampPeriods(events: readonly Event[]): TimelineEvent[] {
  let period = 1;
  const out: TimelineEvent[] = [];
  for (const e of events) {
    if (e.type === 'PERIOD_START') {
      const p = e.payload['period'];
      if (typeof p === 'number') period = p;
    }
    out.push({ ...e, period });
  }
  return out;
}

export type { BsPlayer, RosterMaps };
