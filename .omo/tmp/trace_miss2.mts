import { simulateGame } from '../../src/simulate.js';
import type { GameInput, Player } from '../../src/simulate.js';
import { readFileSync } from 'node:fs';
interface CfgShape { home_team: { id: string; roster: Player[] }; away_team: { id: string; roster: Player[] } }
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8')) as unknown as CfgShape & Record<string, never>;
const input = {
  home: { teamId: 'home', roster: cfg.home_team.roster, lineupPackages: (cfg.home_team as unknown as { lineup_packages: never[] }).lineup_packages ?? [] },
  away: { teamId: 'away', roster: cfg.away_team.roster, lineupPackages: (cfg.away_team as unknown as { lineup_packages: never[] }).lineup_packages ?? [] },
  seed: 42,
} as never;
const res = simulateGame(input as never);
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; phase: string; ball: { x: number; y: number; status: string; holderId: string | null }; players: Array<{ jersey: string; x: number; y: number; action: string; hasBall: boolean }> }> }).snapshots;
// inbound pass: inbounder 20 near baseline. Find pass frames with holder 20 previously near x>0.9 or x<0.05, aimed at 17.
for (let i = 1; i < snaps.length; i++) {
  const f = snaps[i]!;
  if (f.ball.status !== 'pass') continue;
  const prev = snaps[i - 1]!;
  const ib = f.players.find((p) => p.jersey === '20');
  const r17 = f.players.find((p) => p.jersey === '17');
  if (!ib || !r17) continue;
  // inbounder near either baseline
  if (ib.x > 0.93 || ib.x < 0.07) {
    // check prev frame held by 20 with status inbound/held
    const pib = prev.players.find((p) => p.jersey === '20');
    if (pib && (prev.ball.status === 'inbound' || prev.ball.status === 'held') && prev.ball.holderId === '20') {
      console.log(`INBOUND PASS begins t=${f.t_real}: ball->(${f.ball.x.toFixed(2)},${f.ball.y.toFixed(2)}) r17=(${r17.x.toFixed(2)},${r17.y.toFixed(2)})${r17.action}`);
      for (let k = i; k < Math.min(snaps.length, i + 10); k++) {
        const g = snaps[k]!;
        const rr = g.players.find((p) => p.jersey === '17')!;
        console.log(`  t=${g.t_real.toFixed(1)} ball(${g.ball.status})=(${g.ball.x.toFixed(2)},${g.ball.y.toFixed(2)}) r17=(${rr.x.toFixed(2)},${rr.y.toFixed(2)})${rr.action}`);
        if (g.ball.status !== 'pass') break;
      }
    }
  }
}
