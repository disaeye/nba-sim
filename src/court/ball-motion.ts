/**
 * First-class ball motion: held, pass flight, shot flight, loose.
 *
 * ARCHITECTURE INVARIANT (single-point commit): this module advances the
 * ball's POSITION and emits CANDIDATE facts (pass_intercepted, strip,
 * loose_recovered). It NEVER transfers possession. The adjudicator is the
 * sole site that may set the ball to `held` by a new team, and only after a
 * resolve check confirms the steal. The previous implementation returned
 * `ballHeld(interceptor)` from the intercept branch before resolve, which
 * produced a possession-vs-holder desync that froze ~15% of possessions
 * until shot-clock violation. See docs/foundation/architecture.md §1.4.
 */
import type { TeamId } from '../state/types.js';
import type { PoseMap, PoseState, Vec2 } from './poses.js';
import { dist } from './poses.js';
import { loadMobilityConfig, type MobilityConfig } from './mobility.js';
import { distFeet, rimNorm } from './geometry.js';

const PASS_CATCH_RADIUS_FT = 2.5;

export type BallFlightStatus = 'held' | 'pass' | 'shot' | 'loose' | 'inbound' | 'dead';

/** Shot method taxonomy (P1.2) — drives the resolve base-rate per type. */
export type ShotType = 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';

export interface BallMotionState {
  readonly x: number;
  readonly y: number;
  readonly status: BallFlightStatus;
  readonly holderId: string | null;
  readonly fromX: number;
  readonly fromY: number;
  readonly toX: number;
  readonly toY: number;
  readonly flightT: number;
  readonly flightDuration: number;
  readonly receiverId: string | null;
  readonly passerId: string | null;
  readonly shooterId: string | null;
  readonly shotValue: 2 | 3 | null;
  readonly assisterId: string | null;
  readonly team: TeamId | null;
  /** Release zone retained through flight for shot-result adjudication. */
  readonly zone: string | null;
  /** Shot method taxonomy (P1.2): catch_shoot | pull_up | post | drive_finish | other. */
  readonly shotType?: ShotType | null;
  /** realClock at flight start, for watchdog timeout predicates. */
  readonly flightStartReal: number | null;
  /** Shooting team whose miss created this live rebound contest. */
  readonly reboundOffenseTeam?: TeamId | null;
  /** Team allowed to recover a live loose ball, used after turnovers. */
  readonly looseRecoveryTeam?: TeamId | null;
  /** True for an inbound pass — real inbounds are thrown hard and short;
   *  the intercept candidate logic skips them (P5.x). */
  readonly inbound?: boolean;
  /** Pass taxonomy: drive_kick | swing | skip | outlet | normal.
   *  Carried through flight so the completion PASS event can echo it. */
  readonly passType?: string | null;
  /** P5.x: consecutive ticks a defender has been within strip_radius of
   *  the holder. A strip candidate requires sustained contact (≥4 ticks
   *  = 0.4s) — a fresh catch is not instantly picked (NBA control
   *  protection). */
  readonly stripContactTicks?: number;
}

export type BallStepEvent =
  | { readonly kind: 'pass_complete'; readonly receiverId: string }
  | { readonly kind: 'pass_missed'; readonly receiverId: string; readonly x: number; readonly y: number }
  | { readonly kind: 'pass_out_of_bounds'; readonly receiverId: string; readonly x: number; readonly y: number }
  | { readonly kind: 'pass_intercepted'; readonly stealerId: string; readonly victimId: string }
  | { readonly kind: 'shot_arrived'; readonly shooterId: string; readonly shotValue: 2 | 3; readonly shotType?: ShotType | null }
  | { readonly kind: 'strip'; readonly stealerId: string; readonly victimId: string }
  | { readonly kind: 'loose_recovered'; readonly recovererId: string; readonly team: TeamId };

export interface BallStepResult {
  readonly ball: BallMotionState;
  readonly events: readonly BallStepEvent[];
}

function clamp01(n: number): number {
  if (n < 0) return 0;
  if (n > 1) return 1;
  return n;
}

export function ballHeld(
  holder: PoseState,
  team: TeamId,
  reboundOffenseTeam: TeamId | null = null,
  fromPasserId: string | null = null,
): BallMotionState {
  return {
    x: holder.x,
    y: holder.y,
    status: 'held',
    holderId: holder.jersey,
    fromX: holder.x,
    fromY: holder.y,
    toX: holder.x,
    toY: holder.y,
    flightT: 0,
    flightDuration: 0,
    receiverId: null,
    // Retained from the incoming flight: the player who passed to the
    // current holder. The perception layer reads it as lastPasserId to
    // price the "hot potato" return pass (NBA handlers do not whip the
    // ball straight back to the passer they just received from). Cleared
    // by startShot / ballLoose / dead balls.
    passerId: fromPasserId,
    shooterId: null,
    shotValue: null,
    assisterId: null,
    team,
    zone: null,
    reboundOffenseTeam,
    flightStartReal: null,
    looseRecoveryTeam: null,
  };
}
export function ballDead(at: Vec2 = { x: 0.5, y: 0.5 }): BallMotionState {
  return {
    x: at.x,
    y: at.y,
    status: 'dead',
    holderId: null,
    fromX: at.x,
    fromY: at.y,
    toX: at.x,
    toY: at.y,
    flightT: 0,
    flightDuration: 0,
    receiverId: null,
    passerId: null,
    shooterId: null,
    shotValue: null,
    assisterId: null,
    team: null,
    zone: null,
    reboundOffenseTeam: null,
    looseRecoveryTeam: null,
    flightStartReal: null,
  };
}

export function startPass(args: {
  readonly from: PoseState;
  readonly to: PoseState;
  readonly team: TeamId;
  readonly passerId?: string;
  readonly cfg?: MobilityConfig;
  readonly realNow?: number;
  readonly inbound?: boolean;
  readonly passType?: string | null;
}): BallMotionState {
  const cfg = args.cfg ?? loadMobilityConfig();
  const d = dist(args.from.x, args.from.y, args.to.x, args.to.y);
  const duration = Math.max(0.15, d / cfg.ball.pass_speed);
  return {
    x: args.from.x,
    y: args.from.y,
    status: 'pass',
    holderId: null,
    fromX: args.from.x,
    fromY: args.from.y,
    toX: args.to.x,
    toY: args.to.y,
    flightT: 0,
    flightDuration: duration,
    receiverId: args.to.jersey,
    passerId: args.passerId ?? args.from.jersey,
    shooterId: null,
    shotValue: null,
    assisterId: null,
    team: args.team,
    zone: null,
    inbound: args.inbound ?? false,
    flightStartReal: args.realNow ?? null,
    passType: args.passType ?? null,
  };
}

export function startShot(args: {
  readonly from: PoseState;
  readonly rim: Vec2;
  readonly shotValue: 2 | 3;
  readonly team: TeamId;
  readonly flightDuration: number;
  readonly zone: string;
  readonly shotType?: ShotType | null;
  readonly assisterId?: string | null;
  readonly realNow?: number;
}): BallMotionState {
  return {
    x: args.from.x,
    y: args.from.y,
    status: 'shot',
    holderId: null,
    fromX: args.from.x,
    fromY: args.from.y,
    toX: args.rim.x,
    toY: args.rim.y,
    flightT: 0,
    flightDuration: Math.max(0.2, args.flightDuration),
    receiverId: null,
    passerId: null,
    shooterId: args.from.jersey,
    shotValue: args.shotValue,
    assisterId: args.assisterId ?? null,
    team: args.team,
    zone: args.zone,
    shotType: args.shotType ?? 'other',
    flightStartReal: args.realNow ?? null,
  };
}

export function ballLoose(
  at: Vec2,
  reboundOffenseTeam: TeamId | null = null,
  looseRecoveryTeam: TeamId | null = null,
): BallMotionState {
  return {
    x: at.x,
    y: at.y,
    status: 'loose',
    holderId: null,
    fromX: at.x,
    fromY: at.y,
    toX: at.x,
    toY: at.y,
    flightT: 0,
    flightDuration: 0,
    receiverId: null,
    passerId: null,
    shooterId: null,
    shotValue: null,
    assisterId: null,
    team: null,
    zone: null,
    reboundOffenseTeam,
    flightStartReal: null,
    looseRecoveryTeam,
  };
}

/** Point-to-segment distance for mid-pass intercept. */
export function distPointToSegment(
  px: number,
  py: number,
  ax: number,
  ay: number,
  bx: number,
  by: number,
): number {
  const abx = bx - ax;
  const aby = by - ay;
  const apx = px - ax;
  const apy = py - ay;
  const ab2 = abx * abx + aby * aby;
  if (ab2 < 1e-12) return Math.hypot(apx, apy);
  let t = (apx * abx + apy * aby) / ab2;
  if (t < 0) t = 0;
  else if (t > 1) t = 1;
  const cx = ax + abx * t;
  const cy = ay + aby * t;
  return Math.hypot(px - cx, py - cy);
}

function nearestDefender(
  poses: PoseMap,
  offenseTeam: TeamId,
  nearX: number,
  nearY: number,
  radius: number,
): PoseState | null {
  let best: PoseState | null = null;
  let bestD = radius;
  for (const p of Object.values(poses)) {
    if (p.team === offenseTeam) continue;
    const d = dist(p.x, p.y, nearX, nearY);
    if (d <= bestD) {
      bestD = d;
      best = p;
    }
  }
  return best;
}

/**
 * Advance ball by dt. Emits CANDIDATE facts (intercept/strip/loose) but does
 * NOT transfer possession — the adjudicator confirms steals after a resolve
 * draw. On a failed steal the ball continues along its path; the defender may
 * re-trigger a candidate next tick. This keeps possession truth and ball
 * truth in lockstep (single-point commit invariant).
 */
export function stepBall(args: {
  readonly ball: BallMotionState;
  readonly poses: PoseMap;
  readonly dt: number;
  readonly cfg?: MobilityConfig;
  /** When true and a defender is in range, emit intercept/strip candidate via events. */
  readonly allowIntercept?: boolean;
}): BallStepResult {
  const { ball, poses, dt } = args;
  const cfg = args.cfg ?? loadMobilityConfig();
  const allow = args.allowIntercept !== false;
  const events: BallStepEvent[] = [];

  if (ball.status === 'held' && ball.holderId) {
    const holder = poses[ball.holderId];
    if (!holder) return { ball, events };
    const next: BallMotionState = {
      ...ball,
      x: holder.x,
      y: holder.y,
      fromX: holder.x,
      fromY: holder.y,
      toX: holder.x,
      toY: holder.y,
    };
    if (allow && ball.team) {
      const stripper = nearestDefender(
        poses,
        ball.team,
        holder.x,
        holder.y,
        cfg.ball.strip_radius,
      );
      // P5.x: sustained contact required — a defender brushing the holder
      // for one tick is not a strip attempt. The counter resets when the
      // defender leaves the radius.
      const contactTicks = (ball.stripContactTicks ?? 0) + 1;
      if (stripper && contactTicks >= 8) {
        // CANDIDATE only — do not move the ball. Adjudicate resolves.
        events.push({
          kind: 'strip',
          stealerId: stripper.jersey,
          victimId: holder.jersey,
        });
      }
      return {
        ball: { ...next, stripContactTicks: stripper ? contactTicks : 0 },
        events,
      };
    }
    return { ball: next, events };
  }

  if (ball.status === 'pass') {
    const flightT = ball.flightT + dt;
    const u = ball.flightDuration <= 0 ? 1 : Math.min(1, flightT / ball.flightDuration);
    const x = clamp01(ball.fromX + (ball.toX - ball.fromX) * u);
    const y = clamp01(ball.fromY + (ball.toY - ball.fromY) * u);
    const mid: BallMotionState = { ...ball, x, y, flightT };

    if (allow && ball.team && !ball.inbound) {
      for (const p of Object.values(poses)) {
        if (p.team === ball.team) continue;
        const d = distPointToSegment(p.x, p.y, ball.fromX, ball.fromY, ball.toX, ball.toY);
        // Only intercept if near the *current* ball position on the path.
        if (d <= cfg.ball.intercept_radius && dist(p.x, p.y, x, y) <= cfg.ball.intercept_radius * 1.5) {
          // CANDIDATE only — return the in-flight ball, NOT ballHeld(p).
          // Adjudicate confirms the steal via resolveSteal; on failure the
          // pass continues and this defender may re-candidate next tick.
          events.push({
            kind: 'pass_intercepted',
            stealerId: p.jersey,
            victimId: ball.receiverId ?? '?',
          });
          return { ball: mid, events };
        }
      }
    }

    if (u >= 1 && ball.receiverId) {
      const recv = poses[ball.receiverId];
      if (recv) {
        const receiverDistanceFt = distFeet(recv.x, recv.y, ball.toX, ball.toY);
        if (receiverDistanceFt <= PASS_CATCH_RADIUS_FT) {
          events.push({ kind: 'pass_complete', receiverId: ball.receiverId });
          return { ball: ballHeld(recv, ball.team ?? recv.team), events };
        }
        events.push({ kind: 'pass_missed', receiverId: ball.receiverId, x: ball.toX, y: ball.toY });
      } else {
        events.push({ kind: 'pass_out_of_bounds', receiverId: ball.receiverId, x: ball.toX, y: ball.toY });
      }
      return { ball: ballLoose({ x: ball.toX, y: ball.toY }, null, ball.team === 'home' ? 'away' : 'home'), events };
    }
    return { ball: mid, events };
  }

  if (ball.status === 'shot') {
    const flightT = ball.flightT + dt;
    const u = ball.flightDuration <= 0 ? 1 : Math.min(1, flightT / ball.flightDuration);
    // Arc: lift y slightly mid-flight for presentation truth in snapshot.
    const arc = 0.08 * Math.sin(Math.PI * u);
    const x = clamp01(ball.fromX + (ball.toX - ball.fromX) * u);
    const y = clamp01(ball.fromY + (ball.toY - ball.fromY) * u + arc);
    const mid: BallMotionState = { ...ball, x, y, flightT };
    if (u >= 1 && ball.shooterId && ball.shotValue) {
      events.push({ kind: 'shot_arrived', shooterId: ball.shooterId, shotValue: ball.shotValue, shotType: ball.shotType });
      return {
        ball: ballLoose({ x: ball.toX, y: ball.toY }, ball.team),
        events,
      };
    }
    return { ball: mid, events };
  }

  if (ball.status === 'loose') {
    // Loose ball pickup is NOT gated by allowIntercept — it should be
    // checked EVERY tick. The allowIntercept flag controls defensive
    // intercept/strip attempts only. Without this fix, loose balls sat
    // unrecovered for 9+ seconds with players standing 2ft away because
    // pickup only evaluated on the 1-in-5 intercept ticks.
    {
      let best: PoseState | null = null;
      let bestD = cfg.ball.loose_pickup_radius;
      for (const p of Object.values(poses)) {
        if (ball.looseRecoveryTeam && p.team !== ball.looseRecoveryTeam) continue;
        const d = dist(p.x, p.y, ball.x, ball.y);
        if (d <= bestD) {
          bestD = d;
          best = p;
        }
      }
      if (best) {
        events.push({
          kind: 'loose_recovered',
          recovererId: best.jersey,
          team: best.team,
        });
        return { ball: ballHeld(best, best.team, ball.reboundOffenseTeam ?? null), events };
      }
    }
    return { ball, events };
  }

  // inbound / dead: static unless held inbounder pose exists
  if (ball.status === 'inbound' && ball.holderId) {
    const h = poses[ball.holderId];
    if (h) {
      return {
        ball: { ...ball, x: h.x, y: h.y },
        events,
      };
    }
  }

  return { ball, events };
}

export function rimForAttack(attackRight: boolean): Vec2 {
  return rimNorm(attackRight ? 'right' : 'left');
}
