import { describe, expect, it } from 'vitest';

import { makePose } from '../../src/court/poses.js';
import { distFeet } from '../../src/court/geometry.js';
import { decideHandlerAction } from '../../src/decision/expected-value.js';
import type { LiveCourtSense, PlayerSense } from '../../src/perception/live-court.js';

const RIM = { x: 0.05585106382978723, y: 0.5 } as const;

function player(jersey: string, team: 'home' | 'away', x: number, y: number): PlayerSense {
  const pose = makePose({ jersey, team, x, y });
  return {
    jersey,
    team,
    pose,
    distanceToBallFt: distFeet(x, y, 0.3, 0.36),
    distanceToRimFt: distFeet(x, y, RIM.x, RIM.y),
  };
}

function blitzSense(): LiveCourtSense {
  return {
    offense: 'away',
    defense: 'home',
    handler: '11',
    attack: 'left',
    baskets: { home: 'right', away: 'left' },
    attackDirection: -1,
    rim: RIM,
    ball: { x: 0.3, y: 0.36 },
    ballZone: 'frontcourt_center',
    shotClock: 13.5,
    gameClock: 600,
    scoreDiff: 0,
    period: 1,
    offensePlayers: [
      player('11', 'away', 0.3, 0.36),
      player('14', 'away', 0.272, 0.409),
      player('17', 'away', 0.287, 0.72),
      player('18', 'away', 0.239, 0.755),
      player('13', 'away', 0.214, 0.288),
    ],
    defensePlayers: [
      player('1', 'home', 0.28, 0.37),
      player('2', 'home', 0.18, 0.344),
      player('4', 'home', 0.178, 0.622),
      player('5', 'home', 0.247, 0.642),
      player('9', 'home', 0.262, 0.519),
    ],
    abilities: {},
    onBallDefender: '1',
    onBallDistanceFt: 2,
    paintDefenders: 1,
    openTeammates: [],
    catchWindowSeconds: 0,
    passTarget: null,
  };
}

describe('handler expected-value screen reads', () => {
  it('passes to the screener as the first release against a BLITZ', () => {
    const action = decideHandlerAction(
      blitzSense(),
      '11',
      'SETUP',
      'SCREEN_USE',
      'HALFCOURT',
      0,
      'PNR_POP',
      'BLITZ',
      0,
      'SET',
      '14',
    );

    expect(action).toEqual({ kind: 'pass', targetJersey: '14' });
  });
});
  it('executes a physical handoff to a close partner under on-ball pressure', () => {
    const sense = blitzSense();
    const handoff = {
      ...sense,
      offense: 'home' as const,
      defense: 'away' as const,
      handler: '11',
      attack: 'right' as const,
      baskets: { home: 'right' as const, away: 'left' as const },
      attackDirection: 1 as const,
      rim: { x: 0.9441489361702128, y: 0.5 },
      ball: { x: 0.7, y: 0.4 },
      offensePlayers: [
        player('11', 'home', 0.7, 0.4),
        player('14', 'home', 0.685, 0.45),
        player('17', 'home', 0.7, 0.72),
        player('18', 'home', 0.75, 0.75),
        player('13', 'home', 0.72, 0.28),
      ],
      defensePlayers: [
        player('1', 'away', 0.68, 0.4),
        player('2', 'away', 0.2, 0.34),
        player('4', 'away', 0.18, 0.62),
        player('5', 'away', 0.25, 0.64),
        player('9', 'away', 0.26, 0.52),
      ],
      onBallDefender: '1',
      onBallDistanceFt: 2,
      shotClock: 13.5,
    };
    const action = decideHandlerAction(
      handoff,
      '11',
      'SETUP',
      'SET',
      'HALFCOURT',
      0,
      'HANDOFF',
      null,
      0,
      null,
      '14',
    );
    expect(action).toEqual({ kind: 'handoff', targetJersey: '14' });
  });
