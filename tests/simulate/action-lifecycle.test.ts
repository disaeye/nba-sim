import { describe, expect, it } from 'vitest';
import {
  actionInWindup,
  actionRecoverySeconds,
  actionWindupSeconds,
  handlingActionFinished,
  isHandlingAction,
  startBallAction,
} from '../../src/actions/lifecycle.js';
import type { Intent } from '../../src/decision/types.js';

const intent: Intent = {
  jersey: '4', team: 'home', kind: 'pass', targetJersey: '5', zone: 'frontcourt_center',
  task: 'ball_handler', score: 1, targetX: 0.5, targetY: 0.5, slot: null, satisfied: false,
};

describe('ball action physical lifecycle', () => {
  it('requires a pass windup before release', () => {
    const action = startBallAction(intent, 10);
    expect(actionWindupSeconds('pass')).toBeGreaterThan(0);
    expect(actionRecoverySeconds('pass')).toBeGreaterThan(0);
    expect(actionInWindup(action, 10.2)).toBe(true);
    expect(actionInWindup(action, 10.4)).toBe(false);
  });

  it.each(['triple_threat', 'back_to_basket', 'pivot', 'crossover', 'pump_fake'] as const)(
    'models %s as a finite handling beat',
    (kind) => {
      const handling = startBallAction({ ...intent, kind }, 10);
      expect(isHandlingAction(kind)).toBe(true);
      expect(handling.windupSeconds).toBeGreaterThan(0);
      expect(handlingActionFinished(handling, 10 + handling.windupSeconds + handling.recoverySeconds + 0.01)).toBe(true);
      expect(handlingActionFinished(handling, 10.01)).toBe(false);
    },
  );
});
