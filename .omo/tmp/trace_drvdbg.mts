import { simulateGame } from '../../src/simulate.js';
import type { GameInput } from '../../src/simulate.js';
import { readFileSync } from 'node:fs';
const cfg = JSON.parse(readFileSync(process.cwd() + '/config/demo-game.json', 'utf8'));
const input = {
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
} as unknown as GameInput;
process.env.DRIVE_DBG = '1';
simulateGame(input);
const g = globalThis as unknown as Record<string, number>;
console.log('abort-ticks:', g['__drvAbort'] ?? 0);
console.log('stage!=PROGRESSING:', g['__drvAbortReason'] ?? 0);
console.log('uncommitted:', g['__drvAbortUncommitted'] ?? 0);
console.log('collapse:', g['__drvAbortCollapse'] ?? 0);
console.log('late:', g['__drvAbortLate'] ?? 0);
const g2 = globalThis as unknown as Record<string, number>;
console.log('passFacts:', g2['__passFact'] ?? 0, 'noActive:', g2['__passFactNoActive'] ?? 0, 'override:', g2['__passFactOverride'] ?? 0);
for (const k of Object.keys(g2)) if (k.startsWith('__passFactActive')) console.log(k, g2[k]);
console.log('passAfterDrive:', g2['__passAfterDrive'] ?? 0);
for (const k of Object.keys(g2)) if (k.startsWith('__passAfterDrive_active')) console.log(' ', k, g2[k]);
