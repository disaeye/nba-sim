import { describe, it, expect } from 'vitest';
import { makePose, stepPose, setTarget, dist, stepAllPoses } from '../../src/court/poses.js';
import { loadMobilityConfig } from '../../src/court/mobility.js';

describe('continuous poses', () => {
  it('does not teleport: short dt moves partway to target', () => {
    const p0 = makePose({
      jersey: '1',
      team: 'home',
      x: 0.2,
      y: 0.5,
      targetX: 0.9,
      targetY: 0.5,
      action: 'advance',
    });
    const p1 = stepPose(p0, 0.1);
    expect(p1.x).toBeGreaterThan(0.2);
    expect(p1.x).toBeLessThan(0.9);
    expect(p1.arrived).toBe(false);
  });

  it('arrives when close enough after enough time', () => {
    const cfg = loadMobilityConfig();
    let p = makePose({
      jersey: '1',
      team: 'home',
      x: 0.5,
      y: 0.5,
      targetX: 0.55,
      targetY: 0.5,
      action: 'advance',

    });
    for (let i = 0; i < 80; i++) {
      p = stepPose(p, cfg.tick_seconds, cfg);
      if (p.arrived) break;
    }
    expect(p.arrived).toBe(true);
    expect(dist(p.x, p.y, p.targetX, p.targetY)).toBeLessThanOrEqual(cfg.arrival_epsilon + 1e-9);
  });

  it('setTarget clears arrived when far', () => {
    const p0 = makePose({ jersey: '2', team: 'away', x: 0.1, y: 0.1 });
    const p1 = setTarget(p0, 0.9, 0.9, 'cut');
    expect(p1.arrived).toBe(false);
    expect(p1.action).toBe('cut');
  });
  it('nearby defender slows movement without changing the action speed table', () => {
    const base = loadMobilityConfig();
    const cfg = { ...base, physical: { ...base.physical, pressure_radius_ft: 6, pressure_speed_floor: 0.55 } };
    const runner = makePose({ jersey: '1', team: 'home', x: 0.2, y: 0.5, targetX: 0.8, targetY: 0.5, action: 'cut' });
    const far = makePose({ jersey: '11', team: 'away', x: 0.2, y: 0.8 });
    const near = makePose({ jersey: '11', team: 'away', x: 0.2, y: 0.53 });
    const fast = stepPose(runner, 0.1, cfg, { '1': runner, '11': far });
    const slowed = stepPose(runner, 0.1, cfg, { '1': runner, '11': near });
    expect(slowed.x - runner.x).toBeLessThan(fast.x - runner.x);
  });

  it('body contact resolution separates overlapping players', () => {
    const a = makePose({ jersey: '1', team: 'home', x: 0.5, y: 0.5 });
    const b = makePose({ jersey: '11', team: 'away', x: 0.5, y: 0.5 });
    const separated = stepAllPoses({ '1': a, '11': b }, 0, loadMobilityConfig());
    expect(dist(separated['1']!.x, separated['1']!.y, separated['11']!.x, separated['11']!.y) * 94).toBeGreaterThanOrEqual(1.9);
  });

});

