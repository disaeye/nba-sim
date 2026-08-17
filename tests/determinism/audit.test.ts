/**
 * T20 — Forbidden-source audit (I5).
 *
 * The kernel must be fully deterministic. Any call to a wall-clock or
 * non-deterministic source inside `src/` is a contract violation and a
 * likely cause of cross-run divergence (the golden fixtures drift).
 *
 * This test enforces the rule mechanically: ripgrep `src/` for the three
 * forbidden sources named in the foundation invariants (I5) and fail if
 * any match is found.
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

interface RgResult {
  readonly exitCode: number;
  readonly output: string;
}

/**
 * Run `rg PATTERN SRC_PATH`. Returns exitCode + captured output.
 * execSync throws on non-zero exit, so the no-match (exit 1) and error
 * (exit 2) branches are funneled through catch.
 */
function runRg(pattern: string): RgResult {
  try {
    const out = execSync(`rg ${pattern} ${SRC_PATH}`, {
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

/**
 * Assert the given ripgrep pattern matches NOTHING in src/.
 * Throws a readable error including the offending lines on failure.
 */
function expectNoMatches(pattern: string, label: string): void {
  const result = runRg(pattern);
  if (result.exitCode !== 1) {
    throw new Error(
      `Forbidden-source audit FAILED for ${label} (rg exit=${result.exitCode}).\n` +
        `Offending matches in src/:\n${result.output}`,
    );
  }
  // Sanity: exitCode 1 is the only "clean" code. exitCode 2 means rg errored
  // (which would silently hide a real violation) — surface it explicitly.
  expect(result.exitCode, `${label}: expected rg exit 1 (no matches), got exit 2 (rg error)`).toBe(1);
}

describe('forbidden-source audit (I5) — no wall-clock or non-deterministic calls in src/', () => {
  it('contains no Math.random calls', () => {
    expectNoMatches('"Math\\.random"', 'Math.random');
  });

  it('contains no Date.now calls', () => {
    expectNoMatches('"Date\\.now"', 'Date.now');
  });

  it('contains no performance.now calls', () => {
    expectNoMatches('"performance\\.now"', 'performance.now');
  });

  it('audit exitCode 1 means rg ran cleanly (not exitCode 2 = rg error)', () => {
    // This re-runs the Math.random check but asserts the explicit exitCode
    // contract so a future ripgrep version that changes semantics is caught.
    const result = runRg('"Math\\.random"');
    expect(result.exitCode).toBe(1);
  });
});
