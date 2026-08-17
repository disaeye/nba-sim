/**
 * Authoritative 10-player court alignment builders.
 * Presentation and kernel both use these positions — no spectator heuristics.
 *
 * Coordinates: full court top-down, x∈[0,1] length, y∈[0,1] width.
 * Home basket at x=0 (left), away basket at x=1 (right) at tip-off.
 * Offense always attacks the opponent basket.
 */
import type { CourtZone, TeamId } from '../state/types.js';
import type { RoleBinding } from '../identity/types.js';
import { makePose, type PoseMap } from './poses.js';
import {
  buildLiveWorld,
  retargetFromLiveWorld,
  type ResolvedTarget,
} from './relations.js';

export type BasketSide = 'left' | 'right';
export type AlignmentContext =
  | 'jump_ball'
  | 'tip_receive'
  | 'transition'
  | 'halfcourt'
  | 'inbound'
  | 'dead';

export type PlayerTask =
  | 'jump'
  | 'circle_spot'
  | 'receive_tip'
  | 'ball_handler'
  | 'advance'
  | 'space'
  | 'screen'
  | 'relocate'
  | 'cut'
  | 'drive'
  | 'triple_threat'
  | 'back_to_basket'
  | 'pivot'
  | 'crossover'
  | 'pump_fake'
  | 'on_ball_defend'
  | 'deny'
  | 'help'
  | 'tag'
  | 'weak_side'
  | 'box_out'
  | 'inbound'
  | 'idle';

export interface CourtPlayer {
  readonly jersey: string;
  readonly x: number;
  readonly y: number;
  readonly zone: CourtZone;
  readonly task: PlayerTask;
  readonly hasBall: boolean;
  readonly team: TeamId;
}

export interface Alignment {
  readonly context: AlignmentContext;
  readonly offense: TeamId | null;
  readonly players: readonly CourtPlayer[];
}

export interface Baskets {
  readonly home: BasketSide;
  readonly away: BasketSide;
}

export const TIP_OFF_BASKETS: Baskets = { home: 'left', away: 'right' };

export function offenseAttacks(offense: TeamId, baskets: Baskets): BasketSide {
  return offense === 'home' ? baskets.away : baskets.home;
}

function resolvedToCourtPlayer(p: ResolvedTarget): CourtPlayer {
  return {
    jersey: p.jersey,
    team: p.team,
    x: p.x,
    y: p.y,
    zone: p.zone,
    task: p.task,
    hasBall: p.hasBall,
  };
}

function lineupPoses(
  homeLineup: readonly string[],
  awayLineup: readonly string[],
  seed: Readonly<Record<string, { readonly x: number; readonly y: number }>> | undefined,
): PoseMap {
  const poses: Record<string, ReturnType<typeof makePose>> = {};
  for (const j of homeLineup) {
    const s = seed?.[j];
    poses[j] = makePose({
      jersey: j,
      team: 'home',
      x: s?.x ?? 0.5,
      y: s?.y ?? 0.5,
    });
  }
  for (const j of awayLineup) {
    const s = seed?.[j];
    poses[j] = makePose({
      jersey: j,
      team: 'away',
      x: s?.x ?? 0.5,
      y: s?.y ?? 0.5,
    });
  }
  return poses;
}

/**
 * @deprecated Test/compat only. Live kernel must use retargetFromLiveWorld / applyLiveRetarget.
 */
export function buildHalfcourtAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly offense: TeamId;
  readonly binding: RoleBinding;
  readonly ballHandler: string;
  readonly baskets: Baskets;
  readonly ballZone?: CourtZone;
  readonly playId?: string | null;
  readonly ballXY?: { readonly x: number; readonly y: number };
  readonly livePoses?: Readonly<Record<string, { readonly x: number; readonly y: number }>>;
}): Alignment {
  const {
    homeLineup,
    awayLineup,
    offense,
    binding,
    ballHandler,
    baskets,
    playId,
    ballXY,
    livePoses,
  } = args;
  const offLine = offense === 'home' ? homeLineup : awayLineup;
  const handler = offLine.includes(ballHandler) ? ballHandler : binding.primary_creator;
  const poses = lineupPoses(homeLineup, awayLineup, livePoses);
  const bx = ballXY?.x ?? poses[handler]?.x ?? 0.5;
  const by = ballXY?.y ?? poses[handler]?.y ?? 0.5;
  const world = buildLiveWorld({
    poses,
    ballX: bx,
    ballY: by,
    holderId: handler,
    ballStatus: 'held',
    baskets,
    offense,
    mode: 'HALFCOURT',
    playId: playId ?? null,
    binding,
    shotClock: 14,
    gameClock: 600,
  });
  const targets = retargetFromLiveWorld(world, homeLineup, awayLineup, handler);
  return {
    context: 'halfcourt',
    offense,
    players: targets.map(resolvedToCourtPlayer),
  };
}

/**
 * @deprecated Test/compat only. Live kernel must use retargetFromLiveWorld / applyLiveRetarget.
 */
export function buildTransitionAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly offense: TeamId;
  readonly ballHandler: string;
  readonly baskets: Baskets;
}): Alignment {
  const { homeLineup, awayLineup, offense, ballHandler, baskets } = args;
  const offLine = offense === 'home' ? [...homeLineup] : [...awayLineup];
  const handler = offLine.includes(ballHandler) ? ballHandler : (offLine[0] ?? ballHandler);
  const orderedOff = [handler, ...offLine.filter((j) => j !== handler)];
  const binding: RoleBinding = {
    primary_creator: orderedOff[0] ?? handler,
    secondary_creator: orderedOff[1] ?? handler,
    screener: orderedOff[2] ?? handler,
    spacer_strong: orderedOff[3] ?? handler,
    spacer_weak: orderedOff[4] ?? handler,
  };
  const poses = lineupPoses(homeLineup, awayLineup, undefined);
  const world = buildLiveWorld({
    poses,
    ballX: 0.5,
    ballY: 0.5,
    holderId: handler,
    ballStatus: 'held',
    baskets,
    offense,
    mode: 'TRANSITION',
    playId: 'transition_push',
    binding,
    shotClock: 14,
    gameClock: 600,
  });
  const targets = retargetFromLiveWorld(world, homeLineup, awayLineup, handler);
  return {
    context: 'transition',
    offense,
    players: targets.map(resolvedToCourtPlayer),
  };
}

export function alignmentToPayload(a: Alignment): Record<string, unknown> {
  return {
    context: a.context,
    offense: a.offense,
    players: a.players.map((p) => ({
      jersey: p.jersey,
      team: p.team,
      x: p.x,
      y: p.y,
      zone: p.zone,
      task: p.task,
      hasBall: p.hasBall,
    })),
  };
}

export function parseAlignmentPayload(payload: Record<string, unknown>): Alignment | null {
  const context = payload.context;
  const playersRaw = payload.players;
  if (typeof context !== 'string' || !Array.isArray(playersRaw)) return null;
  const players: CourtPlayer[] = [];
  for (const row of playersRaw) {
    if (typeof row !== 'object' || row === null) continue;
    const r = row as Record<string, unknown>;
    if (typeof r.jersey !== 'string' || (r.team !== 'home' && r.team !== 'away')) continue;
    if (typeof r.x !== 'number' || typeof r.y !== 'number') continue;
    players.push({
      jersey: r.jersey,
      team: r.team,
      x: r.x,
      y: r.y,
      zone: (typeof r.zone === 'string' ? r.zone : 'frontcourt_center') as CourtZone,
      task: (typeof r.task === 'string' ? r.task : 'idle') as PlayerTask,
      hasBall: r.hasBall === true,
    });
  }
  if (players.length === 0) return null;
  return {
    context: context as AlignmentContext,
    offense: payload.offense === 'home' || payload.offense === 'away' ? payload.offense : null,
    players,
  };
}
