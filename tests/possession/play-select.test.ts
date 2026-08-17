/**
 * TDD tests for `selectPlay`. RED first, GREEN second.
 *
 * Coverage map:
 *   - TRANSITION mode → returns a TRANSITION play
 *   - HALFCOURT mode → returns a HALFCOURT play
 *   - determinism: same seed → same play
 *   - empty playbook for mode → throws RngInputError('RNG_EMPTY_ARRAY')
 *   - rng draw accounting: exactly one rng.next() per call
 */
import { describe, it, expect } from 'vitest';

import { selectPlay } from '../../src/possession/play-select.js';
import type { PlaysConfig, PossessionMode } from '../../src/possession/types.js';
import { RngInputError } from '../../src/rng/types.js';
import { mulberry32 } from '../../src/rng/mulberry32.js';

import playsJson from '../../config/plays.json' with { type: 'json' };

const config = playsJson as PlaysConfig;

describe('selectPlay', () => {
  describe('mode filtering', () => {
    it('returns a play with mode TRANSITION when mode=TRANSITION', () => {
      const play = selectPlay('TRANSITION', mulberry32(1), config);
      expect(play.mode).toBe('TRANSITION');
    });

    it('returns a play with mode HALFCOURT when mode=HALFCOURT', () => {
      const play = selectPlay('HALFCOURT', mulberry32(1), config);
      expect(play.mode).toBe('HALFCOURT');
    });

    it('returns one of the plays listed in config.plays (not a synthetic)', () => {
      const play = selectPlay('HALFCOURT', mulberry32(1), config);
      const ids = config.plays.map((p) => p.id);
      expect(ids).toContain(play.id);
    });

    it('only ever returns plays matching the requested mode (10 consecutive picks)', () => {
      const rng = mulberry32(7);
      for (let i = 0; i < 10; i++) {
        const play = selectPlay('HALFCOURT', rng, config);
        expect(play.mode).toBe('HALFCOURT');
      }
    });
  });

  describe('determinism', () => {
    it('same seed → same selected play', () => {
      const a = selectPlay('HALFCOURT', mulberry32(42), config);
      const b = selectPlay('HALFCOURT', mulberry32(42), config);
      expect(a.id).toBe(b.id);
    });

    it('different seeds may produce different plays (sanity, not a strict assertion)', () => {
      // Over 5 different seeds, count distinct play ids. With 4 halfcourt
      // plays, a healthy stub should produce at least 2 distinct picks
      // across 5 draws (statistical sanity; not load-bearing).
      const ids = new Set<string>();
      for (let seed = 1; seed <= 5; seed++) {
        ids.add(selectPlay('HALFCOURT', mulberry32(seed), config).id);
      }
      expect(ids.size).toBeGreaterThanOrEqual(2);
    });
  });

  describe('rng draw accounting', () => {
    it('consumes exactly one rng.next() per call', () => {
      // Two aligned rngs: pass one through selectPlay (should burn 1
      // draw); advance the other by exactly 1 raw draw. The next value
      // on each must be identical.
      const rng1 = mulberry32(99);
      const rng2 = mulberry32(99);
      void selectPlay('HALFCOURT', rng1, config);
      void rng2.next();
      expect(rng1.next()).toBe(rng2.next());
    });
  });

  describe('failure paths', () => {
    it('throws RngInputError("RNG_EMPTY_ARRAY") when no plays match the mode', () => {
      const emptyConfig: PlaysConfig = {
        foundation_version: '0.2.0',
        modes: ['TRANSITION', 'HALFCOURT'],
        plays: [],
      };
      try {
        selectPlay('HALFCOURT', mulberry32(1), emptyConfig);
        throw new Error('expected selectPlay to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(RngInputError);
        expect((e as RngInputError).code).toBe('RNG_EMPTY_ARRAY');
      }
    });

    it('throws when the requested mode has no plays but the other mode does', () => {
      const transitionOnly: PlaysConfig = {
        foundation_version: '0.2.0',
        modes: ['TRANSITION', 'HALFCOURT'],
        plays: [
          {
            id: 'transition_push',
            mode: 'TRANSITION',
            slots: ['primary_creator', 'secondary_creator', 'screener', 'spacer_strong', 'spacer_weak'],
            steps: [],
          },
        ],
      };
      const mode: PossessionMode = 'HALFCOURT';
      expect(() => selectPlay(mode, mulberry32(1), transitionOnly)).toThrow(RngInputError);
    });
  });

  describe('immutability', () => {
    it('does not mutate the input config', () => {
      const before = JSON.parse(JSON.stringify(config));
      selectPlay('HALFCOURT', mulberry32(1), config);
      selectPlay('TRANSITION', mulberry32(2), config);
      expect(JSON.parse(JSON.stringify(config))).toEqual(before);
    });
  });
});
