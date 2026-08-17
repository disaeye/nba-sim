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
  const snaps = r.snapshots as readonly import('../src/world/snapshot.js').WorldSnapshot[];
  const kinds = new Map<string, number>();
  for (const s of snaps) {
    if (s.phase === 'LIVE' && s.tactical?.kind) {
      kinds.set(s.tactical.kind, (kinds.get(s.tactical.kind) ?? 0) + 1);
    }
  }
  const total = [...kinds.values()].reduce((a, b) => a + b, 0);
  console.log(`seed ${seed}: ${[...kinds.entries()].sort((a, b) => b[1] - a[1]).map(([k, v]) => `${k}:${(v / total * 100).toFixed(0)}%`).join(' ')}`);
}
