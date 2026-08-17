import { readFileSync } from 'node:fs';
import { simulateGame } from '../src/simulate.js';
const cfg = JSON.parse(readFileSync('config/demo-game.json', 'utf8'));
const input = (seed: number) => ({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});
const r = simulateGame(input(42));
const snaps = r.snapshots as readonly import('../src/world/snapshot.js').WorldSnapshot[];
const FT = 94, WD = 50;
const dist = (ax: number, ay: number, bx: number, by: number) => Math.hypot((ax - bx) * FT, (ay - by) * WD);
let shown = 0;
for (const s of snaps) {
  if (s.phase !== 'LIVE' || s.tactical?.kind !== 'PNR_ROLL' || s.tactical.stage !== 'SCREEN_USE') continue;
  const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
  const h = s.players.find(p => p.jersey === s.tactical!.handler);
  if (!h) continue;
  const obd = s.players.filter(p => p.team === defTeam && p.action === 'on_ball_defend' && dist(p.x, p.y, h.x, h.y) < 8);
  if (obd.length >= 2 && shown < 3) {
    console.log(`\nt=${s.t_real} mode=${s.tactical.screenDefense?.mode} zone=${s.tactical.screenDefense?.zone} handler=${h.jersey}@(${h.x.toFixed(2)},${h.y.toFixed(2)})`);
    for (const p of s.players.filter(p => p.team === defTeam)) {
      console.log(`  ${p.jersey} (${p.x.toFixed(2)},${p.y.toFixed(2)}) act=${p.action} 距球=${dist(p.x, p.y, h.x, h.y).toFixed(1)}ft`);
    }
    for (const a of s.tactical.assignments ?? []) {
      console.log(`  [${a.jersey}] ${a.role} act=${a.action} ->${a.targetJersey}`);
    }
    shown++;
  }
}
