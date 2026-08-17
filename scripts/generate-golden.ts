#!/usr/bin/env node
/**
 * T20 — Regenerate the golden determinism fixtures.
 *
 * Output:
 *   - fixtures/golden-seed-42.json
 *   - fixtures/golden-seed-7.json
 *
 * WHEN TO RUN
 *   Run this script ONLY when the determinism contract is intentionally
 *   changed: a `foundation_version` bump, a kernel draw-order change, or
 *   any edit to plays/duration/resolve/identity that should alter the
 *   event stream. Committing changed golden fixtures is the semantic
 *   signal that the contract has moved.
 *
 * DO NOT run casually between releases — the committed golden fixtures
 * are the I5 determinism guard, and `tests/simulate/canonical-game.test.ts`
 * fails when current code drifts from them.
 *
 * Usage:  npx tsx scripts/generate-golden.ts
 */
import { writeFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { createHash } from 'node:crypto';

import { simulateGame } from '../src/simulate.js';
import type { GameInput } from '../src/simulate.js';
import { FOUNDATION_VERSION } from '../src/sim-utils.js';

import demoGame from '../config/demo-game.json' with { type: 'json' };
import pkg from '../package.json' with { type: 'json' };

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = resolve(__dirname, '..');

const SEEDS = [42, 7] as const;

interface DemoRosterEntry {
  readonly id: string;
  readonly jersey: string;
  readonly teamId: string;
}
interface DemoLineupPackage {
  readonly id: string;
  readonly players: readonly string[];
  readonly usageProfile: {
    readonly creator: readonly string[];
    readonly screener: readonly string[];
    readonly spacer: readonly string[];
  };
}
interface DemoShape {
  readonly home_team: {
    readonly id: string;
    readonly roster: readonly DemoRosterEntry[];
    readonly lineup_packages: readonly DemoLineupPackage[];
  };
  readonly away_team: DemoShape['home_team'];
}

interface GoldenMeta {
  readonly foundation_version: string;
  readonly seed: number;
  readonly generated_at: string;
  readonly nba_sim_version: string;
  readonly input_hash: string;
}

interface GoldenFixture {
  readonly meta: GoldenMeta;
  readonly event_type_sequence: readonly string[];
  readonly final_score: { readonly home: number; readonly away: number };
  readonly event_count: number;
  readonly periods: number;
}

const DEMO = demoGame as DemoShape;

/** Mirror tests/simulate/_helpers.ts:demoInput so the script uses the same input shape. */
function demoInput(seed: number): GameInput {
  return {
    home: {
      teamId: DEMO.home_team.id,
      roster: DEMO.home_team.roster as GameInput['home']['roster'],
      lineupPackages: DEMO.home_team.lineup_packages as GameInput['home']['lineupPackages'],
    },
    away: {
      teamId: DEMO.away_team.id,
      roster: DEMO.away_team.roster as GameInput['away']['roster'],
      lineupPackages: DEMO.away_team.lineup_packages as GameInput['away']['lineupPackages'],
    },
    seed,
  };
}

const inputHash = createHash('sha256')
  .update(JSON.stringify(demoGame))
  .digest('hex')
  .slice(0, 16);

const nbaSimVersion = (pkg as { version: string }).version;
const generatedAt = new Date().toISOString();

for (const seed of SEEDS) {
  const result = simulateGame(demoInput(seed));
  const periodSet = new Set(result.events.map((e) => e.period));

  const fixture: GoldenFixture = {
    meta: {
      foundation_version: FOUNDATION_VERSION,
      seed,
      generated_at: generatedAt,
      nba_sim_version: nbaSimVersion,
      input_hash: inputHash,
    },
    event_type_sequence: result.events.map((e) => e.type),
    final_score: {
      home: result.box_score.home.reduce((s, p) => s + p.points, 0),
      away: result.box_score.away.reduce((s, p) => s + p.points, 0),
    },
    event_count: result.events.length,
    periods: periodSet.size,
  };

  const outPath = resolve(ROOT, 'fixtures', `golden-seed-${seed}.json`);
  writeFileSync(outPath, `${JSON.stringify(fixture, null, 2)}\n`, 'utf8');
  // eslint-disable-next-line no-console -- intentional CLI feedback
  console.error(
    `[golden] seed=${seed} → ${outPath} ` +
      `(events=${fixture.event_count}, H ${fixture.final_score.home}-${fixture.final_score.away} A, ` +
      `periods=${fixture.periods})`,
  );
}
