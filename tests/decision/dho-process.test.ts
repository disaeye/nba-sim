import { describe, expect, it } from 'vitest';
import { makePose } from '../../src/court/poses.js';
import { distFeet } from '../../src/court/geometry.js';
import { decideHandlerAction } from '../../src/decision/expected-value.js';
import type { LiveCourtSense, PlayerSense } from '../../src/perception/live-court.js';

const rim = { x: 0.9441489361702128, y: 0.5 } as const;

function player(jersey: string, team: 'home' | 'away', x: number, y: number): PlayerSense {
  const pose = makePose({ jersey, team, x, y });
  return {
    jersey,
    team,
    pose,
    distanceToBallFt: distFeet(x, y, 0.72, 0.32),
    distanceToRimFt: distFeet(x, y, rim.x, rim.y),
  };
}

function pressuredHandoffSense(handler: string, partner: string): LiveCourtSense {
  const positions: Record<string, [number, number]> = {
    '5': [0.72, 0.32],
    '9': [0.68, 0.35],
    '4': [0.73, 0.68],
    '1': [0.78, 0.25],
    '2': [0.75, 0.75],
  };
  return {
    offense: 'home',
    defense: 'away',
    handler,
    attack: 'right',
    baskets: { home: 'right', away: 'left' },
    attackDirection: 1,
    rim,
    ball: { x: positions[handler]![0], y: positions[handler]![1] },
    ballZone: 'frontcourt_center',
    shotClock: 11,
    gameClock: 600,
    scoreDiff: 0,
    period: 1,
    offensePlayers: Object.entries(positions).map(([jersey, [x, y]]) => player(jersey, 'home', x, y)),
    defensePlayers: [
      player('11', 'away', positions[handler]![0] + 0.02, positions[handler]![1]),
      player('13', 'away', 0.78, 0.18),
      player('14', 'away', 0.78, 0.82),
      player('17', 'away', 0.62, 0.5),
      player('18', 'away', 0.5, 0.5),
    ],
    abilities: {},
    onBallDefender: '11',
    onBallDistanceFt: 2,
    paintDefenders: 2,
    openTeammates: [],
    catchWindowSeconds: 0,
    passTarget: null,
  };
}

describe('DHO process decisions', () => {
  it('returns a physical handoff only for the initial pressured exchange', () => {
    const initial = decideHandlerAction(
      pressuredHandoffSense('5', '9'),
      '5',
      'SETUP',
      'SET',
      'HALFCOURT',
      0,
      'HANDOFF',
      null,
      0,
      null,
      '9',
    );
    expect(initial).toEqual({ kind: 'handoff', targetJersey: '9' });

    const afterExchange = decideHandlerAction(
      pressuredHandoffSense('9', '5'),
      '9',
      'SETUP',
      'SET',
      'HALFCOURT',
      0,
      'HANDOFF',
      null,
      0,
      null,
      '5',
      { giverId: '5', receiverId: '9' },
    );
    expect(afterExchange.kind).not.toBe('handoff');

    const downhill = decideHandlerAction(
      pressuredHandoffSense('9', '5'),
      '9',
      'EXECUTE',
      'ADVANTAGE',
      'HALFCOURT',
      0,
      'HANDOFF',
      null,
      0,
      null,
      '5',
      { giverId: '5', receiverId: '9' },
    );
    expect(downhill.kind).toBe('drive');
  });
});
