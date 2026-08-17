import { describe, it, expect } from 'vitest';

import type { GameState } from '../../src/state/types.js';
import { applyEvent } from '../../src/state/apply.js';
import { makeEvent, makeState } from './_helpers.js';

describe('applyEvent fold', () => {
  describe('every event: append + seq increment', () => {
    it('appends the event to state.events and increments seq', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'STATE_NOTE', payload: { note: 'debug' } });
      const s1 = applyEvent(s0, e);
      expect(s1.events.length).toBe(1);
      expect(s1.events[0]).toBe(e);
      expect(s1.seq).toBe(1);
    });

    it('preserves prior events when appending (timeline grows monotonically)', () => {
      const s0 = makeState();
      const e1 = makeEvent({ type: 'STATE_NOTE', payload: { note: 'first' }, seq: 1 });
      const s1 = applyEvent(s0, e1);
      const e2 = makeEvent({ type: 'STATE_NOTE', payload: { note: 'second' }, seq: 2 });
      const s2 = applyEvent(s1, e2);
      expect(s2.events.length).toBe(2);
      expect(s2.events[0]).toBe(e1);
      expect(s2.events[1]).toBe(e2);
      expect(s2.seq).toBe(2);
    });
  });

  describe('events with empty mutates do not change gameplay state', () => {
    // STATE_NOTE, ALIGN_HALFCOURT, SCREEN_SET, SCREEN_USE, SHOT_RELEASE,
    // FT_ATTEMPT all have empty `mutates` per the catalog — read-only markers.
    it('STATE_NOTE leaves every non-events/seq field equal to input', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'STATE_NOTE', payload: { note: 'x' } });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe(s0.phase);
      expect(s1.score).toEqual(s0.score);
      expect(s1.ball).toEqual(s0.ball);
      expect(s1.possession).toEqual(s0.possession);
      expect(s1.lineups).toEqual(s0.lineups);
      expect(s1.fouls).toEqual(s0.fouls);
      expect(s1.timeouts).toEqual(s0.timeouts);
      expect(s1.period).toBe(s0.period);
      expect(s1.clocks).toEqual(s0.clocks);
    });

    it('ALIGN_HALFCOURT is a no-op on gameplay state', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'ALIGN_HALFCOURT', payload: { team: 'home' } });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe(s0.phase);
      expect(s1.events.length).toBe(1);
    });

    it('SCREEN_SET is a no-op on gameplay state', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'SCREEN_SET', payload: { screener_id: 'h4' }, actors: ['h4'] });
      const s1 = applyEvent(s0, e);
      expect(s1.events.length).toBe(1);
      expect(s1.ball).toEqual(s0.ball);
    });
  });

  describe('SHOT_RESULT', () => {
    it('made=true shot_value=2 for home → score.home +2', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'SHOT_RESULT',
        payload: { shooter_id: 'h1', shot_value: 2, made: true },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.score.home).toBe(2);
      expect(s1.score.away).toBe(0);
    });

    it('made=true shot_value=3 for away → score.away +3', () => {
      const s0 = makeState();
      // Pre-set possession to away so the shooter's team is away.
      const setup: GameState = { ...s0, possession: { team: 'away' } };
      const e = makeEvent({
        type: 'SHOT_RESULT',
        payload: { shooter_id: 'a1', shot_value: 3, made: true },
        actors: ['a1'],
      });
      const s1 = applyEvent(setup, e);
      expect(s1.score.away).toBe(3);
      expect(s1.score.home).toBe(0);
    });

    it('made=false → score unchanged', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'SHOT_RESULT',
        payload: { shooter_id: 'h1', shot_value: 2, made: false },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.score).toEqual({ home: 0, away: 0 });
    });

    it('made=true accumulates with prior score', () => {
      const s0: GameState = { ...makeState(), score: { home: 5, away: 3 } };
      const e = makeEvent({
        type: 'SHOT_RESULT',
        payload: { shooter_id: 'h1', shot_value: 2, made: true },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.score.home).toBe(7);
      expect(s1.score.away).toBe(3);
    });
  });

  describe('FT_RESULT', () => {
    it('made=true for home → score.home +1', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'home' } };
      const e = makeEvent({
        type: 'FT_RESULT',
        payload: { shooter_id: 'h1', attempt_number: 1, made: true },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.score.home).toBe(1);
    });

    it('made=false → score unchanged', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'away' } };
      const e = makeEvent({
        type: 'FT_RESULT',
        payload: { shooter_id: 'a1', attempt_number: 2, made: false },
        actors: ['a1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.score).toEqual({ home: 0, away: 0 });
    });
  });

  describe('PASS / HANDOFF', () => {
    it('PASS sets ball.holderId to receiver_id', () => {
      const s0: GameState = { ...makeState(), ball: { holderId: 'h1', status: 'held', zone: 'frontcourt_center' } };
      const e = makeEvent({
        type: 'PASS',
        payload: { passer_id: 'h1', receiver_id: 'h3' },
        actors: ['h1', 'h3'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.ball.holderId).toBe('h3');
      // Non-touched fields preserved.
      expect(s1.ball.status).toBe('held');
      expect(s1.ball.zone).toBe('frontcourt_center');
    });
    it('PASS flight_start clears holder until flight_complete', () => {
      const s0: GameState = { ...makeState(), ball: { holderId: 'h1', status: 'held', zone: 'frontcourt_center' } };
      const start = makeEvent({
        type: 'PASS',
        payload: { passer_id: 'h1', receiver_id: 'h3', note: 'flight_start' },
        actors: ['h1', 'h3'],
      });
      const inFlight = applyEvent(s0, start);
      expect(inFlight.ball.holderId).toBeNull();
      expect(inFlight.ball.status).toBe('pass');
      const complete = makeEvent({
        type: 'PASS',
        payload: { passer_id: 'h1', receiver_id: 'h3', note: 'flight_complete' },
        actors: ['h1', 'h3'],
        seq: inFlight.seq + 1,
      });
      const received = applyEvent(inFlight, complete);
      expect(received.ball.holderId).toBe('h3');
      expect(received.ball.status).toBe('held');
    });


    it('HANDOFF sets ball.holderId to receiver_id', () => {
      const s0: GameState = { ...makeState(), ball: { holderId: 'h2', status: 'held', zone: 'wing_L' } };
      const e = makeEvent({
        type: 'HANDOFF',
        payload: { giver_id: 'h2', receiver_id: 'h4' },
        actors: ['h2', 'h4'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.ball.holderId).toBe('h4');
    });

    it('ADVANCE_BACKCOURT sets ball.holderId from ballHandlerId', () => {
      const s0: GameState = { ...makeState(), ball: { holderId: 'h1', status: 'held', zone: 'backcourt' } };
      const e = makeEvent({
        type: 'ADVANCE_BACKCOURT',
        payload: { ballHandlerId: 'h1' },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.ball.holderId).toBe('h1');
    });
  });

  describe('REBOUND', () => {
    it('offensive=false → possession flips to opposite of rebounder team', () => {
      // Rebounder is away, defensive board → possession flips to home.
      const s0: GameState = { ...makeState(), possession: { team: 'away' } };
      const e = makeEvent({
        type: 'REBOUND',
        payload: { rebounder_id: 'a3', team: 'away', offensive: false },
        actors: ['a3'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('home');
      expect(s1.ball.holderId).toBe('a3');
    });

    it('offensive=true → possession stays with rebounder team', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'home' } };
      const e = makeEvent({
        type: 'REBOUND',
        payload: { rebounder_id: 'h5', team: 'home', offensive: true },
        actors: ['h5'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('home');
      expect(s1.ball.holderId).toBe('h5');
    });
  });

  describe('POSSESSION_GAINED', () => {
    it('sets possession.team, ball.holderId, and resets shot clock to 24', () => {
      const s0: GameState = {
        ...makeState(),
        clocks: { period: 1, game: 600.0, shot: 3.0 },
        possession: { team: null },
        ball: { holderId: null, status: 'loose', zone: 'backcourt' },
      };
      const e = makeEvent({
        type: 'POSSESSION_GAINED',
        payload: { team: 'home', player_id: 'h1', source: 'tip' },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('home');
      expect(s1.ball.holderId).toBe('h1');
      expect(s1.clocks.shot).toBe(24.0);
      // Non-touched clock fields preserved.
      expect(s1.clocks.game).toBe(600.0);
      expect(s1.clocks.period).toBe(1);
    });
  });

  describe('FOUL', () => {
    it('increments fouls.players[offender] and fouls.team[team]', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'FOUL',
        payload: {
          offender_id: 'h3',
          offender_team: 'home',
          victim_id: 'a2',
          foul_type: 'personal',
        },
        actors: ['h3', 'a2'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.fouls.players['h3']).toBe(1);
      expect(s1.fouls.team.home).toBe(1);
      expect(s1.fouls.team.away).toBe(0);
    });

    it('accumulates per-player fouls across multiple FOUL events', () => {
      const s0 = makeState();
      const e1 = makeEvent({
        type: 'FOUL',
        payload: { offender_id: 'h3', offender_team: 'home', victim_id: 'a2', foul_type: 'personal' },
        actors: ['h3', 'a2'],
      });
      const s1 = applyEvent(s0, e1);
      const e2 = makeEvent({
        type: 'FOUL',
        payload: { offender_id: 'h3', offender_team: 'home', victim_id: 'a4', foul_type: 'personal' },
        actors: ['h3', 'a4'],
      });
      const s2 = applyEvent(s1, e2);
      expect(s2.fouls.players['h3']).toBe(2);
      expect(s2.fouls.team.home).toBe(2);
    });

    it('sets bonus=true when team fou reach 5 (NBA bonus threshold)', () => {
      const s0: GameState = {
        ...makeState(),
        fouls: {
          team: { home: 4, away: 0 },
          players: {},
          bonus: { home: false, away: false },
        },
      };
      const e = makeEvent({
        type: 'FOUL',
        payload: { offender_id: 'h2', offender_team: 'home', victim_id: 'a1', foul_type: 'personal' },
        actors: ['h2', 'a1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.fouls.team.home).toBe(5);
      expect(s1.fouls.bonus.home).toBe(true);
    });
  });

  describe('PERIOD_END', () => {
    it('increments period and sets phase to DEAD_PERIOD_END', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'PERIOD_END',
        payload: { period: 1 },
      });
      const s1 = applyEvent(s0, e);
      expect(s1.period).toBe(2);
      expect(s1.clocks.period).toBe(2);
      expect(s1.phase).toBe('DEAD_PERIOD_END');
      expect(s1.clocks.game).toBe(0);
    });

    it('preserves shot clock value at moment of period end', () => {
      const s0: GameState = { ...makeState(), clocks: { period: 2, game: 5.0, shot: 14.0 } };
      const e = makeEvent({ type: 'PERIOD_END', payload: { period: 2 } });
      const s1 = applyEvent(s0, e);
      expect(s1.clocks.shot).toBe(14.0);
    });
  });

  describe('SUB', () => {
    it('swaps player_out_id for player_in_id in the team lineup', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'SUB',
        payload: { player_out_id: 'h3', player_in_id: 'h6', team: 'home' },
        actors: ['h3', 'h6'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.lineups.home).toEqual(['h1', 'h2', 'h6', 'h4', 'h5']);
      expect(s1.lineups.away).toEqual(['a1', 'a2', 'a3', 'a4', 'a5']);
    });

    it('preserves position in the lineup when swapping', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'SUB',
        payload: { player_out_id: 'a5', player_in_id: 'a6', team: 'away' },
        actors: ['a5', 'a6'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.lineups.away[4]).toBe('a6');
      expect(s1.lineups.away.length).toBe(5);
    });

    it('throws on player_out_id not in lineup (kernel bug)', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'SUB',
        payload: { player_out_id: 'NOPE', player_in_id: 'h6', team: 'home' },
        actors: ['NOPE', 'h6'],
      });
      expect(() => applyEvent(s0, e)).toThrow();
    });
  });

  describe('phase-setting events', () => {
    it('GAME_START sets phase to JUMP_BALL', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'GAME_START' });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('JUMP_BALL');
    });

    it('HALFTIME sets phase to HALFTIME', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'HALFTIME' });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('HALFTIME');
    });

    it('GAME_END sets phase to POST_GAME', () => {
      const s0 = makeState();
      const e = makeEvent({ type: 'GAME_END' });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('POST_GAME');
    });

    it('TIMEOUT_START sets phase to TIMEOUT and decrements remaining for the team', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'TIMEOUT_START',
        payload: { team: 'home' },
      });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('TIMEOUT');
      expect(s1.timeouts.remaining.home).toBe(6);
      expect(s1.timeouts.remaining.away).toBe(7);
    });

    it('FT_START sets phase to FT_SEQUENCE', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'FT_START',
        payload: { shooter_id: 'h1', team: 'home', attempts: 2 },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('FT_SEQUENCE');
    });
  });

  describe('possession-flip events', () => {
    it('STEAL flips possession to stealer team and sets stealer as holder', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'home' }, ball: { holderId: 'h1', status: 'held', zone: 'frontcourt_center' } };
      const e = makeEvent({
        type: 'STEAL',
        payload: { stealer_id: 'a2', victim_id: 'h1' },
        actors: ['a2', 'h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('away');
      expect(s1.ball.holderId).toBe('a2');
    });

    it('TURNOVER with stealer preserves the transferred holder', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'away' }, ball: { holderId: 'a2', status: 'held', zone: 'frontcourt_center' } };
      const e = makeEvent({
        type: 'TURNOVER',
        payload: { player_id: 'h1', team: 'home', stealer_id: 'a2', turnover_type: 'steal' },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.ball.holderId).toBe('a2');
      expect(s1.ball.status).toBe('held');
      expect(s1.possession.team).toBe('away');
    });

    it('TURNOVER flips possession away from the team that turned it over', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'home' }, ball: { holderId: 'h1', status: 'held', zone: 'paint' } };
      const e = makeEvent({
        type: 'TURNOVER',
        payload: { player_id: 'h1', team: 'home' },
        actors: ['h1'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('away');
    });

    it('VIOLATION flips possession away from the violator team', () => {
      const s0 = makeState();
      const e = makeEvent({
        type: 'VIOLATION',
        payload: { violator_id: 'h2', team: 'home', violation_type: 'lane' },
        actors: ['h2'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.possession.team).toBe('away');
    });

    it('OOB sets phase DEAD_OOB and flips possession from team_causing', () => {
      const s0: GameState = { ...makeState(), possession: { team: 'home' } };
      const e = makeEvent({
        type: 'OOB',
        payload: { team_causing: 'home', player_id: 'h4' },
        actors: ['h4'],
      });
      const s1 = applyEvent(s0, e);
      expect(s1.phase).toBe('DEAD_OOB');
      expect(s1.possession.team).toBe('away');
    });
  });

});
