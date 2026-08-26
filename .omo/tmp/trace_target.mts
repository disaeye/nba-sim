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
const evs = (res as unknown as { events: Array<{ type: string; t_game: number; payload: Record<string, unknown> }> }).events;
const snaps = (res as unknown as { snapshots: Array<{ t_real: number; players: Array<{ jersey: string; x: number; y: number; targetX?: number; targetY?: number; action: string; hasBall: boolean }> }> }).snapshots;
// find first few DRIVE events; snapshots use real time; just scan snapshots for hasBall + action=drive and dump target
let shown = 0;
for (let i = 1; i < snaps.length && shown < 25; i++) {
  const h = snaps[i]!.players.find((p) => p.hasBall && p.action === 'drive');
  const prev = snaps[i - 1]!.players.find((p) => p.hasBall && p.action === 'drive');
  if (h && prev && prev.jersey === h.jersey) {
    console.log(`t=${snaps[i]!.t_real.toFixed(1)} ${h.jersey} pos=(${h.x.toFixed(2)},${h.y.toFixed(2)}) target=(${h.targetX?.toFixed(2)},${h.targetY?.toFixed(2)}) action=${h.action}`);
    shown++;
  }
}
