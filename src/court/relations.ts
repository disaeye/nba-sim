import type { TeamId } from '../state/types.js';
import type { RoleBinding } from '../identity/types.js';
import type { PoseMap } from './poses.js';
import type { BallFlightStatus } from './ball-motion.js';
import type { Baskets, BasketSide, PlayerTask } from './alignment.js';
import { offenseAttacks } from './alignment.js';
import {
  distFeet,
  isFrontcourt,
  nx,
  ny,
  rimNorm,
  zoneFromPoint,
  loadCourtGeometryFt,
} from './geometry.js';
import {
  defenseRelationAt,
  jerseyToOffenseRelation,
  offenseRelationsForPlay,
  type OffenseSlotName,
} from './play-slots.js';

export type RelationKind =
  | 'ball_handler_pocket'
  | 'elbow_screen'
  | 'strong_corner'
  | 'weak_corner'
  | 'strong_wing'
  | 'weak_wing'
  | 'paint_post'
  | 'rim_crash'
  | 'trail'
  | 'advance_lane'
  | 'outlet'
  | 'defend_ball'
  | 'defend_deny'
  | 'defend_help'
  | 'defend_tag'
  | 'defend_weak'
  | 'box_out';

export type StrongSide = 'high_y' | 'low_y';

export interface LiveWorld {
  readonly poses: PoseMap;
  readonly ball: {
    readonly x: number;
    readonly y: number;
    readonly holderId: string | null;
    readonly status: BallFlightStatus;
  };
  readonly baskets: Baskets;
  readonly offense: TeamId;
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly playId: string | null;
  readonly binding: RoleBinding;
  readonly shotClock: number;
  readonly gameClock: number;
  /** Offense coach profile — pace identity shapes the advance stride. */
  readonly offenseCoach?: { readonly paceBias?: number } | null;
}

export interface SlotRegion {
  readonly cx: number;
  readonly cy: number;
  readonly epsFt: number;
}

export interface ResolvedTarget {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  readonly task: PlayerTask;
  readonly hasBall: boolean;
  readonly slot: RelationKind;
  readonly satisfied: boolean;
  readonly zone: ReturnType<typeof zoneFromPoint>;
  readonly movementRole?: import('../court/mobility.js').MovementRole;
}

const COURT_MARGIN = 0.02;

function clamp01(n: number): number {
  if (n < 0) return 0;
  if (n > 1) return 1;
  return n;
}

function clampCourt(n: number): number {
  if (n < COURT_MARGIN) return COURT_MARGIN;
  if (n > 1 - COURT_MARGIN) return 1 - COURT_MARGIN;
  return n;
}

export function deriveStrongSide(ballY: number): StrongSide {
  return ballY >= 0.5 ? 'high_y' : 'low_y';
}

export function liveWorldAttack(world: LiveWorld): BasketSide {
  return offenseAttacks(world.offense, world.baskets);
}

function mirrorY(y: number, strong: StrongSide, wantStrong: boolean): number {
  const high = wantStrong ? strong === 'high_y' : strong === 'low_y';
  if (high) return y >= 0.5 ? y : 1 - y;
  return y < 0.5 ? y : 1 - y;
}

/** Ideal region center for a relation given live ball / attack / strong side.
 *  v0.7.2: SportVU data-driven positioning. Handler position evolves with
 *  shot clock; off-ball positions are handler-relative (polar offsets). */
export function regionFor(
  kind: RelationKind,
  attack: BasketSide,
  ballX: number,
  ballY: number,
  strong: StrongSide,
  shotClock: number = 12,
  paceBias: number = 0.5,
): SlotRegion {
  const G = loadCourtGeometryFt();
  const rim = rimNorm(attack);
  const laneL = nx(G.lane_length_ft);
  const ftX = attack === 'right' ? 1 - laneL : laneL;
  const midX = 0.5;
  const pocketX = attack === 'right' ? ftX - nx(2) : ftX + nx(2);
  const elbowYStrong = 0.5 + ny(G.lane_width_ft) / 2 - ny(1);
  const cornerInset = ny(G.three_point_corner_from_sideline_ft + 0.5);
  const cornerYStrong = 1 - cornerInset;
  const cornerYWeak = cornerInset;
  const wingDist = nx(G.three_point_arc_ft - 1);
  const wingX =
    attack === 'right' ? rim.x - wingDist * 0.55 : rim.x + wingDist * 0.55;
  const wingYStrong = 0.5 + ny(14);
  const wingYWeak = 0.5 - ny(14);
  const cornerX =
    attack === 'right'
      ? rim.x - nx(G.three_point_corner_ft - 2)
      : rim.x + nx(G.three_point_corner_ft - 2);

  switch (kind) {
    case 'ball_handler_pocket': {
      // SportVU Law 1: handler distance from rim = f(shotClock)
      // shot>20s → 17ft, 16-20s → 23ft, 12-16s → 22ft, 8-12s → 20ft, 4-8s → 16ft, <4s → 11ft
      if (isFrontcourt(ballX, attack)) {
        // Interpolate handler distance from rim based on shot clock
        const handlerDistRim = shotClock > 20 ? 17
          : shotClock > 16 ? 17 + (23 - 17) * (shotClock - 20) / (24 - 20)
          : shotClock > 12 ? 23 - (23 - 22) * (16 - shotClock) / 4
          : shotClock > 8  ? 22 - (22 - 20) * (12 - shotClock) / 4
          : shotClock > 4  ? 20 - (20 - 16) * (8 - shotClock) / 4
          : 16 - (16 - 11) * (4 - shotClock) / 4;
        const dir = attack === 'right' ? 1 : -1;
        const cx = clampCourt(rim.x + dir * nx(handlerDistRim));
        const cy = clamp01(ballY * 0.6 + 0.5 * 0.4);
        return { cx, cy, epsFt: 6 };
      }
      return { cx: clampCourt(rim.x + (attack === 'right' ? -nx(17) : nx(17))), cy: 0.5, epsFt: 8 };
    }
    case 'elbow_screen': {
      const ey = strong === 'high_y' ? elbowYStrong : 1 - elbowYStrong;
      return { cx: ftX, cy: ey, epsFt: 5 };
    }
    case 'strong_corner':
      return {
        cx: cornerX,
        cy: strong === 'high_y' ? cornerYStrong : cornerYWeak,
        epsFt: 5,
      };
    case 'weak_corner':
      return {
        cx: cornerX,
        cy: strong === 'high_y' ? cornerYWeak : cornerYStrong,
        epsFt: 5,
      };
    case 'strong_wing':
      return {
        cx: wingX,
        cy: strong === 'high_y' ? wingYStrong : wingYWeak,
        epsFt: 6,
      };
    case 'weak_wing':
      return {
        cx: wingX,
        cy: strong === 'high_y' ? wingYWeak : wingYStrong,
        epsFt: 6,
      };
    case 'paint_post':
      return {
        cx: clampCourt(attack === 'right' ? 1 - nx(8) : nx(8)),
        cy: 0.5,
        epsFt: 4,
      };
    case 'rim_crash':
      return { cx: clampCourt(rim.x), cy: clampCourt(rim.y), epsFt: 4 };
    case 'trail':
      return {
        cx: attack === 'right' ? midX - nx(4) : midX + nx(4),
        cy: 0.5,
        epsFt: 6,
      };
    case 'advance_lane': {
      // SportVU: transition handler covers ~7ft/s, needs 42ft to reach frontcourt.
      // Target is incremental: 8ft ahead of current ball position toward midcourt.
      // Coach pace identity: run-first coaches (paceBias→1) push longer strides
      // into the strike zone; grind coaches (→0) walk it up — the visible
      // grab-and-go vs walk-it-up difference.
      const stride = 8 + (paceBias - 0.5) * 6;
      const ahead =
        attack === 'right'
          ? Math.min(0.72, Math.max(ballX + nx(stride), midX + nx(2)))
          : Math.max(0.28, Math.min(ballX - nx(stride), midX - nx(2)));
      return { cx: ahead, cy: clamp01(ballY * 0.5 + 0.5 * 0.5), epsFt: 10 };
    }
    case 'outlet': {
      const own: BasketSide = attack === 'right' ? 'left' : 'right';
      const ownRim = rimNorm(own);
      return {
        cx: clampCourt(ownRim.x + (attack === 'right' ? 0.12 : -0.12)),
        cy: clampCourt(mirrorY(0.35, strong, true)),
        epsFt: 8,
      };
    }
    case 'defend_ball': {
      // SportVU: on-ball defender at 4ft from handler (confirmed correct).
      const gap = nx(4);
      const dir = attack === 'right' ? 1 : -1;
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY),
        epsFt: 4,
      };
    }
    case 'defend_deny': {
      // SportVU: deny defender ~10ft from ball, shading toward matchup.
      const gap = nx(10);
      const dir = attack === 'right' ? 1 : -1;
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY),
        epsFt: 5,
      };
    }
    case 'defend_help': {
      // SportVU: help defender at 16ft from ball (was 10ft — corrected +60%).
      const gap = nx(16);
      const dir = attack === 'right' ? 1 : -1;
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY),
        epsFt: 6,
      };
    }
    case 'defend_tag': {
      // SportVU: tag defender at 14ft from ball (was 8ft — corrected +75%).
      const gap = nx(14);
      const dir = attack === 'right' ? 1 : -1;
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY),
        epsFt: 5,
      };
    }
    case 'defend_weak': {
      // SportVU: weak-side defender at 22ft from ball (was ~12ft y-offset).
      const gap = nx(22);
      const dir = attack === 'right' ? 1 : -1;
      const yOff = strong === 'high_y' ? -ny(14) : ny(14);
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY + yOff),
        epsFt: 8,
      };
    }
    case 'box_out': {
      const gap = nx(6);
      const dir = attack === 'right' ? 1 : -1;
      return {
        cx: clampCourt(ballX + dir * gap),
        cy: clampCourt(ballY),
        epsFt: 6,
      };
    }
    default:
      return { cx: pocketX, cy: 0.5, epsFt: 8 };
  }
}

export function resolveSlotTarget(
  kind: RelationKind,
  world: LiveWorld,
  jersey: string,
  matchupXY?: { readonly x: number; readonly y: number },
): { readonly x: number; readonly y: number; readonly satisfied: boolean } {
  const attack = liveWorldAttack(world);
  const strong = deriveStrongSide(world.ball.y);
  let region = regionFor(kind, attack, world.ball.x, world.ball.y, strong, world.shotClock, world.offenseCoach?.paceBias ?? 0.5);

  if (
    (kind === 'defend_deny' || kind === 'defend_tag' || kind === 'defend_weak') &&
    matchupXY
  ) {
    const rim = rimNorm(attack);
    const t = 0.35;
    region = {
      cx: clampCourt(matchupXY.x * (1 - t) + world.ball.x * t * 0.5 + rim.x * t * 0.5),
      cy: clampCourt(matchupXY.y * (1 - t) + world.ball.y * t),
      epsFt: region.epsFt,
    };
  }

  if (kind === 'defend_ball' && matchupXY) {
    // Keep the marker on the rim-side of the handler. A defender may shade
    // toward the basket, but the visual tracker must never cross the ball
    // carrier during a coarse 0.1s integration step.
    const dir = attack === 'right' ? 1 : -1;
    const rim = rimNorm(attack);
    const gap = Math.min(nx(4), Math.max(nx(1.5), Math.hypot(rim.x - matchupXY.x, rim.y - matchupXY.y) * 0.45));
    region = {
      cx: clampCourt(matchupXY.x + dir * gap),
      cy: clampCourt(matchupXY.y),
      epsFt: 3,
    };
  }

  const pose = world.poses[jersey];
  const px = pose?.x ?? region.cx;
  const py = pose?.y ?? region.cy;
  const cx = clampCourt(region.cx);
  const cy = clampCourt(region.cy);
  const d = distFeet(px, py, cx, cy);
  if (d <= region.epsFt) {
    return { x: clampCourt(px), y: clampCourt(py), satisfied: true };
  }
  return { x: cx, y: cy, satisfied: false };
}

function taskForOffenseRelation(kind: RelationKind, isBall: boolean): PlayerTask {
  if (isBall) {
    return kind === 'advance_lane' ? 'advance' : 'ball_handler';
  }
  switch (kind) {
    case 'elbow_screen':
      return 'screen';
    case 'rim_crash':
      return 'cut';
    case 'advance_lane':
      return 'advance';
    default:
      return 'space';
  }
}

function movementRoleForOffenseSlot(key: OffenseSlotName, isBall: boolean): import('./mobility.js').MovementRole {
  if (isBall) return 'ball_handler';
  if (key === 'screener') return 'screener';
  return 'spacer';
}

function movementRoleForDefenseSlot(kind: RelationKind): import('./mobility.js').MovementRole {
  if (kind === 'defend_ball') return 'on_ball_defender';
  if (kind === 'defend_weak') return 'weak_side_defender';
  if (kind === 'defend_tag') return 'help_defender';
  return 'perimeter_defender';
}

function taskForDefenseRelation(kind: RelationKind): PlayerTask {
  switch (kind) {
    case 'defend_ball':
      return 'on_ball_defend';
    case 'defend_help':
      return 'help';
    case 'defend_tag':
      return 'tag';
    case 'defend_weak':
      return 'weak_side';
    case 'box_out':
      return 'box_out';
    default:
      return 'deny';
  }
}

const OFFENSE_KEYS: readonly OffenseSlotName[] = [
  'primary_creator',
  'secondary_creator',
  'screener',
  'spacer_strong',
  'spacer_weak',
];

/**
 * Live retarget: role relations resolved in current world.
 * Satisfied poses keep current xy as target (no formation stamp).
 */
export function retargetFromLiveWorld(
  world: LiveWorld,
  homeLineup: readonly string[],
  awayLineup: readonly string[],
  ballHandler: string,
): readonly ResolvedTarget[] {
  const defense: TeamId = world.offense === 'home' ? 'away' : 'home';
  const offLine = world.offense === 'home' ? homeLineup : awayLineup;
  const defLine = defense === 'home' ? homeLineup : awayLineup;
  const handler = offLine.includes(ballHandler)
    ? ballHandler
    : world.binding.primary_creator;
  const relMap = offenseRelationsForPlay(world.playId);
  const out: ResolvedTarget[] = [];

  const offTargets: { jersey: string; x: number; y: number; kind: RelationKind }[] = [];

  for (const key of OFFENSE_KEYS) {
    const jersey = world.binding[key];
    if (!offLine.includes(jersey)) continue;
    let kind = relMap[key];
    const isBall = jersey === handler;
    if (isBall) {
      kind = world.mode === 'TRANSITION' ? 'advance_lane' : 'ball_handler_pocket';
    } else if (
      key === 'primary_creator' &&
      handler !== world.binding.primary_creator
    ) {
      kind = jerseyToOffenseRelation(world.binding, world.playId, jersey) ?? 'weak_wing';
    }
    const resolved = resolveSlotTarget(kind, world, jersey);
    offTargets.push({ jersey, x: resolved.x, y: resolved.y, kind });
    out.push({
      jersey,
      team: world.offense,
      x: resolved.x,
      y: resolved.y,
      task: taskForOffenseRelation(kind, isBall),
      hasBall: isBall,
      slot: kind,
      satisfied: resolved.satisfied,
      zone: zoneFromPoint(resolved.x, resolved.y, world.offense, world.baskets),
      movementRole: movementRoleForOffenseSlot(key, isBall),
    });
  }

  for (const j of offLine) {
    if (out.some((p) => p.jersey === j)) continue;
    const kind: RelationKind = j === handler ? 'ball_handler_pocket' : 'trail';
    const resolved = resolveSlotTarget(kind, world, j);
    offTargets.push({ jersey: j, x: resolved.x, y: resolved.y, kind });
    out.push({
      jersey: j,
      team: world.offense,
      x: resolved.x,
      y: resolved.y,
      task: taskForOffenseRelation(kind, j === handler),
      hasBall: j === handler,
      slot: kind,
      satisfied: resolved.satisfied,
      zone: zoneFromPoint(resolved.x, resolved.y, world.offense, world.baskets),
      movementRole: j === handler ? 'ball_handler' : 'spacer',
    });
  }

  // Defensive identity is resolved from the live poses, with the current
  // holder handled first. The old implementation paired defLine[i] with
  // offTargets[i], which made the first defender guard the creator slot even
  // when a different player had received the ball. That leaves the actual
  // handler behind an unassigned matchup during the opening possession.
  const available = [...defLine];
  const matchupByAttacker = new Map<string, string>();
  const attackerOrder = [handler, ...offLine.filter((j) => j !== handler)];
  for (const attackerJersey of attackerOrder) {
    const attacker = world.poses[attackerJersey];
    let bestIndex = 0;
    let bestDistance = Infinity;
    for (let index = 0; index < available.length; index += 1) {
      const defender = world.poses[available[index]!];
      const distance = attacker && defender
        ? distFeet(attacker.x, attacker.y, defender.x, defender.y)
        : Infinity;
      if (distance < bestDistance) {
        bestDistance = distance;
        bestIndex = index;
      }
    }
    const defender = available.splice(bestIndex, 1)[0];
    if (defender) matchupByAttacker.set(attackerJersey, defender);
  }

  attackerOrder.forEach((attackerJersey, index) => {
    const defenderJersey = matchupByAttacker.get(attackerJersey);
    if (!defenderJersey) return;
    const attacker = world.poses[attackerJersey];
    const target = offTargets.find((candidate) => candidate.jersey === attackerJersey);
    const matchupXY = attacker
      ? { x: attacker.x, y: attacker.y }
      : target
        ? { x: target.x, y: target.y }
        : { x: world.ball.x, y: world.ball.y };
    const kind: RelationKind = index === 0 ? 'defend_ball' : defenseRelationAt(index);
    const resolved = resolveSlotTarget(kind, world, defenderJersey, matchupXY);
    out.push({
      jersey: defenderJersey,
      team: defense,
      x: resolved.x,
      y: resolved.y,
      task: taskForDefenseRelation(kind),
      hasBall: false,
      slot: kind,
      satisfied: resolved.satisfied,
      zone: zoneFromPoint(resolved.x, resolved.y, world.offense, world.baskets),
      movementRole: movementRoleForDefenseSlot(kind),
    });
  });

  return separateTargets(out, world);
}

const SEP_OFFSET = 0.04;

function separateTargets(
  targets: readonly ResolvedTarget[],
  world: LiveWorld,
): readonly ResolvedTarget[] {
  const attack = liveWorldAttack(world);
  const rim = rimNorm(attack);
  const ax = rim.x - world.ball.x;
  const ay = rim.y - world.ball.y;
  const len = Math.hypot(ax, ay) || 1;
  const px = -ay / len;
  const py = ax / len;

  const result = [...targets];

  for (const team of ['home', 'away'] as const) {
    const teamIdx: number[] = [];
    for (let i = 0; i < result.length; i++) {
      if (result[i]!.team === team) teamIdx.push(i);
    }
    const bySlot = new Map<string, number>();
    for (const i of teamIdx) {
      const t = result[i]!;
      const key = t.slot;
      const sIdx = bySlot.get(key) ?? 0;
      bySlot.set(key, sIdx + 1);
      if (sIdx === 0) continue;
      const sign = sIdx % 2 === 0 ? 1 : -1;
      const mag = Math.ceil(sIdx / 2) * SEP_OFFSET;
      const x = clampCourt(t.x + sign * mag * px);
      const y = clampCourt(t.y + sign * mag * py);
      result[i] = {
        ...t,
        x,
        y,
        satisfied: false,
        zone: zoneFromPoint(x, y, world.offense, world.baskets),
      };
    }
    for (let pass = 0; pass < 3; pass++) {
      for (let a = 0; a < teamIdx.length; a++) {
        for (let b = a + 1; b < teamIdx.length; b++) {
          const ia = teamIdx[a]!;
          const ib = teamIdx[b]!;
          const ta = result[ia]!;
          const tb = result[ib]!;
          const dx = tb.x - ta.x;
          const dy = tb.y - ta.y;
          const d01 = Math.hypot(dx, dy);
          const dFt = d01 * 94;
          if (dFt >= 3.5 || d01 < 1e-9) continue;
          const push = (3.5 / 94 - d01) / 2;
          const nx = dx / d01;
          const ny = dy / d01;
          result[ia] = {
            ...ta,
            x: clampCourt(ta.x - push * nx),
            y: clampCourt(ta.y - push * ny),
            satisfied: false,
          };
          result[ib] = {
            ...tb,
            x: clampCourt(tb.x + push * nx),
            y: clampCourt(tb.y + push * ny),
            satisfied: false,
          };
        }
      }
    }
  }

  for (let i = 0; i < result.length; i++) {
    result[i] = {
      ...result[i]!,
      zone: zoneFromPoint(
        result[i]!.x,
        result[i]!.y,
        world.offense,
        world.baskets,
      ),
    };
  }
  return result;
}

export function buildLiveWorld(args: {
  readonly poses: PoseMap;
  readonly ballX: number;
  readonly ballY: number;
  readonly holderId: string | null;
  readonly ballStatus: BallFlightStatus;
  readonly baskets: Baskets;
  readonly offense: TeamId;
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly playId: string | null;
  readonly binding: RoleBinding;
  readonly shotClock: number;
  readonly gameClock: number;
  readonly offenseCoach?: { readonly paceBias?: number } | null;
}): LiveWorld {
  return {
    poses: args.poses,
    ball: {
      x: args.ballX,
      y: args.ballY,
      holderId: args.holderId,
      status: args.ballStatus,
    },
    baskets: args.baskets,
    offense: args.offense,
    mode: args.mode,
    playId: args.playId,
    binding: args.binding,
    shotClock: args.shotClock,
    gameClock: args.gameClock,
    offenseCoach: args.offenseCoach ?? null,
  };
}

export function retargetPayloadFromResolved(
  players: readonly ResolvedTarget[],
): Record<string, unknown> {
  return {
    context: 'live_retarget',
    players: players.map((p) => ({
      jersey: p.jersey,
      team: p.team,
      x: p.x,
      y: p.y,
      zone: p.zone,
      task: p.task,
      hasBall: p.hasBall,
      slot: p.slot,
      satisfied: p.satisfied,
      ...(p.movementRole ? { movementRole: p.movementRole } : {}),
    })),
  };
}
