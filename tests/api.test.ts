import { describe, expect, it } from 'vitest';
import {
  FOUNDATION_VERSION,
  buildSpectatorPackage,
  simulateGame,
  validateInput,
} from '../src/index.js';
import { demoInput } from './api-fixture.js';

describe('public API smoke contract', () => {
  it('exports the foundation version and complete simulation pipeline', () => {
    const input = demoInput(7);
    validateInput(input);
    const result = simulateGame(input);
    const pkg = buildSpectatorPackage(result);
    expect(FOUNDATION_VERSION).toMatch(/^\d+\.\d+\.\d+$/);
    expect(pkg.meta.seed).toBe(7);
    expect(pkg.event_count).toBe(result.events.length);
    expect(pkg.stream.tickCount).toBeGreaterThan(0);
    expect(pkg.broadcastSummary.length).toBeGreaterThan(0);
  });
});
