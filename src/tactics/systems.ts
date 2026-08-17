/**
 * Tactical systems — the "体系" layer that sits between lineup roles and
 * concrete action families (TeamPlanKind).
 *
 * A tactical system is a self-contained offensive playbook entry: it
 * declares which roles it wants on the floor, the action families it
 * prefers, and a fitness function for the live lineup. The engine never
 * branches on a position string; it branches on role + capability.
 *
 * Foundation 0.7.0 additive: every existing TeamPlanKind keeps its string
 * value; `systemId` is an optional parallel label used for selection bias
 * and narration. Backward compatible — systems.ts is advisory when a
 * lineup carries no `lineupIdentity`.
 */
import type { LineupCapability, LineupIdentity, LineupRole } from '../identity/types.js';
import type { TeamPlanKind } from './team-plan.js';

export type TacticalSystemId =
  | 'PUSH_PACE'
  | 'PICK_AND_ROLL'
  | 'SPREAD_PICK_POP'
  | 'DRIVE_KICK'
  | 'POST_TOUCH'
  | 'MOTION_OFFBALL'
  | 'ISOLATION'
  | 'HANDOFF_FAMILY';

export interface SystemRoleFit {
  readonly role: LineupRole;
  readonly weight: number;
}

export interface TacticalSystemSpec {
  readonly id: TacticalSystemId;
  readonly label: string;
  /** Action families this system naturally produces. */
  readonly families: readonly TeamPlanKind[];
  /** Roles whose presence raises this system's fitness. */
  readonly roleFit: readonly SystemRoleFit[];
  /**
   * Capability biases. Each entry nudges fitness by `(value - 0.5) * weight`.
   * Missing capabilities are treated as 0.5 (neutral).
   */
  readonly capabilityBias: Partial<Record<keyof LineupCapability, number>>;
}

const NEUTRAL = 0.5;

function capabilityDelta(cap: LineupCapability, key: keyof LineupCapability): number {
  return (cap[key] ?? NEUTRAL) - NEUTRAL;
}

/**
 * Score how well a set of roles + averaged capabilities fits a system.
 * Returns a non-negative fitness; higher is better.
 */
export function systemFitness(
  spec: TacticalSystemSpec,
  roles: readonly LineupRole[],
  capabilities: readonly LineupCapability[],
): number {
  let score = 1;
  const roleSet = new Set(roles);
  for (const fit of spec.roleFit) {
    if (roleSet.has(fit.role)) score += fit.weight;
  }
  const avg: LineupCapability = averageCapabilities(capabilities);
  for (const [key, weight] of Object.entries(spec.capabilityBias)) {
    if (typeof weight === 'number' && weight !== 0) {
      score += capabilityDelta(avg, key as keyof LineupCapability) * weight;
    }
  }
  return Math.max(0, score);
}

function averageCapabilities(caps: readonly LineupCapability[]): LineupCapability {
  if (caps.length === 0) return NEUTRAL_CAPABILITY;
  const keys = Object.keys(NEUTRAL_CAPABILITY) as readonly (keyof LineupCapability)[];
  const out = {} as Record<keyof LineupCapability, number>;
  for (const key of keys) {
    let sum = 0;
    for (const cap of caps) sum += cap[key] ?? NEUTRAL;
    out[key] = sum / caps.length;
  }
  return out;
}

export const NEUTRAL_CAPABILITY: LineupCapability = {
  creation: NEUTRAL,
  pullUp: NEUTRAL,
  catchShoot: NEUTRAL,
  rimFinishing: NEUTRAL,
  passing: NEUTRAL,
  screening: NEUTRAL,
  rolling: NEUTRAL,
  popping: NEUTRAL,
  postPlay: NEUTRAL,
  cutting: NEUTRAL,
  handleSecurity: NEUTRAL,
  transition: NEUTRAL,
  onBallDefense: NEUTRAL,
  helpDefense: NEUTRAL,
};

export const TACTICAL_SYSTEMS: readonly TacticalSystemSpec[] = [
  {
    id: 'PUSH_PACE',
    label: '转换推进',
    families: ['TRANSITION_PUSH'],
    roleFit: [
      { role: 'initiator', weight: 0.47 },
      { role: 'rim_runner', weight: 0.33 },
      { role: 'movement_shooter', weight: 0.2 },
    ],
    capabilityBias: { transition: 2.0, creation: 1.2, rimFinishing: 0.8 },
  },
  {
    id: 'PICK_AND_ROLL',
    label: '挡拆顺下体系',
    families: ['PNR_ROLL'],
    roleFit: [
      { role: 'initiator', weight: 0.33 },
      { role: 'roll_big', weight: 0.43 },
      { role: 'spot_up_shooter', weight: 0.33 },
    ],
    capabilityBias: { screening: 1.2, rolling: 1.2, creation: 0.8, catchShoot: 0.6 },
  },
  {
    id: 'SPREAD_PICK_POP',
    label: '空间挡拆外弹',
    families: ['PNR_POP'],
    roleFit: [
      { role: 'initiator', weight: 0.4 },
      { role: 'pop_big', weight: 0.53 },
      { role: 'movement_shooter', weight: 0.33 },
      { role: 'spot_up_shooter', weight: 0.27 },
    ],
    capabilityBias: { popping: 1.8, catchShoot: 1.4, creation: 1.0 },
  },
  {
    id: 'DRIVE_KICK',
    label: '突分体系',
    families: ['DRIVE_KICK'],
    roleFit: [
      { role: 'initiator', weight: 0.53 },
      { role: 'spot_up_shooter', weight: 0.47 },
      { role: 'movement_shooter', weight: 0.33 },
      { role: 'connector', weight: 0.27 },
    ],
    capabilityBias: { creation: 1.8, rimFinishing: 1.4, passing: 1.2, catchShoot: 1.2 },
  },
  {
    id: 'POST_TOUCH',
    label: '低位背打体系',
    families: ['POST_UP'],
    roleFit: [
      { role: 'post_hub', weight: 0.6 },
      { role: 'initiator', weight: 0.2 },
      { role: 'spot_up_shooter', weight: 0.33 },
    ],
    capabilityBias: { postPlay: 2.0, passing: 0.8, catchShoot: 0.8 },
  },
  {
    id: 'MOTION_OFFBALL',
    label: '无球掩护流动',
    families: ['OFF_BALL_SCREEN'],
    roleFit: [
      { role: 'movement_shooter', weight: 0.53 },
      { role: 'connector', weight: 0.4 },
      { role: 'roll_big', weight: 0.2 },
      { role: 'spot_up_shooter', weight: 0.27 },
    ],
    capabilityBias: { catchShoot: 1.4, cutting: 1.2, screening: 0.8, passing: 0.8 },
  },
  {
    id: 'ISOLATION',
    label: '单打体系',
    families: ['ISO'],
    roleFit: [
      { role: 'initiator', weight: 0.6 },
      { role: 'spot_up_shooter', weight: 0.27 },
    ],
    capabilityBias: { creation: 2.2, pullUp: 1.4, handleSecurity: 1.2 },
  },
  {
    id: 'HANDOFF_FAMILY',
    label: '手递手体系',
    families: ['HANDOFF'],
    roleFit: [
      { role: 'movement_shooter', weight: 0.47 },
      { role: 'initiator', weight: 0.33 },
      { role: 'connector', weight: 0.33 },
    ],
    capabilityBias: { catchShoot: 1.2, creation: 0.8, passing: 0.8 },
  },
];

const SYSTEM_BY_ID = new Map<TacticalSystemId, TacticalSystemSpec>(
  TACTICAL_SYSTEMS.map((spec) => [spec.id, spec]),
);

export function getSystem(id: TacticalSystemId): TacticalSystemSpec {
  const spec = SYSTEM_BY_ID.get(id);
  if (!spec) throw new Error(`tactical system not found: ${id}`);
  return spec;
}

/**
 * Default system inferred from a legacy `usageProfile` when a package has no
 * explicit `lineupIdentity`. Preserves Phase-1 behavior: a screener-heavy
 * unit defaults to PICK_AND_ROLL, a spacer-heavy unit to DRIVE_KICK, etc.
 */
export function inferLegacySystem(identity: LineupIdentity): TacticalSystemId {
  const roleSet = new Set(identity.assignments.flatMap((assignment) => assignment.roles));
  if (roleSet.has('roll_big') || roleSet.has('pop_big')) {
    return roleSet.has('pop_big') ? 'SPREAD_PICK_POP' : 'PICK_AND_ROLL';
  }
  if (roleSet.has('post_hub')) return 'POST_TOUCH';
  if (roleSet.has('movement_shooter')) return 'MOTION_OFFBALL';
  if (roleSet.has('initiator')) return 'DRIVE_KICK';
  return 'ISOLATION';
}
