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

let found = 0;
const seeds = process.argv.slice(2).map(Number).filter((n) => !Number.isNaN(n));
for (const seed of (seeds.length ? seeds : [42])) {
  const res = simulateGame(input(seed));
  const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events ?? [];
  for (let i = 0; i < evs.length; i++) {
    const e = evs[i]!;
    if (e.type === 'INBOUND_START' && e.payload?.['spot_zone'] === 'baseline') {
      const team = e.payload['team'];
      for (let k = i + 1; k < Math.min(i + 30, evs.length); k++) {
        if (evs[k]!.type === 'POSSESSION_GAINED') {
          if (evs[k]!.payload['team'] !== team) {
            found++;
            if (found <= 8) {
              console.log('MISMATCH seed', seed, 't', e.t_game, 'inbound team', team, '-> gained', evs[k]!.payload['team'], 'player', evs[k]!.payload['player_id']);
              console.log('  ctx:', evs.slice(Math.max(0, i - 6), i + 2).map((x) => x.type).join(','));
            }
          }
          break;
        }
      }
    }
  }
}
console.log('total mismatches:', found);
