/**
 * Pins §10 event-stream consumption queries over a hand-built timeline:
 * observed rounds, on-court possessions, minutes load, per-game efficiency
 * and highlight/lowlight judgement.
 */
import { describe, it, expect } from 'vitest';

import { observedRounds, onCourtPossessions, minutesOnCourt, perGameEfficiency, highlightLowlight, elapsedSeconds } from '../../src/playerdata/consume.js';
import type { EventLike } from '../../src/playerdata/consume.js';

function ev(type: string, tGame: number, actors: string[] = [], payload: Record<string, unknown> = {}): EventLike {
  return { type, t_game: tGame, actors, payload };
}

const INITIAL = { home: ['h1', 'h2', 'h3', 'h4', 'h5'], away: ['a1', 'a2', 'a3', 'a4', 'a5'] };

function timeline(): EventLike[] {
  return [
    ev('GAME_START', 720),
    ev('POSSESSION_GAINED', 720, ['h1'], { team: 'home', player_id: 'h1' }),
    ev('SHOT_RESULT', 700, ['h3'], { shooter_id: 'h3', made: true, shot_value: 3 }),
    ev('POSSESSION_GAINED', 698, ['a1'], { team: 'away', player_id: 'a1' }),
    ev('SUB', 690, ['h5', 'h8'], { player_out_id: 'h5', player_in_id: 'h8', team: 'home' }),
    ev('POSSESSION_GAINED', 688, ['h1'], { team: 'home', player_id: 'h1' }),
    ev('SHOT_RESULT', 680, ['h1'], { shooter_id: 'h1', made: false, shot_value: 2 }),
    ev('SHOT_RESULT', 679, ['h1'], { shooter_id: 'h1', made: false, shot_value: 3 }),
    ev('POSSESSION_GAINED', 660, ['a1'], { team: 'away', player_id: 'a1' }),
    ev('SHOT_RESULT', 650, ['a1'], { shooter_id: 'a1', made: true, shot_value: 2 }),
    ev('PERIOD_END', 0, [], { period: 1 }),
  ];
}

describe('elapsedSeconds', () => {
  it('converts (period, t_game) to elapsed', () => {
    expect(elapsedSeconds(1, 720)).toBe(0);
    expect(elapsedSeconds(1, 700)).toBe(20);
    expect(elapsedSeconds(4, 0)).toBe(2880);
    expect(elapsedSeconds(5, 300)).toBe(2880);
    expect(elapsedSeconds(5, 0)).toBe(3180);
  });
});

describe('observedRounds (§10.2 q1)', () => {
  it('counts events where the player is an actor or defender', () => {
    const events = timeline();
    expect(observedRounds(events, 'h1')).toBe(4); // PG + 2×POSSESSION_GAINED + SHOT_RESULT
    expect(observedRounds(events, 'h3')).toBe(1);
    expect(observedRounds(events, 'h5')).toBe(1);
    expect(observedRounds(events, 'nobody')).toBe(0);
  });
});

describe('onCourtPossessions (§10.2 q2)', () => {
  it('counts possessions while the player is on the court — both teams, following subs', () => {
    const events = timeline();
    // h1 is on court for all 4 possessions (2 home + 2 away).
    expect(onCourtPossessions(events, 'h1', INITIAL)).toBe(4);
    // h8 enters at the SUB (t=690): possessions at 688 and 660 → 2.
    expect(onCourtPossessions(events, 'h8', INITIAL)).toBe(2);
    // h5 leaves at 690: possessions at 720 and 698 → 2.
    expect(onCourtPossessions(events, 'h5', INITIAL)).toBe(2);
  });
});

describe('minutesOnCourt (§10.2 q4)', () => {
  it('credits on-court time between events and at period boundaries', () => {
    const events = timeline();
    // h1 on court for the whole 720s of Q1 → 12 minutes.
    expect(minutesOnCourt(events, 'h1', INITIAL)).toBeCloseTo(12, 6);
    // h5 leaves at t_game=690 (30s played) → 0.5 min.
    expect(minutesOnCourt(events, 'h5', INITIAL)).toBeCloseTo(0.5, 6);
    // h8 enters at 690 → 11.5 min.
    expect(minutesOnCourt(events, 'h8', INITIAL)).toBeCloseTo(11.5, 6);
  });
});

describe('perGameEfficiency + highlightLowlight (§10.2 q3)', () => {
  it('aggregates FGM/FGA/PTS and ranks within the supplied roster', () => {
    const HOME = ['h1', 'h2', 'h3', 'h4', 'h5'];
    const eff = perGameEfficiency(timeline(), HOME);
    const h1 = eff.find((r) => r.jersey === 'h1');
    const h3 = eff.find((r) => r.jersey === 'h3');
    expect(h1).toEqual({ jersey: 'h1', fgm: 0, fga: 2, pts: 0, pct: 0, rank: 2 });
    expect(h3).toEqual({ jersey: 'h3', fgm: 1, fga: 1, pts: 3, pct: 1, rank: 1 });
    expect(eff).toHaveLength(2);
    // Away shooters are excluded by the roster filter.
    expect(eff.find((r) => r.jersey === 'a1')).toBeUndefined();
  });

  it('judges highlights (rank ≤ 2) and lowlights (FG%<30% & FGA≥10)', () => {
    const HOME = ['h1', 'h2', 'h3', 'h4', 'h5'];
    const events = timeline();
    const extra: EventLike[] = [];
    for (let i = 0; i < 10; i++) {
      extra.push(ev('SHOT_RESULT', 600 - i, ['a1'], { shooter_id: 'a1', made: false, shot_value: 2 }));
    }
    const eff = perGameEfficiency([...events, ...extra], HOME);
    const { highlight, lowlight } = highlightLowlight(eff);
    // h3 (3 pts) rank 1, h1 (0 pts) rank 2 → both highlighted.
    expect(highlight).toContain('h3');
    expect(highlight).toContain('h1');
    // a1 is filtered out by the roster; its own lowlight is judged on the
    // away roster separately.
    expect(lowlight).not.toContain('h1');
    const awayEff = perGameEfficiency([...events, ...extra], ['a1', 'a2', 'a3', 'a4', 'a5']);
    const { lowlight: awayLow } = highlightLowlight(awayEff);
    // a1: 1/11 FG = 9.1% < 30%, 11 attempts ≥ 10 → lowlight.
    expect(awayLow).toContain('a1');
  });
});
