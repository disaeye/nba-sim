/**
 * Batch calibration harness — P0.4.
 *
 * Runs N seeds × 1 game each, folds every result into the distribution
 * aggregate + the legacy reality aggregate, and prints a DEVIATION-RANKED
 * report: the metrics most out of band come first. This is the loop the
 * calibration workflow runs after every resolve/decision change — a
 * before/after diff of the same command tells you whether the change
 * moved the distributions toward or away from the NBA reference.
 *
 * Usage:
 *   npx tsx scripts/calibrate.ts --config config/demo-game.json --seeds 1..8 [--json]
 */
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { GameResult, Player } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';
import { scanReality } from '../src/audit/reality-scan.js';
import { distributionRates, newDistributionAggregate, foldGameResult, NBA_REFERENCE } from '../src/audit/distributions.js';

interface ConfigShape {
  readonly home_team: { readonly id: string; readonly roster: readonly Player[]; readonly lineup_packages: readonly LineupPackage[] };
  readonly away_team: ConfigShape['home_team'];
}

interface DeviationRow {
  readonly key: string;
  readonly label: string;
  readonly value: number;
  readonly bandMin: number;
  readonly bandMax: number;
  readonly deviation: number; // signed distance from band, in band widths
}

function parseArgs(): { config: string; seeds: number[]; json: boolean } {
  const argv = process.argv.slice(2);
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const config = configValue ?? 'config/demo-game.json';
  const seedsIdx = argv.indexOf('--seeds');
  const rawValue = seedsIdx !== -1 ? argv[seedsIdx + 1] : undefined;
  const raw = rawValue ?? '42';
  const seeds = raw.includes('..')
    ? (() => {
        const parts = raw.split('..').map(Number);
        const from = parts[0];
        const to = parts[1];
        if (from === undefined || to === undefined) throw new Error(`invalid seeds range: ${raw}`);
        return Array.from({ length: to - from + 1 }, (_, offset) => from + offset);
      })()
    : raw.split(',').map(Number).filter(Number.isInteger);
  return { config, seeds: [...new Set(seeds)], json: argv.includes('--json') };
}

function input(config: ConfigShape, seed: number): Parameters<typeof simulateGame>[0] {
  return {
    home: { teamId: config.home_team.id, roster: config.home_team.roster, lineupPackages: config.home_team.lineup_packages },
    away: { teamId: config.away_team.id, roster: config.away_team.roster, lineupPackages: config.away_team.lineup_packages },
    seed,
  };
}

function deviationOf(value: number, min: number, max: number): number {
  if (value >= min && value <= max) return 0;
  const width = Math.max(max - min, 1e-6);
  return value < min ? (value - min) / width : (value - max) / width;
}

export interface CalibrationReport {
  readonly seeds: readonly number[];
  readonly games: number;
  readonly deviations: readonly DeviationRow[];
  readonly worstFive: readonly DeviationRow[];
  readonly realityPassed: boolean;
}

export function calibrate(
  config: ConfigShape,
  seeds: readonly number[],
): CalibrationReport {
  const results: GameResult[] = [];
  for (const seed of seeds) results.push(simulateGame(input(config, seed)));
  const reality = scanReality(results);
  const distribution = newDistributionAggregate();
  for (const result of results) foldGameResult(distribution, result);
  const rates = distributionRates(distribution);
  const deviations: DeviationRow[] = [];
  for (const key of Object.keys(NBA_REFERENCE) as Array<keyof typeof NBA_REFERENCE>) {
    const band = NBA_REFERENCE[key];
    const value = rates[key];
    if (value === 0 && key !== 'scvRate') continue;
    deviations.push({
      key,
      label: band.label,
      value,
      bandMin: band.min,
      bandMax: band.max,
      deviation: deviationOf(value, band.min, band.max),
    });
  }
  deviations.sort((a, b) => Math.abs(b.deviation) - Math.abs(a.deviation));
  return {
    seeds,
    games: seeds.length,
    deviations,
    worstFive: deviations.slice(0, 5),
    realityPassed: reality.passed,
  };
}

async function main(): Promise<void> {
  const { config: configPath, seeds, json } = parseArgs();
  const fs = await import('node:fs/promises');
  const config = JSON.parse(await fs.readFile(resolve(process.cwd(), configPath), 'utf8')) as ConfigShape;
  const report = calibrate(config, seeds);
  if (json) {
    writeFileSync(resolve(process.cwd(), 'calibration-report.json'), JSON.stringify(report, null, 2));
    process.stdout.write(`calibrate: ${report.realityPassed ? 'PASS' : 'FAIL'} → calibration-report.json\n`);
    return;
  }
  process.stdout.write(`calibrate: ${report.games} games, reality-gap ${report.realityPassed ? 'PASS' : 'FAIL'}\n`);
  process.stdout.write(`  worst deviations (band widths from edge):\n`);
  for (const d of report.worstFive) {
    const where = d.deviation < 0 ? 'LOW' : 'HIGH';
    process.stdout.write(`    ${where.padEnd(4)} ${(d.deviation * 100).toFixed(0).padStart(4)}%  ${d.key.padEnd(24)} ${d.value.toFixed(3)}  band [${d.bandMin.toFixed(2)}, ${d.bandMax.toFixed(2)}]  (${d.label})\n`);
  }
  if (!report.realityPassed) process.exitCode = 1;
}

void main();
