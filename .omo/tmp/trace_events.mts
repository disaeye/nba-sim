import { simulateGame } from '../../src/simulate.js';
import type { GameInput, Player } from '../../src/simulate.js';
import type { LineupPackage } from '../../src/identity/types.js';
import { readFileSync } from 'node:fs';

interface ConfigShape {
  home_team: { id: string; roster: Player[]; lineup_packages: LineupPackage[] };
  away_team: ConfigShape['home_team'];
}
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8')) as ConfigShape;
const input = (seed: number): GameInput => ({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});

const seed = Number(process.argv[2] ?? 42);
const targetT = Number(process.argv[3] ?? 15.2);
const res = simulateGame(input(seed));
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events ?? [];
// print events with t_game within [targetT-4, targetT+4]
for (const e of evs) {
  if (e.t_game >= targetT - 4 && e.t_game <= targetT + 4) {
    console.log(e.t_game.toFixed(1), e.type, JSON.stringify(e.payload));
  }
}
