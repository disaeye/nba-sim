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
// 找ZONE_2_3的连续帧,看防守人是否各自独立移动(不同步)
let zoneStart = -1;
let shown = 0;
for (let i = 0; i < snaps.length; i++) {
  const s = snaps[i]!;
  if (s.tactical?.screenDefense?.zone === 'ZONE_2_3' && s.phase === 'LIVE') {
    if (zoneStart < 0) zoneStart = i;
    if (i - zoneStart > 30 && shown < 3) {
      // 打印连续几帧的防守人位置
      const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
      console.log(`\n--- zone frame t=${s.t_real} (${s.tactical.kind})`);
      for (const p of s.players.filter((p) => p.team === defTeam)) {
        console.log(`  ${p.team === 'home' ? 'H' : 'A'}${p.jersey} (${p.x.toFixed(2)},${p.y.toFixed(2)}) ${p.action}`);
      }
      const off = s.players.filter((p) => p.team === s.tactical!.offense);
      const ball = s.players.find((p) => p.hasBall);
      console.log(`  ball=(${ball?.x.toFixed(2)},${ball?.y.toFixed(2)})`);
      shown++;
    }
  } else {
    zoneStart = -1;
  }
}
