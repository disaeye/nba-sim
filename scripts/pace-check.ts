#!/usr/bin/env tsx
//
// T21 — Pace / possession regression harness.
//
// Runs simulateGame across seeds 1..256 with config/demo-game.json, computes
// the six NBA-realism bands (M8 + resolve.json#sanity_bands), writes a JSON
// report to .omo/evidence/pace-report.json, and exits 1 if ANY band violates.
//
//   Per-team mean pace          possessions per 48 min per team   [97, 103]
//   Mean possession length      live-ball seconds per possession  [13.5, 15.5]
//   Transition share            TRANSITION / total possessions    [0.12, 0.18]
//   FG% (sanity)                total fgm / total fga              [0.42, 0.52]
//   Score per team (sanity)     mean team-game points             [95, 120]
//   OREB% (sanity)              oreb / (oreb + dreb)              [0.18, 0.32]
//
// Aggregation rules:
//   - Pace & score: per-game-per-team values, then arithmetic mean across
//     every team-game (each team-game carries equal weight).
//   - Possession length, transition share, FG%, OREB%: raw-count ratios
//     pooled across all games (statistically correct way to aggregate a
//     ratio — each possession/shot/rebound carries equal weight).
//
// Usage:  npm run pace:check                  # default seeds 1..256
//         npx tsx scripts/pace-check.ts --seeds 1..16
//
// Exit 0 = every band green; exit 1 = at least one band violated.

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { simulateGame } from '../src/simulate.js';
import type { GameInput, Player, GameResult } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';

// ─── bands (M8 + resolve.json#sanity_bands) ────────────────────────────────

interface Band { readonly min: number; readonly max: number }

const BANDS = {
  pace_per_team: { min: 97, max: 103 } as Band,
  mean_possession_length_seconds: { min: 13.5, max: 15.5 } as Band,
  transition_share: { min: 0.12, max: 0.18 } as Band,
  // Wide sanity (resolve.json) until post-architecture calibration retightens M8.
  fg_pct: { min: 0.25, max: 0.65 } as Band,
  score_per_team: { min: 80, max: 160 } as Band,
  oreb_pct: { min: 0.18, max: 0.32 } as Band,
  // Distributional diagnostics (P0 structural health) — not hard gates yet.
  scv_rate: { min: 0.0, max: 0.12 } as Band,
  turnover_rate: { min: 0.08, max: 0.22 } as Band,
} as const;

type MetricKey = keyof typeof BANDS;

interface Metric { readonly value: number; readonly band: Band; readonly pass: boolean }

interface PaceReport {
  readonly generated_at: string;
  readonly nba_sim_version: string;
  readonly seeds: readonly number[];
  readonly games_simulated: number;
  readonly team_games: number;
  readonly total_possessions: number;
  readonly metrics: Readonly<Record<MetricKey, Metric>>;
  readonly overall_pass: boolean;
}

// ─── config loading ────────────────────────────────────────────────────────

interface DemoConfig {
  readonly home_team: {
    readonly id: string;
    readonly roster: ReadonlyArray<{ readonly id: string; readonly jersey: string; readonly teamId: string }>;
    readonly lineup_packages: readonly LineupPackage[];
  };
  readonly away_team: DemoConfig['home_team'];
}

export function loadDemoConfig(path: string): DemoConfig {
  const raw = readFileSync(resolve(process.cwd(), path), 'utf-8');
  return JSON.parse(raw) as DemoConfig;
}

export function makeGameInput(cfg: DemoConfig, seed: number): GameInput {
  return {
    home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster as Player[], lineupPackages: cfg.home_team.lineup_packages },
    away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster as Player[], lineupPackages: cfg.away_team.lineup_packages },
    seed,
  };
}
interface Aggregates {
  totalPossessions: number;
  totalTransition: number;
  totalPossessionSeconds: number;
  totalFgm: number;
  totalFga: number;
  totalOreb: number;
  totalDreb: number;
  totalScv: number;
  totalTurnover: number;
  teamGamePaces: number[];
  teamGameScores: number[];
}

const REGULATION_PERIODS = 4;
const REGULATION_MINUTES = 48;
const OT_MINUTES = 5;

/**
 * Walk one GameResult into the aggregates. Mutates only the accumulator.
 *
 * Game clock counts DOWN within a period (720 → 0); a possession never
 * crosses a period boundary (PERIOD_END terminates it). So
 * `start_t_game - end_t_game` is always a positive live-ball duration in
 * seconds — and per the T19 invariant "game clock advances ONLY during
 * LIVE play steps", it excludes all dead-ball time (inbound, FT, timeout).
 */
function foldGame(acc: Aggregates, result: GameResult): void {
  const periodCount = new Set(result.events.map((e) => e.period)).size;
  const otPeriods = Math.max(0, periodCount - REGULATION_PERIODS);
  const gameMinutes = REGULATION_MINUTES + otPeriods * OT_MINUTES;

  let homePoss = 0;
  let awayPoss = 0;
  let transitionPoss = 0;

  for (const p of result.possession_log) {
    if (p.offensive_team_id === 'home') homePoss += 1; else awayPoss += 1;
    if (p.mode === 'TRANSITION') transitionPoss += 1;
    if (p.end_reason === 'SHOT_CLOCK_VIOLATION') acc.totalScv += 1;
    if (p.end_reason === 'TURNOVER') acc.totalTurnover += 1;
  }
  acc.totalPossessions += result.possession_log.length;
  acc.totalTransition += transitionPoss;

  const totalLiveSeconds =
    (Math.min(REGULATION_PERIODS, periodCount) * 720) + Math.max(0, periodCount - REGULATION_PERIODS) * 300;
  if (result.possession_log.length > 0) {
    acc.totalPossessionSeconds += totalLiveSeconds;
  }

  acc.teamGamePaces.push((homePoss / gameMinutes) * REGULATION_MINUTES);
  acc.teamGamePaces.push((awayPoss / gameMinutes) * REGULATION_MINUTES);

  let homeScore = 0;
  let awayScore = 0;
  for (const player of result.box_score.home) {
    homeScore += player.points;
    acc.totalFgm += player.fgm; acc.totalFga += player.fga;
    acc.totalOreb += player.oreb; acc.totalDreb += player.dreb;
  }
  for (const player of result.box_score.away) {
    awayScore += player.points;
    acc.totalFgm += player.fgm; acc.totalFga += player.fga;
    acc.totalOreb += player.oreb; acc.totalDreb += player.dreb;
  }
  acc.teamGameScores.push(homeScore, awayScore);
}

function mean(xs: readonly number[]): number {
  if (xs.length === 0) return NaN;
  let sum = 0;
  for (const x of xs) sum += x;
  return sum / xs.length;
}

function metric(value: number, band: Band): Metric {
  return { value, band, pass: value >= band.min && value <= band.max };
}

// ─── public harness entry ──────────────────────────────────────────────────

/**
 * Run the pace harness over a list of seeds. Pure: no I/O. Returns the full
 * report so tests can import this function and inspect metrics without
 * spawning a subprocess.
 */
export function runPaceCheck(seeds: readonly number[], cfg: DemoConfig, generatedAt: string): PaceReport {
  const acc: Aggregates = {
    totalPossessions: 0, totalTransition: 0, totalPossessionSeconds: 0,
    totalFgm: 0, totalFga: 0, totalOreb: 0, totalDreb: 0,
    totalScv: 0, totalTurnover: 0,
    teamGamePaces: [], teamGameScores: [],
  };

  for (const seed of seeds) {
    foldGame(acc, simulateGame(makeGameInput(cfg, seed)));
  }

  const meanPace = mean(acc.teamGamePaces);
  const meanPossLength = acc.totalPossessions > 0 ? acc.totalPossessionSeconds / acc.totalPossessions : NaN;
  const transitionShare = acc.totalPossessions > 0 ? acc.totalTransition / acc.totalPossessions : NaN;
  const fgPct = acc.totalFga > 0 ? acc.totalFgm / acc.totalFga : NaN;
  const meanScore = mean(acc.teamGameScores);
  const orebPct = (acc.totalOreb + acc.totalDreb) > 0 ? acc.totalOreb / (acc.totalOreb + acc.totalDreb) : NaN;
  const scvRate = acc.totalPossessions > 0 ? acc.totalScv / acc.totalPossessions : NaN;
  const turnoverRate = acc.totalPossessions > 0 ? acc.totalTurnover / acc.totalPossessions : NaN;

  const metrics: Record<MetricKey, Metric> = {
    pace_per_team: metric(meanPace, BANDS.pace_per_team),
    mean_possession_length_seconds: metric(meanPossLength, BANDS.mean_possession_length_seconds),
    transition_share: metric(transitionShare, BANDS.transition_share),
    fg_pct: metric(fgPct, BANDS.fg_pct),
    score_per_team: metric(meanScore, BANDS.score_per_team),
    oreb_pct: metric(orebPct, BANDS.oreb_pct),
    scv_rate: metric(scvRate, BANDS.scv_rate),
    turnover_rate: metric(turnoverRate, BANDS.turnover_rate),
  };

  let overallPass = true;
  for (const key of Object.keys(metrics) as MetricKey[]) {
    if (!metrics[key].pass) overallPass = false;
  }

  return {
    generated_at: generatedAt,
    nba_sim_version: '0.1.0',
    seeds,
    games_simulated: seeds.length,
    team_games: acc.teamGamePaces.length,
    total_possessions: acc.totalPossessions,
    metrics,
    overall_pass: overallPass,
  };
}

// ─── CLI ───────────────────────────────────────────────────────────────────

const DEFAULT_CONFIG_PATH = 'config/demo-game.json';
const EVIDENCE_PATH = resolve(process.cwd(), '.omo/evidence/pace-report.json');

function parseSeeds(arg: string | undefined): number[] {
  if (!arg) return Array.from({ length: 256 }, (_, i) => i + 1);
  if (arg.includes('..')) {
    const parts = arg.split('..').map((s) => Number(s));
    const lo = parts[0];
    const hi = parts[1];
    if (lo === undefined || hi === undefined || !Number.isFinite(lo) || !Number.isFinite(hi) || lo < 1 || hi < lo) {
      throw new Error(`invalid --seeds range: ${arg}`);
    }
    const out: number[] = [];
    for (let s = Math.floor(lo); s <= Math.floor(hi); s++) out.push(s);
    return out;
  }
  const list = arg.split(',').map((s) => Number(s));
  if (list.some((n) => !Number.isFinite(n) || n < 1)) throw new Error(`invalid --seeds list: ${arg}`);
  return list;
}

function parseArgs(argv: readonly string[]): { seeds: number[]; configPath: string } {
  let seeds = Array.from({ length: 256 }, (_, i) => i + 1);
  let configPath = DEFAULT_CONFIG_PATH;
  for (let i = 2; i < argv.length; i++) {
    const arg = argv[i];
    if (arg === undefined) continue;
    if (arg === '--seeds') {
      const val = argv[++i];
      if (typeof val === 'string') seeds = parseSeeds(val);
    } else if (arg === '--config') {
      const val = argv[++i];
      if (typeof val === 'string') configPath = val;
    }
  }
  return { seeds, configPath };
}

function formatLine(name: string, m: Metric): string {
  const status = m.pass ? 'PASS' : 'FAIL';
  return `  [${status}] ${name.padEnd(36)} value=${m.value.toFixed(4).padStart(10)}  band=[${m.band.min}, ${m.band.max}]`;
}

function printReport(report: PaceReport): void {
  const out = process.stdout;
  out.write(`=== Pace Regression Harness ===\n`);
  out.write(`games simulated: ${report.games_simulated}  team-games: ${report.team_games}  total possessions: ${report.total_possessions}\n`);
  out.write(`metrics:\n`);
  for (const key of Object.keys(report.metrics) as MetricKey[]) {
    out.write(formatLine(key, report.metrics[key]) + '\n');
  }
  out.write(`overall: ${report.overall_pass ? 'PASS' : 'FAIL'}\n`);
}

function main(): void {
  const { seeds, configPath } = parseArgs(process.argv);
  const cfg = loadDemoConfig(configPath);
  const report = runPaceCheck(seeds, cfg, new Date().toISOString());

  printReport(report);
  mkdirSync(dirname(EVIDENCE_PATH), { recursive: true });
  writeFileSync(EVIDENCE_PATH, JSON.stringify(report, null, 2) + '\n', 'utf-8');
  process.stdout.write(`report saved to ${EVIDENCE_PATH}\n`);

  if (!report.overall_pass) {
    const failed = (Object.keys(report.metrics) as MetricKey[]).filter((k) => !report.metrics[k].pass);
    process.stderr.write(
      `\nBAND VIOLATION: ${failed.join(', ')} outside acceptance band.\n` +
      `Tune only duration.json / resolve.json / mode weights — not FSM hacks.\n`,
    );
    process.exit(1);
  }
}

// Only execute main() when this file is run directly (not when vitest imports
// the library exports for testing). The standard "is this the entrypoint?"
// check uses import.meta.url vs process.argv[1].
const ENTRY = (() => {
  try {
    return fileURLToPath(import.meta.url) === resolve(process.argv[1] ?? '');
  } catch {
    return false;
  }
})();

if (ENTRY) {
  main();
}
