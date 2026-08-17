/**
 * CLI entry point — runs a single simulation and prints the summary.
 *
 * Usage:  npm run sim -- --seed 42 --config config/demo-game.json
 *
 * Reads the config JSON (two teams with rosters + lineup packages),
 * constructs a GameInput, calls simulateGame, and prints:
 *   Home X - Y Away | Events: N | Periods: M
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from './simulate.js';
import type { GameInput, Player } from './simulate.js';
import type { LineupPackage } from './identity/types.js';

interface ConfigShape {
  home_team: {
    id: string;
    roster: { id: string; jersey: string; teamId: string }[];
    lineup_packages: LineupPackage[];
  };
  away_team: ConfigShape['home_team'];
  /** Optional coach identity per team (scheme/pace biases). */
  readonly coach?: {
    readonly home?: { zoneBias?: number; paceBias?: number; blitzBias?: number };
    readonly away?: { zoneBias?: number; paceBias?: number; blitzBias?: number };
  };
}

function parseArgs(argv: readonly string[]): { seed: number; config: string } {
  let seed = 42;
  let config = 'config/demo-game.json';
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    if (arg === '--seed') {
      const val = argv[++i];
      seed = Number(val);
    } else if (arg === '--config') {
      const val = argv[++i];
      if (typeof val === 'string') config = val;
    }
  }
  return { seed, config };
}

function main(): void {
  const { seed, config } = parseArgs(process.argv);
  const configPath = resolve(process.cwd(), config);
  const raw = readFileSync(configPath, 'utf-8');
  const cfg = JSON.parse(raw) as ConfigShape;

  const input: GameInput = {
    home: {
      teamId: cfg.home_team.id,
      roster: cfg.home_team.roster as Player[],
      lineupPackages: cfg.home_team.lineup_packages,
    },
    away: {
      teamId: cfg.away_team.id,
      roster: cfg.away_team.roster as Player[],
      lineupPackages: cfg.away_team.lineup_packages,
    },
    seed,
    coach: cfg.coach,
  };

  const result = simulateGame(input);
  const homeScore = result.box_score.home.reduce((s, p) => s + p.points, 0);
  const awayScore = result.box_score.away.reduce((s, p) => s + p.points, 0);
  const periods = new Set(result.events.map((e) => e.period)).size;

  process.stdout.write(
    `Home ${homeScore} - ${awayScore} Away | Events: ${result.events.length} | Periods: ${periods}\n`,
  );
}

main();
