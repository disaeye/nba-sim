/**
 * Play-type mix diagnostic — P0.2.
 *
 * Measures the distribution of tactical play kinds (from the frame-level
 * snapshot stream, the same field the spectator renders) and compares it
 * against NBA reference frequencies. This is the distribution-level
 * validation for the tactic selection layer: a hardcore sim must not run
 * PNR 80% of possessions any more than it must shoot 75% threes.
 *
 * Usage: npx tsx scripts/play-type-mix.ts [--seeds 1..16] [--json]
 */
import { writeFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { GameResult } from '../src/simulate.js';
import demoConfig from '../config/demo-game.json' with { type: 'json' };

// ─── NBA 2023-24 play-type reference (per 100 possessions, Synergy-ish) ────

export interface PlayTypeBand {
  readonly min: number;
  readonly max: number;
  readonly label: string;
}

export const NBA_PLAY_TYPE_REFERENCE: Readonly<Record<'PNR_ROLL' | 'PNR_POP' | 'TRANSITION_PUSH' | 'ISO' | 'POST_UP' | 'OFF_BALL_SCREEN' | 'DRIVE_KICK' | 'HANDOFF', PlayTypeBand>> = {
  PNR_ROLL: { min: 0.14, max: 0.22, label: 'pick & roll ball handler' },
  PNR_POP: { min: 0.02, max: 0.06, label: 'pick & pop' },
  TRANSITION_PUSH: { min: 0.10, max: 0.18, label: 'transition' },
  ISO: { min: 0.05, max: 0.10, label: 'isolation' },
  POST_UP: { min: 0.03, max: 0.07, label: 'post-up' },
  OFF_BALL_SCREEN: { min: 0.04, max: 0.09, label: 'off-ball screen' },
  DRIVE_KICK: { min: 0.06, max: 0.12, label: 'drive & kick' },
  HANDOFF: { min: 0.02, max: 0.06, label: 'handoff' },
};

export interface PlayTypeMixReport {
  readonly seeds: readonly number[];
  readonly mix: Readonly<Record<string, number>>;
  readonly total: number;
  readonly deviations: ReadonlyArray<{ readonly kind: string; readonly share: number; readonly band: PlayTypeBand; readonly pass: boolean }>;
  readonly passed: boolean;
}

function parseSeeds(argv: readonly string[]): number[] {
  const index = argv.indexOf('--seeds');
  const raw = index >= 0 ? argv[index + 1] : undefined;
  const value = raw ?? '42';
  const seeds = value.includes('..')
    ? (() => {
        const parts = value.split('..').map(Number);
        const from = parts[0];
        const to = parts[1];
        if (from === undefined || to === undefined || !Number.isInteger(from) || !Number.isInteger(to)) throw new Error(`invalid --seeds range: ${value}`);
        return Array.from({ length: to - from + 1 }, (_, offset) => from + offset);
      })()
    : value.split(',').map(Number).filter(Number.isInteger);
  if (seeds.length === 0) throw new Error('--seeds must contain at least one seed');
  return [...new Set(seeds)];
}

function input(seed: number): Parameters<typeof simulateGame>[0] {
  return {
    home: { teamId: demoConfig.home_team.id, roster: demoConfig.home_team.roster, lineupPackages: demoConfig.home_team.lineup_packages },
    away: { teamId: demoConfig.away_team.id, roster: demoConfig.away_team.roster, lineupPackages: demoConfig.away_team.lineup_packages },
    seed,
  };
}

function collectMix(results: Iterable<GameResult>): { mix: Map<string, number>; total: number } {
  const mix = new Map<string, number>();
  let total = 0;
  for (const result of results) {
    for (const snapshot of result.snapshots ?? []) {
      if (snapshot.phase !== 'LIVE' || !snapshot.tactical) continue;
      const kind = snapshot.tactical.kind;
      // Count every LIVE frame's tactical kind: a possession that runs
      // PNR for 8s and breaks into ISO for 4s contributes to both, which
      // is exactly what the spectator-rendered mix should reflect.
      mix.set(kind, (mix.get(kind) ?? 0) + 1);
      total += 1;
    }
  }
  return { mix, total };
}

export function playTypeMixReport(results: Iterable<GameResult>, seeds: readonly number[]): PlayTypeMixReport {
  const { mix, total } = collectMix(results);
  const deviations = (Object.keys(NBA_PLAY_TYPE_REFERENCE) as Array<keyof typeof NBA_PLAY_TYPE_REFERENCE>).map((kind) => {
    const band = NBA_PLAY_TYPE_REFERENCE[kind];
    const share = total > 0 ? (mix.get(kind) ?? 0) / total : 0;
    return { kind, share, band, pass: share >= band.min && share <= band.max };
  });
  const passed = deviations.every((d) => d.pass);
  return { seeds, mix: Object.fromEntries(mix), total, deviations, passed };
}

function main(): void {
  const argv = process.argv.slice(2);
  const seeds = parseSeeds(argv);
  const report = playTypeMixReport((function* () {
    for (const seed of seeds) yield simulateGame(input(seed));
  })(), seeds);
  if (argv.includes('--json')) {
    writeFileSync(resolve(process.cwd(), 'play-type-mix-report.json'), JSON.stringify(report, null, 2));
    process.stdout.write(`play-type-mix: ${report.passed ? 'PASS' : 'FAIL'} → play-type-mix-report.json\n`);
    return;
  }
  process.stdout.write(`play-type-mix: ${report.passed ? 'PASS' : 'FAIL'} (${report.total} frames, ${seeds.length} games)\n`);
  for (const d of report.deviations) {
    process.stdout.write(`  ${d.kind.padEnd(16)} ${(d.share * 100).toFixed(1).padStart(5)}%  band [${(d.band.min * 100).toFixed(0)}%, ${(d.band.max * 100).toFixed(0)}%]  ${d.pass ? 'ok' : 'OUT'}\n`);
  }
  if (!report.passed) process.exitCode = 1;
}

main();
