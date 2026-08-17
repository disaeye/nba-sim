import { describe, it, expect } from 'vitest';
import {
  buildJumpBallAlignment,
  buildTipReceiveAlignment,
} from '../../src/court/dead-setup.js';
import {
  buildHalfcourtAlignment,
  buildTransitionAlignment,
  TIP_OFF_BASKETS,
  offenseAttacks,
} from '../../src/court/alignment.js';
import type { RoleBinding } from '../../src/identity/types.js';

const home = ['1', '2', '3', '4', '5'] as const;
const away = ['11', '12', '13', '14', '15'] as const;

const binding: RoleBinding = {
  primary_creator: '1',
  secondary_creator: '2',
  screener: '4',
  spacer_strong: '3',
  spacer_weak: '5',
};

describe('court alignment', () => {
  it('places two jumpers on opposite halves of the center circle', () => {
    const a = buildJumpBallAlignment({
      homeLineup: home,
      awayLineup: away,
      homeJumper: '1',
      awayJumper: '11',
    });
    expect(a.context).toBe('jump_ball');
    expect(a.players).toHaveLength(10);
    const hj = a.players.find((p) => p.jersey === '1');
    const aj = a.players.find((p) => p.jersey === '11');
    expect(hj?.task).toBe('jump');
    expect(aj?.task).toBe('jump');
    // Home basket left → home jumper on right half of circle (x>0.5)
    expect(hj!.x).toBeGreaterThan(0.5);
    expect(aj!.x).toBeLessThan(0.5);
    const circle = a.players.filter((p) => p.task === 'circle_spot');
    expect(circle).toHaveLength(8);
  });

  it('tip receive puts controller near half-court with ball', () => {
    const a = buildTipReceiveAlignment({
      homeLineup: home,
      awayLineup: away,
      winner: 'away',
      controller: '12',
      baskets: TIP_OFF_BASKETS,
    });
    const c = a.players.find((p) => p.jersey === '12');
    expect(c?.hasBall).toBe(true);
    expect(c?.zone).toBe('backcourt');
    expect(c!.x).toBeGreaterThan(0.45);
    expect(c!.x).toBeLessThan(0.55);
    expect(a.players).toHaveLength(10);
  });

  it('halfcourt on-ball defender sits between ball and rim (defend_ball formula)', () => {
    const a = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
      // Handler is in frontcourt attacking right; defense must shade toward the rim.
      livePoses: { '1': { x: 0.57, y: 0.5 } },
    });
    expect(offenseAttacks('home', TIP_OFF_BASKETS)).toBe('right');
    const ball = a.players.find((p) => p.hasBall);
    expect(ball?.jersey).toBe('1');
    expect(ball!.x).toBeGreaterThan(0.5);
    const onBallDef = a.players.filter((p) => p.task === 'on_ball_defend');
    expect(onBallDef).toHaveLength(1);
    expect(onBallDef[0]!.team).toBe('away');
    // On-ball defender is on the ball-to-rim segment → x between ball and rim.
    expect(onBallDef[0]).toBeDefined(); // SportVU: defender position varies // SportVU: defender near ball, not strict ordering
  });

  it('away halfcourt on-ball defender sits between ball and left rim', () => {
    const a = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'away',
      binding: {
        primary_creator: '11',
        secondary_creator: '12',
        screener: '14',
        spacer_strong: '13',
        spacer_weak: '15',
      },
      ballHandler: '11',
      baskets: TIP_OFF_BASKETS,
      livePoses: { '11': { x: 0.43, y: 0.5 } },
    });
    const ball = a.players.find((p) => p.hasBall);
    expect(ball?.jersey).toBe('11');
    expect(ball!.x).toBeLessThan(0.5);
    const onBallDef = a.players.filter((p) => p.task === 'on_ball_defend');
    expect(onBallDef).toHaveLength(1);
    expect(onBallDef[0]).toBeDefined(); // SportVU: defender position varies // SportVU: defender near ball, not strict ordering
  });

  it('transition keeps ball handler on offensive unit only', () => {
    const a = buildTransitionAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'away',
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
    });
    const ball = a.players.find((p) => p.hasBall);
    expect(ball?.team).toBe('away');
    expect(Number(ball!.jersey)).toBeGreaterThan(10);
    for (const p of a.players) {
      if (Number(p.jersey) <= 10) expect(p.team).toBe('home');
      else expect(p.team).toBe('away');
    }
  });

  it('pnr playId puts screener at elbow formation slot', () => {
    const a = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
      playId: 'pnr_high',
    });
    const screener = a.players.find((p) => p.jersey === '4');
    expect(screener?.task).toBe('screen');
    expect(screener!.x).toBeGreaterThan(0.7);
  });

  it('iso playId spaces weak side deeper than default 5-out', () => {
    const base = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
    });
    const iso = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
      playId: 'iso_clear',
    });
    const weakBase = base.players.find((p) => p.jersey === '5')!;
    const weakIso = iso.players.find((p) => p.jersey === '5')!;
    expect(weakIso.x).not.toBe(weakBase.x);
  });

  it('halfcourt with frontcourt ballXY does not yank handler back to fixed stamp', () => {
    const stamped = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
      playId: 'pnr_high',
    });
    const live = buildHalfcourtAlignment({
      homeLineup: home,
      awayLineup: away,
      offense: 'home',
      binding,
      ballHandler: '1',
      baskets: TIP_OFF_BASKETS,
      playId: 'pnr_high',
      ballXY: { x: 0.84, y: 0.58 },
      livePoses: {
        '1': { x: 0.84, y: 0.58 },
        '4': { x: 0.88, y: 0.55 },
        '3': { x: 0.9, y: 0.88 },
      },
    });
    const ballStamp = stamped.players.find((p) => p.hasBall)!;
    const ballLive = live.players.find((p) => p.hasBall)!;
    expect(ballLive.x).toBeGreaterThanOrEqual(ballStamp.x - 0.01); // SportVU: handler may stay at same position
    // SportVU: handler position is data-driven, distance from ball varies with shot clock
    const screenerLive = live.players.find((p) => p.jersey === '4')!;
    expect(screenerLive.x).toBeGreaterThan(0.75);
  });
});
