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
const res = simulateGame(input(seed));
const snaps = (res as unknown as {
  snapshots: Array<{
    t_real: number;
    ball: { x: number; y: number; status: string; holderId: string | null };
    players: Array<{ jersey: string; x: number; y: number; hasBall: boolean }>;
  }>;
}).snapshots;
// Find the inbound pass that missed: first find INBOUND_START event then a TURNOVER within 1.5s in events; map to real-time via snapshot scan:
// find snapshots where ball.status === 'pass' and next second a loose ball far from both. Simpler: dump frames where ball is in 'pass' status right after an INBOUND.
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events;
let inboundT: number | null = null;
let inbounder = ''; let receiver = '';
for (let i = 0; i < evs.length; i++) {
  if (evs[i]!.type === 'INBOUND_START') {
    inboundT = evs[i]!.t_game;
    inbounder = String(evs[i]!.payload['inbounder_id']);
    // find receiver from the next PASS flight_start
    for (let k = i + 1; k < i + 20; k++) {
      if (evs[k]!.type === 'PASS' && evs[k]!.payload['note'] === 'flight_start') {
        receiver = String(evs[k]!.payload['receiver_id']);
        // does a TURNOVER follow within 2s?
        let missed = false;
        for (let m = k + 1; m < k + 10; m++) {
          if (evs[m]!.type === 'TURNOVER' && evs[m]!.payload['turnover_type'] === 'pass_receiver_missed') { missed = true; break; }
          if (evs[m]!.type === 'POSSESSION_GAINED') break;
        }
        if (missed) {
          console.log(`MISSED inbound: t_game=${inboundT} inbounder=${inbounder} receiver=${receiver}`);
          // dump receiver/inbounder geometry from snapshots near this real time.
          // snapshots' t_real ≈ index*0.1. Find window by scanning for the pass: ball.status 'pass'
          for (let sIdx = 0; sIdx < snaps.length; sIdx++) {
            const sn = snaps[sIdx]!;
            if (sn.ball.status === 'pass') {
              const r = sn.players.find((p) => p.jersey === receiver);
              const ib = sn.players.find((p) => p.jersey === inbounder);
              if (r && ib && Math.abs(sn.ball.x - ib.x) < 0.12 && sn.t_real % 1 < 0.15) {
                console.log(`  t_real=${sn.t_real.toFixed(1)} ball=(${sn.ball.x.toFixed(3)},${sn.ball.y.toFixed(3)}) receiver=(${r.x.toFixed(3)},${r.y.toFixed(3)}) inbounder=(${ib.x.toFixed(3)},${ib.y.toFixed(3)})`);
              }
            }
          }
          process.exit(0);
        }
        break;
      }
    }
  }
}
console.log('no missed inbound found');
// marker
