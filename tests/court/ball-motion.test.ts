import { describe, it, expect } from 'vitest';
import { makePose } from '../../src/court/poses.js';
import {
  ballHeld,
  startPass,
  stepBall,
  distPointToSegment,
  ballLoose,
} from '../../src/court/ball-motion.js';
import { loadMobilityConfig } from '../../src/court/mobility.js';

describe('ball motion', () => {
  it('held ball tracks holder pose each step', () => {
    let holder = makePose({ jersey: '1', team: 'home', x: 0.3, y: 0.5, hasBall: true });
    let ball = ballHeld(holder, 'home');
    holder = { ...holder, x: 0.4, y: 0.55 };
    const r = stepBall({
      ball,
      poses: { '1': holder },
      dt: 0.1,
      allowIntercept: false,
    });
    expect(r.ball.x).toBeCloseTo(0.4, 5);
    expect(r.ball.y).toBeCloseTo(0.55, 5);
    expect(r.ball.status).toBe('held');
  });

  it('pass completes to receiver over flight time', () => {
    const from = makePose({ jersey: '1', team: 'home', x: 0.2, y: 0.5, hasBall: true });
    const to = makePose({ jersey: '2', team: 'home', x: 0.6, y: 0.5 });
    let ball = startPass({ from, to, team: 'home' });
    const poses = { '1': { ...from, hasBall: false }, '2': to };
    let completed = false;
    for (let i = 0; i < 40; i++) {
      const r = stepBall({ ball, poses, dt: 0.1, allowIntercept: false });
      ball = r.ball;
      if (r.events.some((e) => e.kind === 'pass_complete')) {
        completed = true;
        expect(ball.holderId).toBe('2');
        expect(ball.status).toBe('held');
        break;
      }
    }
    expect(completed).toBe(true);
  });

  it('misses when receiver leaves the release-time target', () => {
    const from = makePose({ jersey: '1', team: 'home', x: 0.2, y: 0.5, hasBall: true });
    const target = makePose({ jersey: '2', team: 'home', x: 0.6, y: 0.5 });
    let ball = startPass({ from, to: target, team: 'home' });
    const poses = { '1': { ...from, hasBall: false }, '2': { ...target, x: 0.8 } };
    let result = stepBall({ ball, poses, dt: 0.1, allowIntercept: false });
    ball = result.ball;
    for (let i = 0; i < 40 && result.events.length === 0; i += 1) {
      result = stepBall({ ball, poses, dt: 0.1, allowIntercept: false });
      ball = result.ball;
    }
    expect(result.events).toContainEqual({ kind: 'pass_missed', receiverId: '2', x: target.x, y: target.y });
    expect(ball.status).toBe('loose');
  });

  it('reports out of bounds when the receiver is absent', () => {
    const from = makePose({ jersey: '1', team: 'home', x: 0.2, y: 0.5, hasBall: true });
    const target = makePose({ jersey: '2', team: 'home', x: 0.6, y: 0.5 });
    let ball = startPass({ from, to: target, team: 'home' });
    const poses = { '1': { ...from, hasBall: false } };
    let result = stepBall({ ball, poses, dt: 0.1, allowIntercept: false });
    ball = result.ball;
    for (let i = 0; i < 40 && result.events.length === 0; i += 1) {
      result = stepBall({ ball, poses, dt: 0.1, allowIntercept: false });
      ball = result.ball;
    }
    expect(result.events).toContainEqual({ kind: 'pass_out_of_bounds', receiverId: '2', x: target.x, y: target.y });
    expect(ball.status).toBe('loose');
  });

  it('defender on pass lane can intercept (geometry candidate)', () => {
    // Single-point-commit: stepBall emits a CANDIDATE only — it does NOT
    // transfer the ball. Adjudicate confirms via resolveSteal.
    const from = makePose({ jersey: '1', team: 'home', x: 0.2, y: 0.5, hasBall: true });
    const to = makePose({ jersey: '2', team: 'home', x: 0.8, y: 0.5 });
    const def = makePose({ jersey: '11', team: 'away', x: 0.5, y: 0.5 });
    let ball = startPass({ from, to, team: 'home' });
    const poses = {
      '1': { ...from, hasBall: false },
      '2': to,
      '11': def,
    };
    let candidate = false;
    for (let i = 0; i < 30; i++) {
      const r = stepBall({ ball, poses, dt: 0.1, allowIntercept: true });
      ball = r.ball;
      if (r.events.some((e) => e.kind === 'pass_intercepted')) {
        candidate = true;
        // Ball stays in flight (status='pass'); adjudicator owns the transfer.
        expect(ball.status).toBe('pass');
        expect(ball.holderId).toBeNull();
        const ev = r.events.find((e) => e.kind === 'pass_intercepted');
        expect(ev && 'stealerId' in ev ? ev.stealerId : null).toBe('11');
        break;
      }
    }
    expect(candidate).toBe(true);
  });

  it('point-to-segment distance is zero on the segment', () => {
    expect(distPointToSegment(0.5, 0.5, 0, 0.5, 1, 0.5)).toBeCloseTo(0, 9);
  });

  it('loose ball recovered when player in pickup radius', () => {
    const p = makePose({ jersey: '5', team: 'home', x: 0.5, y: 0.5 });
    const ball = ballLoose({ x: 0.51, y: 0.5 });
    const r = stepBall({
      ball,
      poses: { '5': p },
      dt: 0.1,
      allowIntercept: true,
      cfg: loadMobilityConfig(),
    });
    expect(r.events.some((e) => e.kind === 'loose_recovered')).toBe(true);
    expect(r.ball.holderId).toBe('5');
  });

  it('physical rebound recovery preserves the miss team and winner', () => {
    const rebounder = makePose({ jersey: '11', team: 'away', x: 0.5, y: 0.5 });
    const loose = ballLoose({ x: 0.5, y: 0.5 }, 'home');
    const r = stepBall({
      ball: loose,
      poses: { '11': rebounder },
      dt: 0.1,
      allowIntercept: true,
      cfg: loadMobilityConfig(),
    });
    expect(r.events).toContainEqual({
      kind: 'loose_recovered',
      recovererId: '11',
      team: 'away',
    });
    expect(r.ball.holderId).toBe('11');
    expect(r.ball.reboundOffenseTeam).toBe('home');
  });
});
