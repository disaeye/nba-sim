import { describe, it, expect } from 'vitest';

import catalogJson from '../../config/event-catalog.json' with { type: 'json' };
import courtZonesJson from '../../config/court-zones.json' with { type: 'json' };
import fsmJson from '../../config/fsm.json' with { type: 'json' };

import type {
  CourtZone,
  Event,
  EventType,
  GameInput,
  GameState,
  Phase,
} from '../../src/state/types.js';
import { createInitialState } from '../../src/state/initial.js';
import { minimalGameInput } from './_helpers.js';

// A `const` array typed against the union is the runtime witness that the
// catalog and the TypeScript union agree. `satisfies readonly EventType[]`
// fails compilation if the array names an EventType not in the union (or
// vice versa, when combined with the Set-equality test below).
const EVENT_TYPE_LIST = [
  'GAME_START',
  'JUMP_BALL_TAP',
  'POSSESSION_GAINED',
  'INBOUND_START',
  'INBOUND_TOUCH',
  'ADVANCE_BACKCOURT',
  'CROSS_HALF',
  'ALIGN_HALFCOURT',
  'PASS',
  'HANDOFF',
  'SCREEN_SET',
  'SCREEN_USE',
  'DRIVE',
  'SHOT_RELEASE',
  'SHOT_RESULT',
  'REBOUND',
  'LOOSE_BALL_RECOVER',
  'STEAL',
  'TURNOVER',
  'FOUL',
  'VIOLATION',
  'SHOT_CLOCK_VIOLATION',
  'PERIOD_END',
  'PERIOD_START',
  'HALFTIME',
  'TIMEOUT_START',
  'TIMEOUT_END',
  'SUB',
  'FT_START',
  'FT_ATTEMPT',
  'FT_RESULT',
  'FT_SEQUENCE_END',
  'MADE_BASKET_DEAD',
  'OOB',
  'HELD_BALL',
  'CLOCK_EXPIRY_ADJUDICATION',
  'GAME_END',
  'STATE_NOTE',
  'JUMP_CIRCLE_ALIGN',
  'ALIGNMENT',
] as const satisfies readonly EventType[];

const COURT_ZONE_LIST = [
  'backcourt',
  'frontcourt_center',
  'slot_L',
  'slot_R',
  'wing_L',
  'wing_R',
  'corner_L',
  'corner_R',
  'elbow_L',
  'elbow_R',
  'paint',
  'dunker_L',
  'dunker_R',
  'rim',
] as const satisfies readonly CourtZone[];

const PHASE_LIST = [
  'PRE_GAME',
  'JUMP_BALL',
  'LIVE',
  'DEAD_OOB',
  'DEAD_FOUL',
  'DEAD_VIOLATION',
  'DEAD_MAKE',
  'DEAD_HELD',
  'DEAD_PERIOD_END',
  'FT_SEQUENCE',
  'TIMEOUT',
  'PERIOD_BREAK',
  'HALFTIME',
  'OVERTIME_SETUP',
  'POST_GAME',
] as const satisfies readonly Phase[];

describe('state types', () => {
  describe('closed-set unions match foundation configs', () => {
    it('EventType union matches every catalog entry, no extras', () => {
      const fromCatalog = catalogJson.events.map((e: { type: string }) => e.type);
      expect(new Set(fromCatalog)).toEqual(new Set(EVENT_TYPE_LIST));
      expect(fromCatalog.length).toBe(EVENT_TYPE_LIST.length); // no dupes in catalog
    });

    it('CourtZone union matches every court-zones.json entry, no extras', () => {
      const fromConfig = courtZonesJson.zones as readonly string[];
      expect(new Set(fromConfig)).toEqual(new Set(COURT_ZONE_LIST));
      expect(fromConfig.length).toBe(COURT_ZONE_LIST.length);
    });

    it('Phase union matches fsm.json phases, no extras', () => {
      const fromFsm = fsmJson.phases as readonly string[];
      expect(new Set(fromFsm)).toEqual(new Set(PHASE_LIST));
      expect(fromFsm.length).toBe(PHASE_LIST.length);
    });
  });

  describe('createInitialState', () => {
    it('returns phase PRE_GAME, score 0-0, period 1', () => {
      const s = createInitialState(minimalGameInput());
      expect(s.phase).toBe('PRE_GAME');
      expect(s.score).toEqual({ home: 0, away: 0 });
      expect(s.period).toBe(1);
      expect(s.clocks.period).toBe(1);
    });

    it('starts with no ball holder, ball status dead, zone null', () => {
      const s = createInitialState(minimalGameInput());
      expect(s.ball.holderId).toBeNull();
      expect(s.ball.status).toBe('dead');
      expect(s.ball.zone).toBeNull();
    });

    it('starts with possession null, bonus false, timeouts full', () => {
      const s = createInitialState(minimalGameInput());
      expect(s.possession.team).toBeNull();
      expect(s.fouls.bonus).toEqual({ home: false, away: false });
      expect(s.fouls.team).toEqual({ home: 0, away: 0 });
      expect(s.timeouts.remaining).toEqual({ home: 7, away: 7 });
    });

    it('starts with empty events and seq 0', () => {
      const s = createInitialState(minimalGameInput());
      expect(s.events).toEqual([]);
      expect(s.seq).toBe(0);
    });

    it('seeds lineups from starters in the order given', () => {
      const input: GameInput = {
        home: { id: 'home', starters: ['h1', 'h2', 'h3', 'h4', 'h5'] },
        away: { id: 'away', starters: ['a1', 'a2', 'a3', 'a4', 'a5'] },
      };
      const s = createInitialState(input);
      expect(s.lineups.home).toEqual(['h1', 'h2', 'h3', 'h4', 'h5']);
      expect(s.lineups.away).toEqual(['a1', 'a2', 'a3', 'a4', 'a5']);
    });

    it('clocks use createClocks() defaults (game 720, shot 24)', () => {
      const s = createInitialState(minimalGameInput());
      expect(s.clocks.game).toBe(720.0);
      expect(s.clocks.shot).toBe(24.0);
    });

    it('returns a fresh object every call (no shared mutable state)', () => {
      const a = createInitialState(minimalGameInput());
      const b = createInitialState(minimalGameInput());
      expect(a).not.toBe(b);
      expect(a.events).not.toBe(b.events);
      expect(a.lineups).not.toBe(b.lineups);
      // Mutating one must not affect the other.
      a.events.push({} as Event);
      expect(b.events.length).toBe(0);
    });
  });

  describe('GameState shape compile-time check', () => {
    // A literal matching the full GameState interface compiles iff every
    // required field is present and correctly typed. This pins the type
    // surface against silent field removal or type widening.
    it('a literal GameState value satisfies the interface', () => {
      const s: GameState = {
        phase: 'PRE_GAME' as Phase,
        clocks: { period: 1, game: 720.0, shot: 24.0 },
        realClock: 0,
        ball: { holderId: null, status: 'dead', zone: null },
        ballMotion: {
          x: 0.5,
          y: 0.5,
          status: 'dead',
          holderId: null,
          fromX: 0.5,
          fromY: 0.5,
          toX: 0.5,
          toY: 0.5,
          flightT: 0,
          flightDuration: 0,
          receiverId: null,
          passerId: null,
          shooterId: null,
          shotValue: null,
          assisterId: null,
          team: null,
          zone: null,
          flightStartReal: null,
        },
        poses: {},
        possession: { team: null },
        score: { home: 0, away: 0 },
        lineups: { home: ['h1', 'h2', 'h3', 'h4', 'h5'], away: ['a1', 'a2', 'a3', 'a4', 'a5'] },
        fouls: {
          team: { home: 0, away: 0 },
          players: {},
          bonus: { home: false, away: false },
        },
        timeouts: { remaining: { home: 7, away: 7 } },
        period: 1,
        events: [],
        seq: 0,
        baskets: { home: 'left', away: 'right' },
        abilities: {},
        playerData: {},
        coach: { home: undefined, away: undefined },
        chemistry: { home: [], away: [] },
        moraleExec: { home: 1, away: 1 },
        creatorOrder: { home: [], away: [] },
        alignment: null,
      };
      expect(s.phase).toBe('PRE_GAME');
    });
  });
});
