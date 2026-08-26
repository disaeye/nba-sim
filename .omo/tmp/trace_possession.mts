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

type Ev = { type: string; t_game: number; payload: Record<string, unknown> };
const seeds = process.argv.slice(2).map(Number).filter((n) => !Number.isNaN(n));
for (const seed of (seeds.length ? seeds : [42])) {
  const res = simulateGame(input(seed));
  const evs = (res as unknown as { events: Ev[] }).events;
  // replay possession team through the fold, catching TURNOVER events whose
  // `team` (the penalized team) does NOT match the inbound thrower context.
  let phantomInboundTO = 0;
  let inboundTOs = 0;
  for (let i = 0; i < evs.length; i++) {
    const e = evs[i]!;
    if (e.type === 'INBOUND_START') {
      // scan until POSSESSION_GAINED or TURNOVER
      for (let k = i + 1; k < Math.min(i + 12, evs.length); k++) {
        const f = evs[k]!;
        if (f.type === 'POSSESSION_GAINED') break;
        if (f.type === 'TURNOVER') {
          inboundTOs++;
          if (f.payload['team'] === e.payload['team']) {
            phantomInboundTO++;
            console.log(`seed ${seed} t=${e.t_game} PHANTOM inbound TO: inbound team=${e.payload['team']} but TO charged to same team (${f.payload['turnover_type']})`);
          }
          break;
        }
      }
    }
  }
  console.log(`seed ${seed}: inbound-pass turnovers ${inboundTOs}, phantom (charged to throwing team) ${phantomInboundTO}`);
}
