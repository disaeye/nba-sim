/**
 * T18 RED → GREEN tests for runFtSequence.
 *
 * The FT sequence is the canonical event chain:
 *   FT_START → (FT_ATTEMPT → FT_RESULT)+ → FT_SEQUENCE_END
 *
 * Each FT_RESULT consumes exactly one rng.next() via resolveFt. The
 * AND-ONE rule (T4 invariants §AND-ONE) pins: andOne=true forces 1
 * attempt regardless of the `attempts` arg. Tests cover:
 *   - event counts and types for 1/2/3 attempts and the and-one path
 *   - catalog `required_payload_fields` populated on each event
 *   - fold-behavior: FT_RESULT made events increment score via applyEvent
 *   - determinism: same seed → identical make/miss sequence
 */
import { describe, it, expect } from 'vitest';

import { mulberry32 } from '../../src/rng/index.js';
import type { Rng } from '../../src/rng/types.js';
import {
  loadResolveConfig,
  makeResolveContext,
  runFtSequence,
} from '../../src/resolve/index.js';
import type { FtSequenceConfig } from '../../src/resolve/types.js';
import { createInitialState } from '../../src/state/initial.js';
import { applyEvent } from '../../src/state/apply.js';
import type { Event, GameInput } from '../../src/state/types.js';

const config = loadResolveConfig();
const ctx = makeResolveContext(config);

const STARTERS: GameInput = {
  home: { id: 'home', starters: ['h1', 'h2', 'h3', 'h4', 'h5'] },
  away: { id: 'away', starters: ['a1', 'a2', 'a3', 'a4', 'a5'] },
};

const FT_TYPES = ['FT_START', 'FT_ATTEMPT', 'FT_RESULT', 'FT_SEQUENCE_END'] as const;

function ftTypes(events: readonly Event[]): string[] {
  return events.map((e) => e.type);
}

function ftResults(events: readonly Event[]): { n: number; made: boolean }[] {
  return events
    .filter((e) => e.type === 'FT_RESULT')
    .map((e) => ({
      n: e.payload['attempt_number'] as number,
      made: e.payload['made'] as boolean,
    }));
}

// ─── event count + structure ───────────────────────────────────────────────

describe('runFtSequence — event counts and structure', () => {
  it('2 attempts: FT_START + 2×(FT_ATTEMPT+FT_RESULT) + FT_SEQUENCE_END = 6 events', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    expect(events).toHaveLength(6);
    expect(ftTypes(events)).toEqual([
      'FT_START',
      'FT_ATTEMPT', 'FT_RESULT',
      'FT_ATTEMPT', 'FT_RESULT',
      'FT_SEQUENCE_END',
    ]);
  });

  it('3 attempts: FT_START + 3×(FT_ATTEMPT+FT_RESULT) + FT_SEQUENCE_END = 8 events', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    expect(events).toHaveLength(8);
    expect(events.filter((e) => e.type === 'FT_ATTEMPT')).toHaveLength(3);
    expect(events.filter((e) => e.type === 'FT_RESULT')).toHaveLength(3);
  });

  it('andOne=true: 1 FT attempt regardless of `attempts` arg', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: true,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    // FT_START + 1×(FT_ATTEMPT+FT_RESULT) + FT_SEQUENCE_END = 4 events
    expect(events).toHaveLength(4);
    expect(events.filter((e) => e.type === 'FT_ATTEMPT')).toHaveLength(1);
    expect(events.filter((e) => e.type === 'FT_RESULT')).toHaveLength(1);
  });

  it('all emitted types belong to the FT catalog', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    for (const e of events) {
      expect(FT_TYPES).toContain(e.type);
    }
  });
});

// ─── payload contract (catalog required_payload_fields) ───────────────────

describe('runFtSequence — payload fields per event catalog', () => {
  it('FT_START carries shooter_id, team, attempts', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'away', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    const start = events.find((e) => e.type === 'FT_START')!;
    expect(start.payload['shooter_id']).toBe('h1');
    expect(start.payload['team']).toBe('away');
    expect(start.payload['attempts']).toBe(2);
  });

  it('FT_ATTEMPT carries shooter_id, attempt_number', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    const attempts = events.filter((e) => e.type === 'FT_ATTEMPT');
    expect(attempts).toHaveLength(3);
    expect(attempts[0]!.payload['attempt_number']).toBe(1);
    expect(attempts[1]!.payload['attempt_number']).toBe(2);
    expect(attempts[2]!.payload['attempt_number']).toBe(3);
    for (const a of attempts) expect(a.payload['shooter_id']).toBe('h1');
  });

  it('FT_RESULT carries shooter_id, attempt_number, made', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    const results = events.filter((e) => e.type === 'FT_RESULT');
    expect(results).toHaveLength(2);
    for (const r of results) {
      expect(r.payload['shooter_id']).toBe('h1');
      expect(typeof r.payload['attempt_number']).toBe('number');
      expect(typeof r.payload['made']).toBe('boolean');
    }
  });

  it('FT_SEQUENCE_END has empty actors and no required payload', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 1, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(1), ctx);
    const end = events.find((e) => e.type === 'FT_SEQUENCE_END')!;
    expect(end.actors).toEqual([]);
    // next_phase is optional per catalog; absence is valid.
  });
});

// ─── draw accounting ───────────────────────────────────────────────────────

describe('runFtSequence — single-stream draw accounting', () => {
  it('consumes exactly N rng.next() for N attempts (one resolveFt per attempt)', () => {
    // The runFtSequence body emits FT_START/END/ATTEMPT without touching
    // rng; only the FT_RESULT resolve consumes draws. For attempts=3 we
    // therefore expect exactly 3 draws.
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const rngFn = mulberry32(1);
    const rngRaw = mulberry32(1);
    void runFtSequence(ft, rngFn, ctx);
    // Burn 3 raw draws; the 4th should match rngFn's next draw.
    void rngRaw.next(); void rngRaw.next(); void rngRaw.next();
    expect(rngFn.next()).toBe(rngRaw.next());
  });

  it('and-one path also consumes exactly one draw (1 attempt)', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: true,
    };
    const rngFn = mulberry32(7);
    const rngRaw = mulberry32(7);
    void runFtSequence(ft, rngFn, ctx);
    void rngRaw.next();
    expect(rngFn.next()).toBe(rngRaw.next());
  });
});

// ─── fold behavior — score increment ───────────────────────────────────────

describe('runFtSequence — fold behavior (applyEvent increments score)', () => {
  it('home shooter: each made FT_RESULT increments state.score.home by 1', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(7), ctx);
    const madeCount = events.filter(
      (e) => e.type === 'FT_RESULT' && (e.payload['made'] as boolean) === true,
    ).length;

    const initial = createInitialState(STARTERS);
    const finalState = events.reduce(applyEvent, initial);

    expect(finalState.score.home).toBe(madeCount);
    expect(finalState.score.away).toBe(0);
  });

  it('away shooter: each made FT_RESULT increments state.score.away by 1', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'a3', team: 'away', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(11), ctx);
    const madeCount = events.filter(
      (e) => e.type === 'FT_RESULT' && (e.payload['made'] as boolean) === true,
    ).length;

    const initial = createInitialState(STARTERS);
    const finalState = events.reduce(applyEvent, initial);

    expect(finalState.score.away).toBe(madeCount);
    expect(finalState.score.home).toBe(0);
  });

  it('input state is not mutated by the fold', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 2, andOne: false,
    };
    const events = runFtSequence(ft, mulberry32(3), ctx);
    const initial = createInitialState(STARTERS);
    const snapshot = JSON.stringify(initial);
    void events.reduce(applyEvent, initial);
    expect(JSON.stringify(initial)).toBe(snapshot);
  });
});

// ─── determinism ───────────────────────────────────────────────────────────

describe('runFtSequence — determinism', () => {
  it('same seed → identical make/miss sequence', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const a = ftResults(runFtSequence(ft, mulberry32(99), ctx));
    const b = ftResults(runFtSequence(ft, mulberry32(99), ctx));
    expect(a).toEqual(b);
  });

  it('different seeds → different make/miss sequences (over enough attempts)', () => {
    // 3 attempts × 1 draw each — different seeds diverge. Run multiple
    // sequences to rule out the small-N false-positive where two seeds
    // happen to produce identical 3-bit patterns.
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    let differed = false;
    for (let s = 1; s < 50 && !differed; s++) {
      const a = ftResults(runFtSequence(ft, mulberry32(s), ctx));
      const b = ftResults(runFtSequence(ft, mulberry32(s + 100), ctx));
      if (JSON.stringify(a) !== JSON.stringify(b)) differed = true;
    }
    expect(differed).toBe(true);
  });

  it('emitted event types are byte-stable across runs of the same seed', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 3, andOne: false,
    };
    const a = runFtSequence(ft, mulberry32(5), ctx).map((e) => e.type);
    const b = runFtSequence(ft, mulberry32(5), ctx).map((e) => e.type);
    expect(a).toEqual(b);
  });
});

// ─── RNG parameter — runs without consuming the rng until first FT_RESULT ──

describe('runFtSequence — idempotent on ctx', () => {
  it('does not mutate the passed-in ResolveContext', () => {
    const ft: FtSequenceConfig = {
      shooterId: 'h1', team: 'home', attempts: 2, andOne: false,
    };
    const rng: Rng = mulberry32(1);
    const before = JSON.stringify(ctx);
    void runFtSequence(ft, rng, ctx);
    expect(JSON.stringify(ctx)).toBe(before);
  });
});
