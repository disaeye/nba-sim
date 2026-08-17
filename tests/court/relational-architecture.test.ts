/**
 * A1 — Relational architecture lock test (relational-tactics perfection plan).
 *
 * Live execution modules must compose pose targets through the relational
 * (slot-based) live-retarget path — `retargetFromLiveWorld` / `applyLiveRetarget`
 * — never by calling the absolute halfcourt / transition builders directly.
 * The absolute builders `buildHalfcourtAlignment` and `buildTransitionAlignment`
 * belong to dead-ball / cold-start paths (tip, FT, inbound) and must NOT be
 * reachable from any LIVE-tick module.
 *
 * This test enforces the rule mechanically by ripgrepping each live module
 * for any occurrence of the forbidden call patterns. Per the plan brief we
 * reject ALL occurrences (including imports / comments / dead code), so the
 * only safe way for a live module to mention these names is to not mention
 * them at all. `simulate.ts` is then positively asserted to import and use
 * the relational live-retarget pattern, so the architecture stays honest.
 *
 * ripgrep exit-code semantics (the inverse of grep):
 *   0 — match(es) found
 *   1 — no matches
 *   2 — error (e.g. bad regex, missing path)
 *
 * So a passing audit is `exitCode === 1` (clean).
 */
import { describe, it, expect } from 'vitest';
import { execSync } from 'node:child_process';
import { resolve, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const SRC_PATH = resolve(__dirname, '..', '..', 'src');

const LIVE_MODULES = [
  'simulate.ts',
  'sim-tick.ts',
  'decision/step.ts',
] as const;

interface RgResult {
  readonly exitCode: number;
  readonly output: string;
}

/**
 * Run `rg PATTERN FILE`. Returns exitCode + captured output.
 * execSync throws on non-zero exit, so the no-match (exit 1) and error
 * (exit 2) branches are funneled through catch — matches the audit.test.ts
 * helper so behavior is uniform across static-audit tests in this repo.
 */
function runRg(pattern: string, file: string): RgResult {
  // Single-quoted so `\(` survives shell parsing; bare `${pattern}` would let
  // the shell eat `(` and ripgrep would exit 2 (regex parse error), masking
  // the real audit outcome as an rg error.
  try {
    const out = execSync(`rg '${pattern}' '${SRC_PATH}/${file}'`, {
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    return { exitCode: 0, output: out };
  } catch (e) {
    const err = e as { status?: number; stdout?: Buffer | string };
    const code = typeof err.status === 'number' ? err.status : 1;
    const raw = err.stdout;
    const out = typeof raw === 'string' ? raw : raw?.toString() ?? '';
    return { exitCode: code, output: out };
  }
}

function modulePath(file: string): string {
  return `${SRC_PATH}/${file}`;
}

function expectNoMatches(pattern: string, file: string, label: string): void {
  const result = runRg(pattern, file);
  if (result.exitCode !== 1) {
    throw new Error(
      `A1 architecture lock FAILED for ${label} (rg exit=${result.exitCode}).\n` +
        `File: ${modulePath(file)}\n` +
        `Pattern: ${pattern}\n` +
        `Offending matches:\n${result.output}`,
    );
  }
  expect(
    result.exitCode,
    `${label}: expected rg exit 1 (no matches), got exit 2 (rg error)`,
  ).toBe(1);
}

describe('A1 — relational architecture lock (live modules must use retargetFromLiveWorld, not absolute halfcourt/transition builders)', () => {
  for (const file of LIVE_MODULES) {
    it(`${file} does not call buildHalfcourtAlignment(`, () => {
      expectNoMatches('buildHalfcourtAlignment\\(', file, `${file} → buildHalfcourtAlignment(`);
    });

    it(`${file} does not call buildTransitionAlignment(`, () => {
      expectNoMatches('buildTransitionAlignment\\(', file, `${file} → buildTransitionAlignment(`);
    });
  }

  it('simulate.ts imports retargetFromLiveWorld (relational live-retarget path)', () => {
    const result = runRg('retargetFromLiveWorld', 'simulate.ts');
    if (result.exitCode !== 0) {
      throw new Error(
        `A1 architecture lock FAILED: simulate.ts must import & use retargetFromLiveWorld.\n` +
          `File: ${modulePath('simulate.ts')}\n` +
          `rg exit=${result.exitCode}; ripgrep output:\n${result.output}`,
      );
    }
    expect(result.exitCode).toBe(0);
  });

  it('simulate.ts defines and uses applyLiveRetarget (the relational live-retarget wrapper)', () => {
    const result = runRg('applyLiveRetarget', 'simulate.ts');
    if (result.exitCode !== 0) {
      throw new Error(
        `A1 architecture lock FAILED: simulate.ts must use applyLiveRetarget.\n` +
          `File: ${modulePath('simulate.ts')}\n` +
          `rg exit=${result.exitCode}; ripgrep output:\n${result.output}`,
      );
    }
    expect(result.exitCode).toBe(0);
  });
});
