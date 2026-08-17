/**
 * Export a SpectatorPackage JSON for the web 2D replay shell.
 *
 * Usage:
 *   npm run export-game -- --seed 42 --out spectator/game.json
 */
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { simulateGame } from './simulate.js';
import type { GameInput, Player } from './simulate.js';
import type { LineupPackage } from './identity/types.js';
import { buildSpectatorPackage } from './spectator/export.js';

interface ConfigShape {
  home_team: {
    id: string;
    name?: string;
    roster: { id: string; jersey: string; teamId: string }[];
    lineup_packages: LineupPackage[];
  };
  away_team: ConfigShape['home_team'];
  readonly coach?: GameInput['coach'];
}

function parseArgs(argv: readonly string[]): {
  seed: number;
  config: string;
  out: string;
} {
  let seed = 42;
  let config = 'config/demo-game.json';
  let out = 'spectator/game.json';
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    if (arg === '--seed') seed = Number(argv[++i]);
    else if (arg === '--config') {
      const v = argv[++i];
      if (typeof v === 'string') config = v;
    } else if (arg === '--out') {
      const v = argv[++i];
      if (typeof v === 'string') out = v;
    }
  }
  return { seed, config, out };
}

function main(): void {
  const { seed, config, out } = parseArgs(process.argv);
  const cfg = JSON.parse(readFileSync(resolve(process.cwd(), config), 'utf-8')) as ConfigShape;
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
  const pkg = buildSpectatorPackage(result, {
    homeName: cfg.home_team.name ?? 'Home',
    awayName: cfg.away_team.name ?? 'Away',
  });

  const outPath = resolve(process.cwd(), out);
  mkdirSync(dirname(outPath), { recursive: true });
  writeFileSync(outPath, JSON.stringify(pkg), 'utf-8');
  process.stdout.write(
    `exported ${pkg.event_count} events / ${pkg.stream.tickCount} render frames @ ${pkg.stream.dt}s → ${out}\n`,
  );
}

main();
