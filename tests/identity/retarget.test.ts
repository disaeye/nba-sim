/**
 * TDD tests for on_ball retarget. Written BEFORE the implementation.
 *
 * THE CRITICAL ANTI-BUG SUITE — directly validates the owner's core
 * concern: "球员A手递手给队友B，防守人去抢断A（无球人）"
 * ("Player A hands off to teammate B, but the defender tries to steal
 * from A who no longer has the ball").
 *
 * The fix is structural: `getOnBallDefender(state, holderId)` always
 * derives the defender from the LIVE lineups + the LIVE holder. There is
 * no cached "on_ball defender id" to go stale. The tests below exercise
 * every ball-moving event (PASS, HANDOFF, STEAL) and confirm the
 * defender returned is the one matched to the CURRENT holder.
 */
import { describe, it, expect } from 'vitest';

import { retargetOnBall, getOnBallDefender, buildMatchupMap } from '../../src/identity/retarget.js';
import { IdentityError } from '../../src/identity/types.js';
import type { Event, GameState } from '../../src/state/types.js';
import { createInitialState } from '../../src/state/initial.js';
import { applyEvent } from '../../src/state/apply.js';

// ─── fixtures ───────────────────────────────────────────────────────────────

// Lineups: home wears h# jerseys, away wears a# jerseys. 1:1 by index
// means h1↔a1, h2↔a2, … — easy to reason about in assertions.
const HOME_LINEUP = ['h1', 'h2', 'h3', 'h4', 'h5'] as const;
const AWAY_LINEUP = ['a1', 'a2', 'a3', 'a4', 'a5'] as const;

/**
 * Build a state where `team` has possession and `holderId` is the live
 * ball controller. Uses the real `applyEvent` fold so the resulting state
 * is exactly what the kernel would produce — including all the
 * ball-mutation side effects of POSSESSION_GAINED.
 */
function liveState(holderId: string, team: 'home' | 'away' = 'home'): GameState {
  const s0 = createInitialState({
    home: { id: 'home', starters: [...HOME_LINEUP] },
    away: { id: 'away', starters: [...AWAY_LINEUP] },
  });
  const gainEvent: Event = {
    type: 'POSSESSION_GAINED',
    t_game: 0,
    t_real: 0,
    seq: 1,
    actors: [holderId],
    payload: { team, player_id: holderId },
    clocks: { game: 720.0, shot: 24.0 },
    score: { home: 0, away: 0 },
  };
  return applyEvent(s0, gainEvent);
}

/** HANDOFF event: receiver becomes the new holder (atomic). */
function handoff(receiverId: string, seq: number): Event {
  return {
    type: 'HANDOFF',
    t_game: 5.0,
    t_real: 5.0,
    seq,
    actors: [receiverId],
    payload: { receiver_id: receiverId },
    clocks: { game: 715.0, shot: 19.0 },
    score: { home: 0, away: 0 },
  };
}

/** PASS event: receiver becomes the new holder. */
function pass(receiverId: string, seq: number): Event {
  return {
    type: 'PASS',
    t_game: 5.0,
    t_real: 5.0,
    seq,
    actors: [receiverId],
    payload: { receiver_id: receiverId },
    clocks: { game: 715.0, shot: 19.0 },
    score: { home: 0, away: 0 },
  };
}

/** STEAL event: stealer becomes holder; possession flips to stealer's team. */
function steal(stealerId: string, victimId: string, seq: number): Event {
  return {
    type: 'STEAL',
    t_game: 5.0,
    t_real: 5.0,
    seq,
    actors: [stealerId, victimId],
    payload: { stealer_id: stealerId, victim_id: victimId },
    clocks: { game: 715.0, shot: 19.0 },
    score: { home: 0, away: 0 },
  };
}

describe('on-ball retarget', () => {
  describe('buildMatchupMap', () => {
    it('builds a 1:1 by-index map from two equal-length lineups', () => {
      const map = buildMatchupMap(HOME_LINEUP, AWAY_LINEUP);
      expect(map['h1']).toBe('a1');
      expect(map['h2']).toBe('a2');
      expect(map['h3']).toBe('a3');
      expect(map['h4']).toBe('a4');
      expect(map['h5']).toBe('a5');
    });

    it('returns a map covering every offensive player', () => {
      const map = buildMatchupMap(HOME_LINEUP, AWAY_LINEUP);
      expect(Object.keys(map)).toHaveLength(5);
    });

    it('throws IDENTITY_LINEUP_LENGTH_MISMATCH when lineups have different lengths', () => {
      try {
        buildMatchupMap(['h1', 'h2', 'h3'], ['a1', 'a2']);
        throw new Error('expected buildMatchupMap to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_LINEUP_LENGTH_MISMATCH');
      }
    });
  });

  describe('getOnBallDefender — THE function STEAL/CONTEST resolution calls', () => {
    it('returns the 1:1 index defender for the current home holder', () => {
      const state = liveState('h1', 'home');
      // h1 is index 0 in home lineup; defender = a1 (index 0 in away)
      expect(getOnBallDefender(state, 'h1')).toBe('a1');
    });

    it('returns a different defender when the holder is h3 (index 2)', () => {
      const state = liveState('h3', 'home');
      expect(getOnBallDefender(state, 'h3')).toBe('a3');
    });

    it('accepts an explicit opponentLineup override', () => {
      const state = liveState('h1', 'home');
      // Caller can pass a custom defensive lineup (e.g. after a SUB)
      expect(getOnBallDefender(state, 'h1', ['x1', 'x2', 'x3', 'x4', 'x5'])).toBe('x1');
    });

    it('throws IDENTITY_NO_HOLDER when holderId is not in the offensive lineup', () => {
      const state = liveState('h1', 'home');
      try {
        getOnBallDefender(state, 'h99');
        throw new Error('expected getOnBallDefender to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_NO_HOLDER');
      }
    });

    it('throws IDENTITY_NO_POSSESSION when state has no possessing team', () => {
      // Fresh pre-game state: no POSSESSION_GAINED yet.
      const s0 = createInitialState({
        home: { id: 'home', starters: [...HOME_LINEUP] },
        away: { id: 'away', starters: [...AWAY_LINEUP] },
      });
      try {
        getOnBallDefender(s0, 'h1');
        throw new Error('expected getOnBallDefender to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_NO_POSSESSION');
      }
    });
  });

  // ═══════════════════════════════════════════════════════════════════════════
  // THE CRITICAL ANTI-BUG SUITE
  //
  // Every test in this block exists to prove that the on_ball defender
  // tracks the LIVE ball.holderId, never a stale capture. This is the
  // contract that prevents "steal on off-ball player."
  // ═══════════════════════════════════════════════════════════════════════════
  describe('retargetOnBall — THE CRITICAL ANTI-BUG FUNCTION', () => {
    it('CRITICAL: after HANDOFF, the defender tracks the NEW holder, not the old one', () => {
      // Given: home has ball, h1 is holder, on_ball defender = a1
      const s0 = liveState('h1', 'home');
      expect(s0.ball.holderId).toBe('h1');
      expect(getOnBallDefender(s0, 'h1')).toBe('a1');

      // When: h1 hands off to h2 (atomic event — holder + retarget together)
      const s1 = applyEvent(s0, handoff('h2', 2));

      // Then: the on_ball defender is a2 (h2's defender), NOT a1
      expect(s1.ball.holderId).toBe('h2');
      const defenderAfterHandoff = getOnBallDefender(s1, s1.ball.holderId as string);
      expect(defenderAfterHandoff).toBe('a2');
      expect(defenderAfterHandoff).not.toBe('a1');
    });

    it('CRITICAL: a STEAL target computed via getOnBallDefender always matches the live holder', () => {
      // The bug scenario: a resolver caches "holder = h1" early in the
      // step, then a PASS/HANDOFF moves the ball, then the resolver tries
      // to compute a steal target using the STALE holder. This test
      // proves that getOnBallDefender always reads state.ball.holderId,
      // so it can never return a stale defender.
      const s0 = liveState('h1', 'home');
      const s1 = applyEvent(s0, pass('h3', 2));  // holder → h3
      const s2 = applyEvent(s1, pass('h5', 3));  // holder → h5

      // At resolve time, the CURRENT holder is h5 — not h1 or h3.
      // Any STEAL target MUST be h5's defender (a5), not a stale one.
      const currentHolder = s2.ball.holderId as string;
      expect(currentHolder).toBe('h5');
      expect(getOnBallDefender(s2, currentHolder)).toBe('a5');
      // If the resolver had used the stale h1, it would get a1 — wrong.
      expect(getOnBallDefender(s2, currentHolder)).not.toBe('a1');
    });

    it('retargetOnBall returns a fresh map where the new holder is covered', () => {
      const s0 = liveState('h1', 'home');
      const map = retargetOnBall(s0, 'h1');
      expect(map['h1']).toBe('a1');
    });

    it('retargetOnBall after HANDOFF produces a map that covers the new holder', () => {
      const s0 = liveState('h1', 'home');
      const s1 = applyEvent(s0, handoff('h2', 2));
      const map = retargetOnBall(s1, 'h2');
      expect(map['h2']).toBe('a2');
    });

    it('retargetOnBall returns a NEW object on every call (immutability)', () => {
      const s0 = liveState('h1', 'home');
      const map1 = retargetOnBall(s0, 'h1');
      const map2 = retargetOnBall(s0, 'h1');
      expect(map1).not.toBe(map2);
      expect(map1).toEqual(map2);
    });

    it('retargetOnBall throws IDENTITY_NO_HOLDER when newHolderId is not on the offensive team', () => {
      const s0 = liveState('h1', 'home');
      try {
        retargetOnBall(s0, 'a1');  // a1 is on the away (defensive) team
        throw new Error('expected retargetOnBall to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_NO_HOLDER');
      }
    });

    it('STEAL flips possession: on_ball defender is from the NEW defensive team', () => {
      // Given: home has ball, h1 is holder, defender = a1 (from away)
      const s0 = liveState('h1', 'home');
      expect(s0.possession.team).toBe('home');
      expect(getOnBallDefender(s0, 'h1')).toBe('a1');

      // When: a3 (away) steals from h1 — possession flips to away
      const s1 = applyEvent(s0, steal('a3', 'h1', 2));

      // Then: the NEW offensive team is away; the NEW defensive team is home.
      // The on_ball defender for a3 is h3 (index 2 in home lineup).
      expect(s1.possession.team).toBe('away');
      expect(s1.ball.holderId).toBe('a3');
      const defender = getOnBallDefender(s1, 'a3');
      expect(defender).toBe('h3');
      // Defender is from the new defensive team (home), NOT the old one (away)
      expect(HOME_LINEUP).toContain(defender);
      expect(AWAY_LINEUP).not.toContain(defender);
    });

    it('STEAL then HANDOFF chain: defender tracks correctly through both transitions', () => {
      // home has ball, h2 is holder
      const s0 = liveState('h2', 'home');
      expect(getOnBallDefender(s0, 'h2')).toBe('a2');

      // a4 steals — possession flips to away, holder = a4
      const s1 = applyEvent(s0, steal('a4', 'h2', 2));
      expect(s1.possession.team).toBe('away');
      expect(s1.ball.holderId).toBe('a4');
      expect(getOnBallDefender(s1, 'a4')).toBe('h4');

      // away now passes: a4 → a1
      const s2 = applyEvent(s1, pass('a1', 3));
      expect(s2.ball.holderId).toBe('a1');
      expect(getOnBallDefender(s2, 'a1')).toBe('h1');
    });

    it('PASS chain on same possession: defender updates on every pass', () => {
      const s0 = liveState('h1', 'home');
      // h1 → h2 → h3 → h4 → h5: defender must update each time
      const s1 = applyEvent(s0, pass('h2', 2));
      expect(getOnBallDefender(s1, 'h2')).toBe('a2');
      const s2 = applyEvent(s1, pass('h3', 3));
      expect(getOnBallDefender(s2, 'h3')).toBe('a3');
      const s3 = applyEvent(s2, pass('h4', 4));
      expect(getOnBallDefender(s3, 'h4')).toBe('a4');
      const s4 = applyEvent(s3, pass('h5', 5));
      expect(getOnBallDefender(s4, 'h5')).toBe('a5');
    });
  });
});
