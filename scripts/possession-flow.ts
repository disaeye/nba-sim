// 进攻执行流程: 球权开始→终结的叙事完整性
import { readFileSync } from 'node:fs';
import { simulateGame } from '../src/simulate.js';
const cfg = JSON.parse(readFileSync('config/demo-game.json', 'utf8'));
const input = (seed: number) => ({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});
for (const seed of [42, 7]) {
  const r = simulateGame(input(seed));
  // 进攻回合长度分布(从POSSESSION_GAINED到球权结束)
  const gains = r.events.filter(e => e.type === 'POSSESSION_GAINED');
  let longRounds = 0, total = 0;
  const samples: Array<{ start: number; len: number; end: string }> = [];
  for (let i = 0; i < gains.length - 1; i++) {
    const g = gains[i]!;
    const next = gains[i + 1]!;
    // 同队? (OREB不产生新gain,所以相邻gain通常是不同队)
    const len = next.t_real - g.t_real;
    total++;
    if (len > 30) {
      longRounds++;
      // 找结束事件
      const endEv = r.events.find(e => e.seq > g.seq && e.seq < next.seq && ['MADE_BASKET_DEAD', 'TURNOVER', 'SHOT_CLOCK_VIOLATION'].includes(e.type));
      if (samples.length < 4) samples.push({ start: g.t_real, len, end: endEv?.type ?? '?' });
    }
  }
  console.log(`seed ${seed}: 球权间隔=${total} >30s=${longRounds} (${(longRounds / total * 100).toFixed(1)}%)`);
  for (const sm of samples) console.log(`  start=${sm.start.toFixed(0)} len=${sm.len.toFixed(0)}s end=${sm.end}`);
}
