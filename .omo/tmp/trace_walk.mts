import { simulateGame } from '../../src/simulate.js';
import type { GameInput, Player } from '../../src/simulate.js';
import type { LineupPackage } from '../../src/identity/types.js';
import { readFileSync } from 'node:fs';

interface ConfigShape {
  home_team: { id: string; roster: Player[]; lineup_packages: LineupPackage[] };
  away_team: ConfigShape['home_team'];
}
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8')) as ConfigShape;
const input: GameInput = {
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
};
const res = simulateGame(input);
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; phase: string; ball: { x: number; y: number; status: string }; players: Array<{ jersey: string; x: number; y: number; action: string; hasBall: boolean }> }> }).snapshots;
// dump frames 686-692 (before the pass): all players with actions, to see the dead-ball walk
for (let k = 6855; k < 6915; k += 4) {
  const f = snaps[k]!;
  const rows = f.players.map((p) => `${p.jersey}${p.hasBall ? '*' : ''}(${p.x.toFixed(2)},${p.y.toFixed(2)})${p.action.slice(0,7)}`);
  console.log(`t=${f.t_real.toFixed(1)} ${f.phase} ball=${f.ball.status}(${f.ball.x.toFixed(2)},${f.ball.y.toFixed(2)})`);
  console.log('   ' + rows.join(' '));
}
