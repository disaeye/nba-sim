/**
 * Player × action mobility table.
 * Speed is court-length units per second (full court length = 1.0).
 */
import mobilityJson from '../../config/mobility.json' with { type: 'json' };

export interface BallMobility {
  readonly pass_speed: number;
  readonly shot_flight_min: number;
  readonly shot_flight_max: number;
  /** Horizontal shot speed (court-length units/s) for distance-based
   *   flight time: duration = clamp(dist / speed, min, max).
   *   ≈20.7 ft/s reproduces real NBA flight times (3PT ≈ 1.35s). */
  readonly shot_flight_speed: number;
  readonly intercept_radius: number;
  readonly strip_radius: number;
  readonly loose_pickup_radius: number;
}

export interface PhysicalMobility {
  readonly body_radius_ft: number;
  readonly pressure_radius_ft: number;
  readonly pressure_speed_floor: number;
  readonly contact_iterations: number;
  /** Speed multiplier for dead-ball repositioning (FT walk, inbound
   *   setups, tip formation) — real players walk, they do not jog. */
  readonly dead_ball_speed_scale: number;
}

export interface MobilityConfig {
  readonly foundation_version: string;
  readonly court_length_unit: number;
  readonly tick_seconds: number;
  readonly arrival_epsilon: number;
  readonly default_action_speeds: Readonly<Record<string, number>>;
  readonly player_overrides: Readonly<Record<string, Readonly<Record<string, number>>>>;
  readonly role_action_speeds?: Readonly<Record<string, Readonly<Record<string, number>>>>;
  readonly physical: PhysicalMobility;
  readonly ball: BallMobility;
}

/**
 * Movement role. Selects a role-specific speed profile (config/mobility.json
 * → role_action_speeds) so different body types move and react differently:
 * a primary creator bursts faster than a rim-protecting big, a spacer jogs
 * to the corner at spacer pace, an on-ball defender shuffles quicker than
 * a weak-side helper. Roles replace the Phase-1 jersey-id overrides (which
 * only mirrored position labels and broke homogeneity).
 */
export type MovementRole =
  | 'ball_handler'
  | 'screener'
  | 'spacer'
  | 'on_ball_defender'
  | 'help_defender'
  | 'weak_side_defender'
  | 'perimeter_defender';

const VALID_ROLES: ReadonlySet<MovementRole> = new Set<MovementRole>([
  'ball_handler', 'screener', 'spacer',
  'on_ball_defender', 'help_defender', 'weak_side_defender', 'perimeter_defender',
]);

export function isValidMovementRole(role: unknown): role is MovementRole {
  return typeof role === 'string' && VALID_ROLES.has(role as MovementRole);
}

let cached: MobilityConfig | null = null;

/**
 * P4.1 runtime per-player speed overrides derived from PlayerData.physical.SPD
 * (straight-line speed, 20..99). Set once per game by simulate.ts; speedFor
 * consults it before the static table. Absent playerData → no override
 * (the static role/action tables apply).
 */
const runtimeSpeedOverrides = new Map<string, Readonly<Record<string, number>>>();

/** Register per-jersey speed overrides derived from physical SPD. */
export function setRuntimeSpeedOverrides(
  overrides: Readonly<Record<string, Readonly<Record<string, number>>>>,
): void {
  runtimeSpeedOverrides.clear();
  for (const [jersey, profile] of Object.entries(overrides)) {
    runtimeSpeedOverrides.set(jersey, profile);
  }
}

/** Clear runtime speed overrides (game teardown). */
export function clearRuntimeSpeedOverrides(): void {
  runtimeSpeedOverrides.clear();
}

export function loadMobilityConfig(): MobilityConfig {
  if (cached) return cached;
  cached = mobilityJson as MobilityConfig;
  return cached;
}

/** Reset cache (tests only). */
export function resetMobilityCacheForTests(): void {
  cached = null;
  runtimeSpeedOverrides.clear();
}

/**
 * Speed for (jersey, action, role). Resolution order:
 *   1. runtime playerData-derived override (P4.1 — SPD-scaled)
 *   2. role × action   — role-specific profile (differentiates body types)
 *   3. jersey × action — legacy per-player override (back-compat)
 *   4. action default  — the baseline table
 *   5. idle fallback
 * Unknown action falls back to idle speed, then 0.1.
 */
export function speedFor(
  jersey: string,
  action: string,
  cfg: MobilityConfig = loadMobilityConfig(),
  role?: MovementRole,
): number {
  const runtime = runtimeSpeedOverrides.get(jersey)?.[action];
  if (typeof runtime === 'number') return runtime;
  if (role) {
    const roleAction = cfg.role_action_speeds?.[role]?.[action];
    if (typeof roleAction === 'number') return roleAction;
  }
  const override = cfg.player_overrides[jersey]?.[action];
  if (typeof override === 'number') return override;
  const def = cfg.default_action_speeds[action];
  if (typeof def === 'number') return def;
  const idle = cfg.default_action_speeds['idle'];
  return typeof idle === 'number' ? idle : 0.1;
}
