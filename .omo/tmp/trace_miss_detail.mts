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
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; ball: { x: number; y: number; status: string }; players: Array<{ jersey: string; x: number; y: number; action: string; hasBall: boolean }> }> }).snapshots;
// find the missed inbound near t_real where ball flies to (0.382,0.269) while receiver 17 at (0.309,0.848): search for that frame
for (let i = 0; i < snaps.length; i++) {
  const sn = snaps[i]!;
  if (sn.ball.status === 'pass') {
    const r = sn.players.find((p) => p.jersey === '17');
    const ib = sn.players.find((p) => p.jersey === '20');
    if (r && ib && Math.abs(r.x - 0.309) < 0.02 && Math.abs(r.y - 0.848) < 0.03) {
      // dump 30 frames around i
      for (let k = Math.max(0, i - 12); k < Math.min(snaps.length, i + 12); k++) {
        const f = snaps[k]!;
        const r17 = f.players.find((p) => p.jersey === '17');
        const b20 = f.players.find((p) => p.jersey === '20');
        console.log(`t=${f.t_real.toFixed(1)} ball(${f.ball.status})=(${f.ball.x.toFixed(2)},${f.ball.y.toFixed(2)}) r17=(${r17?.x.toFixed(2)},${r17?.y.toFixed(2)})${r17?.action} ib20=(${b20?.x.toFixed(2)},${b20?.y.toFixed(2)})${b20?.action}`);
      }
      break;
    }
  }
}
