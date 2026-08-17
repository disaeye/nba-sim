/**
 * T9 — live retarget targets must not clamp-pile on x∈{0,1}.
 */
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
    out[jersey] = makePose({
      jersey,
      team: Number(jersey) <= 10 ? 'home' : 'away',
      x: s.x,
      y: s.y,
    });
  }
  return out;
}

describe('T9 edge clamp / OOB targets', () => {
  it('regionFor defend_help near left baseline does not sit on x=0', () => {
    const r = regionFor('defend_help', 'left', 0.08, 0.5, 'high_y');
    expect(r.cx).toBeGreaterThan(0.01);
    expect(r.cx).toBeLessThan(0.99);
  });

  it('resolveSlotTarget for defend_ball near rim does not return x∈{0,1}', () => {
    const poses = posesAt({
      '1': { x: 0.12, y: 0.5 },
      '11': { x: 0.1, y: 0.48 },
      '2': { x: 0.3, y: 0.4 },
      '3': { x: 0.3, y: 0.6 },
      '4': { x: 0.25, y: 0.5 },
      '5': { x: 0.35, y: 0.5 },
      '12': { x: 0.2, y: 0.3 },
      '13': { x: 0.2, y: 0.7 },
      '14': { x: 0.15, y: 0.4 },
      '15': { x: 0.15, y: 0.6 },
    });
    const world = buildLiveWorld({
      poses,
      ballX: 0.08,
      ballY: 0.5,
      holderId: '1',
      ballStatus: 'held',
      baskets: TIP_OFF_BASKETS,
      offense: 'away',
      mode: 'HALFCOURT',
      playId: 'pnr_high',
      binding: {
        primary_creator: '11',
        secondary_creator: '12',
        screener: '14',
        spacer_strong: '13',
        spacer_weak: '15',
      },
      shotClock: 14,
      gameClock: 400,
    });
    const r = resolveSlotTarget('defend_ball', world, '1', { x: 0.08, y: 0.5 });
    expect(r.x).toBeGreaterThan(0.01);
    expect(r.x).toBeLessThan(0.99);
    expect(r.y).toBeGreaterThan(0.01);
    expect(r.y).toBeLessThan(0.99);
  });

  it('retargetFromLiveWorld never assigns target x∈{0,1} for frontcourt samples', () => {
    const poses = posesAt({
      '1': { x: 0.7, y: 0.5 },
      '2': { x: 0.75, y: 0.3 },
      '3': { x: 0.8, y: 0.7 },
      '4': { x: 0.72, y: 0.55 },
      '5': { x: 0.78, y: 0.45 },
      '11': { x: 0.85, y: 0.5 },
      '12': { x: 0.88, y: 0.3 },
      '13': { x: 0.88, y: 0.7 },
      '14': { x: 0.9, y: 0.4 },
      '15': { x: 0.9, y: 0.6 },
    });
    const world = buildLiveWorld({
      poses,
      ballX: 0.92,
      ballY: 0.5,
      holderId: '1',
      ballStatus: 'held',
      baskets: TIP_OFF_BASKETS,
      offense: 'home',
      mode: 'HALFCOURT',
      playId: 'pnr_high',
      binding,
      shotClock: 12,
      gameClock: 500,
    });
    void deriveStrongSide;
    const targets = retargetFromLiveWorld(world, home, away, '1');
    for (const t of targets) {
      expect(t.x, `${t.jersey} x=${t.x}`).toBeGreaterThan(0.01);
      expect(t.x, `${t.jersey} x=${t.x}`).toBeLessThan(0.99);
      expect(t.y).toBeGreaterThan(0.01);
      expect(t.y).toBeLessThan(0.99);
    }
  });
});
