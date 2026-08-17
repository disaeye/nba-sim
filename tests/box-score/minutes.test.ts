/**
 * I6 — minutes honesty (plan T7).
 *
 * `minutes` is the per-player seconds-on-court, derived by walking the
 * event timeline: every interval between consecutive events is credited
 * to the 5 players per team who were on court during that interval.
 * Lineup changes via SUB take effect for the following interval.
 *
 * Honest invariants:
 *   - sum(minutes) per team == 5 * totalSeconds  (5 on court at all times)
 *   - every jersey that appeared on court has minutes > 0
 *   - jersey that never appeared has minutes == 0
 *
 * `minutes` is in SECONDS per engine convention (config/schemas/game-result.schema.json).
 */
import { describe, it, expect } from 'vitest';
import { computeBoxScore } from '../../src/box-score.js';
import type { Event, EventType } from '../../src/state/types.js';
import type { Player } from '../../src/box-score.js';

// ─── fixtures ───────────────────────────────────────────────────────────────

const HOME_ROSTER: Player[] = [
  { id: 'p1', jersey: '1', teamId: 'home' },
  { id: 'p2', jersey: '2', teamId: 'home' },
  { id: 'p3', jersey: '3', teamId: 'home' },
  { id: 'p4', jersey: '4', teamId: 'home' },
  { id: 'p5', jersey: '5', teamId: 'home' },
  { id: 'p6', jersey: '6', teamId: 'home' },
  { id: 'p7', jersey: '7', teamId: 'home' },
  { id: 'p8', jersey: '8', teamId: 'home' },
  { id: 'p9', jersey: '9', teamId: 'home' },
  { id: 'p10', jersey: '10', teamId: 'home' },
];

const AWAY_ROSTER: Player[] = [
  { id: 'p11', jersey: '11', teamId: 'away' },
  { id: 'p12', jersey: '12', teamId: 'away' },
  { id: 'p13', jersey: '13', teamId: 'away' },
  { id: 'p14', jersey: '14', teamId: 'away' },
  { id: 'p15', jersey: '15', teamId: 'away' },
  { id: 'p16', jersey: '16', teamId: 'away' },
  { id: 'p17', jersey: '17', teamId: 'away' },
  { id: 'p18', jersey: '18', teamId: 'away' },
  { id: 'p19', jersey: '19', teamId: 'away' },
  { id: 'p20', jersey: '20', teamId: 'away' },
];

function rosters() {
  return {
    home: new Map(HOME_ROSTER.map((p) => [p.jersey, p])),
    away: new Map(AWAY_ROSTER.map((p) => [p.jersey, p])),
  };
}

const HOME_STARTERS = ['1', '2', '3', '4', '5'];
const AWAY_STARTERS = ['11', '12', '13', '14', '15'];
const REG_SEC = 720; // one regulation period

/** Minimal Event envelope with the t_game the minutes walker needs. */
function ev(type: EventType, tGame: number, payload: Record<string, unknown> = {}): Event {
  return {
    type,
    t_game: tGame,
    t_real: 0,
    seq: 0,
    actors: [],
    payload,
    clocks: { game: tGame, shot: 0 },
    score: { home: 0, away: 0 },
  };
}

const OPTS = {
  initialLineups: { home: HOME_STARTERS, away: AWAY_STARTERS },
};

// ─── tests ──────────────────────────────────────────────────────────────────

describe('I6 minutes — unit (computeBoxScore)', () => {
  it('no subs in one period: 5 starters each get 720s; bench gets 0', () => {
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '1' }),
      ev('PASS', 360, { passer_id: '1', receiver_id: '2' }),
      ev('PASS', 180, { passer_id: '2', receiver_id: '3' }),
      ev('PERIOD_END', 0, { period: 1 }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters(), OPTS);

    for (const j of HOME_STARTERS) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home starter ${j}`).toBe(REG_SEC);
    }
    for (const j of AWAY_STARTERS) {
      const p = box.away.find((x) => x.jersey === j);
      expect(p?.minutes, `away starter ${j}`).toBe(REG_SEC);
    }
    // Bench never appeared → 0.
    for (const j of ['6', '7', '8', '9', '10']) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home bench ${j}`).toBe(0);
    }

    const homeSum = box.home.reduce((s, p) => s + p.minutes, 0);
    const awaySum = box.away.reduce((s, p) => s + p.minutes, 0);
    expect(homeSum).toBe(5 * REG_SEC);
    expect(awaySum).toBe(5 * REG_SEC);
  });

  it('mid-period SUB: outgoing starter and incoming bench each get partial time', () => {
    // Starter '5' plays first 360s; then '6' subs in for remaining 360s.
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '1' }),
      ev('PASS', 360, { passer_id: '1', receiver_id: '2' }),
      ev('SUB', 360, { player_out_id: '5', player_in_id: '6', team: 'home' }),
      ev('PASS', 180, { passer_id: '1', receiver_id: '2' }),
      ev('PERIOD_END', 0, { period: 1 }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters(), OPTS);

    const starter5 = box.home.find((p) => p.jersey === '5');
    const bench6 = box.home.find((p) => p.jersey === '6');
    expect(starter5?.minutes).toBe(360);
    expect(bench6?.minutes).toBe(360);

    // The four undisrupted starters each get the full period.
    for (const j of ['1', '2', '3', '4']) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home starter ${j}`).toBe(REG_SEC);
    }

    const homeSum = box.home.reduce((s, p) => s + p.minutes, 0);
    expect(homeSum).toBe(5 * REG_SEC);
  });

  it('halftime swap: starters play Q1+Q2 (1440s); bench plays Q3+Q4 (1440s)', () => {
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '1' }),
      ev('PERIOD_END', 0, { period: 1 }),
      ev('PERIOD_START', REG_SEC, { period: 2 }),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'away', player_id: '12' }),
      ev('PERIOD_END', 0, { period: 2 }),
      // Halftime swap — 5 SUBs per team at the dead-ball, t_game=0.
      ev('HALFTIME', 0),
      ev('SUB', 0, { player_out_id: '1', player_in_id: '6', team: 'home' }),
      ev('SUB', 0, { player_out_id: '2', player_in_id: '7', team: 'home' }),
      ev('SUB', 0, { player_out_id: '3', player_in_id: '8', team: 'home' }),
      ev('SUB', 0, { player_out_id: '4', player_in_id: '9', team: 'home' }),
      ev('SUB', 0, { player_out_id: '5', player_in_id: '10', team: 'home' }),
      ev('SUB', 0, { player_out_id: '11', player_in_id: '16', team: 'away' }),
      ev('SUB', 0, { player_out_id: '12', player_in_id: '17', team: 'away' }),
      ev('SUB', 0, { player_out_id: '13', player_in_id: '18', team: 'away' }),
      ev('SUB', 0, { player_out_id: '14', player_in_id: '19', team: 'away' }),
      ev('SUB', 0, { player_out_id: '15', player_in_id: '20', team: 'away' }),
      ev('PERIOD_START', REG_SEC, { period: 3 }),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '6' }),
      ev('PERIOD_END', 0, { period: 3 }),
      ev('PERIOD_START', REG_SEC, { period: 4 }),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'away', player_id: '17' }),
      ev('PERIOD_END', 0, { period: 4 }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters(), OPTS);

    for (const j of HOME_STARTERS) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home starter ${j}`).toBe(2 * REG_SEC);
    }
    for (const j of ['6', '7', '8', '9', '10']) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home bench ${j}`).toBe(2 * REG_SEC);
    }
    for (const j of AWAY_STARTERS) {
      const p = box.away.find((x) => x.jersey === j);
      expect(p?.minutes, `away starter ${j}`).toBe(2 * REG_SEC);
    }
    for (const j of ['16', '17', '18', '19', '20']) {
      const p = box.away.find((x) => x.jersey === j);
      expect(p?.minutes, `away bench ${j}`).toBe(2 * REG_SEC);
    }

    const homeSum = box.home.reduce((s, p) => s + p.minutes, 0);
    const awaySum = box.away.reduce((s, p) => s + p.minutes, 0);
    // 4 periods * 720s * 5 players = 14400.
    expect(homeSum).toBe(4 * REG_SEC * 5);
    expect(awaySum).toBe(4 * REG_SEC * 5);
  });

  it('overtime: regulation + 1 OT — players who played OT get 300 extra seconds', () => {
    // Starters play all of regulation. Bench subs in for OT only.
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '1' }),
      ev('PERIOD_END', 0, { period: 1 }),
      ev('PERIOD_START', REG_SEC, { period: 2 }),
      ev('PERIOD_END', 0, { period: 2 }),
      ev('PERIOD_START', REG_SEC, { period: 3 }),
      ev('PERIOD_END', 0, { period: 3 }),
      ev('PERIOD_START', REG_SEC, { period: 4 }),
      ev('PERIOD_END', 0, { period: 4 }),
      // OT setup — bench in for the entire OT.
      ev('SUB', 0, { player_out_id: '1', player_in_id: '6', team: 'home' }),
      ev('SUB', 0, { player_out_id: '2', player_in_id: '7', team: 'home' }),
      ev('SUB', 0, { player_out_id: '3', player_in_id: '8', team: 'home' }),
      ev('SUB', 0, { player_out_id: '4', player_in_id: '9', team: 'home' }),
      ev('SUB', 0, { player_out_id: '5', player_in_id: '10', team: 'home' }),
      ev('PERIOD_START', 300, { period: 5 }),
      ev('POSSESSION_GAINED', 300, { team: 'home', player_id: '6' }),
      ev('PERIOD_END', 0, { period: 5 }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters(), OPTS);

    // Starters: 4 * 720 = 2880s each.
    for (const j of HOME_STARTERS) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home starter ${j}`).toBe(4 * REG_SEC);
    }
    // Bench: 300s OT only.
    for (const j of ['6', '7', '8', '9', '10']) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home bench ${j}`).toBe(300);
    }
    const homeSum = box.home.reduce((s, p) => s + p.minutes, 0);
    // 4 reg periods (4*720) + 1 OT (300) = 3180 total game-seconds; * 5 on court.
    expect(homeSum).toBe((4 * REG_SEC + 300) * 5);
  });

  it('no initialLineups → minutes stays 0 (caller did not provide data)', () => {
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('PASS', 360, { passer_id: '1', receiver_id: '2' }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters());

    for (const p of [...box.home, ...box.away]) {
      expect(p.minutes).toBe(0);
    }
  });

  it('every jersey that appeared on court has minutes > 0', () => {
    // Two SUBs: starter '4' out at 600s, starter '5' out at 300s.
    const events: Event[] = [
      ev('GAME_START', REG_SEC),
      ev('POSSESSION_GAINED', REG_SEC, { team: 'home', player_id: '1' }),
      ev('SUB', 600, { player_out_id: '4', player_in_id: '9', team: 'home' }),
      ev('SUB', 300, { player_out_id: '5', player_in_id: '10', team: 'home' }),
      ev('PERIOD_END', 0, { period: 1 }),
      ev('GAME_END', 0),
    ];

    const box = computeBoxScore(events, rosters(), OPTS);

    const appeared = ['1', '2', '3', '4', '5', '9', '10'];
    for (const j of appeared) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home appeared ${j}`).toBeGreaterThan(0);
    }
    // Benches that never appeared remain at 0.
    for (const j of ['6', '7', '8']) {
      const p = box.home.find((x) => x.jersey === j);
      expect(p?.minutes, `home unused ${j}`).toBe(0);
    }
  });
});
