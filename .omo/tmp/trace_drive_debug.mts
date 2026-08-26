import { simulateGame } from '../../src/simulate.js';
import type { GameInput } from '../../src/simulate.js';
import { readFileSync } from 'node:fs';
// Instrument stepSimulationTick indirectly: wrap runDecisionStep? Not exported.
// Instead re-simulate and log a counter via monkey-patching Math? Simplest: check the DRIVE
// event stream vs subsequent PASS and correlate with shotClock: the lateClockPullUp fires at
// shotClock<=4 — that's the remaining abort path I left open. Sample drives ending in PASS
// with their shot clock at the end:
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8'));
const input = {
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
} as unknown as GameInput;
const res = simulateGame(input);
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown>; clocks: { shot: number } }> }).events;
let n = 0;
for (let i = 0; i < evs.length && n < 12; i++) {
  if (evs[i]!.type !== 'DRIVE') continue;
  const j = evs[i]!.payload['ballHandlerId'];
  for (let k = i + 1; k < Math.min(i + 30, evs.length); k++) {
    const e = evs[k]!;
    if (e.type === 'PASS' && e.payload['passer_id'] === j) {
      console.log(`DRIVE t=${evs[i]!.t_game.toFixed(1)} p${j} -> PASS at t=${e.t_game.toFixed(1)} shotClock=${e.clocks.shot.toFixed(1)}`);
      n++;
      break;
    }
    if (e.type === 'SHOT_RELEASE' && e.payload['shooter_id'] === j) { break; }
  }
}
