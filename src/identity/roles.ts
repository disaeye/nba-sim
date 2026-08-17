/**
 * Bridge from the legacy `usageProfile` (creator/screener/spacer priority
 * lists) to the new `LineupIdentity` (roles + capabilities).
 *
 * Every package without an explicit `lineupIdentity` is materialized through
 * this helper so downstream tactical code only has to reason about roles.
 * The mapping is deliberately coarse: legacy Phase-1 packages carry no
 * capability information, so capabilities default to neutral (0.5) with
 * small biases derived from which priority list a jersey appears in.
 */
import type {
  LineupCapability,
  LineupIdentity,
  LineupPackage,
  LineupRole,
  LineupRoleAssignment,
  UsageProfile,
} from './types.js';
import type { PlayerData } from '../playerdata/types.js';

const NEUTRAL = 0.5;

function biased(overrides: Partial<LineupCapability>): LineupCapability {
  return {
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
    ...overrides,
  };
}

function rolesFor(
  jersey: string,
  profile: UsageProfile,
  capabilities?: LineupCapability,
): LineupRole[] {
  const roles: LineupRole[] = [];
  const isCreator = profile.creator.includes(jersey);
  const isScreener = profile.screener.includes(jersey);
  const isSpacer = profile.spacer.includes(jersey);
  const cap = capabilities;
  if (isCreator) {
    // A creator with strong post ability is a post hub (POST_TOUCH fit);
    // otherwise the standard initiator.
    if (cap && cap.postPlay >= 0.65 && cap.pullUp < 0.65) {
      roles.push('initiator', 'post_hub');
    } else {
      roles.push('initiator');
    }
    return roles;
  }
  if (isScreener) {
    // Roll big (classic PNR roll/rim) vs pop big (stretch five): a
    // screener with shooting ability pops, one without rolls. This is
    // what lets SPREAD_PICK_POP and POST_TOUCH compete with PICK_AND_ROLL.
    if (cap && cap.catchShoot >= 0.62) {
      roles.push('pop_big', 'rim_runner');
    } else {
      roles.push('roll_big', 'rim_runner');
    }
    if (cap && cap.postPlay >= 0.6) roles.push('post_hub');
    return roles;
  }
  if (isSpacer) {
    // A spacer with cutting ability is a movement shooter (MOTION_OFFBALL
    // fit); a pure shooter stays a spot-up.
    if (cap && cap.cutting >= 0.6 && cap.pullUp >= 0.55) {
      roles.push('movement_shooter', 'spot_up_shooter');
    } else {
      roles.push('spot_up_shooter');
    }
    return roles;
  }
  roles.push('connector');
  return roles;
}

function capabilitiesFor(
  jersey: string,
  profile: UsageProfile,
  playerData?: Readonly<Record<string, PlayerData>>,
): LineupCapability {
  const data = playerData?.[jersey];
  const a = data?.ability;
  // Real ability values (0..1 normalized from 20..99) when available;
  // the static biases remain the fallback for legacy packages.
  if (a) {
    const n = (v: number | undefined, fallback: number) =>
      v === undefined ? fallback : Math.max(0.15, Math.min(0.95, (v - 20) / 79));
    return biased({
      creation: n(a.POCK, 0.5),
      pullUp: n(a.PUM, 0.5),
      catchShoot: n(a.CS3, 0.5),
      rimFinishing: n(a.FINS, 0.5),
      passing: n(a.PASS, 0.5),
      screening: n(a.SCRN, 0.5),
      rolling: n(a.FINC, 0.5),
      popping: n(a.CS3, 0.5),
      postPlay: n(a.POST, 0.5),
      cutting: n(a.FINC, 0.5),
      handleSecurity: n(a.HND, 0.5),
      transition: n(data.physical.SPD, 0.5),
    });
  }
  if (profile.creator.includes(jersey)) {
    return biased({ creation: 0.75, pullUp: 0.7, passing: 0.65, handleSecurity: 0.7, transition: 0.7 });
  }
  if (profile.screener.includes(jersey)) {
    return biased({ screening: 0.8, rolling: 0.75, rimFinishing: 0.7, postPlay: 0.6 });
  }
  if (profile.spacer.includes(jersey)) {
    return biased({ catchShoot: 0.78, cutting: 0.55 });
  }
  return biased({});
}

/** Resolve a package's effective LineupIdentity, falling back to usageProfile. */
export function resolveLineupIdentity(
  pkg: LineupPackage,
  playerData?: Readonly<Record<string, PlayerData>>,
  abilityOverrides?: Readonly<Record<string, Partial<LineupCapability>>>,
): LineupIdentity {
  if (pkg.lineupIdentity) return pkg.lineupIdentity;
  const assignments: LineupRoleAssignment[] = pkg.players.map((jersey) => {
    const base = capabilitiesFor(jersey, pkg.usageProfile, playerData);
    const merged = abilityOverrides?.[jersey] ? { ...base, ...abilityOverrides[jersey] } : base;
    return {
      jersey,
      roles: rolesFor(jersey, pkg.usageProfile, merged),
      capabilities: merged,
    };
  });
  const primary = assignments[0]?.roles[0] ?? 'initiator';
  return { primary, assignments };
}

/** Look up the role list for a jersey within a package's effective identity. */
export function rolesForJersey(pkg: LineupPackage, jersey: string): readonly LineupRole[] {
  return resolveLineupIdentity(pkg).assignments.find((assignment) => assignment.jersey === jersey)?.roles ?? [];
}

/** Look up the capability vector for a jersey within a package. */
export function capabilitiesForJersey(pkg: LineupPackage, jersey: string): LineupCapability {
  const identity = resolveLineupIdentity(pkg);
  return identity.assignments.find((assignment) => assignment.jersey === jersey)?.capabilities ?? biased({});
}
