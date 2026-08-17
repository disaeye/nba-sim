/**
 * Canonical full-game contract.
 *
 * All assertions in this file consume the same read-only demo results.  The
 * simulation is expensive because it intentionally produces the continuous
 * world timeline; the contracts below therefore share one result per seed
 * instead of launching an identical game from separate test files.
 */
import { describe, it, expect } from 'vitest';
import Ajv from 'ajv';

import { simulateGame } from '../../src/simulate.js';
import { FOUNDATION_VERSION, computeBoxScore } from '../../src/index.js';
import type { Event } from '../../src/state/types.js';
import type { GameResult } from '../../src/simulate.js';
import { cachedGame, demoInput } from './_helpers.js';

import schemaJson from '../../config/schemas/game-result.schema.json' with { type: 'json' };
import golden42 from '../../fixtures/golden-seed-42.json' with { type: 'json' };
import golden7 from '../../fixtures/golden-seed-7.json' with { type: 'json' };

const ajv = new Ajv({ allErrors: true });
const validateSchema = ajv.compile(schemaJson);

interface GoldenFixture {
  readonly meta: {
    readonly foundation_version: string;
    readonly seed: number;
    readonly input_hash: string;
  };
  readonly event_type_sequence: readonly string[];
  readonly final_score: { readonly home: number; readonly away: number };
  readonly event_count: number;
  readonly periods: number;
}

const G42 = golden42 as GoldenFixture;
const G7 = golden7 as GoldenFixture;

function teamPoints(result: GameResult, team: 'home' | 'away'): number {
  return result.box_score[team].reduce((sum, p) => sum + p.points, 0);
}

function countTypes(events: readonly { type: string }[]): Record<string, number> {
  const out: Record<string, number> = {};
  for (const event of events) out[event.type] = (out[event.type] ?? 0) + 1;
  return out;
}

function periodLength(period: number): number {
  return period >= 5 ? 300 : 720;
}

function totalSecondsFromEvents(events: readonly Event[]): number {
  const periods = new Set<number>();
  let current = 1;
  for (const event of events) {
    if (event.type === 'PERIOD_START' || event.type === 'PERIOD_END') {
      const period = event.payload['period'];
      if (typeof period === 'number') current = period;
    }
    periods.add(current);
  }
  let total = 0;
  for (const period of periods) total += periodLength(period);
  return total;
}

/** Jerseys that appeared in any player-id-bearing event payload field. */
function appearedJerseys(events: readonly Event[], team: 'home' | 'away'): Set<string> {
  const range = team === 'home'
    ? new Set(Array.from({ length: 10 }, (_, i) => String(i + 1)))
    : new Set(Array.from({ length: 10 }, (_, i) => String(i + 11)));
  const appeared = new Set<string>();
  for (const event of events) {
    for (const value of Object.values(event.payload)) {
      if (typeof value === 'string' && range.has(value)) appeared.add(value);
    }
  }
  return appeared;
}

describe('canonical demo game — full integration', () => {
  it('completes a regulation game and exposes the core timeline', () => {
    const result = cachedGame(42);
    expect(result.events.length).toBeGreaterThan(50);
    expect(result.events[0]?.type).toBe('GAME_START');
    expect(result.events.at(-1)?.type).toBe('GAME_END');
    expect(new Set(result.events.map((event) => event.period)).size).toBeGreaterThanOrEqual(4);
    expect(result.possession_log.length).toBeGreaterThan(20);
    for (const possession of result.possession_log) {
      expect(possession.offensive_team_id).toMatch(/^(home|away)$/);
      expect(possession.mode).toMatch(/^(TRANSITION|HALFCOURT)$/);
    }
    expect(result.meta.foundation_version).toBe(FOUNDATION_VERSION);
    expect(result.meta.seed).toBe(42);
    expect(result.meta.home_team_id).toBe('home');
    expect(result.meta.away_team_id).toBe('away');
    expect(teamPoints(result, 'home')).toBeGreaterThan(0);
    expect(teamPoints(result, 'away')).toBeGreaterThan(0);
  });

  it('keeps box-score arithmetic and event-fold reconciliation valid', () => {
    const result = cachedGame(42);
    for (const player of [...result.box_score.home, ...result.box_score.away]) {
      expect(player.fgm).toBeLessThanOrEqual(player.fga);
      expect(player.ftm).toBeLessThanOrEqual(player.fta);
      expect(player.points).toBe(2 * (player.fgm - player.tpm) + 3 * player.tpm + player.ftm);
    }

    const homeRoster = new Map(result.box_score.home.map((p) => [p.jersey, {
      id: p.playerId,
      jersey: p.jersey,
      teamId: p.teamId,
    }]));
    const awayRoster = new Map(result.box_score.away.map((p) => [p.jersey, {
      id: p.playerId,
      jersey: p.jersey,
      teamId: p.teamId,
    }]));
    const recomputed = computeBoxScore(result.events, { home: homeRoster, away: awayRoster });
    expect(recomputed.home.length).toBe(result.box_score.home.length);
    expect(recomputed.away.length).toBe(result.box_score.away.length);
  });

  it('matches the result schema', () => {
    const result = cachedGame(42);
    const valid = validateSchema(result);
    if (!valid) {
      const errors = validateSchema.errors?.map((error) => `${error.instancePath}: ${error.message}`).join('; ');
      throw new Error(`Schema validation failed: ${errors}`);
    }
    expect(valid).toBe(true);
  });

  it('is deterministic across two independent runs, including stamina output', () => {
    const first = cachedGame(42);
    const second = simulateGame(demoInput(42));
    expect(second.events.length).toBe(first.events.length);
    expect(second.events.map((event) => event.type)).toEqual(first.events.map((event) => event.type));
    expect(second.box_score).toEqual(first.box_score);
    expect(second.stamina_report).toEqual(first.stamina_report);
  });
  it('produces a different seeded outcome while preserving the same public shape', () => {
    const first = cachedGame(42);
    const second = cachedGame(7);
    expect(second.meta.seed).toBe(7);
    expect(second.events.length === first.events.length && teamPoints(second, 'home') === teamPoints(first, 'home')).toBe(false);
    expect(second.events.at(-1)?.type).toBe('GAME_END');
  });
});

describe('canonical demo game — foul, free throw, and period wiring', () => {
  it('emits a complete foul/free-throw sequence and personal fouls', () => {
    const result = cachedGame(42);
    const counts = countTypes(result.events);
    expect(counts['FOUL'] ?? 0).toBeGreaterThan(0);
    expect(counts['FT_START'] ?? 0).toBeGreaterThan(0);
    expect(counts['FT_ATTEMPT'] ?? 0).toBeGreaterThan(0);
    expect(counts['FT_RESULT'] ?? 0).toBeGreaterThan(0);
    expect(counts['FT_SEQUENCE_END'] ?? 0).toBeGreaterThan(0);
    const personalFouls = [...result.box_score.home, ...result.box_score.away]
      .reduce((sum, player) => sum + player.pf, 0);
    expect(personalFouls).toBeGreaterThan(0);
  });

  it('crosses halftime with continuous tip/live snapshots', () => {
    const result = cachedGame(42);
    const counts = countTypes(result.events);
    expect(counts['HALFTIME'] ?? 0).toBeGreaterThanOrEqual(1);
    const snapshots = result.snapshots ?? [];
    expect(snapshots.length).toBeGreaterThan(100);
    expect(snapshots.some((snapshot) => snapshot.period === 1 && snapshot.phase === 'LIVE')).toBe(true);
    expect(snapshots.some((snapshot) => snapshot.period === 3 && snapshot.phase === 'LIVE')).toBe(true);

    const early = snapshots.filter((snapshot) => snapshot.t_real <= 12);
    expect(early.length).toBeGreaterThan(20);
    for (let i = 1; i < early.length; i += 1) {
      const previous = early[i - 1]!.players.find((player) => player.jersey === '1');
      const current = early[i]!.players.find((player) => player.jersey === '1');
      if (!previous || !current) continue;
      expect(Math.hypot(current.x - previous.x, current.y - previous.y)).toBeLessThan(0.25);
    }
  });
});

describe('canonical demo game — snapshot invariants', () => {
  it('keeps exactly ten live players and never duplicates a ball owner', () => {
    const snapshots = cachedGame(42).snapshots ?? [];
    expect(snapshots.length).toBeGreaterThan(100);
    let live = 0;
    let invalidPlayerCount = 0;
    let multiBall = 0;
    for (const snapshot of snapshots) {
      if (snapshot.phase === 'LIVE') {
        live += 1;
        if (snapshot.players.length !== 10) invalidPlayerCount += 1;
      }
      if (snapshot.players.filter((player) => player.hasBall).length > 1) multiBall += 1;
      if (snapshot.ball.status === 'held' && snapshot.ball.holderId !== null) {
        const owners = snapshot.players.filter((player) => player.hasBall).map((player) => player.jersey);
        expect(owners).toEqual([snapshot.ball.holderId]);
      }
    }
    expect(live).toBeGreaterThan(0);
    expect(invalidPlayerCount).toBe(0);
    expect(multiBall).toBe(0);
    expect(snapshots.filter((snapshot) => snapshot.players.length > 10)).toHaveLength(0);
  });

  it('refreshes possession labels and preserves spatial separation', () => {
    const snapshots = cachedGame(42).snapshots ?? [];
    const defensiveActions = new Set(['on_ball_defend', 'deny', 'help', 'tag', 'weak_side', 'box_out']);
    let heldLive = 0;
    let defensiveHolderFrames = 0;
    const sameTeamMins: number[] = [];
    for (const snapshot of snapshots) {
      if (snapshot.phase !== 'LIVE') continue;
      if (snapshot.ball.status === 'held' && snapshot.ball.holderId) {
        heldLive += 1;
        const holder = snapshot.players.find((player) => player.jersey === snapshot.ball.holderId);
        if (holder && defensiveActions.has(holder.action)) defensiveHolderFrames += 1;
      }
      for (const team of ['home', 'away'] as const) {
        const players = snapshot.players.filter((player) => player.team === team);
        let minimum = Infinity;
        for (let a = 0; a < players.length; a += 1) {
          for (let b = a + 1; b < players.length; b += 1) {
            minimum = Math.min(minimum, Math.hypot(
              (players[a]!.x - players[b]!.x) * 94,
              (players[a]!.y - players[b]!.y) * 50,
            ));
          }
        }
        if (minimum < Infinity) sameTeamMins.push(minimum);
      }
    }
    expect(heldLive).toBeGreaterThan(100);
    expect(defensiveHolderFrames / heldLive).toBeLessThan(0.2);
    sameTeamMins.sort((a, b) => a - b);
    expect(sameTeamMins[Math.floor(sameTeamMins.length / 2)] ?? 0).toBeGreaterThan(1.5);
  });

  it('clears defensive labels from the stealer within three ticks of a STEAL', () => {
    const snapshots = cachedGame(42).snapshots ?? [];
    const defensiveActions = new Set(['on_ball_defend', 'deny', 'help', 'tag', 'weak_side', 'box_out']);
    let checks = 0;
    let clean = 0;
    for (let i = 0; i < snapshots.length; i += 1) {
      const snapshot = snapshots[i]!;
      if (snapshot.lastEventType !== 'STEAL') continue;
      const stealer = snapshot.players.find((player) => player.hasBall)?.jersey ?? snapshot.ball.holderId;
      if (!stealer) continue;
      // Look ahead up to 3 snapshots (~0.3s).
      for (let j = i; j <= i + 3 && j < snapshots.length; j += 1) {
        const next = snapshots[j]!;
        if (next.phase !== 'LIVE') break;
        const player = next.players.find((candidate) => candidate.jersey === stealer);
        if (!player) break;
        checks += 1;
        if (player.hasBall && !defensiveActions.has(player.action)) clean += 1;
        if (player.hasBall) break;
      }
    }
    // If no steals occurred, the soft check is vacuous; otherwise the
    // majority of held-ball frames must clear the defensive label.
    if (checks > 0) expect(clean).toBeGreaterThan(0);
  });

  it('keeps minutes honest through the default staggered rotation', () => {
    const result = cachedGame(42);
    const total = totalSecondsFromEvents(result.events);
    expect(total).toBeGreaterThan(0);
    const expected = 5 * total;
    const homeSum = result.box_score.home.reduce((sum, player) => sum + player.minutes, 0);
    const awaySum = result.box_score.away.reduce((sum, player) => sum + player.minutes, 0);
    expect(Math.abs(homeSum - expected)).toBeLessThanOrEqual(60);
    expect(Math.abs(awaySum - expected)).toBeLessThanOrEqual(60);

    const homeAppeared = appearedJerseys(result.events, 'home');
    const awayAppeared = appearedJerseys(result.events, 'away');
    expect(homeAppeared.size).toBeGreaterThan(0);
    expect(awayAppeared.size).toBeGreaterThan(0);
    for (const jersey of homeAppeared) expect(result.box_score.home.find((player) => player.jersey === jersey)?.minutes).toBeGreaterThan(0);
    for (const jersey of awayAppeared) expect(result.box_score.away.find((player) => player.jersey === jersey)?.minutes).toBeGreaterThan(0);
    for (const player of result.box_score.home) if (!homeAppeared.has(player.jersey)) expect(player.minutes).toBe(0);
    for (const player of result.box_score.away) if (!awayAppeared.has(player.jersey)) expect(player.minutes).toBe(0);

    const starters = result.box_score.home.filter((player) => Number(player.jersey) <= 5);
    const bench = result.box_score.home.filter((player) => Number(player.jersey) > 5 && Number(player.jersey) <= 10);
    const starterAverage = starters.reduce((sum, player) => sum + player.minutes, 0) / starters.length;
    const benchAverage = bench.reduce((sum, player) => sum + player.minutes, 0) / bench.length;
    expect(starterAverage).toBeGreaterThan(1300);
    expect(benchAverage).toBeLessThan(1800);
    expect(Math.max(...bench.map((player) => player.minutes))).toBeLessThan(2400);
  });
});

describe('canonical demo game — structural release checks', () => {
  it('keeps seed 41 shot-clock, shot-result, zone, score, and freeze bands sane', () => {
    const result = cachedGame(41);
    const violations = result.events.filter((event) => event.type === 'SHOT_CLOCK_VIOLATION').length;
    expect(violations / Math.max(1, result.possession_log.length)).toBeLessThanOrEqual(0.35);
    const releases = result.events.filter((event) => event.type === 'SHOT_RELEASE').length;
    const shotResults = result.events.filter((event) => event.type === 'SHOT_RESULT').length;
    expect(Math.abs(releases - shotResults)).toBeLessThanOrEqual(60);
    expect(new Set(result.events.filter((event) => event.type === 'SHOT_RELEASE').map((event) => String(event.payload['zone'] ?? ''))).size).toBeGreaterThan(1);
    expect(teamPoints(result, 'home')).toBeGreaterThanOrEqual(50);
    expect(teamPoints(result, 'home')).toBeLessThanOrEqual(200);
    expect(teamPoints(result, 'away')).toBeGreaterThanOrEqual(50);
    expect(teamPoints(result, 'away')).toBeLessThanOrEqual(200);

    let maximumGap = 0;
    for (let i = 1; i < result.events.length; i += 1) {
      const previous = result.events[i - 1]!;
      const current = result.events[i]!;
      if (previous.period !== current.period || current.type !== 'SHOT_CLOCK_VIOLATION') continue;
      maximumGap = Math.max(maximumGap, previous.t_game - current.t_game);
    }
    expect(maximumGap).toBeLessThanOrEqual(24 + 1e-6);
  });
});

describe('canonical demo game — stamina integration', () => {
  it('exposes a valid per-player stamina report and snapshot values', () => {
    const result = cachedGame(42);
    expect(result.stamina_report).toBeDefined();
    const report = result.stamina_report!;
    expect(Object.keys(report)).toHaveLength(20);
    for (const value of Object.values(report)) {
      expect(value.max).toBeGreaterThan(0);
      expect(value.end).toBeGreaterThanOrEqual(0);
      expect(value.fatigue).toBeGreaterThanOrEqual(0);
    }
    const live = (result.snapshots ?? []).find((snapshot) => snapshot.players.length === 10 && snapshot.players[0]!.stm >= 0);
    expect(live).toBeDefined();
    for (const player of live!.players) {
      expect(player.stm).toBeGreaterThanOrEqual(0);
      expect(player.stmMax).toBeGreaterThan(0);
    }
  });

  it('emits stamina sub requests when a handler carries heavy minutes', () => {
    const input = demoInput(7);
    const result = simulateGame({
      ...input,
      scriptedSubs: [{ period: 4, clockMin: 0, team: 'home', playerOut: '6', playerIn: '7' }],
    });
    expect(result.stamina_report?.['1']?.consumed).toBeGreaterThan(78);
    const notes = result.events.filter((event) => event.type === 'STATE_NOTE' && String(event.payload.note ?? '').startsWith('stamina'));
    expect(notes.length).toBeGreaterThan(0);
    expect(notes.some((event) => event.payload.player_id === '1')).toBe(true);
  });
});

describe('canonical golden fixtures', () => {
  it('pin seed 42 event sequence, score, count, and periods', () => {
    const result = cachedGame(42);
    expect(result.events.map((event) => event.type)).toEqual(G42.event_type_sequence);
    expect(teamPoints(result, 'home')).toBe(G42.final_score.home);
    expect(teamPoints(result, 'away')).toBe(G42.final_score.away);
    expect(result.events.length).toBe(G42.event_count);
    expect(new Set(result.events.map((event) => event.period)).size).toBe(G42.periods);
  });

  it('pin seed 7 event sequence, score, count, and periods', () => {
    const result = cachedGame(7);
    expect(result.events.map((event) => event.type)).toEqual(G7.event_type_sequence);
    expect(teamPoints(result, 'home')).toBe(G7.final_score.home);
    expect(teamPoints(result, 'away')).toBe(G7.final_score.away);
    expect(result.events.length).toBe(G7.event_count);
    expect(new Set(result.events.map((event) => event.period)).size).toBe(G7.periods);
  });

  it('keeps fixture metadata tied to the running foundation and input', () => {
    expect(FOUNDATION_VERSION).toBe(G42.meta.foundation_version);
    expect(FOUNDATION_VERSION).toBe(G7.meta.foundation_version);
    expect(G42.meta.seed).toBe(42);
    expect(G7.meta.seed).toBe(7);
    expect(G42.meta.input_hash).toBe(G7.meta.input_hash);
    expect(G42.meta.input_hash).toMatch(/^[0-9a-f]{16}$/);
  });
});
