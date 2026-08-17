/**
 * Continuous player pose state with velocity-based physics.
 *
 * The model is now a steering controller: each player has a position
 * (x, y) and a velocity (vx, vy) in court-normalized units per second.
 * Every tick the controller steers toward the target by accelerating
 * along the direction-to-target vector, decelerating when close, and
 * applying drag. This replaces the old linear-interpolation model where
 * players moved at constant speed and stopped instantly on arrival.
 *
 * Key behaviours this produces:
 *  - Smooth acceleration from standstill, not instant full-speed lurch
 *  - Deceleration approaching the target, not a hard snap
 *  - Momentum on direction change — a cutting player takes a beat to turn
 *  - Idle jitter at the target — players never fully freeze; they maintain
 *    a subtle organic movement simulating stance adjustments, ball-watching,
 *    and weight shifting
 */
import type { TeamId } from '../state/types.js';
import { loadMobilityConfig, speedFor, isValidMovementRole, type MobilityConfig, type MovementRole } from './mobility.js';
import { clampFrameDisplacement, MAX_FRAME_FTPS } from './displacement.js';

export interface Vec2 {
  readonly x: number;
  readonly y: number;
}

export interface PoseState {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  /** Instantaneous velocity in court-normalized units per second. */
  readonly vx: number;
  readonly vy: number;
  readonly targetX: number;
  readonly targetY: number;
  readonly action: string;
  readonly hasBall: boolean;
  readonly arrived: boolean;
  /** Movement role selecting the speed profile; undefined falls back to defaults. */
  readonly movementRole?: MovementRole;
}

export type PoseMap = Readonly<Record<string, PoseState>>;

export function clamp01(n: number): number {
  if (n < 0) return 0;
  if (n > 1) return 1;
  return n;
}

export function dist(ax: number, ay: number, bx: number, by: number): number {
  return Math.hypot(bx - ax, by - ay);
}

// ─── deterministic per-player jitter source ────────────────────────────────
/** Deterministic pseudo-random in [-1, 1) keyed by jersey + tick seed. */
function jitter(jersey: string, seed: number): number {
  const n = jersey.charCodeAt(0) * 31 + seed * 17 + 7;
  return ((n * 9301 + 49297) % 233280) / 116640 - 1;
}

export function makePose(args: {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  readonly targetX?: number;
  readonly targetY?: number;
  readonly action?: string;
  readonly hasBall?: boolean;
  readonly movementRole?: MovementRole;
}): PoseState {
  const tx = args.targetX ?? args.x;
  const ty = args.targetY ?? args.y;
  const eps = loadMobilityConfig().arrival_epsilon;
  const arrived = dist(args.x, args.y, tx, ty) <= eps;
  return {
    jersey: args.jersey,
    team: args.team,
    x: clamp01(args.x),
    y: clamp01(args.y),
    vx: 0,
    vy: 0,
    targetX: clamp01(tx),
    targetY: clamp01(ty),
    action: args.action ?? 'idle',
    hasBall: args.hasBall === true,
    arrived,
    ...(isValidMovementRole(args.movementRole) ? { movementRole: args.movementRole } : {}),
  };
}

export function setTarget(
  pose: PoseState,
  targetX: number,
  targetY: number,
  action: string,
  movementRole?: MovementRole,
): PoseState {
  const eps = loadMobilityConfig().arrival_epsilon;
  const tx = clamp01(targetX);
  const ty = clamp01(targetY);
  const arrived = dist(pose.x, pose.y, tx, ty) <= eps;
  return {
    ...pose,
    targetX: tx,
    targetY: ty,
    action,
    arrived,
    ...(movementRole !== undefined && isValidMovementRole(movementRole) ? { movementRole } : {}),
  };
}

function pressureFactor(pose: PoseState, poses: PoseMap, cfg: MobilityConfig): number {
  let nearest = Infinity;
  for (const other of Object.values(poses)) {
    if (other.team === pose.team) continue;
    nearest = Math.min(nearest, Math.hypot((other.x - pose.x) * 94, (other.y - pose.y) * 50));
  }
  if (!Number.isFinite(nearest) || nearest >= cfg.physical.pressure_radius_ft) return 1;
  const ratio = Math.max(0, nearest / cfg.physical.pressure_radius_ft);
  // Ball handler under tight pressure (defender ≤3ft) is further slowed:
  // a real player dribbling through contact moves at ~40-50% speed, not
  // 70%. Without this the handler drove 17ft in 1s through a defender
  // at 2ft — physically impossible. The 0.5 floor for handlers vs 0.7
  // for off-ball players reflects the dribble constraint.
  const floor = pose.hasBall ? Math.max(0.55, cfg.physical.pressure_speed_floor - 0.15) : cfg.physical.pressure_speed_floor;
  return floor + (1 - floor) * ratio;
}

export function resolveBodyContacts(poses: PoseMap, cfg: MobilityConfig): PoseMap {
  const next: Record<string, PoseState> = { ...poses };
  const minDistance = cfg.physical.body_radius_ft * 2;
  // Velocity damping applied along the contact axis: a mover that just got
  // pushed out of a body must not re-penetrate on the next tick. The old
  // resolver only corrected positions, so a 20ft/s cutter against a
  // stationary defender alternated "penetrate → push out" every tick —
  // the measured 0.1s displacement summed motion + push (30–33ft/s) and
  // bodies visibly merged.
  const pushVelRetain = 0.15;
  for (let iteration = 0; iteration < cfg.physical.contact_iterations; iteration += 1) {
    const rows = Object.values(next);
    for (let a = 0; a < rows.length; a += 1) {
      for (let b = a + 1; b < rows.length; b += 1) {
        // Gauss-Seidel: read the CURRENT pushed positions (next), so pushes
        // from earlier pairs accumulate instead of overwriting each other.
        const first = next[rows[a]!.jersey]!;
        const second = next[rows[b]!.jersey]!;
        const dx = (second.x - first.x) * 94;
        const dy = (second.y - first.y) * 50;
        const distance = Math.hypot(dx, dy);
        if (distance >= minDistance) continue;
        const length = distance || 1;
        const push = minDistance - distance;
        const ux = distance > 0 ? dx / length : 1;
        const uy = distance > 0 ? dy / length : 0;
        const firstHolder = first.hasBall;
        const secondHolder = second.hasBall;
        // Ball handler gets pushed at 30% (not 0%): the old code let the
        // handler phase through defenders like a ghost, advancing 17ft in
        // 2s through a defender at 2-4ft. A real handler driving into a
        // stationary defender is slowed by contact — they can beat the
        // defender with angle/speed but cannot walk through their torso.
        // The defender takes 70% of the push (the handler is stronger
        // through contact), the handler takes 30%.
        const firstPush = firstHolder ? push * 0.3 : secondHolder ? push * 0.7 : push / 2;
        const secondPush = secondHolder ? push * 0.3 : firstHolder ? push * 0.7 : push / 2;
        // A shooter is planted through the gather/wind-up. Contact may move
        // the defender, but it must not let the release point drift on every
        // collision iteration.
        const adjustedFirstPush = first.action === 'shoot' ? firstPush * 0.15 : firstPush;
        const adjustedSecondPush = second.action === 'shoot' ? secondPush * 0.15 : secondPush;
        // Normalized-space separation direction (court units) for velocity
        // damping — velocity lives in court units, not ft-scaled space.
        const ndx = second.x - first.x;
        const ndy = second.y - first.y;
        const ndist = Math.hypot(ndx, ndy);
        const nux = ndist > 1e-12 ? ndx / ndist : 1;
        const nuy = ndist > 1e-12 ? ndy / ndist : 0;
        if (adjustedFirstPush > 0) {
          const into = first.vx * nux + first.vy * nuy;
          // P5.x: cap the per-frame push — multiple overlapping pairs and
          // Gauss-Seidel iterations can otherwise accumulate a 3ft+ single
          // frame jump (measured 28-31ft/s teleport frames when a deny
          // defender sat on a moving attacker). 1.5ft is the max legal
          // separation impulse per 0.1s tick.
          const capped = Math.min(adjustedFirstPush, 1.5);
          next[first.jersey] = {
            ...first,
            x: clamp01(first.x - (capped * ux) / 94),
            y: clamp01(first.y - (capped * uy) / 50),
            vx: into > 0 ? first.vx - into * (1 - pushVelRetain) * nux : first.vx,
            vy: into > 0 ? first.vy - into * (1 - pushVelRetain) * nuy : first.vy,
          };
        }
        if (adjustedSecondPush > 0) {
          const into = second.vx * nux + second.vy * nuy;
          const capped = Math.min(adjustedSecondPush, 1.5);
          next[second.jersey] = {
            ...second,
            x: clamp01(second.x + (capped * ux) / 94),
            y: clamp01(second.y + (capped * uy) / 50),
            vx: into < 0 ? second.vx - into * (1 - pushVelRetain) * nux : second.vx,
            vy: into < 0 ? second.vy - into * (1 - pushVelRetain) * nuy : second.vy,
          };
        }
      }
    }
  }
  return next;
}

/**
 * Steering controller: accelerate toward target, decelerate when close,
 * apply drag, and maintain idle jitter at the target.
 *
 * The acceleration and drag constants are derived from the max speed for
 * the player's current action. This makes different actions *feel*
 * different: a drive accelerates hard, a space jog settles gently, a
 * screen-setter shuffles with short controlled steps.
 */
export function stepPose(
  pose: PoseState,
  dt: number,
  cfg: MobilityConfig = loadMobilityConfig(),
  poses: PoseMap = { [pose.jersey]: pose },
  speedScale = 1,
): PoseState {
  if (dt <= 0) return pose;

  const maxSpeed = speedFor(pose.jersey, pose.action, cfg, pose.movementRole) * pressureFactor(pose, poses, cfg) * speedScale;
  const eps = cfg.arrival_epsilon;

  // Direction to target
  const dx = pose.targetX - pose.x;
  const dy = pose.targetY - pose.y;
  const d = Math.hypot(dx, dy);
  const isArrived = d <= eps;

  // ── Idle jitter: even when arrived, players maintain organic micro-movement ──
  if (isArrived) {
    // Speed of jitter depends on action — ball handlers and defenders
    // jitter more actively than spacers standing in the corner.
    const jitterMag = pose.action === 'shoot'
      ? 0  // a shooter in their windup is planted — no jitter, no drift
      : pose.hasBall
      ? 0.04  // dribbling in place — visible weight shifts and ball control motion
      : pose.action === 'on_ball_defend'
        ? 0.0010  // defensive stance — active feet
        : 0.0004; // spacer — mostly still, occasional weight shift

    // Time-varying seed: use the fractional fine-structure of position
    // (which drifts with jitter) so the jitter does not lock to a static
    // offset. Without this a frozen handler's tickSeed is constant and the
    // jitter is identical every frame — the handler looks glued to the floor.
    const tickSeed = pose.x * 10000 + pose.y * 10000 + pose.vx * 100000 + pose.vy * 100000;
    const jx = jitter(pose.jersey, tickSeed) * jitterMag;
    const jy = jitter(pose.jersey, tickSeed + 1) * jitterMag;

    // Apply strong drag to kill residual velocity, then add jitter
    const drag = 0.5;
    const nvx = pose.vx * drag + jx;
    const nvy = pose.vy * drag + jy;
    return {
      ...pose,
      x: clamp01(pose.x + nvx * dt),
      y: clamp01(pose.y + nvy * dt),
      vx: nvx,
      vy: nvy,
      arrived: true,
    };
  }

  // ── Steering: accelerate toward target ────────────────────────────────
  // Acceleration scales with max speed — faster actions also accelerate faster.
  // A player reaches ~63% of max speed in 1/accel time.
  const accel = maxSpeed * 6.0;   // reaches ~63% max in ~0.17s
  const decel = maxSpeed * 10.0;  // stronger deceleration for controlled stops

  // Normalized direction
  const dirX = dx / d;
  const dirY = dy / d;

  // Current speed along the target direction
  const speedAlong = pose.vx * dirX + pose.vy * dirY;
  // Current speed perpendicular to target direction (to be damped)
  const perpX = -dirY;
  const perpY = dirX;
  const speedPerp = pose.vx * perpX + pose.vy * perpY;

  // Distance at which we start decelerating (braking distance)
  // v² = 2·a·d  →  d = v² / (2·a)
  const brakeDist = (speedAlong * speedAlong) / (2 * decel);

  let ax: number, ay: number;
  if (d < brakeDist) {
    // Decelerate — we're close enough that we need to slow down to stop
    ax = -dirX * decel;
    ay = -dirY * decel;
  } else {
    // Accelerate toward target
    ax = dirX * accel;
    ay = dirY * accel;
  }


  // Apply acceleration
  let nvx = pose.vx + ax * dt;
  let nvy = pose.vy + ay * dt;

  // Clamp to max speed
  const nSpeed = Math.hypot(nvx, nvy);
  if (nSpeed > maxSpeed && nSpeed > 0) {
    nvx = (nvx / nSpeed) * maxSpeed;
    nvy = (nvy / nSpeed) * maxSpeed;
  }

  // If very close and moving slow, snap to target to avoid oscillation —
  // but the snap must never teleport: approach at maxSpeed instead of
  // jumping up to eps*1.5 (2.8ft) in one 0.1s tick (measured 30-31ft/s
  // arrival frames when the containment target moved mid-approach).
  let nx = pose.x + nvx * dt;
  let ny = pose.y + nvy * dt;
  if (d < eps * 1.5 && Math.hypot(nvx, nvy) < maxSpeed * 0.15) {
    const arriveMove = maxSpeed * dt;
    if (d > arriveMove) {
      nx = pose.x + dirX * arriveMove;
      ny = pose.y + dirY * arriveMove;
    } else {
      nx = pose.targetX;
      ny = pose.targetY;
    }
    nvx *= 0.3;
    nvy *= 0.3;
  }

  return {
    ...pose,
    x: clamp01(nx),
    y: clamp01(ny),
    vx: nvx,
    vy: nvy,
    arrived: false,
  };
}

export function stepAllPoses(
  poses: PoseMap,
  dt: number,
  cfg: MobilityConfig = loadMobilityConfig(),
  speedScale = 1,
): PoseMap {
  const moved: Record<string, PoseState> = {};
  for (const pose of Object.values(poses)) moved[pose.jersey] = stepPose(pose, dt, cfg, poses, speedScale);
  let resolved = resolveBodyContacts(moved, cfg);
  // Per-frame displacement guard. The previous inline clamp measured the
  // normalized delta with a single 94 ft scale (`hypot(dx, dy)` vs
  // `(25*dt)/94`), which mis-judged y-axis motion (50 ft scale) and let the
  // collision resolver ship 28-33 ft/s teleport frames on diagonals.
  // `clampFrameDisplacement` applies the correct anisotropic real-feet bound
  // (x·94, y·50) and is the single source of truth.
  if (dt > 0) {
    const next: Record<string, PoseState> = {};
    // Collision correction may add an impulse beyond the action's intended
    // speed. During dead-ball choreography the whole frame budget must scale
    // down too, otherwise a lineup walk can still expose an 8–16ft/s sprint
    // when a player is pushed out of a lane-space overlap.
    const frameMaxFtps = MAX_FRAME_FTPS * Math.max(0, Math.min(1, speedScale));
    for (const [jersey, pose] of Object.entries(resolved)) {
      const original = poses[jersey]!;
      const c = clampFrameDisplacement(original.x, original.y, pose.x, pose.y, dt, frameMaxFtps);
      next[jersey] = (c.x === pose.x && c.y === pose.y)
        ? pose
        : { ...pose, x: c.x, y: c.y };
    }
    resolved = next;
  }
  return resolved;
}

export function posesFromLineups(
  home: readonly string[],
  away: readonly string[],
  homeSpots: readonly Vec2[],
  awaySpots: readonly Vec2[],
): PoseMap {
  const out: Record<string, PoseState> = {};
  home.forEach((j, i) => {
    const s = homeSpots[i] ?? { x: 0.25, y: 0.2 + i * 0.15 };
    out[j] = makePose({ jersey: j, team: 'home', x: s.x, y: s.y, action: 'idle' });
  });
  away.forEach((j, i) => {
    const s = awaySpots[i] ?? { x: 0.75, y: 0.2 + i * 0.15 };
    out[j] = makePose({ jersey: j, team: 'away', x: s.x, y: s.y, action: 'idle' });
  });
  return out;
}
