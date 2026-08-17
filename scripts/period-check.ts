import { readFileSync } from 'node:fs';
import { simulateGame } from '../src/simulate.js';
const cfg = JSON.parse(readFileSync('config/demo-game.json', 'utf8'));
const input = (seed: number) => ({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});
for (const seed of [5, 7, 42]) {
  const r = simulateGame(input(seed));
  const periods = new Map<number, number>();
  for (const e of r.events) {
    periods.set(e.period, (periods.get(e.period) ?? 0) + 1);
  }
  const shots = r.events.filter(e => e.type === 'SHOT_RELEASE');
  const shotPeriods = new Map<number, number>();
  for (const s of shots) shotPeriods.set(s.period, (shotPeriods.get(s.period) ?? 0) + 1);
  const lastEvent = r.events[r.events.length - 1]!;
  console.log(`seed ${seed}: score=${r.box_score.home.reduce((s,p)=>s+(p.points??0),0)}-${r.box_score.away.reduce((s,p)=>s+(p.points??0),0)}`);
  console.log(`  events by period: ${[...periods.entries()].sort((a,b)=>a[0]-b[0]).map(([k,v])=>`P${k}:${v}`).join(' ')}`);
  console.log(`  shots by period: ${[...shotPeriods.entries()].sort((a,b)=>a[0]-b[0]).map(([k,v])=>`P${k}:${v}`).join(' ')}`);
  console.log(`  last event: ${lastEvent.type} @ t_real=${lastEvent.t_real} period=${lastEvent.period} game=${lastEvent.clocks.game}`);
  const gameEnds = r.events.filter(e => e.type === 'GAME_END');
  console.log(`  GAME_END: ${gameEnds.length}`);
}
