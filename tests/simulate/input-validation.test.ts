import { describe, expect, it } from 'vitest';
import { demoInput } from './_helpers.js';
import { validateInput } from '../../src/index.js';
import type { GameInput } from '../../src/simulate.js';
function withHome(input: GameInput, patch: Partial<GameInput['home']>): GameInput {
  return { ...input, home: { ...input.home, ...patch } };
}

describe('public GameInput validation', () => {
  it('accepts the canonical demo input', () => {
    expect(() => validateInput(demoInput(42))).not.toThrow();
  });

  it('rejects invalid seeds before simulation starts', () => {
    expect(() => validateInput({ ...demoInput(), seed: -1 })).toThrow(/seed/);
    expect(() => validateInput({ ...demoInput(), seed: 1.5 })).toThrow(/seed/);
  });

  it('rejects duplicate or unknown starters', () => {
    const duplicate = withHome(demoInput(), {
      lineupPackages: [{ ...demoInput().home.lineupPackages[0]!, players: ['1', '1', '3', '4', '5'] }],
    });
    expect(() => validateInput(duplicate)).toThrow(/unique/);

    const missing = withHome(demoInput(), {
      lineupPackages: [{ ...demoInput().home.lineupPackages[0]!, players: ['1', '2', '3', '4', '99'] }],
    });
    expect(() => validateInput(missing)).toThrow(/not in roster/);
  });

  it('rejects out-of-range player abilities', () => {
    const input = withHome(demoInput(), {
      roster: demoInput().home.roster.map((player, index) => index === 0
        ? { ...player, abilities: { creation: 2 } }
        : player),
    });
    expect(() => validateInput(input)).toThrow(/\[0,1\]/);
  });
});
