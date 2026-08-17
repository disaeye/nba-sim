import { describe, it, expect } from 'vitest';
import { cachedGame } from '../simulate/_helpers.js';
import { renderFromSnapshots } from '../../src/render/from-snapshots.js';

describe('renderFromSnapshots', () => {
  it('maps kernel snapshots 1:1 (stride 1) with court coords', () => {
    const result = cachedGame(42);
    const snaps = result.snapshots ?? [];
    expect(snaps.length).toBeGreaterThan(100);
    const frames = renderFromSnapshots(snaps, result.events, { stride: 1 });
    expect(frames.length).toBe(snaps.length);
    expect(frames[0]!.players.length).toBe(10);
    expect(frames[0]!.t).toBe(snaps[0]!.t_real);
    const mid = frames[Math.floor(frames.length / 2)]!;
    expect(mid.ball.x).toBeGreaterThanOrEqual(0);
    expect(mid.ball.x).toBeLessThanOrEqual(1);
  });

  it('stride > 1 subsamples without dual interpolation path', () => {
    const result = cachedGame(7);
    const snaps = result.snapshots ?? [];
    const frames = renderFromSnapshots(snaps, result.events, { stride: 10 });
    expect(frames.length).toBe(Math.ceil(snaps.length / 10));
  });
});
