import { readFileSync } from 'node:fs';
import { simulateGame } from './src/simulate.js';
const gen = JSON.parse(readFileSync('config/generated-roster.json', 'utf8'));
simulateGame({ home: { teamId: gen.home_team.id, roster: gen.home_team.roster, lineupPackages: gen.home_team.lineup_packages }, away: { teamId: gen.away_team.id, roster: gen.away_team.roster, lineupPackages: gen.away_team.lineup_packages }, seed: 42 });
console.log('done');
