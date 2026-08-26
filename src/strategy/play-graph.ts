/**
 * PlayGraph: Multi-branch directed tactical flow state machine.
 *
 * Implements the first-principles dynamic branching:
 * If primary action (e.g. High PNR) meets Drop/Switch/Blitz or stalls,
 * the possession seamlessly branches into Short-Roll, Pop, or Secondary DHO,
 * preventing broken-play stagnation and static confusion.
 */
import type { TeamPlanKind } from '../tactics/types.js';
import type { StrategyObjective } from './types.js';
import type { LiveCourtSense } from '../perception/live-court.js';

export type DefensiveRead = 'DROP' | 'SWITCH' | 'BLITZ' | 'DENIED' | 'STALLED' | 'ADVANTAGE_CREATED';

export interface TacticalBranchNode {
  readonly id: string;
  readonly kind: TeamPlanKind;
  readonly objective: StrategyObjective;
  readonly maxDwellSeconds: number;
  readonly nextOn: {
    readonly onDrop?: TeamPlanKind;
    readonly onSwitch?: TeamPlanKind;
    readonly onBlitz?: TeamPlanKind;
    readonly onStall?: TeamPlanKind;
    readonly onAdvantage?: TeamPlanKind;
  };
}

export const MODERN_PLAY_GRAPH: Readonly<Record<TeamPlanKind, TacticalBranchNode>> = {
  PNR_ROLL: {
    id: 'pnr_primary',
    kind: 'PNR_ROLL',
    objective: 'PAINT_TOUCH',
    maxDwellSeconds: 3.5,
    nextOn: {
      onDrop: 'PNR_POP',
      onSwitch: 'ISO',
      onBlitz: 'DRIVE_KICK',
      onStall: 'HANDOFF',
      onAdvantage: 'PNR_ROLL',
    },
  },
  PNR_POP: {
    id: 'pnr_pop',
    kind: 'PNR_POP',
    objective: 'GENERATE_THREE',
    maxDwellSeconds: 3.0,
    nextOn: {
      onSwitch: 'ISO',
      onStall: 'HANDOFF',
      onAdvantage: 'PNR_POP',
    },
  },
  HANDOFF: {
    id: 'dho_primary',
    kind: 'HANDOFF',
    objective: 'NORMAL_FLOW',
    maxDwellSeconds: 3.0,
    nextOn: {
      onDeny: 'OFF_BALL_SCREEN',
      onStall: 'ISO',
      onAdvantage: 'DRIVE_KICK',
    },
  },
  OFF_BALL_SCREEN: {
    id: 'offball_pin',
    kind: 'OFF_BALL_SCREEN',
    objective: 'GENERATE_THREE',
    maxDwellSeconds: 3.0,
    nextOn: {
      onDeny: 'HANDOFF',
      onStall: 'ISO',
      onAdvantage: 'OFF_BALL_SCREEN',
    },
  },
  POST_UP: {
    id: 'post_seal',
    kind: 'POST_UP',
    objective: 'POST_TOUCH',
    maxDwellSeconds: 4.0,
    nextOn: {
      onBlitz: 'DRIVE_KICK',
      onStall: 'ISO',
      onAdvantage: 'POST_UP',
    },
  },
  DRIVE_KICK: {
    id: 'drive_kick',
    kind: 'DRIVE_KICK',
    objective: 'GENERATE_THREE',
    maxDwellSeconds: 2.5,
    nextOn: {
      onStall: 'HANDOFF',
      onAdvantage: 'DRIVE_KICK',
    },
  },
  ISO: {
    id: 'iso_clear',
    kind: 'ISO',
    objective: 'ATTACK_MISMATCH',
    maxDwellSeconds: 4.0,
    nextOn: {
      onBlitz: 'DRIVE_KICK',
      onStall: 'HANDOFF',
      onAdvantage: 'ISO',
    },
  },
  TRANSITION_PUSH: {
    id: 'transition',
    kind: 'TRANSITION_PUSH',
    objective: 'PUSH_PACE',
    maxDwellSeconds: 4.0,
    nextOn: {
      onStall: 'PNR_ROLL',
    },
  },
};

/**
 * Evaluates live defense geometry to produce a dynamic read.
 */
export function evaluateDefensiveRead(sense: LiveCourtSense, currentKind: TeamPlanKind): DefensiveRead {
  if (!sense.handlerPose) return 'ADVANTAGE_CREATED';
  const hx = sense.handlerPose.x;
  const hy = sense.handlerPose.y;

  // 1) Blitz / Double-Team: multiple defenders swarming the ball handler (<= 6.0 ft)
  const nearDefenders = sense.defensePlayers.filter((d) => {
    const dFt = Math.hypot((d.pose.x - hx) * 94, (d.pose.y - hy) * 50);
    return dFt <= 6.0;
  });
  if (nearDefenders.length >= 2) return 'BLITZ';

  // 2) Drop Coverage: rim protector sagging deep in paint (>= 2 paint defenders or big dropped deep)
  if (sense.paintDefenders >= 2 && (currentKind === 'PNR_ROLL' || currentKind === 'DRIVE_KICK')) {
    return 'DROP';
  }

  // 3) Deny / Pressure: handler space pressure is intense and passing lanes are tightly guarded
  if (sense.spacePressure > 0.75) {
    return 'DENIED';
  }

  // 4) Stalled: shot clock running down (< 6.5s) without high advantage
  if (sense.shotClock < 6.5 && sense.spacePressure > 0.4) {
    return 'STALLED';
  }

  // 5) Advantage Created: handler beat primary defender or has daylight
  if (sense.spacePressure < 0.35) {
    return 'ADVANTAGE_CREATED';
  }

  return 'ADVANTAGE_CREATED';
}

/**
 * Transitions from current tactic to next branch if condition is met.
 */
export function routeTacticalBranch(currentKind: TeamPlanKind, read: DefensiveRead): TeamPlanKind {
  const node = MODERN_PLAY_GRAPH[currentKind];
  if (!node) return currentKind;

  switch (read) {
    case 'DROP':
      return node.nextOn.onDrop ?? currentKind;
    case 'SWITCH':
      return node.nextOn.onSwitch ?? currentKind;
    case 'BLITZ':
      return node.nextOn.onBlitz ?? currentKind;
    case 'DENIED':
      return node.nextOn.onDeny ?? currentKind;
    case 'STALLED':
      return node.nextOn.onStall ?? currentKind;
    case 'ADVANTAGE_CREATED':
      return node.nextOn.onAdvantage ?? currentKind;
    default:
      return currentKind;
  }
}
