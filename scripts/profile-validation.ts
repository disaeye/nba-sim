/**
 * Individual profile validation — P0.3.
 *
 * The hardcore-realism test for the DECISION layer: does a player whose
 * tendency profile says "shoot threes" actually shoot threes? The engine
 * has per-player tendency truth (T3/TMID/TDRIVE/TPOST, a 0..99 four-set
 * summing to 100); if the observed shot-selection distribution does not
 * correlate with the tendency profile, the decision layer is ignoring the
 * player's identity.
 *
 * Usage:
 *   npx tsx scripts/profile-validation.ts --config config/generated-roster.json --seeds 1..8 [--json]
 */
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { GameResult, Player } from '../src/simulate.js';
import type { PlayerData, TendencyKey } from '../src/playerdata/types.js';

interface RosterEntry {
  id: string;
  jersey: string;
  teamId: string;
  playerData: PlayerData;
}

interface ConfigShape {
  readonly home_team: { readonly id: string; readonly roster: readonly RosterEntry[]; readonly lineup_packages: readonly unknown[] };
  readonly away_team: ConfigShape['home_team'];
}

interface ShotCounts {
  t3: number;
  tmid: number;
  tdrive: number;
  tpost: number;
  fga: number;
}

const TENDENCY_KEYS: readonly TendencyKey[] = ['T3', 'TMID', 'TDRIVE', 'TPOST'];
const SHOT_BUCKETS: readonly ('t3' | 'tmid' | 'tdrive' | 'tpost')[] = ['t3', 'tmid', 'tdrive', 'tpost'];

function shotBucketOf(payload: Record<string, unknown>): 't3' | 'tmid' | 'tdrive' | 'tpost' | null {
  // Prefer the resolve taxonomy when present (P1.2 target).
  const method = payload['shot_method'];
  if (typeof method === 'string') {
    if (method === 'post') return 'tpost';
    if (method === 'drive_finish') return 'tdrive';
    if (method === 'pull_up' || method === 'catch_shoot') {
      const value = payload['shot_value'];
      return value === 3 ? 't3' : 'tmid';
    }
    return null;
  }
  // Legacy zone heuristic.
  const zone = payload['zone'];
  const value = payload['shot_value'];
  if (typeof zone === 'string' && typeof value === 'number') {
    if (value === 3) return 't3';
    if (zone === 'rim' || zone === 'dunker_L' || zone === 'dunker_R' || zone === 'paint') return 'tdrive';
    return 'tmid';
  }
  return null;
}

function parseArgs(): { config: string; seeds: number[]; json: boolean } {
  const argv = process.argv.slice(2);
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const config = configValue ?? 'config/generated-roster.json';
  const seedsIdx = argv.indexOf('--seeds');
  const raw = seedsIdx !== -1 ? argv[seedsIdx + 1] : '42';
  const seedsValue = raw ?? '42';
  const seeds = seedsValue.includes('..')
    ? (() => {
        const parts = seedsValue.split('..').map(Number);
        const from = parts[0];
        const to = parts[1];
        if (from === undefined || to === undefined) throw new Error(`invalid seeds range: ${seedsValue}`);
        return Array.from({ length: to - from + 1 }, (_, offset) => from + offset);
      })()
    : seedsValue.split(',').map(Number).filter(Number.isInteger);
  return { config, seeds: [...new Set(seeds)], json: argv.includes('--json') };
}

function input(config: ConfigShape, seed: number): Parameters<typeof simulateGame>[0] {
  return {
    home: { teamId: config.home_team.id, roster: config.home_team.roster as unknown as readonly Player[], lineupPackages: config.home_team.lineup_packages as never },
    away: { teamId: config.away_team.id, roster: config.away_team.roster as unknown as readonly Player[], lineupPackages: config.away_team.lineup_packages as never },
    seed,
  };
}

function collectShotCounts(results: Iterable<GameResult>): Map<string, ShotCounts> {
  const counts = new Map<string, ShotCounts>();
  for (const result of results) {
    for (const e of result.events) {
      if (e.type !== 'SHOT_RESULT') continue;
      const shooter = e.payload['shooter_id'];
      if (typeof shooter !== 'string') continue;
      let c = counts.get(shooter);
      if (!c) {
        c = { t3: 0, tmid: 0, tdrive: 0, tpost: 0, fga: 0 };
        counts.set(shooter, c);
      }
      c.fga += 1;
      const bucket = shotBucketOf(e.payload);
      if (bucket) c[bucket] += 1;
    }
  }
  return counts;
}

/** Pearson correlation between two paired arrays. */
function pearson(a: readonly number[], b: readonly number[]): number {
  if (a.length !== b.length || a.length < 2) return Number.NaN;
  const n = a.length;
  const mean = (v: readonly number[]): number => v.reduce((s, x) => s + x, 0) / n;
  const ma = mean(a);
  const mb = mean(b);
  let num = 0;
  let da = 0;
  let db = 0;
  for (let i = 0; i < n; i++) {
    const xa = a[i]! - ma;
    const xb = b[i]! - mb;
    num += xa * xb;
    da += xa * xa;
    db += xb * xb;
  }
  const denom = Math.sqrt(da * db);
  return denom === 0 ? Number.NaN : num / denom;
}

export interface ProfileValidationReport {
  readonly seeds: readonly number[];
  readonly players: ReadonlyArray<{
    readonly jersey: string;
    readonly tendency: Readonly<Record<TendencyKey, number>>;
    readonly observed: ShotCounts;
    readonly fga: number;
  }>;
  readonly correlationByBucket: Readonly<Record<string, number>>;
  readonly passed: boolean;
}

export function profileValidationReport(
  config: ConfigShape,
  seeds: readonly number[],
): ProfileValidationReport {
  const players = [...config.home_team.roster, ...config.away_team.roster];
  const counts = collectShotCounts((function* () {
    for (const seed of seeds) yield simulateGame(input(config, seed));
  })());
  const rows = players
    .filter((p) => p.playerData)
    .map((p) => ({
      jersey: p.jersey,
      tendency: p.playerData!.tendency,
      observed: counts.get(p.jersey) ?? { t3: 0, tmid: 0, tdrive: 0, tpost: 0, fga: 0 },
      fga: counts.get(p.jersey)?.fga ?? 0,
    }));
  const correlationByBucket: Record<string, number> = {};
  for (let i = 0; i < SHOT_BUCKETS.length; i++) {
    const key = TENDENCY_KEYS[i]!;
    const bucket = SHOT_BUCKETS[i]!;
    const withFga = rows.filter((r) => r.fga >= 10);
    const tendencies = withFga.map((r) => r.tendency[key]);
    const observedShares = withFga.map((r) => r.observed[bucket] / (r.observed.fga || 1));
    correlationByBucket[key] = pearson(tendencies, observedShares);
  }
  const passed = Object.values(correlationByBucket).every((c) => Number.isFinite(c) && c > 0.3);
  return { seeds, players: rows, correlationByBucket, passed };
}

async function main(): Promise<void> {
  const { config: configPath, seeds, json } = parseArgs();
  const config = JSON.parse(await readConfig(configPath)) as ConfigShape;
  const report = profileValidationReport(config, seeds);
  if (json) {
    writeFileSync(resolve(process.cwd(), 'profile-validation-report.json'), JSON.stringify(report, null, 2));
    process.stdout.write(`profile-validation: ${report.passed ? 'PASS' : 'FAIL'} → profile-validation-report.json\n`);
    return;
  }
  process.stdout.write(`profile-validation: ${report.passed ? 'PASS' : 'FAIL'} (${seeds.length} games)\n`);
  for (const key of TENDENCY_KEYS) {
    const c = report.correlationByBucket[key];
    process.stdout.write(`  ${key}: ${c === undefined || !Number.isFinite(c) ? 'n/a' : c.toFixed(3)}\n`);
  }
  for (const row of report.players.slice(0, 10)) {
    process.stdout.write(`  #${row.jersey} T3=${row.tendency.T3} observed3=${((row.observed.t3 / (row.observed.fga || 1)) * 100).toFixed(0)}% fga=${row.observed.fga}\n`);
  }
  if (!report.passed) process.exitCode = 1;
}

async function readConfig(path: string): Promise<string> {
  return await import('node:fs/promises').then((fs) => fs.readFile(resolve(process.cwd(), path), 'utf8'));
}

void main();
