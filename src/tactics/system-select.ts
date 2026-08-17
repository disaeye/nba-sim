/**
 * Choose a tactical system for one possession from a lineup's effective
 * identity. Each system's fitness is the sum of role-fit weights plus a
 * capability-bias term; the winner is selected by a single weighted draw
 * so the possession's RNG draw order stays auditable.
 */
import type { LineupPackage } from '../identity/types.js';
import { resolveLineupIdentity } from '../identity/roles.js';
import { weighted } from '../rng/helpers.js';
import type { Rng } from '../rng/types.js';
import {
  TACTICAL_SYSTEMS,
  getSystem,
  inferLegacySystem,
  systemFitness,
} from './systems.js';

export type TacticalSystemId = import('./systems.js').TacticalSystemId;

const FAMILY_BY_KIND: Record<string, TacticalSystemId> = {
  TRANSITION_PUSH: 'PUSH_PACE',
  PNR_ROLL: 'PICK_AND_ROLL',
  PNR_POP: 'SPREAD_PICK_POP',
  DRIVE_KICK: 'DRIVE_KICK',
  POST_UP: 'POST_TOUCH',
  OFF_BALL_SCREEN: 'MOTION_OFFBALL',
  ISO: 'ISOLATION',
  HANDOFF: 'HANDOFF_FAMILY',
};

/** Inverse map — the executable family represented by a selected system. */
export function familyForSystem(systemId: TacticalSystemId): import('./team-plan.js').TeamPlanKind {
  const spec = getSystem(systemId);
  return spec.families[0] ?? 'ISO';
}

/** System whose primary family matches the given TeamPlanKind. */
export function systemForKind(kind: string): TacticalSystemId {
  return FAMILY_BY_KIND[kind] ?? 'ISOLATION';
}

/** Legacy fallback when no lineupIdentity and no rng draw is wanted. */
export function defaultSystem(pkg: LineupPackage): TacticalSystemId {
  const identity = resolveLineupIdentity(pkg);
  return inferLegacySystem(identity);
}
