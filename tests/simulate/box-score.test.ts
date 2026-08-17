/**
 * T19 box-score aggregation tests.
 *
 * Tests `computeBoxScore(events, rosters)` in isolation using
 * hand-crafted event sequences. The simulate integration tests cover
 * the end-to-end path; here we verify the per-stat-type aggregation
 * logic.
 *
 * Coverage map:
 *   - SHOT_RESULT made/miss → fgm/fga/tpm/tpa/points
 *   - REBOUND offensive/defensive → oreb/dreb
 *   - STEAL → stl
 *   - TURNOVER → tov
 *   - FOUL → pf
 *   - FT_RESULT made/miss → ftm/fta/points
 *   - sum(player points) === team score
 *   - fgm <= fga
 *   - players not in events appear with all-zero stats
 */
import { describe, it, expect } from 'vitest';
import { computeBoxScore } from '../../src/box-score.js';
import type { Player, PlayerBox } from '../../src/box-score.js';

// ─── fixtures ───────────────────────────────────────────────────────────────

const HOME_ROSTER: Player[] = [
  { id: 'p1', jersey: '1', teamId: 'home' },
  { id: 'p2', jersey: '2', teamId: 'home' },
];
const AWAY_ROSTER: Player[] = [
  { id: 'p3', jersey: '3', teamId: 'away' },
  { id: 'p4', jersey: '4', teamId: 'away' },
];

function rosters() {
  return {
    home: new Map(HOME_ROSTER.map((p) => [p.jersey, p])),
    away: new Map(AWAY_ROSTER.map((p) => [p.jersey, p])),
  };
}

/** Build a minimal event with the given type + payload, defaulting envelope fields. */
function ev(type: string, payload: Record<string, unknown>): any {
  return {
    type,
    t_game: 0,
    t_real: 0,
    seq: 0,
    actors: [],
    payload,
    clocks: { game: 0, shot: 0 },
    score: { home: 0, away: 0 },
    period: 1,
  };
}

describe('computeBoxScore', () => {
  it('empty events → all roster players with zero stats', () => {
    const box = computeBoxScore([], rosters());
    expect(box.home).toHaveLength(2);
    expect(box.away).toHaveLength(2);
    for (const p of [...box.home, ...box.away]) {
      expect(p.points).toBe(0);
      expect(p.fgm).toBe(0);
      expect(p.fga).toBe(0);
      expect(p.minutes).toBe(0);
    }
  });

  it('SHOT_RESULT made 2pt → fgm=1, fga=1, tpm=0, tpa=0, points=2', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: true }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.fgm).toBe(1);
    expect(p1.fga).toBe(1);
    expect(p1.tpm).toBe(0);
    expect(p1.tpa).toBe(0);
    expect(p1.points).toBe(2);
  });

  it('SHOT_RESULT made 3pt → fgm=1, fga=1, tpm=1, tpa=1, points=3', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 3, made: true }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.fgm).toBe(1);
    expect(p1.fga).toBe(1);
    expect(p1.tpm).toBe(1);
    expect(p1.tpa).toBe(1);
    expect(p1.points).toBe(3);
  });

  it('SHOT_RESULT missed 2pt → fgm=0, fga=1, points=0', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.fgm).toBe(0);
    expect(p1.fga).toBe(1);
    expect(p1.points).toBe(0);
  });

  it('SHOT_RESULT missed 3pt → fgm=0, fga=1, tpa=1, tpm=0', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 3, made: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.fga).toBe(1);
    expect(p1.tpa).toBe(1);
    expect(p1.fgm).toBe(0);
    expect(p1.tpm).toBe(0);
  });

  it('multiple shots accumulate correctly', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: true }),
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 3, made: true }),
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: false }),
      ev('SHOT_RESULT', { shooter_id: '2', shot_value: 3, made: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.fgm).toBe(2);
    expect(p1.fga).toBe(3);
    expect(p1.tpm).toBe(1);
    expect(p1.tpa).toBe(1);
    expect(p1.points).toBe(5); // 2 + 3
    const p2 = box.home.find((p) => p.jersey === '2')!;
    expect(p2.fga).toBe(1);
    expect(p2.tpa).toBe(1);
    expect(p2.points).toBe(0);
  });

  it('REBOUND offensive → oreb+1; defensive → dreb+1', () => {
    const events = [
      ev('REBOUND', { rebounder_id: '1', team: 'home', offensive: true }),
      ev('REBOUND', { rebounder_id: '3', team: 'away', offensive: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.oreb).toBe(1);
    expect(p1.dreb).toBe(0);
    const p3 = box.away.find((p) => p.jersey === '3')!;
    expect(p3.dreb).toBe(1);
    expect(p3.oreb).toBe(0);
  });

  it('STEAL → stl+1', () => {
    const events = [
      ev('STEAL', { stealer_id: '3', victim_id: '1' }),
    ];
    const box = computeBoxScore(events, rosters());
    const p3 = box.away.find((p) => p.jersey === '3')!;
    expect(p3.stl).toBe(1);
  });

  it('TURNOVER → tov+1', () => {
    const events = [
      ev('TURNOVER', { player_id: '1', team: 'home' }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.tov).toBe(1);
  });

  it('FOUL → pf+1', () => {
    const events = [
      ev('FOUL', { offender_id: '3', offender_team: 'away', victim_id: '1', foul_type: 'personal' }),
    ];
    const box = computeBoxScore(events, rosters());
    const p3 = box.away.find((p) => p.jersey === '3')!;
    expect(p3.pf).toBe(1);
  });

  it('FT_RESULT made → ftm+1, fta+1, points+1', () => {
    const events = [
      ev('FT_RESULT', { shooter_id: '1', attempt_number: 1, made: true }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.ftm).toBe(1);
    expect(p1.fta).toBe(1);
    expect(p1.points).toBe(1);
  });

  it('FT_RESULT missed → ftm=0, fta+1, points=0', () => {
    const events = [
      ev('FT_RESULT', { shooter_id: '1', attempt_number: 1, made: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    expect(p1.ftm).toBe(0);
    expect(p1.fta).toBe(1);
    expect(p1.points).toBe(0);
  });

  it('combined stats: points reconcile with fgm/tpm/ftm', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: true }),
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 3, made: true }),
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: false }),
      ev('FT_RESULT', { shooter_id: '1', attempt_number: 1, made: true }),
      ev('FT_RESULT', { shooter_id: '1', attempt_number: 2, made: false }),
    ];
    const box = computeBoxScore(events, rosters());
    const p1 = box.home.find((p) => p.jersey === '1')!;
    // points = 2*(fgm-tpm) + 3*tpm + ftm = 2*1 + 3*1 + 1 = 6
    expect(p1.points).toBe(6);
    expect(p1.fgm).toBe(2);
    expect(p1.fga).toBe(3);
    expect(p1.tpm).toBe(1);
    expect(p1.tpa).toBe(1);
    expect(p1.ftm).toBe(1);
    expect(p1.fta).toBe(2);
  });

  it('team total: sum of player points === event-derived team score', () => {
    const events = [
      ev('SHOT_RESULT', { shooter_id: '1', shot_value: 2, made: true }),
      ev('SHOT_RESULT', { shooter_id: '2', shot_value: 3, made: true }),
      ev('FT_RESULT', { shooter_id: '1', attempt_number: 1, made: true }),
      ev('SHOT_RESULT', { shooter_id: '3', shot_value: 2, made: true }),
    ];
    const box = computeBoxScore(events, rosters());
    const homeTotal = box.home.reduce((s, p) => s + p.points, 0);
    const awayTotal = box.away.reduce((s, p) => s + p.points, 0);
    expect(homeTotal).toBe(6); // 2 + 3 + 1
    expect(awayTotal).toBe(2);
  });
});
