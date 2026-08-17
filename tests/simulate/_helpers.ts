/**
 * Test fixtures for the simulate integration tests.
 *
 * Builds a `GameInput` from `config/demo-game.json` so tests exercise
 * the real foundation config rather than a hand-rolled parallel fixture.
 */
import demoGame from '../../config/demo-game.json' with { type: 'json' };

import { simulateGame } from '../../src/simulate.js';
import type { GameInput, GameResult } from '../../src/simulate.js';
interface DemoShape {
  home_team: {
    id: string;
    roster: ReadonlyArray<{ id: string; jersey: string; teamId: string }>;
    lineup_packages: ReadonlyArray<{
      id: string;
      players: readonly string[];
      usageProfile: { creator: readonly string[]; screener: readonly string[]; spacer: readonly string[] };
    }>;
  };
  away_team: DemoShape['home_team'];
}

const DEMO = demoGame as DemoShape;

/** Build a GameInput from the real demo-game.json. */
export function demoInput(seed: number = 42): GameInput {
  return {
    home: {
      teamId: DEMO.home_team.id,
      roster: DEMO.home_team.roster as GameInput['home']['roster'],
      lineupPackages: DEMO.home_team.lineup_packages as GameInput['home']['lineupPackages'],
    },
    away: {
      teamId: DEMO.away_team.id,
      roster: DEMO.away_team.roster as GameInput['away']['roster'],
      lineupPackages: DEMO.away_team.lineup_packages as GameInput['away']['lineupPackages'],
    },
    seed,
  };
}

const GAME_CACHE = new Map<number, GameResult>();

/** Reuse deterministic demo games within a test worker. Callers treat results as read-only. */
export function cachedGame(seed: number = 42): GameResult {
  const existing = GAME_CACHE.get(seed);
  if (existing) return existing;
  const result = simulateGame(demoInput(seed));
  GAME_CACHE.set(seed, result);
  return result;
}
