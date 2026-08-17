import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/index.js';
import type { Event, GameState } from '../../src/state/types.js';
import { applyEvent } from '../../src/state/apply.js';
import { step } from '../../src/state/step.js';
import { makeEvent, makeState } from './_helpers.js';

describe('applyEvent immutability', () => {
  it('returns a NEW object — input identity is unchanged', () => {
    const s0 = makeState();
    const e = makeEvent({
      type: 'SHOT_RESULT',
      payload: { shooter_id: 'h1', shot_value: 2, made: true },
      actors: ['h1'],
    });
    const s1 = applyEvent(s0, e);
    expect(Object.is(s1, s0)).toBe(false);
  });

  it('returns the SAME output for an identical input cloned deeply (deep equality)', () => {
    const s0 = makeState();
    const e = makeEvent({
      type: 'SHOT_RESULT',
      payload: { shooter_id: 'h1', shot_value: 2, made: true },
      actors: ['h1'],
    });
    const clone = structuredClone(s0);
    const a = applyEvent(s0, e);
    const b = applyEvent(clone, e);
    expect(JSON.stringify(a)).toBe(JSON.stringify(b));
  });

  it('does NOT grow the input state.events array', () => {
    const s0 = makeState();
    expect(s0.events.length).toBe(0);
    const e = makeEvent({ type: 'STATE_NOTE', payload: { note: 'x' } });
    void applyEvent(s0, e);
    expect(s0.events.length).toBe(0);
  });

  it('does NOT mutate input score on SHOT_RESULT', () => {
    const s0 = makeState();
    const e = makeEvent({
      type: 'SHOT_RESULT',
      payload: { shooter_id: 'h1', shot_value: 2, made: true },
      actors: ['h1'],
    });
    void applyEvent(s0, e);
    expect(s0.score).toEqual({ home: 0, away: 0 });
  });

  it('does NOT mutate input ball.holderId on PASS', () => {
    const s0: GameState = {
      ...makeState(),
      ball: { holderId: 'h1', status: 'held', zone: 'frontcourt_center' },
    };
    const e = makeEvent({
      type: 'PASS',
      payload: { passer_id: 'h1', receiver_id: 'h3' },
      actors: ['h1', 'h3'],
    });
    void applyEvent(s0, e);
    expect(s0.ball.holderId).toBe('h1');
  });

  it('does NOT mutate input fouls on FOUL', () => {
    const s0 = makeState();
    const e = makeEvent({
      type: 'FOUL',
      payload: { offender_id: 'h3', offender_team: 'home', victim_id: 'a2', foul_type: 'personal' },
      actors: ['h3', 'a2'],
    });
    void applyEvent(s0, e);
    expect(s0.fouls.team.home).toBe(0);
    expect(s0.fouls.players['h3']).toBeUndefined();
  });

  it('does NOT mutate input possession on REBOUND defensive', () => {
    const s0: GameState = { ...makeState(), possession: { team: 'away' } };
    const e = makeEvent({
      type: 'REBOUND',
      payload: { rebounder_id: 'a3', team: 'away', offensive: false },
      actors: ['a3'],
    });
    void applyEvent(s0, e);
    expect(s0.possession.team).toBe('away');
  });

  it('does NOT mutate input lineups on SUB', () => {
    const s0 = makeState();
    const e = makeEvent({
      type: 'SUB',
      payload: { player_out_id: 'h3', player_in_id: 'h6', team: 'home' },
      actors: ['h3', 'h6'],
    });
    void applyEvent(s0, e);
    expect(s0.lineups.home).toEqual(['h1', 'h2', 'h3', 'h4', 'h5']);
  });

  it('does NOT mutate input events array reference (returns a new array)', () => {
    const s0 = makeState();
    const eventsRefBefore = s0.events;
    const e = makeEvent({ type: 'STATE_NOTE', payload: { note: 'x' } });
    const s1 = applyEvent(s0, e);
    expect(s1.events).not.toBe(eventsRefBefore);
    expect(eventsRefBefore.length).toBe(0); // original untouched
  });

  it('does NOT mutate input seq (returns a new number for seq)', () => {
    const s0 = makeState();
    expect(s0.seq).toBe(0);
    const e = makeEvent({ type: 'STATE_NOTE', payload: { note: 'x' } });
    const s1 = applyEvent(s0, e);
    expect(s0.seq).toBe(0);
    expect(s1.seq).toBe(1);
  });

  it('preserves immutability under a chain of 5 folds', () => {
    const s0 = makeState();
    const chain: Event[] = [
      makeEvent({ type: 'GAME_START', seq: 1 }),
      makeEvent({ type: 'POSSESSION_GAINED', payload: { team: 'home', player_id: 'h1' }, actors: ['h1'], seq: 2 }),
      makeEvent({ type: 'PASS', payload: { passer_id: 'h1', receiver_id: 'h3' }, actors: ['h1', 'h3'], seq: 3 }),
      makeEvent({ type: 'SHOT_RESULT', payload: { shooter_id: 'h3', shot_value: 2, made: true }, actors: ['h3'], seq: 4 }),
      makeEvent({ type: 'REBOUND', payload: { rebounder_id: 'a5', team: 'away', offensive: false }, actors: ['a5'], seq: 5 }),
    ];
    const final = chain.reduce(applyEvent, s0);
    expect(final.seq).toBe(5);
    expect(final.events.length).toBe(5);
    expect(final.score.home).toBe(2);
    expect(final.possession.team).toBe('home'); // away got the DREB → flipped to home
    expect(final.ball.holderId).toBe('a5');
    // Original state untouched.
    expect(s0.seq).toBe(0);
    expect(s0.events.length).toBe(0);
    expect(s0.score).toEqual({ home: 0, away: 0 });
  });
});

describe('step immutability (shell)', () => {
  it('returns { state, events: [] } — no events emitted in shell mode', () => {
    const s0 = makeState();
    const rng = mulberry32(1);
    const result = step(s0, rng);
    expect(result.events).toEqual([]);
  });

  it('returns the input state reference unchanged (shell no-op)', () => {
    const s0 = makeState();
    const rng = mulberry32(1);
    const result = step(s0, rng);
    // The shell returns the input state by reference so that the wrapper
    // object identity changes but the inner state is preserved; this is
    // what every later fill (T15+) will replace with a real fold.
    expect(result.state).toBe(s0);
  });

  it('returns a NEW wrapper object (not the input)', () => {
    const s0 = makeState();
    const rng = mulberry32(1);
    const result = step(s0, rng);
    expect(Object.is(result, s0)).toBe(false);
  });

  it('does NOT mutate the input state', () => {
    const s0 = makeState();
    const snapshot = structuredClone(s0);
    const rng = mulberry32(1);
    void step(s0, rng);
    expect(s0).toEqual(snapshot);
  });

  it('does NOT consume from the rng stream in shell mode', () => {
    // Two aligned rngs: one passed through step (no-op), one raw.
    const rngStep = mulberry32(42);
    const rngRaw = mulberry32(42);
    const s0 = makeState();
    void step(s0, rngStep);
    // If step consumed any draws, the next values would diverge.
    expect(rngStep.next()).toBe(rngRaw.next());
  });
});
