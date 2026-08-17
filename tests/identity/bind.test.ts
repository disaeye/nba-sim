/**
 * TDD tests for `bindRoles`. Written BEFORE the implementation; the RED
 * phase is the import failing because `src/identity/bind.ts` does not exist
 * yet. The GREEN phase is the implementation making every assertion pass.
 *
 * Coverage map (matches the task spec bullet list):
 *   - starters package: primary_creator = first creator jersey
 *   - bench package: different primary (the role-variation narrative)
 *   - all 5 slots filled, no duplicates, every jersey in package.players
 *   - throws IDENTITY_SLOT_NOT_FILLED when too few players
 *   - throws IDENTITY_DUPLICATE_JERSEY when 1-creator package forces a dup
 *   - throws IDENTITY_JERSEY_NOT_IN_PACKAGE when usageProfile leaks a bad id
 *   - immutability: returns a new object, does not mutate input
 */
import { describe, it, expect } from 'vitest';

import { bindRoles } from '../../src/identity/bind.js';
import { IdentityError } from '../../src/identity/types.js';
import type { LineupPackage, PlayContract } from '../../src/identity/types.js';
import { loadLineupPackage } from '../../src/identity/loader.js';

// Every play in config/plays.json declares exactly these 5 offense slots.
const FULL_PLAY: PlayContract = {
  id: 'pnr_high',
  slots: ['primary_creator', 'secondary_creator', 'screener', 'spacer_strong', 'spacer_weak'],
};

// Mirrors config/demo-game.json home_team.lineup_packages[0] (starters).
const HOME_STARTERS: LineupPackage = {
  id: 'starters',
  players: ['1', '2', '3', '4', '5'],
  usageProfile: { creator: ['1', '2'], screener: ['4'], spacer: ['3', '5'] },
};

// Mirrors config/demo-game.json home_team.lineup_packages[1] (bench_unit).
const HOME_BENCH: LineupPackage = {
  id: 'bench_unit',
  players: ['6', '7', '8', '9', '10'],
  usageProfile: { creator: ['7', '9'], screener: ['8'], spacer: ['6', '10'] },
};

describe('bindRoles', () => {
  describe('with home starters package', () => {
    it('assigns primary_creator to the first jersey in usageProfile.creator', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(binding.primary_creator).toBe('1');
    });

    it('assigns secondary_creator to the second creator jersey', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(binding.secondary_creator).toBe('2');
    });

    it('assigns screener from the screener list', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(binding.screener).toBe('4');
    });

    it('assigns spacer_strong and spacer_weak from the spacer list in priority order', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(binding.spacer_strong).toBe('3');
      expect(binding.spacer_weak).toBe('5');
    });

    it('fills all 5 slots with no duplicates', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      const jerseys = Object.values(binding);
      expect(jerseys).toHaveLength(5);
      expect(new Set(jerseys).size).toBe(5);
    });

    it('every bound jersey is a member of package.players', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      for (const jersey of Object.values(binding)) {
        expect(HOME_STARTERS.players).toContain(jersey);
      }
    });

    it('produces the exact expected binding for the starters package', () => {
      expect(bindRoles(HOME_STARTERS, FULL_PLAY)).toEqual({
        primary_creator: '1',
        secondary_creator: '2',
        screener: '4',
        spacer_strong: '3',
        spacer_weak: '5',
      });
    });
  });

  describe('with home bench_unit package (role variation)', () => {
    it('assigns a DIFFERENT primary_creator than the starters package', () => {
      const startersBinding = bindRoles(HOME_STARTERS, FULL_PLAY);
      const benchBinding = bindRoles(HOME_BENCH, FULL_PLAY);
      // Starters primary = "1", bench primary = "7" — role priority lives
      // on the package, not the player.
      expect(startersBinding.primary_creator).toBe('1');
      expect(benchBinding.primary_creator).toBe('7');
      expect(benchBinding.primary_creator).not.toBe(startersBinding.primary_creator);
    });

    it('assigns secondary_creator to the second bench creator ("9")', () => {
      const binding = bindRoles(HOME_BENCH, FULL_PLAY);
      expect(binding.secondary_creator).toBe('9');
    });

    it('produces the exact expected binding for the bench package', () => {
      expect(bindRoles(HOME_BENCH, FULL_PLAY)).toEqual({
        primary_creator: '7',
        secondary_creator: '9',
        screener: '8',
        spacer_strong: '6',
        spacer_weak: '10',
      });
    });
  });

  describe('config/demo-game.json loader parity', () => {
    it('loadLineupPackage("home", "starters") matches the hardcoded fixture', () => {
      const loaded = loadLineupPackage('home', 'starters');
      expect(loaded.players).toEqual(HOME_STARTERS.players);
      expect(loaded.usageProfile.creator).toEqual(HOME_STARTERS.usageProfile.creator);
      expect(loaded.usageProfile.screener).toEqual(HOME_STARTERS.usageProfile.screener);
      expect(loaded.usageProfile.spacer).toEqual(HOME_STARTERS.usageProfile.spacer);
    });

    it('bindRoles produces the same primary_creator whether the package is hardcoded or loaded', () => {
      const hardcoded = bindRoles(HOME_STARTERS, FULL_PLAY);
      const loaded = bindRoles(loadLineupPackage('home', 'starters'), FULL_PLAY);
      expect(hardcoded).toEqual(loaded);
    });

    it('loadLineupPackage throws IDENTITY_PACKAGE_NOT_FOUND for an unknown package id', () => {
      try {
        loadLineupPackage('home', 'nonexistent');
        throw new Error('expected loadLineupPackage to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_PACKAGE_NOT_FOUND');
      }
    });
  });

  describe('validation — throws on malformed packages', () => {
    it('throws IDENTITY_SLOT_NOT_FILLED when package has fewer than 5 players', () => {
      const shortPkg: LineupPackage = {
        id: 'short',
        players: ['1', '2', '3', '4'],
        usageProfile: { creator: ['1', '2'], screener: ['4'], spacer: ['3'] },
      };
      try {
        bindRoles(shortPkg, FULL_PLAY);
        throw new Error('expected bindRoles to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_SLOT_NOT_FILLED');
      }
    });

    it('throws IDENTITY_DUPLICATE_JERSEY when a 1-creator package forces primary == secondary', () => {
      // Per the spec: secondary_creator = creators[1] ?? creators[0].
      // With only one creator, secondary falls back to creators[0], which
      // duplicates primary_creator. The validation step catches this and
      // throws — surfacing the incompatibility between the package and a
      // play that needs two distinct creators.
      const oneCreatorPkg: LineupPackage = {
        id: 'one_creator',
        players: ['1', '2', '3', '4', '5'],
        usageProfile: { creator: ['1'], screener: ['4'], spacer: ['3', '5'] },
      };
      try {
        bindRoles(oneCreatorPkg, FULL_PLAY);
        throw new Error('expected bindRoles to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_DUPLICATE_JERSEY');
      }
    });

    it('throws IDENTITY_JERSEY_NOT_IN_PACKAGE when usageProfile references a jersey not in players', () => {
      // The foundation schema enforces usageProfile jerseys ⊆ players, but
      // bindRoles receives a TS interface, not a schema-validated JSON.
      // The post-bind validation is the safety net for hand-crafted inputs.
      const leakyPkg: LineupPackage = {
        id: 'leaky',
        players: ['1', '2', '3', '4', '5'],
        usageProfile: { creator: ['99', '2'], screener: ['4'], spacer: ['3', '5'] },
      };
      try {
        bindRoles(leakyPkg, FULL_PLAY);
        throw new Error('expected bindRoles to throw');
      } catch (e) {
        expect(e).toBeInstanceOf(IdentityError);
        expect((e as IdentityError).code).toBe('IDENTITY_JERSEY_NOT_IN_PACKAGE');
      }
    });
  });

  describe('immutability', () => {
    it('returns a new object that is not the same reference as the input', () => {
      const binding = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(binding).not.toBe(HOME_STARTERS);
    });

    it('does not mutate the input package arrays', () => {
      const inputPlayers = [...HOME_STARTERS.players];
      const inputCreators = [...HOME_STARTERS.usageProfile.creator];
      const inputScreeners = [...HOME_STARTERS.usageProfile.screener];
      const inputSpacers = [...HOME_STARTERS.usageProfile.spacer];

      bindRoles(HOME_STARTERS, FULL_PLAY);

      expect(HOME_STARTERS.players).toEqual(inputPlayers);
      expect(HOME_STARTERS.usageProfile.creator).toEqual(inputCreators);
      expect(HOME_STARTERS.usageProfile.screener).toEqual(inputScreeners);
      expect(HOME_STARTERS.usageProfile.spacer).toEqual(inputSpacers);
    });

    it('produces equal output for equal inputs (deterministic)', () => {
      const a = bindRoles(HOME_STARTERS, FULL_PLAY);
      const b = bindRoles(HOME_STARTERS, FULL_PLAY);
      expect(a).toEqual(b);
      // Different object identities (no shared reference)
      expect(a).not.toBe(b);
    });
  });
});
