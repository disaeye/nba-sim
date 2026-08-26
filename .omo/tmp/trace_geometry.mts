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
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events;
// Print all snapshots around the FIRST inbound pass_receiver_missed: find event, then print snapshots in window.
// snapshots aren't in events; use result.snapshots
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; players: Array<{ jersey: string; x: number; y: number; targetX?: number; targetY?: number }> }> }).snapshots;
const toIdx = evs.findIndex((e) => e.type === 'TURNOVER' && e.payload['turnover_type'] === 'pass_receiver_missed');
console.log('first inbound-miss TO at t_game', evs[toIdx]?.t_game, JSON.stringify(evs[toIdx]?.payload));
const t = evs[toIdx]?.t_game ?? 0;
// snapshots are indexed by real time; find window: real time near t (they differ). Search snapshots where a PASS flight is in progress near the receiver.
// Just print events around it:
for (const e of evs) {
  if (Math.abs(e.t_game - t) < 2.5) console.log(e.t_game.toFixed(1), e.type, JSON.stringify(e.payload).slice(0, 140));
}
