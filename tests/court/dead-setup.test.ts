import { describe, expect, it } from 'vitest';
import {
  buildFtAlignment,
  buildInboundAlignment,
  buildJumpBallAlignment,
  buildTipReceiveAlignment,
} from '../../src/court/dead-setup.js';
import { TIP_OFF_BASKETS } from '../../src/court/alignment.js';
import { buildCourtDrawSpec, distFeet, nx } from '../../src/court/geometry.js';

const MIN_SEPARATION_FT = 3;

interface PositionedPlayer {
  readonly jersey: string;
  readonly team: string;
  readonly x: number;
  readonly y: number;
}

function assertMinPairwiseSeparation(players: readonly PositionedPlayer[]): void {
  const offenders: string[] = [];
  for (let i = 0; i < players.length; i++) {
    for (let j = i + 1; j < players.length; j++) {
      const a = players[i];
      const b = players[j];
      if (a === undefined || b === undefined) continue;
      const d = distFeet(a.x, a.y, b.x, b.y);
      if (d < MIN_SEPARATION_FT) {
        offenders.push(`${a.team}#${a.jersey}↔${b.team}#${b.jersey}=${d.toFixed(2)}ft`);
      }
    }
  }
  expect(offenders, `pairs under ${MIN_SEPARATION_FT}ft`).toEqual([]);
}

const home = ['1', '2', '3', '4', '5'] as const;
const away = ['11', '12', '13', '14', '15'] as const;

describe('dead-ball court setup', () => {
  it('Given home shooting right, when building a free-throw alignment, then all players occupy a legal lane setup', () => {
    const alignment = buildFtAlignment({
      homeLineup: home,
      awayLineup: away,
      shooterId: '1',
      shootingTeam: 'home',
      baskets: TIP_OFF_BASKETS,
    });

    const shooter = alignment.players.find((player) => player.jersey === '1');
    const defenders = alignment.players.filter((player) => player.team === 'away');
    const boxOutPlayers = alignment.players.filter((player) => player.task === 'box_out');
    const ftLineX = buildCourtDrawSpec().right.ftLine.x1;

    expect(alignment.context).toBe('dead');
    expect(alignment.players).toHaveLength(10);
    expect(shooter).toMatchObject({ team: 'home', hasBall: true, task: 'ball_handler' });
    expect(Math.abs((shooter?.x ?? 0) - ftLineX)).toBeLessThanOrEqual(nx(1));
    expect(defenders).toHaveLength(5);
    expect(boxOutPlayers).not.toHaveLength(0);
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given a home backcourt inbound, when building an inbound alignment, then the inbounder has the ball near the sideline and a receiver is present', () => {
    const alignment = buildInboundAlignment({
      homeLineup: home,
      awayLineup: away,
      inboundTeam: 'home',
      inbounder: '1',
      receiver: '2',
      baskets: TIP_OFF_BASKETS,
      spot: 'sideline',
    });

    const inbounder = alignment.players.find((player) => player.jersey === '1');
    const receiver = alignment.players.find((player) => player.jersey === '2');

    expect(alignment.context).toBe('inbound');
    expect(alignment.players).toHaveLength(10);
    expect(inbounder).toMatchObject({ team: 'home', hasBall: true, task: 'inbound' });
    expect(inbounder?.x).toBeLessThanOrEqual(nx(10));
    expect(receiver).toMatchObject({ team: 'home', hasBall: false, task: 'receive_tip' });
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given a free-throw alignment, when measuring every pair of players, then each pair is at least 3ft apart (R-2b lane spread)', () => {
    const alignment = buildFtAlignment({
      homeLineup: home,
      awayLineup: away,
      shooterId: '1',
      shootingTeam: 'home',
      baskets: TIP_OFF_BASKETS,
    });

    expect(alignment.players).toHaveLength(10);
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given an inbound alignment, when measuring every pair of players, then each pair is at least 3ft apart (R-2b backcourt spread)', () => {
    const alignment = buildInboundAlignment({
      homeLineup: home,
      awayLineup: away,
      inboundTeam: 'home',
      inbounder: '1',
      receiver: '2',
      baskets: TIP_OFF_BASKETS,
      spot: 'sideline',
    });

    expect(alignment.players).toHaveLength(10);
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given a free-throw alignment for away (left attack), when measuring every pair of players, then each pair is at least 3ft apart (R-2b symmetric)', () => {
    const alignment = buildFtAlignment({
      homeLineup: home,
      awayLineup: away,
      shooterId: '11',
      shootingTeam: 'away',
      baskets: TIP_OFF_BASKETS,
    });

    expect(alignment.players).toHaveLength(10);
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given an inbound alignment for away (left attack), when measuring every pair of players, then each pair is at least 3ft apart (R-2b symmetric)', () => {
    const alignment = buildInboundAlignment({
      homeLineup: home,
      awayLineup: away,
      inboundTeam: 'away',
      inbounder: '11',
      receiver: '12',
      baskets: TIP_OFF_BASKETS,
      spot: 'sideline',
    });

    expect(alignment.players).toHaveLength(10);
    assertMinPairwiseSeparation(alignment.players);
  });

  it('Given tip-off lineups, when building a jump-ball alignment, then ten players have the jump-ball context', () => {
    const alignment = buildJumpBallAlignment({
      homeLineup: home,
      awayLineup: away,
      homeJumper: '5',
      awayJumper: '15',
    });

    expect(alignment.context).toBe('jump_ball');
    expect(alignment.players).toHaveLength(10);
    expect(alignment.players.filter((player) => player.task === 'jump')).toHaveLength(2);
  });

  it('Given the away team wins the tip, when building the tip-receive alignment, then its controller has the ball in the correct context', () => {
    const alignment = buildTipReceiveAlignment({
      homeLineup: home,
      awayLineup: away,
      winner: 'away',
      controller: '12',
      baskets: TIP_OFF_BASKETS,
    });

    const controller = alignment.players.find((player) => player.jersey === '12');

    expect(alignment.context).toBe('tip_receive');
    expect(alignment.players).toHaveLength(10);
    expect(controller).toMatchObject({ team: 'away', hasBall: true, task: 'receive_tip' });
  });
});
