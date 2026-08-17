/**
 * Text live broadcast — prints narrated lines for a full simulation.
 *
 * Usage:
 *   npm run broadcast -- --seed 42 --config config/demo-game.json
 *   npm run broadcast -- --seed 42 --delay 15 --filter high
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { setTimeout as sleep } from 'node:timers/promises';
import { simulateGame } from './simulate.js';
import type { GameInput, Player } from './simulate.js';
import type { LineupPackage } from './identity/types.js';
import { formatBroadcastLine, narrateEvent } from './spectator/narrate.js';

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
  delayMs: number;
  filter: 'all' | 'mid' | 'high';
  lang: 'cn' | 'en';
} {
  let seed = 42;
  let config = 'config/demo-game.json';
  let delayMs = 0;
  let filter: 'all' | 'mid' | 'high' = 'all';
  let lang: 'cn' | 'en' = 'cn';
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    if (arg === '--seed') seed = Number(argv[++i]);
    else if (arg === '--config') {
      const v = argv[++i];
      if (typeof v === 'string') config = v;
    } else if (arg === '--delay') delayMs = Number(argv[++i]);
    else if (arg === '--filter') {
      const v = argv[++i];
      if (v === 'mid' || v === 'high' || v === 'all') filter = v;
    } else if (arg === '--lang') {
      const v = argv[++i];
      if (v === 'en' || v === 'cn') lang = v;
    }
  }
  return { seed, config, delayMs, filter, lang };
}

function intensityOk(
  intensity: 'low' | 'mid' | 'high',
  filter: 'all' | 'mid' | 'high',
): boolean {
  if (filter === 'all') return true;
  if (filter === 'mid') return intensity !== 'low';
  return intensity === 'high';
}

async function main(): Promise<void> {
  const { seed, config, delayMs, filter, lang } = parseArgs(process.argv);
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
  const homeName = cfg.home_team.name ?? 'Home';
  const awayName = cfg.away_team.name ?? 'Away';
  const homeScore = result.box_score.home.reduce((s, p) => s + p.points, 0);
  const awayScore = result.box_score.away.reduce((s, p) => s + p.points, 0);

  process.stdout.write(
    `═══ ${homeName} vs ${awayName} · seed ${seed} · foundation ${result.meta.foundation_version} ═══\n`,
  );

  for (const e of result.events) {
    if (e.type === 'PASS' && e.payload['note'] === 'flight_start') continue;
    const n = narrateEvent(e);
    if (!intensityOk(n.intensity, filter)) continue;
    process.stdout.write(formatBroadcastLine(e, lang) + '\n');
    if (delayMs > 0) await sleep(delayMs);
  }

  process.stdout.write(
    `═══ FINAL ${homeName} ${homeScore} - ${awayScore} ${awayName} · ${result.events.length} events ═══\n`,
  );
}

main().catch((err: unknown) => {
  const msg = err instanceof Error ? err.message : String(err);
  process.stderr.write(`broadcast failed: ${msg}\n`);
  process.exit(1);
});
