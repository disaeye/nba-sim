/**
 * WorldSnapshot — authoritative continuous world state at realClock.
 * Position truth lives here, not on events.
 */
import type { Phase, TeamId } from '../state/types.js';
import type { PoseMap } from '../court/poses.js';
import type { BallFlightStatus, BallMotionState } from '../court/ball-motion.js';

export interface SnapshotPlayer {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  readonly zone: string;
  readonly action: string;
  readonly hasBall: boolean;
  readonly arrived: boolean;
  /** Current stamina 0..100 (§8). */
  readonly stm: number;
  /** Stamina ceiling (STM_max). */
  readonly stmMax: number;
}

export interface SnapshotBall {
  readonly x: number;
  readonly y: number;
  readonly status: BallFlightStatus;
  readonly holderId: string | null;
}
export interface TacticalRoutePoint {
  readonly x: number;
  readonly y: number;
}

/** Action-specific waypoints in normalized court coordinates. */
export interface TacticalRouteSnapshot {
  readonly kind: string;
  readonly points: readonly TacticalRoutePoint[];
}

export interface TacticalAssignmentSnapshot {
  readonly jersey: string;
  readonly role: string;
  readonly action: string;
  readonly targetJersey: string | null;
  readonly lane?: string;
  /** Explicit offensive path; the first point follows the current pose. */
  readonly route?: TacticalRouteSnapshot;
}
export interface ScreenDefenseSnapshot {
  readonly mode: 'DROP' | 'SWITCH' | 'BLITZ' | 'HEDGE' | 'ICE';
  readonly onBallDefender: string | null;
  readonly screenerDefender: string | null;
  readonly switchesAtUse: boolean;
  readonly zone: 'MAN' | 'ZONE_2_3';
}


export interface TacticalSnapshot {
  readonly kind: string;
  readonly systemId?: string;
  readonly stage: string;
  readonly offense: TeamId;
  readonly handler: string;
  /** Designed feed target for OFF_BALL_SCREEN / POST_UP second actions. */
  readonly feedTargetJersey?: string | null;
  readonly assignments: readonly TacticalAssignmentSnapshot[];
  readonly screenDefense?: ScreenDefenseSnapshot;
  readonly activeAction?: {
    readonly jersey: string;
    readonly kind: string;
    readonly stage: string;
    readonly windupSeconds: number;
    readonly recoverySeconds: number;
    readonly elapsedSeconds: number;
    readonly targetZone?: string;
  };
}

export interface WorldSnapshot {
  readonly t_real: number;
  readonly t_game: number;
  readonly shotClock: number;
  readonly period: number;
  readonly phase: Phase;
  readonly score: { readonly home: number; readonly away: number };
  readonly ball: SnapshotBall;
  readonly players: readonly SnapshotPlayer[];
  readonly tactical?: TacticalSnapshot;
  /** Seq of semantic event emitted on this tick, if any. */
  readonly lastEventSeq: number | null;
  readonly lastEventType: string | null;
  /** ALL semantic event seqs emitted on this tick, in emission order. Same-tick
   *  event pairs (SHOT_RESULT→MADE_BASKET_DEAD, REBOUND→LOOSE_BALL_RECOVER)
   *  previously collapsed to the rank-last event in spectator frames — the
   *  stream consumer could never see the first semantic member of the pair. */
  readonly tickEventSeqs: readonly number[];
}

function positionZone(x: number, y: number): string {
  if (x < 0.16 || x > 0.84) return 'rim';
  if (x < 0.30 || x > 0.70) return 'paint';
  // NBA corner three strip: y within 19ft of the sideline (the corner
  // arc joins at 22ft from baseline, 3ft from sideline; the strip is
  // 19ft of y-offset). The old 0.24/0.76 (13ft) boundary mislabeled
  // corner threes at y-offset 14-19ft as wing shots (measured: 9 corner
  // 3PA/game hidden in the wing bucket).
  if (y < 0.12 || y > 0.88) return 'corner';
  if (y < 0.34 || y > 0.66) return 'wing';
  return 'top';
}

export function snapshotFromState(args: {
  readonly t_real: number;
  readonly t_game: number;
  readonly shotClock: number;
  readonly period: number;
  readonly phase: Phase;
  readonly score: { readonly home: number; readonly away: number };
  readonly poses: PoseMap;
  readonly ball: BallMotionState;
  readonly tactical?: TacticalSnapshot;
  readonly stamina?: Readonly<Record<string, { readonly max: number; stm: number }>>;
  readonly lastEventSeq?: number | null;
  readonly lastEventType?: string | null;
  /** All event seqs emitted this tick (defaults to the single lastEventSeq). */
  readonly tickEventSeqs?: readonly number[];
}): WorldSnapshot {
  const players: SnapshotPlayer[] = [];
  for (const p of Object.values(args.poses)) {
    const st = args.stamina?.[p.jersey];
    players.push({
      jersey: p.jersey,
      team: p.team,
      x: p.x,
      y: p.y,
      action: p.action,
      zone: positionZone(p.x, p.y),
      hasBall: p.hasBall,
      arrived: p.arrived,
      stm: st ? Math.round(st.stm * 10) / 10 : -1,
      stmMax: st?.max ?? -1,
    });
  }

  return {
    t_real: Math.round(args.t_real * 10) / 10,
    t_game: Math.round(args.t_game * 10) / 10,
    shotClock: Math.round(args.shotClock * 10) / 10,
    period: args.period,
    phase: args.phase,
    score: { home: args.score.home, away: args.score.away },
    ball: {
      x: args.ball.x,
      y: args.ball.y,
      status: args.ball.status,
      holderId: args.ball.holderId,
    },
    players,
    tactical: args.tactical,
    lastEventSeq: args.lastEventSeq ?? null,
    lastEventType: args.lastEventType ?? null,
    tickEventSeqs: args.tickEventSeqs ?? (args.lastEventSeq != null ? [args.lastEventSeq] : []),
  };
}
