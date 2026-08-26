import { simulateGame } from '../../src/simulate.js';
import type { GameInput } from '../../src/simulate.js';
import { readFileSync } from 'node:fs';
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8'));
const input = {
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
} as unknown as GameInput;
process.env.DRIVE_DBG = '1';
const res = simulateGame(input);
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events;
const scv = evs.filter((e) => e.type === 'SHOT_CLOCK_VIOLATION');
console.log('SCV count:', scv.length, 'first at t_game', scv[0]?.t_game);
// dump events before first SCV
const t0 = scv[0]!.t_game;
for (const e of evs) {
  if (e.t_game <= t0 && e.t_game >= t0 - 30) console.log(e.t_game.toFixed(1), e.type, JSON.stringify(e.payload).slice(0, 90));
}
