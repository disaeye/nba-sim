/** Full reality scan. It runs before regression tests in the quality gate. */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { GameInput, Player, GameResult } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';
import { scanReality, type RealityScanReport } from '../src/audit/reality-scan.js';

interface ConfigShape {
  readonly home_team: { readonly id: string; readonly roster: readonly Player[]; readonly lineup_packages: readonly LineupPackage[] };
  readonly away_team: ConfigShape['home_team'];
}

function parseSeeds(argv: readonly string[]): number[] {
  const index = argv.indexOf('--seeds');
  const raw = index >= 0 ? argv[index + 1] : undefined;
  const value = raw ?? '1..32';
  const seeds = value.includes('..')
    ? (() => {
        const parts = value.split('..').map(Number);
        const from = parts[0];
        const to = parts[1];
        if (from === undefined || to === undefined || !Number.isInteger(from) || !Number.isInteger(to) || from < 0 || to < from) throw new Error(`invalid --seeds range: ${value}`);
        return Array.from({ length: to - from + 1 }, (_, offset) => from + offset);
      })()
    : value.split(',').map(Number).filter((seed) => Number.isInteger(seed) && seed >= 0);
  if (seeds.length === 0) throw new Error('--seeds must contain at least one non-negative integer');
  return [...new Set(seeds)];
}

function input(config: ConfigShape, seed: number): GameInput {
  return {
    home: { teamId: config.home_team.id, roster: config.home_team.roster, lineupPackages: config.home_team.lineup_packages },
    away: { teamId: config.away_team.id, roster: config.away_team.roster, lineupPackages: config.away_team.lineup_packages },
    seed,
  };
}

function print(report: RealityScanReport, json: boolean): void {
  if (json) {
    process.stdout.write(`${JSON.stringify(report, null, 2)}\n`);
    return;
  }
  process.stdout.write(`reality-scan: ${report.passed ? 'PASS' : 'FAIL'} (${report.games} games)\n`);
  for (const [key, metric] of Object.entries(report.diagnostics)) {
    process.stdout.write(`diagnostic ${key}: ${metric.value.toFixed(4)} [${metric.band.min}, ${metric.band.max}]\n`);
  }
  for (const finding of report.findings.slice(0, 30)) {
    process.stdout.write(`[${finding.severity}] ${finding.layer}/${finding.code} seed=${finding.seed} seq=${finding.seq ?? '-'} ${finding.message}\n`);
    process.stdout.write(`  evidence=${JSON.stringify(finding.evidence)}\n  root=${finding.likelyRootCause}\n`);
  }
}

function* simulate(config: ConfigShape, seeds: readonly number[]): Iterable<GameResult> {
  for (const seed of seeds) yield simulateGame(input(config, seed));
}

function main(): void {
  const argv = process.argv.slice(2);
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const configPath = configValue !== undefined ? configValue : 'config/demo-game.json';
  const config = JSON.parse(readFileSync(resolve(process.cwd(), configPath), 'utf8')) as ConfigShape;
  const report = scanReality(simulate(config, parseSeeds(argv)));
  print(report, argv.includes('--json'));
  if (!report.passed) process.exitCode = 1;
}

main();
