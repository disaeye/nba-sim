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
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; phase: string; ball: { x: number; y: number; status: string; holderId: string | null }; players: Array<{ jersey: string; x: number; y: number; action: string; hasBall: boolean }> }> }).snapshots;
// find ALL inbound-flights that ended in pass_missed: scan for 'pass' following 'inbound' status with any holder near a baseline
let total = 0; let missed = 0;
for (let i = 1; i < snaps.length; i++) {
  const prev = snaps[i - 1]!;
  if (prev.ball.status !== 'inbound') continue;
  const f = snaps[i]!;
  if (f.ball.status !== 'pass') continue;
  total++;
  // follow flight to outcome
  let outcome = 'complete';
  for (let k = i; k < Math.min(snaps.length, i + 12); k++) {
    const st = snaps[k]!.ball.status;
    if (st === 'held') { outcome = 'complete'; break; }
    if (st === 'loose') { outcome = 'missed'; missed++; 
      const ib = f.players.find((p) => p.hasBall || p.jersey === prev.ball.holderId);
      const recv = snaps[i]!.players.find((p) => p.action === 'receive_tip' || p.action === 'pass_receive');
      console.log(`MISS t=${f.t_real}: inbounder(${prev.ball.holderId}) ball->(${f.ball.x.toFixed(2)},${f.ball.y.toFixed(2)})`);
      // print receiver candidates and their positions during flight
      for (let m = i; m <= k; m += 2) {
        const g = snaps[m]!;
        const r = g.players.filter((p) => p.team === undefined ? false : true); // all
        const rec = g.players.find((p) => p.action === 'receive_tip');
        if (rec) console.log(`   t=${g.t_real.toFixed(1)} ball=(${g.ball.x.toFixed(2)},${g.ball.y.toFixed(2)}) recv=(${rec.x.toFixed(2)},${rec.y.toFixed(2)})`);
      }
      break;
    }
  }
  void outcome;
  i; // no-op
}
console.log(`inbound flights: ${total}, missed: ${missed}`);
