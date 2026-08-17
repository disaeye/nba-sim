import type { ActionKind, Intent } from '../decision/types.js';

export type ActionStage = 'PLANNED' | 'INITIATED' | 'PROGRESSING' | 'COMPLETED' | 'ABORTED';

export interface ActiveBallAction {
  readonly jersey: string;
  readonly kind: ActionKind;
  readonly stage: ActionStage;
  readonly startedAt: number;
  readonly windupSeconds: number;
  readonly recoverySeconds: number;
  readonly intent: Intent;
}

export function actionWindupSeconds(kind: ActionKind): number {
  switch (kind) {
    case 'pass': return 0.35;
    case 'handoff': return 0.45;
    case 'drive': return 0.30;
    case 'crossover': return 0.28;
    case 'pivot': return 0.22;
    case 'back_to_basket': return 0.35;
    case 'triple_threat': return 0.18;
    case 'pump_fake': return 0.25;
    case 'shoot': return 0.70;
    case 'advance': return 0.15;
    default: return 0;
  }
}

export function actionRecoverySeconds(kind: ActionKind): number {
  switch (kind) {
    case 'pass': return 0.25;
    case 'handoff': return 0.35;
    case 'drive': return 0.45;
    case 'crossover': return 0.32;
    case 'pivot': return 0.28;
    case 'back_to_basket': return 0.30;
    case 'triple_threat': return 0.18;
    case 'pump_fake': return 0.30;
    case 'shoot': return 0.60;
    default: return 0;
  }
}

const PHYSICAL_BALL_ACTIONS: ReadonlySet<ActionKind> = new Set([
  'advance', 'pass', 'handoff', 'drive', 'triple_threat', 'back_to_basket', 'pivot', 'crossover', 'pump_fake', 'shoot',
]);
export type HandlingActionKind =
  | 'triple_threat'
  | 'back_to_basket'
  | 'pivot'
  | 'crossover'
  | 'pump_fake';

export function isHandlingAction(kind: ActionKind): kind is HandlingActionKind {
  return kind === 'triple_threat'
    || kind === 'back_to_basket'
    || kind === 'pivot'
    || kind === 'crossover'
    || kind === 'pump_fake';
}

/** Handling beats complete without emitting a ball fact; the next decision
 * reads the new defender/ball geometry and chooses the continuation. */
export function handlingActionFinished(action: ActiveBallAction, now: number): boolean {
  return isHandlingAction(action.kind)
    && actionElapsed(action, now) >= action.windupSeconds + action.recoverySeconds;
}
export function startsPhysicalBallAction(
  previous: ActiveBallAction | null,
  intent: Intent,
  holderId: string | null,
): boolean {
  if (holderId !== intent.jersey || !PHYSICAL_BALL_ACTIONS.has(intent.kind)) return false;
  if (previous === null || previous.jersey !== intent.jersey) return true;
  if (previous.kind !== intent.kind) return true;
  return previous.stage === 'COMPLETED' || previous.stage === 'ABORTED';
}

export function startBallAction(intent: Intent, startedAt: number): ActiveBallAction {
  return {
    jersey: intent.jersey,
    kind: intent.kind,
    stage: 'INITIATED',
    startedAt,
    windupSeconds: actionWindupSeconds(intent.kind),
    recoverySeconds: actionRecoverySeconds(intent.kind),
    intent,
  };
}

export function actionElapsed(action: ActiveBallAction, now: number): number {
  return Math.max(0, now - action.startedAt);
}

export function actionInWindup(action: ActiveBallAction, now: number): boolean {
  return actionElapsed(action, now) < action.windupSeconds;
}

export function actionInRecovery(action: ActiveBallAction, now: number): boolean {
  return action.stage === 'COMPLETED' && actionElapsed(action, now) < action.windupSeconds + action.recoverySeconds;
}

export function progressBallAction(action: ActiveBallAction, now?: number): ActiveBallAction {
  if (action.stage === 'INITIATED' && (now === undefined || !actionInWindup(action, now))) {
    return { ...action, stage: 'PROGRESSING' };
  }
  return action;
}
