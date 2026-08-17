import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';

const cfg = JSON.parse(readFileSync(resolve(process.cwd(), 'config/demo-game.json'), 'utf-8')) as Record<string, Record<string, unknown>>;
const ht = cfg.home_team!, at = cfg.away_team!;
const result = simulateGame({
  home: { teamId: ht.id as string, roster: ht.roster as never, lineupPackages: ht.lineup_packages as never },
  away: { teamId: at.id as string, roster: at.roster as never, lineupPackages: at.lineup_packages as never },
  seed: 42,
});

const snaps = result.snapshots ?? [];
let liveSnaps = 0;
let holderMoved = 0, holderStatic = 0;
const defenderOnBallGap: number[] = [];
const teamSpread: number[] = [];
let prev: typeof snaps[0] | null = null;

for (const s of snaps) {
  if (s.phase !== 'LIVE') continue;
  liveSnaps++;
  if (prev && s.ball.status === 'held' && prev.ball.status === 'held' && s.ball.holderId === prev.ball.holderId) {
    const dx = (s.ball.x - prev.ball.x) * 94;
    const dy = (s.ball.y - prev.ball.y) * 50;
    if (Math.hypot(dx, dy) > 0.3) holderMoved++; else holderStatic++;
  }
  if (s.ball.status === 'held' && s.ball.holderId) {
    const holder = s.players.find((p) => p.jersey === s.ball.holderId);
    if (holder) {
      const opp = holder.team === 'home' ? 'away' : 'home';
      let minD = Infinity;
      for (const d of s.players.filter((p) => p.team === opp)) {
        const dist = Math.hypot((d.x - holder.x) * 94, (d.y - holder.y) * 50);
        if (dist < minD) minD = dist;
      }
      if (minD < Infinity) defenderOnBallGap.push(minD);
    }
  }
  for (const team of ['home', 'away'] as const) {
    const players = s.players.filter((p) => p.team === team);
    let minDist = Infinity;
    for (let i = 0; i < players.length; i++) {
      for (let j = i + 1; j < players.length; j++) {
        const d = Math.hypot((players[i]!.x - players[j]!.x) * 94, (players[i]!.y - players[j]!.y) * 50);
        if (d < minDist) minDist = d;
      }
    }
    teamSpread.push(minDist);
  }
  prev = s;
}

console.log('live snapshots:', liveSnaps);
console.log(`holder: ${holderMoved} moving, ${holderStatic} static (${(100 * holderStatic / (holderMoved + holderStatic)).toFixed(1)}% static)`);
const dMean = defenderOnBallGap.reduce((a, b) => a + b, 0) / defenderOnBallGap.length;
console.log(`defender-on-ball: mean=${dMean.toFixed(1)}ft min=${Math.min(...defenderOnBallGap).toFixed(1)} max=${Math.max(...defenderOnBallGap).toFixed(1)}`);
const sMean = teamSpread.reduce((a, b) => a + b, 0) / teamSpread.length;
console.log(`team min-spread: mean=${sMean.toFixed(1)}ft min=${Math.min(...teamSpread).toFixed(1)}ft`);

// player total movement
const move: Record<string, number> = {};
for (let i = 1; i < snaps.length; i++) {
  const s = snaps[i]!, p = snaps[i - 1]!;
  if (s.phase !== 'LIVE') continue;
  for (const pl of s.players) {
    const pp = p.players.find((x) => x.jersey === pl.jersey);
    if (!pp) continue;
    const d = Math.hypot((pl.x - pp.x) * 94, (pl.y - pp.y) * 50);
    move[pl.jersey] = (move[pl.jersey] ?? 0) + d;
  }
}
const sorted = Object.entries(move).sort((a, b) => b[1] - a[1]);
console.log('total movement (ft):', sorted.map(([k, v]) => `${k}:${Math.round(v)}`).join(' '));

// box score per player
console.log('\nbox score home:');
for (const p of result.box_score.home) {
  console.log(`  #${p.jersey}: ${p.points}pts ${p.fgm}/${p.fga}FG ${p.tpm}/${p.tpa}3PT ${p.ftm}/${p.fta}FT ${p.oreb}orb ${p.dreb}drb ${p.ast}ast ${p.tov}tov ${p.stl}stl ${p.pf}pf`);
}
console.log('box score away:');
for (const p of result.box_score.away) {
  console.log(`  #${p.jersey}: ${p.points}pts ${p.fgm}/${p.fga}FG ${p.tpm}/${p.tpa}3PT ${p.ftm}/${p.fta}FT ${p.oreb}orb ${p.dreb}drb ${p.ast}ast ${p.tov}tov ${p.stl}stl ${p.pf}pf`);
}
