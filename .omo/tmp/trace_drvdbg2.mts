import { simulateGame } from '../../src/simulate.js';
import type { GameInput } from '../../src/simulate.js';
import { readFileSync } from 'node:fs';
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8'));
const input = {
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
} as unknown as GameInput;
const res = simulateGame(input);
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; seq: number; payload: Record<string, unknown> }> }).events;
// For each DRIVE, list ALL events within +1.5s with seq ordering
let n = 0;
for (let i = 0; i < evs.length && n < 6; i++) {
  if (evs[i]!.type !== 'DRIVE') continue;
  const j = evs[i]!.payload['ballHandlerId'];
  const row: string[] = [];
  for (let k = i; k < Math.min(i + 12, evs.length); k++) {
    const e = evs[k]!;
    if (evs[i]!.t_game - e.t_game > 1.6) break;
    row.push(`${e.t_game.toFixed(1)}:${e.type}`);
  }
  console.log(`DRIVE p${j} seq${evs[i]!.seq}: ${row.join(' ')}`);
  n++;
}
