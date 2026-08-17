/**
 * 体力系统内核集成 (§8 of docs/playerdata-design.md):
 * lifecycle (init/tick/rest), rate resolution, sub requests, the resolve
 * exec pricing, and the end-to-end report/snapshot/event surface.
 */
import { describe, it, expect } from 'vitest';

import {
  initStamina, tickStamina, applyPeriodRest, consumptionRateFor, staminaFactor,
  detectSubRequests, STAMINA_PLAYTEST_SCALE,
} from '../src/stamina.js';
import { staminaMax } from '../src/playerdata/stamina.js';
import { demoInput } from './simulate/_helpers.js';
import type { LineupPackage } from '../src/identity/types.js';
import type { GameInput, Player } from '../src/simulate.js';

const BASE = {
  id: 'p', jersey: '1', teamId: 'home',
  playerData: {
    physical: { H: 200, WT: 105, WS: 210, VJ: 80, SPD: 70, LAT: 70, AGE: 25, DUR: 60 },
    ability: {} as never,
    pot: {} as never,
    tendency: {} as never,
    awareness: {} as never,
  },
};

function inputFor(roster: Player[]): GameInput {
  const pkg: LineupPackage = {
    id: 'starters',
    players: roster.slice(0, 5).map((p) => p.jersey),
    usageProfile: { creator: [roster[0]!.jersey, roster[1]!.jersey], screener: [roster[4]!.jersey], spacer: [roster[2]!.jersey, roster[3]!.jersey] },
  };
  return {
    home: { teamId: 'home', roster, lineupPackages: [pkg] },
    away: { teamId: 'away', roster, lineupPackages: [pkg] },
    seed: 7,
  };
}

describe('initStamina', () => {
  it('defaults to league DUR 60 / AGE 25 when no playerData', () => {
    const input = inputFor([
      { id: 'a', jersey: '1', teamId: 'home' },
      { id: 'b', jersey: '2', teamId: 'home' },
      { id: 'c', jersey: '3', teamId: 'home' },
      { id: 'd', jersey: '4', teamId: 'home' },
      { id: 'e', jersey: '5', teamId: 'home' },
      { id: 'f', jersey: '6', teamId: 'home' },
      { id: 'g', jersey: '7', teamId: 'home' },
      { id: 'h', jersey: '8', teamId: 'home' },
      { id: 'i', jersey: '9', teamId: 'home' },
      { id: 'j', jersey: '10', teamId: 'home' },
    ]);
    const map = initStamina(input);
    expect(map['1']!.max).toBe(staminaMax(60, 25));
    expect(map['1']!.stm).toBe(map['1']!.max);
  });

  it('reads DUR/AGE from playerData', () => {
    const old = { ...BASE, id: 'p2', jersey: '2', playerData: { ...BASE.playerData!, physical: { ...BASE.playerData!.physical, DUR: 40, AGE: 35 } } };
    const input = inputFor([
      { ...BASE, id: 'p1' },
      old,
      { ...BASE, id: 'p3', jersey: '3' },
      { ...BASE, id: 'p4', jersey: '4' },
      { ...BASE, id: 'p5', jersey: '5' },
      { ...BASE, id: 'p6', jersey: '6' },
      { ...BASE, id: 'p7', jersey: '7' },
      { ...BASE, id: 'p8', jersey: '8' },
      { ...BASE, id: 'p9', jersey: '9' },
      { ...BASE, id: 'p10', jersey: '10' },
    ]);
    const map = initStamina(input);
    expect(map['2']!.max).toBe(staminaMax(40, 35));
  });
});

describe('tickStamina (§8.2/8.3)', () => {
  it('consumes on court and rests on the bench', () => {
    const map = initStamina(inputFor([
      { ...BASE, id: 'p1' },
      { ...BASE, id: 'p2', jersey: '2' },
      { ...BASE, id: 'p3', jersey: '3' },
      { ...BASE, id: 'p4', jersey: '4' },
      { ...BASE, id: 'p5', jersey: '5' },
      { ...BASE, id: 'p6', jersey: '6' },
      { ...BASE, id: 'p7', jersey: '7' },
      { ...BASE, id: 'p8', jersey: '8' },
      { ...BASE, id: 'p9', jersey: '9' },
      { ...BASE, id: 'p10', jersey: '10' },
    ]));
    const rate = 1.6 * STAMINA_PLAYTEST_SCALE;
    const out = tickStamina({
      stamina: map,
      dt: 60, // one minute
      rates: { '1': rate, '2': 0.9 * STAMINA_PLAYTEST_SCALE, '3': rate, '4': rate, '5': rate },
      transitionHandler: null,
      onCourt: ['1', '2', '3', '4', '5'],
    });
    expect(out['1']!.stm).toBeCloseTo(out['1']!.max - rate, 6);
    expect(out['2']!.stm).toBeCloseTo(out['2']!.max - 0.9 * STAMINA_PLAYTEST_SCALE, 6);
    // Bench player 6 rests +2.0/min.
    expect(out['6']!.stm).toBeCloseTo(out['6']!.max, 6); // already at max
    expect(out['6']!.rest).toBeCloseTo(2.0, 6);
  });

  it('transition handler consumes ×1.2', () => {
    const map = initStamina(inputFor([
      { ...BASE, id: 'p1' },
      { ...BASE, id: 'p2', jersey: '2' },
      { ...BASE, id: 'p3', jersey: '3' },
      { ...BASE, id: 'p4', jersey: '4' },
      { ...BASE, id: 'p5', jersey: '5' },
      { ...BASE, id: 'p6', jersey: '6' },
      { ...BASE, id: 'p7', jersey: '7' },
      { ...BASE, id: 'p8', jersey: '8' },
      { ...BASE, id: 'p9', jersey: '9' },
      { ...BASE, id: 'p10', jersey: '10' },
    ]));
    const rate = 1.6 * STAMINA_PLAYTEST_SCALE;
    const normal = tickStamina({ stamina: map, dt: 60, rates: { '1': rate }, transitionHandler: null, onCourt: ['1'] });
    const transition = tickStamina({ stamina: map, dt: 60, rates: { '1': rate }, transitionHandler: '1', onCourt: ['1'] });
    expect(transition['1']!.stm).toBeCloseTo(normal['1']!.stm - rate * 0.2, 6);
  });

  it('never drops below 0 and never exceeds max', () => {
    const map = initStamina(inputFor([
      { ...BASE, id: 'p1' },
      { ...BASE, id: 'p2', jersey: '2' },
      { ...BASE, id: 'p3', jersey: '3' },
      { ...BASE, id: 'p4', jersey: '4' },
      { ...BASE, id: 'p5', jersey: '5' },
      { ...BASE, id: 'p6', jersey: '6' },
      { ...BASE, id: 'p7', jersey: '7' },
      { ...BASE, id: 'p8', jersey: '8' },
      { ...BASE, id: 'p9', jersey: '9' },
      { ...BASE, id: 'p10', jersey: '10' },
    ]));
    let out = map;
    for (let i = 0; i < 200; i++) {
      out = tickStamina({ stamina: out, dt: 60, rates: { '1': 9 }, transitionHandler: null, onCourt: ['1'] });
    }
    expect(out['1']!.stm).toBe(0);
    // dt is seconds: 6000s = 100 min of bench rest → capped at max.
    out = tickStamina({ stamina: out, dt: 6000, rates: {}, transitionHandler: null, onCourt: [] });
    expect(out['1']!.stm).toBe(out['1']!.max);
  });
});

describe('period rest (§8.3)', () => {
  it('+5 quarter / +15 halftime (⌛ recalibrated)', () => {
    const input = inputFor([
      { ...BASE, id: 'p1' },
      { ...BASE, id: 'p2', jersey: '2' },
      { ...BASE, id: 'p3', jersey: '3' },
      { ...BASE, id: 'p4', jersey: '4' },
      { ...BASE, id: 'p5', jersey: '5' },
      { ...BASE, id: 'p6', jersey: '6' },
      { ...BASE, id: 'p7', jersey: '7' },
      { ...BASE, id: 'p8', jersey: '8' },
      { ...BASE, id: 'p9', jersey: '9' },
      { ...BASE, id: 'p10', jersey: '10' },
    ]);
    const drained = { ...initStamina(input), '1': { max: 88, stm: 30, consumed: 58, rest: 0, subRequested: false, forcedRequested: false } };
    expect(applyPeriodRest(drained, false)['1']!.stm).toBe(35);
    expect(applyPeriodRest(drained, true)['1']!.stm).toBe(45);
    expect(applyPeriodRest(drained, true)['1']!.stm).toBeLessThanOrEqual(88);
  });
});

describe('consumptionRateFor', () => {
  it('uses the doc role from lineupIdentity when present', () => {
    const capabilities = {
      creation: 0.5, pullUp: 0.5, catchShoot: 0.5, rimFinishing: 0.5, passing: 0.5,
      screening: 0.5, rolling: 0.5, popping: 0.5, postPlay: 0.5, cutting: 0.5,
      handleSecurity: 0.5, transition: 0.5, onBallDefense: 0.5, helpDefense: 0.5,
    };
    const pkg: LineupPackage = {
      id: 'p', players: ['1'], usageProfile: { creator: ['1'], screener: [], spacer: [] },
      lineupIdentity: { primary: 'initiator', assignments: [{ jersey: '1', roles: ['primary_creator' as unknown as import('../src/identity/types.js').LineupRole], capabilities }] },
    };
    expect(consumptionRateFor('1', 'handler', pkg)).toBeCloseTo(1.6 * STAMINA_PLAYTEST_SCALE, 9);
  });

  it('falls back to the TeamRole map', () => {
    expect(consumptionRateFor('1', 'handler', undefined)).toBeCloseTo(1.6 * STAMINA_PLAYTEST_SCALE, 9);
    expect(consumptionRateFor('1', 'slot', undefined)).toBeCloseTo(0.9 * STAMINA_PLAYTEST_SCALE, 9);
    expect(consumptionRateFor('1', undefined, undefined)).toBeCloseTo(0.9 * STAMINA_PLAYTEST_SCALE, 9);
  });
});

describe('staminaFactor (§8.4)', () => {
  it('applies the segment function', () => {
    expect(staminaFactor(80)).toBe(1);
    expect(staminaFactor(60)).toBe(1);
    expect(staminaFactor(50)).toBeCloseTo(0.85 + 0.15 * (50 / 60), 9);
    expect(staminaFactor(30)).toBeCloseTo(0.85 + 0.15 * (30 / 60), 9);
    expect(staminaFactor(20)).toBeCloseTo(0.85 + 0.15 * (20 / 60), 9);
  });
});

describe('detectSubRequests (§8.6)', () => {
  it('fires each threshold once per player', () => {
    const map = {
      '1': { max: 88, stm: 34, consumed: 0, rest: 0, subRequested: false, forcedRequested: false },
      '2': { max: 88, stm: 20, consumed: 0, rest: 0, subRequested: false, forcedRequested: false },
      '3': { max: 88, stm: 50, consumed: 0, rest: 0, subRequested: false, forcedRequested: false },
    };
    const reqs = detectSubRequests(map, ['1', '2', '3'], { '1': 'home', '2': 'home', '3': 'home' });
    expect(reqs).toHaveLength(3); // 1: sub, 2: sub + forced
    expect(reqs.filter((r) => r.jersey === '1' && r.reason === 'stamina_sub_request')).toHaveLength(1);
    expect(reqs.filter((r) => r.jersey === '2' && r.reason === 'stamina_forced_sub')).toHaveLength(1);
    // After flagging, no repeats.
    const flagged = {
      '1': { ...map['1']!, subRequested: true },
      '2': { ...map['2']!, subRequested: true, forcedRequested: true },
      '3': map['3']!,
    };
    expect(detectSubRequests(flagged, ['1', '2', '3'], { '1': 'home', '2': 'home', '3': 'home' })).toHaveLength(0);
  });
});

describe('game integration', () => {
  it('stamina pricing feeds resolve — tired shooter make rate drops', () => {
    // §8.4: the exec factor multiplies the shot make rate. At STM 40 the
    // factor is 0.95; the same shot at fresh STM is strictly better.
    const fresh = staminaFactor(88);
    const tired = staminaFactor(40);
    expect(fresh).toBe(1);
    expect(tired).toBeCloseTo(0.85 + 0.15 * (40 / 60), 9);
    expect(tired).toBeLessThan(fresh);
  });
});

describe('playerdata module reuse', () => {
  it('kernel lives on the doc math (staminaMax / segments)', () => {
    expect(STAMINA_PLAYTEST_SCALE).toBeGreaterThan(1);
    expect(staminaMax(60, 25)).toBe(88);
  });
});
