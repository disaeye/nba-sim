import { describe, it, expect } from 'vitest';
import {
  buildLiveWorld,
  regionFor,
  resolveSlotTarget,
  retargetFromLiveWorld,
  deriveStrongSide,
} from '../../src/court/relations.js';
import { makePose, type PoseMap } from '../../src/court/poses.js';
import { TIP_OFF_BASKETS } from '../../src/court/alignment.js';
import type { RoleBinding } from '../../src/identity/types.js';
import { distFeet } from '../../src/court/geometry.js';

const binding: RoleBinding = {
  primary_creator: '1',
  secondary_creator: '2',
  screener: '4',
  spacer_strong: '3',
  spacer_weak: '5',
};

const home = ['1', '2', '3', '4', '5'] as const;
const away = ['11', '12', '13', '14', '15'] as const;

function posesAt(spots: Record<string, { x: number; y: number }>): PoseMap {
  const out: Record<string, ReturnType<typeof makePose>> = {};
  for (const [jersey, s] of Object.entries(spots)) {
    const homeJ = Number(jersey) <= 10;
    out[jersey] = makePose({
      jersey,
      team: homeJ ? 'home' : 'away',
      x: s.x,
      y: s.y,
      targetX: s.x,
      targetY: s.y,
    });
  }
  return out;
}

describe('relational live retarget', () => {
  it('T1: frontcourt player already in corner relation keeps target near current pose', () => {
    const strong = deriveStrongSide(0.9);
    const corner = regionFor('strong_corner', 'right', 0.7, 0.9, strong);
    const poses = posesAt({
      '1': { x: 0.7, y: 0.5 },
      '2': { x: 0.75, y: 0.4 },
      '3': { x: corner.cx, y: corner.cy },
      '4': { x: 0.8, y: 0.55 },
      '5': { x: 0.85, y: 0.15 },
      '11': { x: 0.88, y: 0.5 },
      '12': { x: 0.9, y: 0.4 },
      '13': { x: 0.9, y: 0.6 },
      '14': { x: 0.92, y: 0.3 },
      '15': { x: 0.92, y: 0.7 },
    });
    const world = buildLiveWorld({
      poses,
      ballX: 0.7,
      ballY: 0.5,
      holderId: '1',
      ballStatus: 'held',
      baskets: TIP_OFF_BASKETS,
      offense: 'home',
      mode: 'HALFCOURT',
      playId: 'pnr_high',
      binding,
      shotClock: 20,
      gameClock: 500,
    });
    const targets = retargetFromLiveWorld(world, home, away, '1');
    const spacer = targets.find((p) => p.jersey === '3')!;
    expect(spacer.satisfied).toBe(true);
    expect(distFeet(spacer.x, spacer.y, corner.cx, corner.cy)).toBeLessThan(3);
    expect(spacer.x).toBeGreaterThan(0.5);
    expect(Math.abs(spacer.x - poses['3']!.x)).toBeLessThan(0.02);
    expect(Math.abs(spacer.y - poses['3']!.y)).toBeLessThan(0.02);
  });

  it('T2: scattered frontcourt poses do not all snap to identical ideal stamps', () => {
    const poses = posesAt({
      '1': { x: 0.68, y: 0.55 },
      '2': { x: 0.72, y: 0.25 },
      '3': { x: 0.88, y: 0.88 },
      '4': { x: 0.8, y: 0.6 },
      '5': { x: 0.9, y: 0.12 },
      '11': { x: 0.75, y: 0.5 },
      '12': { x: 0.78, y: 0.3 },
      '13': { x: 0.78, y: 0.7 },
      '14': { x: 0.85, y: 0.4 },
      '15': { x: 0.85, y: 0.65 },
    });
    const world = buildLiveWorld({
      poses,
      ballX: 0.68,
      ballY: 0.55,
      holderId: '1',
      ballStatus: 'held',
      baskets: TIP_OFF_BASKETS,
      offense: 'home',
      mode: 'HALFCOURT',
      playId: 'pnr_high',
      binding,
      shotClock: 18,
      gameClock: 400,
    });
    const a = retargetFromLiveWorld(world, home, away, '1');
    const b = retargetFromLiveWorld(world, home, away, '1');
    const xs = a.filter((p) => p.team === 'home').map((p) => p.x);
    const unique = new Set(xs.map((x) => x.toFixed(3)));
    expect(unique.size).toBeGreaterThan(1);
    const ball = a.find((p) => p.hasBall)!;
    expect(distFeet(ball.x, ball.y, ball.x, ball.y)).toBeLessThan(50); // SportVU: handler-relative positioning
    expect(b.find((p) => p.jersey === '1')!.x).toBe(ball.x);
  });

  it('resolveSlotTarget: backcourt ball handler advances toward pocket not stay', () => {
    const poses = posesAt({
      '1': { x: 0.25, y: 0.5 },
      '2': { x: 0.3, y: 0.4 },
      '3': { x: 0.3, y: 0.6 },
      '4': { x: 0.28, y: 0.5 },
      '5': { x: 0.22, y: 0.5 },
      '11': { x: 0.6, y: 0.5 },
      '12': { x: 0.65, y: 0.4 },
      '13': { x: 0.65, y: 0.6 },
      '14': { x: 0.7, y: 0.45 },
      '15': { x: 0.7, y: 0.55 },
    });
    const world = buildLiveWorld({
      poses,
      ballX: 0.25,
      ballY: 0.5,
      holderId: '1',
      ballStatus: 'held',
      baskets: TIP_OFF_BASKETS,
      offense: 'home',
      mode: 'HALFCOURT',
      playId: 'pnr_high',
      binding,
      shotClock: 20,
      gameClock: 500,
    });
    const r = resolveSlotTarget('ball_handler_pocket', world, '1');
    expect(r.satisfied).toBe(false);
    expect(r.x).toBeGreaterThan(0.5);
  });
});
