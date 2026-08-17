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
// 统计: 联防帧的wings/lows分布
const dists: Array<{ wings: number; lows: number; mids: number; overlap: number }> = [];
for (const s of snaps) {
  if (s.phase !== 'LIVE' || s.tactical?.screenDefense?.zone !== 'ZONE_2_3') continue;
  const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
  const defs = s.players.filter(p => p.team === defTeam);
  if (defs.length !== 5) continue;
  const rimDists = defs.map(d => Math.min(dist(d.x, d.y, 0.0559, 0.5), dist(d.x, d.y, 0.9441, 0.5)));
  const wings = rimDists.filter(d => d > 17).length;
  const lows = rimDists.filter(d => d < 12).length;
  const mids = rimDists.filter(d => d >= 12 && d <= 17).length;
  let overlap = 0;
  for (let a = 0; a < defs.length; a++) for (let b = a + 1; b < defs.length; b++) {
    if (dist(defs[a]!.x, defs[a]!.y, defs[b]!.x, defs[b]!.y) < 8) overlap++;
  }
  dists.push({ wings, lows, mids, overlap });
}
const n = dists.length;
const shape = dists.filter(d => d.wings >= 2 && d.lows >= 2).length;
const midCollapse = dists.filter(d => d.wings < 2 && d.lows < 2 && d.mids >= 2).length;
const overlapCollapse = dists.filter(d => d.overlap >= 2).length;
console.log(`[ZONE_SHAPE] n=${n} 2上3下=${(shape / n * 100).toFixed(0)}% midCollapse=${(midCollapse / n * 100).toFixed(0)}% overlap=${(overlapCollapse / n * 100).toFixed(0)}%`);
// 典型塌缩帧
let shown = 0;
for (let i = 0; i < dists.length && shown < 3; i++) {
  const d = dists[i]!;
  if (d.wings < 2 && d.lows < 2 && d.mids >= 2) {
    // 找对应帧
    let idx = 0;
    for (const s of snaps) {
      if (s.phase !== 'LIVE' || s.tactical?.screenDefense?.zone !== 'ZONE_2_3') continue;
      if (idx++ === i) {
        const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
        console.log(`\nt=${s.t_real} dists: ${s.players.filter(p => p.team === defTeam).map(p => Math.min(dist(p.x, p.y, 0.0559, 0.5), dist(p.x, p.y, 0.9441, 0.5)).toFixed(0)).join(', ')}ft`);
        console.log(`  ${s.players.filter(p => p.team === defTeam).map(p => `${p.jersey}@(${p.x.toFixed(2)},${p.y.toFixed(2)})`).join(' ')}`);
        shown++;
        break;
      }
    }
  }
}
