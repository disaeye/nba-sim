import demoGame from '../config/demo-game.json' with { type: 'json' };
import type { GameInput } from '../src/simulate.js';

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

export function demoInput(seed = 42): GameInput {
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
