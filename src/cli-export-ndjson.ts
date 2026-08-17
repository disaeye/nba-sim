/**
 * Export a slim SpectatorPackage (meta/box/court/broadcast) as JSON plus a
 * separate NDJSON line stream of render ticks for the web 2D replay shell.
 *
 * Streaming: ticks are rendered one-by-one and appended to the NDJSON file,
 * so peak memory stays flat (~snapshots + events, no 34k-frame array in RAM).
 * The `.ndjson.gz` twin is written for static hosts without transparent gzip
 * (e.g. `python3 -m http.server`); the python server negotiates it directly.
 *
 * Usage:
 *   npm run export-ndjson -- --seed 42 --out spectator/game
 *     → spectator/game.json (slim) + spectator/game.ticks.ndjson + .gz
 */
import { readFileSync, writeFileSync, mkdirSync, openSync, closeSync, statSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { gzipSync } from 'node:zlib';
import { simulateGame } from './simulate.js';
import type { GameInput, Player, GameResult } from './simulate.js';
import type { LineupPackage } from './identity/types.js';
import { renderTicks } from './render/from-snapshots.js';
import { formatBroadcastLine } from './spectator/narrate.js';
import { buildCourtDrawSpec } from './court/geometry.js';
import { FOUNDATION_VERSION } from './sim-utils.js';

interface ConfigShape {
  home_team: {
    id: string;
    name?: string;
    roster: { id: string; jersey: string; teamId: string; playerData?: Player['playerData'] }[];
    lineup_packages: LineupPackage[];
  };
  away_team: ConfigShape['home_team'];
  readonly coach?: GameInput['coach'];
}

function parseArgs(argv: readonly string[]): {
  seed: number;
  config: string;
  out: string;
  gzip: boolean;
} {
  let seed = 42;
  let config = 'config/demo-game.json';
  let out = 'spectator/game';
  let gzip = true;
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
    } else if (arg === '--no-gzip') gzip = false;
  }
  return { seed, config, out, gzip };
}

function summarizeBroadcast(events: readonly GameResult['events'][number][]): { text: string; eventSeq: number; period: number; gameClock: number }[] {
  const out: { text: string; eventSeq: number; period: number; gameClock: number }[] = [];
  let previousClock = '';
  let previousType = '';
  for (const event of events) {
    const clock = `${event.period}:${Math.floor(event.clocks.game)}`;
    if (event.type === 'ADVANCE_BACKCOURT' && previousClock === clock) continue;
    if (event.type === 'PASS' && event.payload['note'] === 'flight_start') continue;
    if (event.type === 'PASS' && previousType === 'PASS' && previousClock === clock) continue;
    out.push({ text: formatBroadcastLine(event, 'cn'), eventSeq: event.seq, period: event.period, gameClock: event.clocks.game });
    previousClock = clock;
    previousType = event.type;
  }
  return out;
}

function main(): void {
  const { seed, config, out, gzip } = parseArgs(process.argv);
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
    // Coach identity (scheme/pace biases) from the config — the input
    // validator accepts it, but the CLI dropped it, so coach profiles in
    // any config file were silently ignored (all three biases dead).
    coach: cfg.coach,
  };

  const result = simulateGame(input);
  const base = resolve(process.cwd(), out);
  mkdirSync(dirname(base), { recursive: true });
  const jsonPath = base.endsWith('.json') ? base : `${base}.json`;
  const ndjsonPath = jsonPath.replace(/\.json$/, '.ticks.ndjson');

  // Stream ticks to NDJSON — one compact object per line, rendered on the fly.
  const fd = openSync(ndjsonPath, 'w');
  const chunks: Buffer[] = [];
  let tickCount = 0;
  let lastT = 0;
  try {
    for (const tick of renderTicks(result.snapshots ?? [], result.events, { stride: 1, language: 'cn', asTicks: true })) {
      chunks.push(Buffer.from(`${JSON.stringify(tick)}\n`, 'utf8'));
      tickCount++;
      lastT = tick.t;
      // Drain every 500 lines so the chunk buffer stays bounded.
      if (chunks.length >= 500) {
        writeSyncAll(fd, chunks);
        chunks.length = 0;
      }
    }
    writeSyncAll(fd, chunks);
  } finally {
    closeSync(fd);
  }

  // Slim JSON: meta + box + court + broadcast + stream meta (ticks excluded).
  const broadcast: string[] = [];
  const broadcastEventSeqs: number[] = [];
  for (const event of result.events) {
    if (event.type === 'PASS' && event.payload['note'] === 'flight_start') continue;
    broadcast.push(formatBroadcastLine(event, 'cn'));
    broadcastEventSeqs.push(event.seq);
  }
  const slim = {
    meta: {
      ...result.meta,
      foundation_version: FOUNDATION_VERSION,
      home_name: cfg.home_team.name ?? 'Home',
      away_name: cfg.away_team.name ?? 'Away',
      exported_at: new Date().toISOString(),
      stream_dt: 0.1,
      // Per-jersey body dimensions for the spectator's player icons — the
      // ticks carry no height/weight; the shell reads this table to draw
      // each player's silhouette proportional to his physique.
      player_bodies: Object.fromEntries(
        [...(cfg.home_team.roster ?? []), ...(cfg.away_team.roster ?? [])].map((p) => [
          p.jersey,
          {
            h_cm: p.playerData?.physical?.H ?? null,
            wt_kg: p.playerData?.physical?.WT ?? null,
          },
        ]),
      ),
    },
    box_score: result.box_score,
    court: buildCourtDrawSpec(),
    stream: {
      dt: 0.1,
      tickCount,
      duration: lastT,
      // Relative URL of the NDJSON tick stream — the shell fetches it from
      // the served spectator dir (server negotiates gzip transparently).
      // A `?v=<mtime>` cache-buster defeats a CDN edge (EdgeOne/Cloudflare)
      // caching an old ndjson across kernel regenerations; the query is
      // ignored by the python server's path routing.
      ticksUrl: `${ndjsonPath.split('/').pop()}?v=${Math.floor(statSync(ndjsonPath).mtimeMs)}`,
    },
    broadcast,
    broadcastEventSeqs,
    broadcastSummary: summarizeBroadcast(result.events),
    event_count: result.events.length,
  };
  writeFileSync(jsonPath, JSON.stringify(slim), 'utf-8');

  const ndjsonSize = statSync(ndjsonPath).size;
  if (gzip) {
    const gz = gzipSync(readFileSync(ndjsonPath), { level: 9 });
    const gzPath = `${ndjsonPath}.gz`;
    writeFileSync(gzPath, gz);
    process.stdout.write(
      `exported ${result.events.length} events / ${tickCount} frames @ 0.1s\n` +
        `  ${jsonPath} (${(statSync(jsonPath).size / 1024).toFixed(0)}KB)\n` +
        `  ${ndjsonPath} (${(ndjsonSize / 1048576).toFixed(1)}MB)\n` +
        `  ${gzPath} (${(gz.length / 1048576).toFixed(1)}MB)\n`,
    );
  } else {
    process.stdout.write(
      `exported ${result.events.length} events / ${tickCount} frames @ 0.1s → ${ndjsonPath}\n`,
    );
  }
}

function writeSyncAll(fd: number, chunks: Buffer[]): void {
  const buf = Buffer.concat(chunks);
  let offset = 0;
  while (offset < buf.length) {
    offset += writeSync(fd, buf, offset, buf.length - offset);
  }
}

import { writeSync } from 'node:fs';

main();
