/**
 * TDD tests for the rules FSM driver (todo 15).
 *
 * Covers:
 *   - transition(state, trigger): table-driven lookup of (phase, trigger)
 *     into TransitionResult | null.
 *   - periodRouter(periodJustEnded, scoreDiff): deterministic routing for
 *     end-of-period transitions.
 *   - canCallTimeout(state, team): DEAD_* phase + remaining > 0.
 *   - canSubstitute(state): phase ∈ legal_phases (FT_SEQUENCE excluded).
 *   - timeoutAllotment(period): 7 reg / 4 in Q4 / 2 OT.
 *   - isBonus(teamFouls): threshold ≥ 5.
 */
import { describe, it, expect } from 'vitest';

import type { GameInput, GameState, Phase, TeamId } from '../../src/state/types.js';
import { createInitialState } from '../../src/state/initial.js';
import {
  canCallTimeout,
  canSubstitute,
  isBonus,
  periodRouter,
  timeoutAllotment,
  transition,
} from '../../src/rules/index.js';

// ─── fixtures ───────────────────────────────────────────────────────────────

const INPUT: GameInput = {
  home: { id: 'home', starters: ['h1', 'h2', 'h3', 'h4', 'h5'] },
  away: { id: 'away', starters: ['a1', 'a2', 'a3', 'a4', 'a5'] },
};

function makeState(): GameState {
  return createInitialState(INPUT);
}

function withPhase(phase: Phase): GameState {
  return { ...makeState(), phase };
}

function withTimeouts(phase: Phase, team: TeamId, remaining: number): GameState {
  const base = withPhase(phase);
  return {
    ...base,
    timeouts: {
      remaining:
        team === 'home'
          ? { ...base.timeouts.remaining, home: remaining }
          : { ...base.timeouts.remaining, away: remaining },
    },
  };
}

// ─── transition ─────────────────────────────────────────────────────────────

describe('transition', () => {
  it('returns { phase: JUMP_BALL } for PRE_GAME + GAME_START', () => {
    const result = transition(withPhase('PRE_GAME'), 'GAME_START');
    expect(result).not.toBeNull();
    expect(result?.phase).toBe('JUMP_BALL');
    expect(result?.emit_events).toEqual(['GAME_START']);
  });

  it('returns { phase: DEAD_MAKE } for LIVE + MADE_BASKET with full emit_events', () => {
    const result = transition(withPhase('LIVE'), 'MADE_BASKET');
    expect(result).not.toBeNull();
    expect(result?.phase).toBe('DEAD_MAKE');
    expect(result?.emit_events).toEqual(['SHOT_RESULT', 'MADE_BASKET_DEAD']);
  });

  it('carries the row notes through to the result', () => {
    const result = transition(withPhase('PRE_GAME'), 'GAME_START');
    expect(result).not.toBeNull();
    expect(typeof result?.notes).toBe('string');
    expect(result?.notes.length).toBeGreaterThan(0);
  });

  it('returns null for an invalid trigger from JUMP_BALL', () => {
    const result = transition(withPhase('JUMP_BALL'), 'INVALID_TRIGGER');
    expect(result).toBeNull();
  });

  it('returns null when the trigger is valid but the phase is wrong', () => {
    // GAME_START is only legal from PRE_GAME, not from LIVE.
    const result = transition(withPhase('LIVE'), 'GAME_START');
    expect(result).toBeNull();
  });

  it('does not mutate the input state', () => {
    const state = withPhase('PRE_GAME');
    void transition(state, 'GAME_START');
    expect(state.phase).toBe('PRE_GAME');
  });
});

// ─── periodRouter ───────────────────────────────────────────────────────────

describe('periodRouter', () => {
  it('Q1 ended → PERIOD_BREAK regardless of score', () => {
    expect(periodRouter(1, 0)).toBe('PERIOD_BREAK');
    expect(periodRouter(1, 10)).toBe('PERIOD_BREAK');
  });

  it('Q2 ended → HALFTIME regardless of score', () => {
    expect(periodRouter(2, 0)).toBe('HALFTIME');
    expect(periodRouter(2, 10)).toBe('HALFTIME');
  });

  it('Q3 ended → PERIOD_BREAK', () => {
    expect(periodRouter(3, 0)).toBe('PERIOD_BREAK');
  });

  it('Q4 tied → OVERTIME_SETUP', () => {
    expect(periodRouter(4, 0)).toBe('OVERTIME_SETUP');
  });

  it('Q4 not tied → POST_GAME', () => {
    expect(periodRouter(4, 5)).toBe('POST_GAME');
  });

  it('OT tied → OVERTIME_SETUP', () => {
    expect(periodRouter(5, 0)).toBe('OVERTIME_SETUP');
  });

  it('OT not tied → POST_GAME', () => {
    expect(periodRouter(5, 3)).toBe('POST_GAME');
  });
});

// ─── canCallTimeout ─────────────────────────────────────────────────────────

describe('canCallTimeout', () => {
  it('returns false when phase is LIVE (ball is live)', () => {
    expect(canCallTimeout(withPhase('LIVE'), 'home')).toBe(false);
  });

  it('returns true when phase is DEAD_OOB and team has timeouts remaining', () => {
    expect(canCallTimeout(withTimeouts('DEAD_OOB', 'home', 3), 'home')).toBe(true);
  });

  it('returns false when team has 0 timeouts remaining', () => {
    expect(canCallTimeout(withTimeouts('DEAD_OOB', 'home', 0), 'home')).toBe(false);
  });
});

// ─── canSubstitute ──────────────────────────────────────────────────────────

describe('canSubstitute', () => {
  it('returns false when phase is LIVE', () => {
    expect(canSubstitute(withPhase('LIVE'))).toBe(false);
  });

  it('returns true when phase is DEAD_OOB', () => {
    expect(canSubstitute(withPhase('DEAD_OOB'))).toBe(true);
  });

  it('returns false when phase is FT_SEQUENCE (mid-FT)', () => {
    expect(canSubstitute(withPhase('FT_SEQUENCE'))).toBe(false);
  });
});

// ─── timeoutAllotment ───────────────────────────────────────────────────────

describe('timeoutAllotment', () => {
  it('returns 7 for period 1 (regulation)', () => {
    expect(timeoutAllotment(1)).toBe(7);
  });

  it('returns 7 for period 3 (regulation)', () => {
    expect(timeoutAllotment(3)).toBe(7);
  });

  it('returns 4 for period 4 (max in Q4)', () => {
    expect(timeoutAllotment(4)).toBe(4);
  });

  it('returns 2 for period 5 (overtime)', () => {
    expect(timeoutAllotment(5)).toBe(2);
  });
});

// ─── isBonus ────────────────────────────────────────────────────────────────

describe('isBonus', () => {
  it('returns false when team fouls = 4', () => {
    expect(isBonus(4)).toBe(false);
  });

  it('returns true when team fouls = 5', () => {
    expect(isBonus(5)).toBe(true);
  });

  it('returns true when team fouls = 6 (deep bonus)', () => {
    expect(isBonus(6)).toBe(true);
  });
});
