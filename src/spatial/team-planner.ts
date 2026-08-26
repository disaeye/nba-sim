import { loadCourtGeometryFt, nx, ny, zoneFromPoint } from '../court/geometry.js';
import type { PlayerTask } from '../court/alignment.js';
import type { RelationKind } from '../court/relations.js';
import type { Intent } from '../decision/types.js';
import type { LiveCourtSense } from '../perception/live-court.js';
import type { TeamId } from '../state/types.js';
import type { TeamAssignment, TeamPlan, TacticalRoute } from '../tactics/team-plan.js';
import type { TeamPlanKind } from '../tactics/types.js';
import type { MovementRole } from '../court/mobility.js';
import { getFormationSlotPosition } from '../tactics/formations.js';
import { loadDecisionConfig } from '../policy/config.js';
import { coordinateWeaksideMotion } from '../strategy/dual-track-coordinator.js';
import { evaluateDefensiveScheme } from '../strategy/defense-schemes.js';
import { loadResolveConfig, makeResolveContext } from '../resolve/index.js';
/** Rate model for the coverage threat math (open 3PT expected points). */
const RESOLVE = makeResolveContext(loadResolveConfig());
const SPATIAL = loadDecisionConfig().spatial;
const TACTICS_SPACER_DENY_FT = loadDecisionConfig().tactics.spacer_denied_distance_ft;
export interface PlannedPlayer {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  readonly task: PlayerTask;
  readonly slot: RelationKind;
  readonly targetJersey: string | null;
  readonly movementRole?: MovementRole;
}

export interface SpatialPlan {
  readonly players: readonly PlannedPlayer[];
  readonly onBallDefender: string | null;
  /** Current-read routes for the five offensive assignments. */
  readonly routes: Readonly<Record<string, TacticalRoute>>;
}

const COURT_MARGIN = 0.02;
// L4 (reality law): real NBA halfcourt spacing holds ~25ft mean pairwise
// distance at every shot-clock phase (p25 22.1 / med 25.0). A 9ft floor
// let two spacers share one pocket; config owns the value now
// (decision.json spatial.min_team_separation_ft).
const MIN_TEAM_SEPARATION_FT = SPATIAL.min_team_separation_ft ?? 13.5;

function clampCourt(value: number): number {
  return Math.max(COURT_MARGIN, Math.min(1 - COURT_MARGIN, value));
}

/**
 * Transition-retreat anchor for one defender.
 *
 * Real transition defense is a continuous gradient, not a mode switch:
 * the further the ball has advanced toward the offensive rim, the closer
 * the defender may sit to their man. The old implementation used a fixed
 * 0.65 paint / 0.35 man blend while the ball was in the backcourt and
 * snapped to man coverage at half court — defenders were still 15-20ft
 * from their assignment when the offense crossed, so every transition
 * conceded a free look (user report: 攻守转换防守漏勺).
 *
 * Geometry drives the blend here:
 *   - `ballDepthFt` = how far the ball has advanced past the defensive
 *     baseline toward the offensive rim (0 = own baseline, 47 = half
 *     court, 94 = offensive rim).
 *   - `paintWeight` = configured start weight at ballDepth 0, decaying
 *     linearly to 0 by `transition_settle_depth_ft` (ball already in
 *     scoring range → pure man coverage). No step function, no mode
 *     switch, no clock proxy.
 *   - A hard cap keeps the defender within `transition_max_gap_ft` of
 *     their assignment at all times — retreat without abandonment.
 *
 * Pure: same inputs → same point. The policy numbers live in
 * config/decision.json#spatial, never inlined at call sites.
 */
function transitionRetreatPoint(
  attackerX: number,
  attackerY: number,
  defRimX: number,
  defRimY: number,
  ballDepthFt: number,
): { readonly x: number; readonly y: number } {
  // Paint-First Transition Anchor:
  // When transitioning, defenders prioritize securing the central paint/rim corridor first (0.75+ weight)
  // before expanding out to pick up assignments as the ball crosses into the halfcourt.
  // When attack direction is 1 (toward right/x=0.94), defense rim is at left (x=0.06).
  // Anchor point should be in front of defensive rim (towards midcourt x=0.5):
  // if defRimX < 0.5 (left rim), anchor is at x = defRimX + 16ft = 0.23
  // if defRimX > 0.5 (right rim), anchor is at x = defRimX - 16ft = 0.77
  const paintAnchorX = defRimX < 0.5 ? defRimX + 16 / 94.0 : defRimX - 16 / 94.0;
  const paintAnchorY = 0.5;
  const progress = Math.min(1, Math.max(0, (ballDepthFt - 15) / 35.0)); // 0 = backcourt, 1 = frontcourt
  const paintWeight = 0.85 * (1 - progress);
  const manWeight = 1 - paintWeight;
  const ax = paintAnchorX * paintWeight + attackerX * manWeight;
  const ay = paintAnchorY * paintWeight + attackerY * manWeight;
  return {
    x: clampCourt(ax),
    y: clampCourt(ay),
  };
}

function offenseTask(assignment: TeamAssignment): PlayerTask {
  if (assignment.action === 'advance') return 'advance';
  if (assignment.action === 'screen') return 'screen';
  if (assignment.action === 'triple_threat') return 'triple_threat';
  if (assignment.action === 'back_to_basket') return 'back_to_basket';
  if (assignment.action === 'pivot') return 'pivot';
  if (assignment.action === 'crossover') return 'crossover';
  if (assignment.action === 'pump_fake') return 'pump_fake';
  // A screener that pops (relocate) must actually MOVE to the arc.
  if (assignment.action === 'relocate') return 'relocate';
  if (assignment.action === 'cut') return 'cut';
  if (assignment.action === 'drive') return 'drive';
  return assignment.role === 'handler' ? 'ball_handler' : 'space';
}
function movementRoleForOffense(assignment: TeamAssignment): MovementRole {
  if (assignment.role === 'handler') return 'ball_handler';
  if (assignment.role === 'screener') return 'screener';
  // P7: a cutter/interior_finisher's rim routes use the screener burst
  // profile (hard rim runs), not the spacer shuffle.
  if (assignment.offenseRole === 'cutter' || assignment.offenseRole === 'interior_finisher') {
    return 'screener';
  }
  return 'spacer';
}

function movementRoleForDefense(task: PlayerTask, slot: RelationKind): MovementRole {
  if (task === 'on_ball_defend' || slot === 'defend_ball') return 'on_ball_defender';
  if (task === 'weak_side' || slot === 'defend_weak') return 'weak_side_defender';
  if (task === 'tag' || slot === 'defend_tag') return 'help_defender';
  // deny/help defenders guarding the perimeter use the perimeter profile.
  return 'perimeter_defender';
}

type RoutePoint = { readonly x: number; readonly y: number };

function routePoint(x: number, y: number): RoutePoint {
  return { x: clampCourt(x), y: clampCourt(y) };
}

function routeForOffense(
  assignment: TeamAssignment,
  sense: LiveCourtSense,
  planKind: TeamPlanKind,
  current: RoutePoint,
  target: RoutePoint,
  feedTarget: string | null,
): TacticalRoute | undefined {
  const rim = sense.rim;
  const back = rim.x < 0.5 ? 1 : -1;
  const receiver = assignment.targetJersey
    ? sense.offensePlayers.find((player) => player.jersey === assignment.targetJersey)?.pose
    : undefined;
  const receiverPoint = receiver
    ? routePoint(receiver.targetX, receiver.targetY)
    : target;
  const towardRim = (distanceFt: number, y: number): RoutePoint => routePoint(
    rim.x + back * nx(distanceFt),
    y,
  );
  const arcBend = routePoint(
    (current.x + target.x) / 2 + back * nx(3),
    (current.y + target.y) / 2,
  );

  switch (assignment.action) {
    case 'pass':
      return { kind: 'pass_lane', points: [current, receiverPoint] };
    case 'handoff':
      return { kind: 'handoff_lane', points: [current, receiverPoint] };
    case 'advance': {
      const midpoint = routePoint(
        (current.x + target.x) / 2,
        current.y + (0.5 - current.y) * 0.35,
      );
      return { kind: 'advance_lane', points: [current, midpoint, target] };
    }
    case 'drive': {
      const laneEntry = towardRim(12, current.y + (rim.y - current.y) * 0.45);
      return { kind: 'drive_lane', points: [current, laneEntry, target] };
    }
    case 'crossover': {
      const crossSide = current.y >= 0.5 ? -1 : 1;
      const crossover = routePoint(
        current.x + (rim.x - current.x) * 0.35,
        current.y + crossSide * ny(4),
      );
      return { kind: 'crossover', points: [current, crossover, target] };
    }
    case 'back_to_basket':
      return { kind: 'post_seal', points: [current, target] };
    case 'screen':
      return { kind: planKind === 'OFF_BALL_SCREEN' ? 'pin_down' : 'screen_angle', points: [current, target] };
    case 'cut': {
      const kind = planKind === 'OFF_BALL_SCREEN' && assignment.jersey === feedTarget
        ? 'curl'
        : assignment.role === 'screener' && (planKind === 'PNR_ROLL' || planKind === 'PNR_POP')
          ? 'roll'
          : 'cut';
      const bend = towardRim(kind === 'roll' ? 10 : 14, current.y + (rim.y - current.y) * 0.5);
      return { kind, points: [current, bend, target] };
    }
    case 'relocate': {
      const kind = planKind === 'PNR_POP' && assignment.role === 'screener'
        ? 'pop'
        : planKind === 'OFF_BALL_SCREEN' && assignment.role === 'screener'
          ? 'pin_down'
          : planKind === 'POST_UP' && assignment.jersey === feedTarget
            ? 'seal'
            : 'relocate';
      return { kind, points: [current, arcBend, target] };
    }
    case 'space': {
      if (planKind === 'POST_UP' && assignment.jersey === feedTarget) {
        return { kind: 'seal', points: [current, target] };
      }
      return { kind: 'spacing', points: [current, arcBend, target] };
    }
    case 'spot_up':
      return { kind: 'spacing', points: [current, target] };
    case 'flare':
      return { kind: 'relocate', points: [current, arcBend, target] };
    case 'fill':
      return { kind: 'relocate', points: [current, arcBend, target] };
    case 'flash':
      return { kind: 'cut', points: [current, target] };
    case 'seal':
      return { kind: 'post_seal', points: [current, target] };
    default:
      // Holding, shooting, and handling beats are intentionally stationary;
      // a route would suggest movement the kernel does not authorize.
      return undefined;
  }
}

function routesForOffense(
  assignments: readonly TeamAssignment[],
  sense: LiveCourtSense,
  players: readonly PlannedPlayer[],
  planKind: TeamPlanKind,
  feedTarget: string | null,
): Readonly<Record<string, TacticalRoute>> {
  const planned = new Map(players.filter((player) => player.team === sense.offense).map((player) => [player.jersey, player]));
  const routes: Record<string, TacticalRoute> = {};
  for (const assignment of assignments) {
    const target = planned.get(assignment.jersey);
    const current = sense.offensePlayers.find((player) => player.jersey === assignment.jersey)?.pose;
    if (!target || !current) continue;
    const route = routeForOffense(
      assignment,
      sense,
      planKind,
      routePoint(current.x, current.y),
      routePoint(target.x, target.y),
      feedTarget,
    );
    if (route) routes[assignment.jersey] = route;
  }
  return routes;
}

function slotForAssignment(assignment: TeamAssignment): RelationKind {
  if (assignment.role === 'handler') {
    return assignment.action === 'advance' ? 'advance_lane' : assignment.action === 'drive' ? 'paint_post' : 'ball_handler_pocket';
  }
  if (assignment.role === 'screener') return assignment.action === 'screen' ? 'elbow_screen' : assignment.action === 'cut' ? 'rim_crash' : 'strong_wing';
  if (assignment.role === 'strong_corner') return 'strong_corner';
  if (assignment.role === 'weak_corner') return 'weak_corner';
  return 'weak_wing';
}

function desiredOffenseTarget(
  assignment: TeamAssignment,
  sense: LiveCourtSense,
  plan: TeamPlan,
  matchups: Readonly<Record<string, string>>,
  feedTarget: string | null = null,
): { readonly x: number; readonly y: number } {
  const planKind = plan.kind;
  // toward mid-court — it is FIXED by which basket is attacked, never by
  // the handler's current x. This prevents all spacing targets from
  // flipping to the wrong half when a driver crosses the rim line.
  const back = sense.rim.x < 0.5 ? 1 : -1;
  const handler = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose;
  const rim = sense.rim;

  // If we are in initial formation setup (stage is ADVANCE or SET, and not transition push),
  // use the distinctive formation geometry setup slots
  if ((plan.stage === 'ADVANCE' || plan.stage === 'SET') && planKind !== 'TRANSITION_PUSH' && plan.formation) {
    const formationTarget = getFormationSlotPosition(plan.formation, assignment.role, rim, back);
    if (formationTarget) {
      return formationTarget;
    }
  }

  const handlerInBackcourt = handler ? (sense.rim.x > 0.5 ? handler.x < 0.5 : handler.x > 0.5) : false;
  if (planKind === 'TRANSITION_PUSH' && handlerInBackcourt && assignment.role !== 'handler') {
    if (assignment.role === 'screener') {
      // Rim Runner: runs directly to the rim slot ahead of the ball to pin the deep protector.
      return { x: clampCourt(rim.x + back * nx(5.5)), y: clampCourt(0.5 + ((handler?.y ?? 0.5) > 0.5 ? -0.06 : 0.06)) };
    }
    const laneSide = assignment.role === 'strong_corner' ? 1 : -1;
    const side = sense.ball.y >= 0.5 ? laneSide : -laneSide;
    if (assignment.role === 'slot') {
      // Early Offense Trailer / Drag Screener: trails ball at top of key (28ft) for transition pullup or quick drag screen.
      return { x: clampCourt(rim.x + back * nx(28.0)), y: 0.5 };
    }
    // Wing Fillers: sprint wide down the sidelines into deep corners to stretch the break defense horizontally.
    return {
      x: clampCourt(rim.x + back * nx(23.5)),
      y: clampCourt(side > 0 ? 0.90 : 0.10),
    };
  }
  // ── P7 role-based off-ball routing (behavior identity) ───────────────
  // TeamRole remains the geometric slot; offenseRole owns the behavioral
  // destination. Public corner roles must stay on a corner-three locus even
  // when the player is a cutter/interior finisher in another possession.
  const offenseRole = assignment.offenseRole;
  const role = offenseRole ?? (assignment.role === 'screener'
    ? planKind === 'POST_UP' ? 'post_scorer' : 'interior_finisher'
    : planKind === 'TRANSITION_PUSH' ? 'transition_runner' : 'floor_spacer');
  // L4 (reality law): an NBA corner shooter stands IN the corner — 2-3ft
  // from the sideline, 1-2ft from the baseline plane of the rim, 22ft from
  // the basket (SportVU: real set-offense mean pairwise spacing 25ft vs
  // the sim's 15-18ft). The old corner solved the arc equation with
  // y only 5ft from the sideline, pushing x 10ft upcourt — a "corner" at
  // the foul-line extended. Park the corner at the true baseline corner:
  // y 3ft from the sideline, x 2ft beyond the rim along the baseline.
  // L4/semantic fix: corner SIDES are sticky per player, not derived from the
  // live ball. Ball-relative sides flipped every time the handler probed
  // across mid-lane — corners crossed 30ft repeatedly and the set never
  // settled (organizedShare 0; real sets hold their corners). The strong
  // corner takes the side the PLAYER already occupies; the weak corner the
  // other. Only a genuine possession start (no history) falls back to the
  // ball side.
  const cornerYFor = (side: 'strong' | 'weak'): number => {
    const current = sense.offensePlayers.find((p) => p.jersey === assignment.jersey)?.pose;
    const ownSideHigh = current ? current.y >= 0.5 : sense.ball.y >= 0.5;
    const y = side === 'strong' ? (ownSideHigh ? 0.94 : 0.06) : (ownSideHigh ? 0.06 : 0.94);
    return clampCourt(y);
  };
  const cornerTarget = (side: 'strong' | 'weak'): RoutePoint => {
    const y = cornerYFor(side);
    const x = rim.x + back * nx(2);
    return routePoint(x, y);
  };
  const arcTarget = (side: 'strong' | 'weak'): RoutePoint => {
    const y = cornerYFor(side) === 0.9 ? 0.72 : 0.28;
    const yOffsetFt = Math.abs(y - 0.5) * 50;
    const x = rim.x + back * nx(loadCourtGeometryFt().three_point_arc_ft - 0.5);
    return routePoint(x, yOffsetFt > 0 ? y : 0.5);
  };
  // A public corner assignment is a corner responsibility first. Only a
  // declared movement/cut route may leave the corner, and even then the
  // target is the named action's route rather than a generic spacer pocket.
  if ((assignment.role === 'strong_corner' || assignment.role === 'weak_corner')
    && role !== 'cutter' && role !== 'interior_finisher'
    && assignment.action !== 'cut' && assignment.action !== 'relocate') {
    return cornerTarget(assignment.role === 'strong_corner' ? 'strong' : 'weak');
  }
  // Screener geometry is tactic-specific: an interior_finisher or hub role
  // still has to occupy the screen, roll, or pop target before its generic
  // role destination can apply. Letting the behavior role win here sent a
  // PNR screener directly to the rim/low-post target, leaving the handler
  // twenty feet away while the assignment still claimed `screen`.
  const isScreener = assignment.role === 'screener';
  if (!isScreener && (role === 'cutter' || role === 'interior_finisher')) {
    if (planKind === 'TRANSITION_PUSH' && handlerInBackcourt) {
      return { x: clampCourt(rim.x + back * nx(4)), y: clampCourt(0.5 + ((handler?.y ?? 0.5) > 0.5 ? -0.08 : 0.08)) };
    }
    if (assignment.action === 'cut') {
      return { x: clampCourt(rim.x + back * nx(3)), y: rim.y };
    }
    // L4 (reality law): between rolls/cuts the finisher spaces to the WEAK
    // CORNER (22ft) — real NBA bigs occupy the corner between actions
    // (drop-coverage era five-out). The short corner at 14ft still parked
    // one man inside the arc and held mean-pairwise at ~19ft vs real 25.
    const side = sense.ball.y >= 0.5 ? -1 : 1;
    return {
      x: clampCourt(rim.x + back * nx(2)),
      y: clampCourt(0.5 + side * 0.44),
    };
  }
  // L4 (reality law): real NBA set offenses keep only 0-1 players inside
  // 10ft of the rim (SportVU: 55.6% of set-offense frames have ZERO paint
  // residents; the sim had 2 permanent ones — measured mean pairwise
  // spacing 15-18ft vs real 25ft). The hub is a TRAILER/playmaker, not a
  // nail occupant: hold the top of the arc (24ft, y=0.5) unless actively
  // sealing. The post_scorer keeps a genuine post position at 11ft only
  // while a post play is live (cut/relocate); between post touches he
  // holds the elbow-extended 15ft flash.
  if (!isScreener && role === 'hub') {
    if (assignment.action === 'cut') {
      const sealSide = sense.ball.y >= 0.5 ? 1 : -1;
      return {
        x: clampCourt(rim.x + back * nx(6)),
        y: clampCourt(0.5 + sealSide * ny(5)),
      };
    }
    // L4: break to the 45° weak-side wing — the old y=0.5 trailer stacked
    // three men on the top ray with the handler and slot.
    const weakSide = sense.ball.y >= 0.5 ? -1 : 1;
    return { x: clampCourt(rim.x + back * nx(23)), y: clampCourt(0.5 + weakSide * ny(12)) };
  }
  if (!isScreener && role === 'post_scorer') {
    if (assignment.action === 'cut' || assignment.action === 'relocate') {
      const sealSide = sense.ball.y >= 0.5 ? 1 : -1;
      return {
        x: clampCourt(rim.x + back * nx(assignment.action === 'cut' ? 6 : 9)),
        y: clampCourt(0.5 + sealSide * ny(assignment.action === 'cut' ? 5 : 8)),
      };
    }
    return { x: clampCourt(rim.x + back * nx(15)), y: clampCourt(0.5 + (sense.ball.y >= 0.5 ? -0.12 : 0.12)) };
  }
  if (!isScreener && role === 'movement_shooter' && assignment.action !== 'space') {
    const side = assignment.role === 'strong_corner' ? 'strong' : 'weak';
    if (assignment.action === 'cut') return arcTarget(side);
    if (assignment.action === 'relocate') return arcTarget(side);
  }
  if (role === 'transition_runner' && planKind === 'TRANSITION_PUSH' && handlerInBackcourt) {
    const laneSide = assignment.jersey.charCodeAt(0) % 2 === 0 ? 1 : -1;
    return {
      x: clampCourt(rim.x + back * nx(22)),
      y: clampCourt(0.5 + laneSide * ny(12)),
    };
  }
  if (assignment.role === 'handler') {
    if (assignment.action === 'advance') {
      const toward = sense.rim.x < 0.5 ? -1 : 1;
      const handlerY = handler?.y ?? sense.ball.y;
      const handlerX = handler?.x ?? sense.ball.x;
      // Incremental advance with a frontcourt cap: push 8ft ahead of the
      // ball until 2ft past midcourt, then stop in the 3PT striking zone
      // (0.52–0.72 for right attack, mirrored for left). The old target
      // (ball + 8ft, no cap) ran the handler all the way to the far
      // baseline corner, where the ADVANCE phase could never complete —
      // possessions died on the shot clock at x≈0.99.
      // Coach pace identity: a run-first coach (paceBias→1) pushes longer
      // strides and attacks the strike zone immediately (the grab-and-go
      // look); a grind coach (paceBias→0) takes shorter steps and settles
      // earlier — the walk-it-up look. This is the physical channel that
      // makes paceBias visible in the frame stream.
      const pace = sense.coach?.[sense.offense]?.paceBias ?? 0.5;
      const strideFt = 8 + (pace - 0.5) * 6; // 5ft (grind) .. 11ft (run)
      // L4: the strike cap feeds directly into set spacing — a handler
      // settling at 30+ft drags mean-pairwise down 3-4ft. Real NBA
      // handlers initiate at 24-27ft ( SportVU handler rim-dist median
      // 25.9 at clock 18-12). Cap deeper for run pace, shallower for grind.
      const strikeDeep = 0.56 + pace * 0.08; // 0.56 (run) .. 0.64 was 0.52-0.64
      const x = toward === 1
        ? Math.min(strikeDeep, Math.max(handlerX + nx(strideFt), 0.52))
        : Math.max(1 - strikeDeep, Math.min(handlerX - nx(strideFt), 0.48));
      return {
        x: clampCourt(x),
        y: clampCourt(handlerY + (0.5 - handlerY) * 0.18),
      };
    }
    if (assignment.action === 'drive' || assignment.action === 'crossover') {
      if (plan.stage === 'SCREEN_USE' || plan.stage === 'ADVANTAGE') {
        const screenerJersey = plan.assignments.find((a) => a.role === 'screener')?.jersey;
        const screener = screenerJersey ? sense.offensePlayers.find((p) => p.jersey === screenerJersey)?.pose : undefined;
        if (screener) {
          // Rub-off: attack tightly around the screener's shoulder toward the paint
          const shoulderSide = screener.y > (handler?.y ?? 0.5) ? 1 : -1;
          const rubY = screener.y + shoulderSide * ny(3.5);
          const rubX = screener.x - back * nx(5.0);
          return { x: clampCourt(rubX), y: clampCourt(rubY) };
        }
      }
      return { x: clampCourt(rim.x + back * nx(4)), y: rim.y };
    }
    if (assignment.action === 'back_to_basket') {
      const hy = handler?.y ?? sense.ball.y;
      return { x: clampCourt(rim.x + back * nx(8)), y: clampCourt(hy) };
    }
    if (assignment.action === 'pivot' || assignment.action === 'triple_threat' || assignment.action === 'pump_fake') {
      return { x: clampCourt(handler?.x ?? sense.ball.x), y: clampCourt(handler?.y ?? sense.ball.y) };
    }
    if (assignment.action === 'shoot') {
      return { x: clampCourt(handler?.x ?? sense.ball.x), y: clampCourt(handler?.y ?? sense.ball.y) };
    }
    if (assignment.action === 'relocate') {
      const hy = handler?.y ?? sense.ball.y;
      const distance = planKind === 'ISO' ? 18 : 22;
      const strikeX = clampCourt(rim.x + back * nx(distance));
      const strikeY = hy >= 0.5 ? clampCourt(0.5 + ny(8)) : clampCourt(0.5 - ny(8));
      const probeBucket = Math.floor(sense.gameClock / 2.5);
      const ph = (assignment.jersey.charCodeAt(0) * 131 + probeBucket * 29) % 233280;
      const probeX = ((ph / 233280) - 0.5) * nx(5);
      const probeY = (((ph * 9301 + 49297) % 233280) / 233280 - 0.5) * ny(7);
      return { x: clampCourt(strikeX + probeX), y: clampCourt(strikeY + probeY) };
    }
    // Idle dribble: a handler reading the defense does not freeze. They
    // make small jab steps and weight shifts. For ISO specifically, the
    // handler should also DRIFT TOWARD THE RIM — real ISO players
    // continuously probe toward the basket, forcing the defender to
    // react. The old orbit was stationary (±4ft around a fixed anchor),
    // which looked like a statue vibrating. The new motion combines a
    // Lissajous orbit with a slow rim-ward drift that resets, reading
    // as probing steps toward the basket.
    const hx = handler?.x ?? sense.ball.x;
    const hy = handler?.y ?? sense.ball.y;
    const phase = sense.gameClock * Math.PI;
    const jx = Math.sin(phase) * nx(3);
    const jy = Math.cos(phase * 1.3) * ny(2);
    // Rim-ward probe: drift 2ft toward the rim every ~3s, then reset.
    // This makes the handler visibly press toward the basket.
    const probePhase = (sense.gameClock % 3) / 3;
    const probeDist = Math.sin(probePhase * Math.PI) * 2; // 0→2→0ft over 3s
    const rimDx = (sense.rim.x - hx) * 94;
    const rimDy = (sense.rim.y - hy) * 50;
    const rimLen = Math.hypot(rimDx, rimDy) || 1;
    const probeX = (rimDx / rimLen) * probeDist / 94;
    const probeY = (rimDy / rimLen) * probeDist / 50;
    return { x: clampCourt(hx + jx + probeX), y: clampCourt(hy + jy + probeY) };
  }
  if (assignment.role === 'screener') {
    // ── per-tactic screener targets ────────────────────────────────────
    // PNR_POP: screener pops to the three-point line after setting screen.
    // Keep the pop on the handler's side of the floor. The old fixed
    // lower-wing target made a top-side high pick-and-pop cross the entire
    // lane (e.g. handler y=.26, pop target y=.70), so the popper was still
    // in the paint when the drive read finished. A pop is a same-side
    // separation action; use the current screener side only as the fallback
    // when the handler is exactly on the center line.
    if (planKind === 'PNR_POP' && (assignment.action === 'relocate' || assignment.action === 'space')) {
      const geometry = loadCourtGeometryFt();
      const cur = sense.offensePlayers.find((p) => p.jersey === assignment.jersey)?.pose;
      const popSide = (handler?.y ?? cur?.y ?? 0.5) >= 0.5 ? 1 : -1;
      const popYOff = 10;
      const popY = 0.5 + popSide * ny(popYOff);
      const popX = clampCourt(rim.x + back * nx(Math.sqrt(Math.max(4, geometry.three_point_arc_ft ** 2 - popYOff ** 2)) + 0.5));
      if (cur) return { x: clampCourt(cur.x + (popX - cur.x) * 0.35), y: clampCourt(cur.y + (popY - cur.y) * 0.35) };
      return { x: popX, y: popY };
    }
    // POST_UP: screener seals at the low post block
    if (planKind === 'POST_UP' && (assignment.action === 'relocate' || assignment.action === 'hold')) {
      // Low post position: 6ft from rim, slightly off-center
      return { x: clampCourt(rim.x + back * nx(6)), y: clampCourt(0.5 - ny(6)) };
    }
    // OFF_BALL_SCREEN: the screen is a PIN-DOWN for the shooter to be freed —
    // the body goes between the strong-corner shooter and HIS defender, not
    // between the handler and the ball defender (the traced possession showed
    // ball-screen geometry freeing nobody: 54 physical picks, 0 cutter shots).
    if (planKind === 'OFF_BALL_SCREEN' && (assignment.action === 'relocate' || assignment.action === 'screen')) {
      // Pin the FEED TARGET's defender (the designated curler, from the plan) —
      // not "whoever is relocating" (the round-5 heuristic pinned the handler
      // half the time; measured 153 pins, 0 curls).
      const feedJersey = feedTarget ?? null;
      const target = feedJersey !== null ? sense.offensePlayers.find((p) => p.jersey === feedJersey) ?? null : null;
      if (target) {
        let bestD: typeof sense.defensePlayers[number] | null = null;
        let bestDd = Infinity;
        for (const d of sense.defensePlayers) {
          const dd = Math.hypot((d.pose.x - target.pose.x) * 94, (d.pose.y - target.pose.y) * 50);
          if (dd < bestDd) { bestDd = dd; bestD = d; }
        }
        if (bestD && bestDd < 12) {
          // Stand on the shooter's defender's outside shoulder, 1.5ft toward
          // the shooter — the classic pin angle.
          const dx = (target.pose.x - bestD.pose.x) * 94;
          const dy = (target.pose.y - bestD.pose.y) * 50;
          const len = Math.hypot(dx, dy) || 1;
          return {
            x: clampCourt(bestD.pose.x + (dx / len) * 1.5 / 94),
            y: clampCourt(bestD.pose.y + (dy / len) * 1.5 / 50),
          };
        }
      }
    }
    // The handler target is handled above; this branch is reserved for the
    // screener's defender-facing approach geometry.
    // A screener in 'relocate' or 'screen' (approaching the screen point)
    // stands BETWEEN the handler and the on-ball defender — that is the
    // physical definition of a screen. Once the screen is set, the target
    // must remain at the recorded anchor until SCREEN_USE; otherwise the
    // live on-ball defender can move toward the handler and pull the target
    // across the floor, making a committed screen chase a moving defender.
    if ((assignment.action === 'relocate' || assignment.action === 'screen') && handler) {
      // Semantic fix (eternal-approach bug): a real screener PICKS A SPOT
      // and plants — the target must not be recomputed from the live
      // defender every tick (the moving point made screeners wander 8-20ft
      // from the handler for entire possessions; the set never formed).
      // If the plan carries a locked anchor, walk to it and STAND THERE.
      if (plan.screenAnchor) {
        return { x: clampCourt(plan.screenAnchor.x), y: clampCourt(plan.screenAnchor.y) };
      }
      const obd = sense.defensePlayers.find((player) => player.jersey === sense.onBallDefender);
      const obdDist = obd ? Math.hypot((obd.pose.x - handler.x) * 94, (obd.pose.y - handler.y) * 50) : Infinity;
      if (obd && obdDist <= 10) {
        // Defender engaged: screen body between handler and defender.
        const dxFt = (obd.pose.x - handler.x) * 94;
        const dyFt = (obd.pose.y - handler.y) * 50;
        const distance = Math.hypot(dxFt, dyFt) || 1;
        const screenDepthFt = Math.max(distance - 1.2, 1.8);
        const side = Math.sign(handler.y - obd.pose.y) || 1;
        const perpX = -(dyFt / distance);
        const perpY = (dxFt / distance);
        const sideFt = distance < 4 ? 1.2 : 0;
        return {
          x: clampCourt(handler.x + (dxFt / distance) * screenDepthFt / 94 + perpX * sideFt / 94),
          y: clampCourt(handler.y + (dyFt / distance) * screenDepthFt / 50 + perpY * side * sideFt / 50),
        };
      }
      // Fallback: defender far or absent — approach the handler's side.
      return { x: clampCourt(handler.x + back * nx(1)), y: clampCourt(handler.y + ny(2) * (handler.y >= 0.5 ? 1 : -1)) };
    }
    // cut = roll to the rim
    if (assignment.action === 'cut') return { x: clampCourt(rim.x + back * nx(3)), y: rim.y };
    const geometry = loadCourtGeometryFt();
    const dYOff = 10;
    return {
      x: clampCourt(rim.x + back * nx(Math.sqrt(Math.max(4, geometry.three_point_arc_ft ** 2 - dYOff ** 2)) + 0.5)),
      y: clampCourt(0.5 + ny(dYOff)),
    };
  }
  const current = sense.offensePlayers.find((player) => player.jersey === assignment.jersey)?.pose;
  // Corner escape is baseline-bound: the corner's y-offset is clamped to
  // its own corner band (0.02..0.34 for the bottom corner, 0.66..0.98 for
  // the top). A defender pressing a corner spacer pushes them ALONG the
  // baseline (toward the wing or the corner), never across the lane —
  // the old free-form escapeY carried both corners toward the middle and
  // collapsed the set (space/space pairs <5ft).
  const clampCornerY = (baseY: number, escapeOffset: number): number => {
    const cornerBand: [number, number] = baseY < 0.5 ? [0.02, 0.34] : [0.66, 0.98];
    return clampCourt(Math.min(cornerBand[1], Math.max(cornerBand[0], baseY + escapeOffset)));
  };
  // Organic off-ball motion: a spacer standing frozen in the corner reads as
  // a statue. Real spacing relocates on ball movement, so each pocket gets a
  // small deterministic drift that changes every ~2.5s (bucketed by game
  // clock + jersey). Amplitude keeps them inside their zone; the drift is
  // absolute (not a chase) so players always arrive.
  // Small organic drift only: ±3ft keeps the spacer a live catch target
  // without wandering out of their spot. The old ±10ft y-drift turned a
  // corner spacer into a rover that spent most of the possession in
  // mid-range — the "everyone floats in the middle" look.
  const driftBucket = Math.floor(sense.gameClock / 2.5);
  const hash = (assignment.jersey.charCodeAt(0) * 31 + driftBucket * 17) % 233280;
  const driftA = (hash / 233280) - 0.5;
  const driftB = ((hash * 9301 + 49297) % 233280) / 233280 - 0.5;
  const driftY = driftA * ny(3);
  const driftX = driftB * nx(3);
  // Drive-lane clearing: when the handler is ATTACKING (drive action inside
  // 18ft), a spacer whose standing spot lies in the drive path (within 6ft of
  // the handler→rim segment) slides OFF the lane — to the perpendicular side,
  // keeping arc depth. Without this, 47% of drive-side teammates walked INTO
  // the path (measured), dragging their defenders into free help position and
  // clogging the finish. Offenders: rim-relative relocate/drift targets that
  // never read the live drive.
  const math_hypot = (a: number, b: number): number => Math.sqrt(a * a + b * b);
  const handlerPose = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose;
  const laneClear = (spot: { x: number; y: number }): { x: number; y: number } => {
    if (!handlerPose) return spot;
    const attacking = handlerPose.action === 'drive'
      || (handlerPose.action === 'ball_handler'
        && math_hypot(handlerPose.x - rim.x, handlerPose.y - rim.y) < 0.19);
    if (!attacking) return spot;
    const ax = handlerPose.x * 94, ay = handlerPose.y * 50;
    const bx = rim.x * 94, by = rim.y * 50;
    const px = spot.x * 94, py = spot.y * 50;
    const dx = bx - ax, dy = by - ay;
    const L2 = dx * dx + dy * dy;
    if (L2 < 1) return spot;
    const t = Math.max(0, Math.min(1, ((px - ax) * dx + (py - ay) * dy) / L2));
    if (t < 0.1) return spot; // behind the ball — not in the path
    const perp = Math.abs((px - ax) * dy - (py - ay) * dx) / Math.sqrt(L2);
    if (perp > 6) return spot; // already off the lane
    // push to the perpendicular side that's AWAY from the rim-line midpoint
    const side = ((px - ax) * dy - (py - ay) * dx) >= 0 ? 1 : -1;
    const ux = -dy / Math.sqrt(L2), uy = dx / Math.sqrt(L2);
    return { x: clampCourt((px + side * ux * 8) / 94), y: clampCourt((py + side * uy * 8) / 50) };
  };
  // Both side variants are computed and the one FARTHER from the ball wins
  // (ties → the standard strong-side spot). This is what keeps a wing PNR
  // from collapsing four players within 10ft of a parked handler — the
  // pockets are rim-relative (22ft), and the handler's strike pocket sits
  // at the same radius, so without this guard the strong-corner/slot spots
  // land 5-12ft from the ball.
  const ballDistFt = (x: number, y: number): number =>
    Math.hypot((x - sense.ball.x) * 94, (y - sense.ball.y) * 50);
  const fartherFromBall = (
    a: { readonly x: number; readonly y: number },
    b: { readonly x: number; readonly y: number },
  ): { readonly x: number; readonly y: number } => (ballDistFt(a.x, a.y) >= ballDistFt(b.x, b.y) ? a : b);
  const escapeX = 0;
  const escapeY = 0;
  // L4: true corner band (3ft from sideline) — see cornerYFor note above.
  // L4/semantic: sticky side — see cornerYFor. The spacer pocket's strong/
  // weak bands must not flip with the live ball either (same 30ft-crossing
  // bug as the corner target).
  const pocketPose = sense.offensePlayers.find((p) => p.jersey === assignment.jersey)?.pose;
  const pocketSideHigh = pocketPose ? pocketPose.y >= 0.5 : sense.ball.y >= 0.5;
  const strongY = pocketSideHigh ? 0.94 : 0.06;
  const weakY = strongY === 0.94 ? 0.06 : 0.94;
  // OFF_BALL_SCREEN curls off the pin to the wing at SCREEN_USE; POST_UP
  // seals at the low block. These replace the generic pocket for that ONE
  // player (measured before: pins formed but the freed man never curled;
  // 153 pins → 0 shots; POST 57 possessions → 0 entries).
  if (feedTarget === assignment.jersey) {
    if (planKind === 'OFF_BALL_SCREEN') {
      const curlX = clampCourt(rim.x + back * nx(22));
      const curlY = clampCourt(sense.ball.y >= 0.5 ? 0.5 + ny(10) : 0.5 - ny(10));
      const cur = sense.offensePlayers.find((p) => p.jersey === assignment.jersey)?.pose;
      if (cur) return { x: clampCourt(cur.x + (curlX - cur.x) * 0.5), y: clampCourt(cur.y + (curlY - cur.y) * 0.5) };
      return { x: curlX, y: curlY };
    }
    if (planKind === 'POST_UP') {
      // Seal on the low block, ball-side: 6ft from rim toward the handler's side.
      const hy = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose.y ?? 0.5;
      return { x: clampCourt(rim.x + back * nx(6)), y: clampCourt(0.5 + (hy >= 0.5 ? ny(5) : -ny(5))) };
    }
  }
  // ── Defensive-read relocation (L4 semantic subsystem) ─────────────────
  // Real off-ball players READ their defender and relocate when denied:
  // a corner pressed tight flashes to the wing (stay on your side), a
  // wing run off the line drifts to the corner. This is a READ, not a
  // constant — it fires only when the nearest defender is inside the deny
  // distance, and the escape stays on the player's own side.
  const nearestDefToFt = (x: number, y: number): number => Math.min(
    ...sense.defensePlayers.map((d) => Math.hypot((d.pose.x - x) * 94, (d.pose.y - y) * 50)),
  );
  const defensiveReadEscape = (spot: { x: number; y: number }, ownSideHigh: boolean): { x: number; y: number } | null => {
    if (nearestDefToFt(spot.x, spot.y) >= TACTICS_SPACER_DENY_FT + 0.5) return null;
    // Denied: slide 6-8ft ALONG the arc away from the defender, staying on
    // the same side (a real flash, not a floor-crossing).
    const def = sense.defensePlayers.reduce((best, d) => {
      const dd = Math.hypot((d.pose.x - spot.x) * 94, (d.pose.y - spot.y) * 50);
      const bd = Math.hypot((best.pose.x - spot.x) * 94, (best.pose.y - spot.y) * 50);
      return dd < bd ? d : best;
    });
    const awayY = def.pose.y >= spot.y ? spot.y - ny(7) : spot.y + ny(7);
    const clampedY = ownSideHigh ? Math.max(0.55, awayY) : Math.min(0.45, awayY);
    const yOffFt = Math.abs(clampedY - 0.5) * 50;
    const x = rim.x + back * nx(Math.sqrt(Math.max(4, loadCourtGeometryFt().three_point_arc_ft ** 2 - yOffFt ** 2)) + 0.5);
    return { x: clampCourt(x), y: clampCourt(clampedY) };
  };

  // ── Weak-side dual-track secondary tactical targets ───────────────────
  if (plan.weaksideAction) {
    const ws = plan.weaksideAction;
    if (ws.screenerId === assignment.jersey && ws.screenerTarget) {
      return { x: clampCourt(ws.screenerTarget.x), y: clampCourt(ws.screenerTarget.y) };
    }
    if (ws.cutterId === assignment.jersey && ws.cutterTarget) {
      return { x: clampCourt(ws.cutterTarget.x), y: clampCourt(ws.cutterTarget.y) };
    }
  }
  if (assignment.role === 'strong_corner') {
    // OFF_BALL_SCREEN: spacer curls off the screen to an open wing for a catch-and-shoot 3
    if (planKind === 'OFF_BALL_SCREEN' && assignment.action === 'cut') {
      const wingX = clampCourt(rim.x + back * nx(23));
      const wingY = clampCourt(sense.ball.y >= 0.5 ? 0.5 + ny(12) : 0.5 - ny(12));
      return { x: wingX, y: wingY };
    }
    // Strong corner HOLDS its side — flipping strong/weak every decision
    // (whichever is farther from the ball) made the spacer shuttle across
    // the floor and never settle into the corner. Only swap when the ball
    // is literally on top of the spot.
    // A ball parked in the strong corner (handler probing baseline) must
    // NOT flip the corner to the weak side: that stacks two corners on
    // one side (measured 24.6% of live frames). Real corners slide ALONG
    // the baseline toward the wing when the ball occupies their spot —
    // same side, 8-12ft up the line — staying a one-pass-away catch
    // target without collapsing the set.
    // The corner pocket sits ON the corner-three locus (22ft rim distance),
    // NOT at a fixed x-offset: the old rim + back*nx(22.5) with the corner
    // y-band (±20ft) composed to a 30ft rim distance — a wing-sideline spot
    // where corner threes were geometrically unreachable (0 corner 3PA).
    const cornerX = (yBandFt: number): number =>
      clampCourt(rim.x + back * nx(Math.sqrt(Math.max(9, loadCourtGeometryFt().three_point_corner_ft ** 2 - yBandFt ** 2)) - 2) + escapeX + driftX);
    const strong = { x: cornerX(Math.abs(strongY - 0.5) * 50), y: clampCornerY(strongY, escapeY + driftY) };
    const weak = { x: cornerX(Math.abs(weakY - 0.5) * 50), y: clampCornerY(weakY, escapeY + driftY) };
    if (ballDistFt(strong.x, strong.y) < 8) {
      // Slide up the baseline toward the wing (away from the ball along
      // the corner's own side): y moves 8ft toward the mid-line while
      // x holds the corner depth. Never cross to the weak side.
      const slideY = strongY < 0.5 ? strongY + ny(8) : strongY - ny(8);
      const slide = { x: cornerX(Math.abs(slideY - 0.5) * 50), y: clampCornerY(slideY, driftY * 0.5) };
      return laneClear(slide);
    }
    // Defensive-read relocation: a pressed corner flashes up the arc on
    // its own side (real NBA deny reads).
    const denied = defensiveReadEscape(strong, strongY >= 0.5);
    if (denied) return clampToArc(denied.x, denied.y, rim.x, rim.y);
    return clampToArc(laneClear(strong).x, laneClear(strong).y, rim.x, rim.y);
  }

  if (assignment.role === 'weak_corner') {
    const cornerX2 = (yBandFt: number): number =>
      clampCourt(rim.x + back * nx(Math.sqrt(Math.max(9, loadCourtGeometryFt().three_point_corner_ft ** 2 - yBandFt ** 2)) - 2) + escapeX + driftX);
    const strong = { x: cornerX2(Math.abs(strongY - 0.5) * 50), y: clampCornerY(strongY, escapeY + driftY) };
    const weak = { x: cornerX2(Math.abs(weakY - 0.5) * 50), y: clampCornerY(weakY, escapeY + driftY) };
    if (ballDistFt(weak.x, weak.y) < 8) {
      // Same-side baseline slide for the weak corner when the ball
      // probes its side.
      const slideY = weakY < 0.5 ? weakY + ny(8) : weakY - ny(8);
      const slide = { x: cornerX2(Math.abs(slideY - 0.5) * 50), y: clampCornerY(slideY, driftY * 0.5) };
      const cleared = laneClear(slide);
      return clampToArc(cleared.x, cleared.y, rim.x, rim.y);
    }
    // Defensive-read relocation for the weak corner too.
    const denied2 = defensiveReadEscape(weak, weakY >= 0.5);
    if (denied2) return clampToArc(denied2.x, denied2.y, rim.x, rim.y);
    const cleared2 = laneClear(weak);
    return clampToArc(cleared2.x, cleared2.y, rim.x, rim.y);
  }
  // Slot stays weak-side but rejects a closeout as a consequence of pressure.
  // Wing slots sit ON the 3pt arc: the arc is a 23.75ft circle, so the
  // x-offset must shrink as the y-offset grows (a flat nx(24) at y=0.8
  // put the player 6-8ft BEYOND the arc — the 31ft heaves). Stand 0.5ft
  // outside the arc at the player's actual y.
  const arcStrong = (yOffFt: number) => Math.sqrt(Math.max(4, loadCourtGeometryFt().three_point_arc_ft ** 2 - yOffFt ** 2));
  // Slot y-offset uses 0.5 (not 0.8) to stay ~12ft from the corner
  // spacer. The old 0.8 put the slot at y=0.18 when weakY=0.10 —
  // only 4ft from the weak corner, producing 20% of offensive pairs
  // within 8ft (NBA ~2-4%). The 0.5 factor puts the slot at y=0.30,
  // 10ft from the corner at y=0.10.
  const strongYOff = Math.abs((weakY - 0.5) * 0.5) * 50;
  const weakYOff = strongYOff;
  // The slot's y-escape is weak-side bound, mirroring the corner clamp:
  // the raw escapeY (defender-relative vertical push) can drag the slot
  // across the mid-line into the strong corner's band — the measured
  // space/space clogs were slot+strong_corner pairs 2-4ft apart while
  // the weak side stood empty. The slot's band is the middle third
  // (0.30..0.70); escaping stays inside it so the set keeps its shape.
  const clampSlotY = (baseY: number, escapeOffset: number): number =>
    clampCourt(Math.min(0.70, Math.max(0.30, baseY + escapeOffset)));
  const slotStrong = { x: clampCourt(rim.x + back * nx(arcStrong(strongYOff) + 0.5) + escapeX + driftX), y: clampSlotY(0.5 + (weakY - 0.5) * 0.5, escapeY + driftY * 0.6) };
  const slotWeak = { x: clampCourt(rim.x + back * nx(arcStrong(weakYOff) + 0.5) + escapeX + driftX), y: clampSlotY(0.5 - (weakY - 0.5) * 0.5, escapeY + driftY * 0.6) };
  // P3.x: a genuine mid-range spacing variant — real sets keep one
  // player at the short corner / elbow (~1/8 of the time) for the
  // short-roll and dribble-handoff game. The rest of the time the slot
  // holds the wing line; NBA mid-range shot share is ~8-12%, not 25%.
  // L4 (reality law): the slot's side must be STABLE — fartherFromBall
  // flips the pocket every time the ball crosses mid-lane, so the set
  // ended up with two corners + top + ONE wing + a post (measured angle
  // histogram: +30..+120 rays empty; mean pairwise 18ft vs real 25).
  // Real five-out fills BOTH 45° wings: the slot takes the weak-side wing
  // (opposite the ball) and stays there for the possession.
  const weakWingY = weakY === 0.06 ? 0.32 : 0.68;
  const slotSide = { x: clampCourt(rim.x + back * nx(arcStrong(Math.abs(weakWingY - 0.5) * 50) + 0.5) + escapeX + driftX), y: clampSlotY(weakWingY, escapeY * 0.3) };
  const s = fartherFromBall(slotSide, slotWeak); const clearedSlot = laneClear(s); return clampToArc(clearedSlot.x, clearedSlot.y, rim.x, rim.y);
}
// Clamp a spacer spot to the arc ring: ON the line (22-24ft), never deep
// inside (the old clamp only capped the far side — inside-arc "wings" at
// 12-19ft collapsed the set). L4 reality law; see basketball_laws.json.
function clampToArc(x: number, y: number, rimX: number, rimY: number): { x: number; y: number } {
  const dx = (x - rimX) * 94;
  const dy = (y - rimY) * 50;
  const dist = Math.hypot(dx, dy);
  const corner = Math.abs(y - 0.5) * 50 > 19;
  const maxDist = (corner ? loadCourtGeometryFt().three_point_corner_ft : loadCourtGeometryFt().three_point_arc_ft) + 0.5;
  if (dist <= maxDist && dist >= 21) return { x, y };
  // L4 (reality law): a settled spacer stands ON the arc ring — 21ft+
  // from the rim. Targets inside 21ft (the old 12-19ft slot/wing pockets)
  // pushed OUT to the ring; targets beyond the max pulled IN. The real
  // NBA set holds mean pairwise spacing of 25ft because all four spacers
  // occupy the ring, not a scattered 17-30ft fan.
  const targetDist = Math.max(21.5, Math.min(maxDist, dist));
  if (dist <= 0.01) return { x: clampCourt(rimX + nx(21.5) / 94), y };
  const scale = targetDist / dist;
  return { x: clampCourt(rimX + (dx * scale) / 94), y: clampCourt(rimY + (dy * scale) / 50) };
}


/** Distance from a matchup's defender to the attacker (feet). */
function matchupDefenderDistance(
  jersey: string,
  sense: LiveCourtSense,
  matchups: Readonly<Record<string, string>>,
): number {
  const defenderJersey = matchups[jersey];
  if (!defenderJersey) return 10;
  const defender = sense.defensePlayers.find((player) => player.jersey === defenderJersey);
  const attacker = sense.offensePlayers.find((player) => player.jersey === jersey);
  if (!defender || !attacker) return 10;
  return Math.hypot(
    (defender.pose.x - attacker.pose.x) * 94,
    (defender.pose.y - attacker.pose.y) * 50,
  );
}

function solveTeamSeparation(players: PlannedPlayer[]): PlannedPlayer[] {
  // Perf (calibration loop): this runs every planning tick and was 7% of the
  // game's CPU. The pair scan is 8 Gauss-Seidel iterations over 45 pairs —
  // but a relaxation pass only needs a second sweep when a push actually
  // moved someone into a NEW violation. Early-exit when a full iteration
  // makes no changes; in the common steady state (formation already legal)
  // that's ONE pass instead of eight.
  const result = players.map((player) => ({ ...player }));
  for (let iteration = 0; iteration < 8; iteration += 1) {
    let moved = false;
    for (let a = 0; a < result.length; a += 1) {
      for (let b = a + 1; b < result.length; b += 1) {
        const first = result[a]!;
        const second = result[b]!;
        if (first.team !== second.team) continue;
        const dx = second.x - first.x;
        const dy = second.y - first.y;
        const distanceFt = Math.hypot(dx * 94, dy * 50);
        // Screens are intentionally close to the handler, but a handler and
        // screener still need a legal body-radius gap. The old 1.5ft target
        // was smaller than the 2ft collision diameter and let integration
        // frames visibly merge.
        const isScreenPair = (first.task === 'screen' && second.task === 'ball_handler')
          || (second.task === 'screen' && first.task === 'ball_handler');
        // Two off-ball spacers are separated by the same planner floor as
        // other teammates; live steering owns any transient crossing.
        const requiredFt = isScreenPair ? 2.2 : MIN_TEAM_SEPARATION_FT;
        if (distanceFt >= requiredFt) continue;
        moved = true;
        const length = Math.hypot(dx, dy) || 1;
        // Unit fix: the push converts the missing FEET into per-axis
        // normalized offsets — x divides by 94, y by 50 (the old single
        // /94 made every y-axis push 47% short, so vertical separation
        // converged far slower than horizontal).
        const ux = dx / length;
        const uy = dy / length;
        const pushFt = (requiredFt - distanceFt) / 2;
        result[a] = { ...first, x: clampCourt(first.x - ux * pushFt / 94), y: clampCourt(first.y - uy * pushFt / 50) };
        result[b] = { ...second, x: clampCourt(second.x + ux * pushFt / 94), y: clampCourt(second.y + uy * pushFt / 50) };
      }
    }
    if (!moved) break;
  }
  return result;
}

function defenseTarget(args: {
  readonly attacker: PlannedPlayer;
  readonly defenderJersey: string;
  readonly sense: LiveCourtSense;
  readonly plan?: TeamPlan;
}): PlannedPlayer {
  const { attacker, defenderJersey, sense, plan } = args;
  const currentAttacker = sense.offensePlayers.find((player) => player.jersey === attacker.jersey);
  const attackerX = currentAttacker?.pose.x ?? attacker.x;
  const attackerY = currentAttacker?.pose.y ?? attacker.y;
  const isOnBall = attacker.jersey === sense.handler;
  const screenDefense = plan?.screenDefense;
  const screen = plan?.assignments.find((assignment) => assignment.role === 'screener');
  const screenPose = screen ? sense.offensePlayers.find((player) => player.jersey === screen.jersey)?.pose : null;
  // P5.x: a SWITCH exchange only fires once the screen is PHYSICALLY SET
  // (screener within 3.5ft of the handler — the same geometry factScreenSet
  // uses). Before that the screener defender stays on the screener; an
  // early swap made both defenders cross the handler's path mid-advance,
  // stacking movement + contact pushes into 30ft/s single frames.
  const handlerPoseOf = sense.offensePlayers.find((player) => player.jersey === sense.handler)?.pose;
  const screenPos = screenPose;
  const handlerPos = handlerPoseOf;
  const screenPhysicallySet = screenPos !== null && screenPos !== undefined && handlerPos !== null && handlerPos !== undefined
    && Math.hypot((screenPos.x - handlerPos.x) * 94, (screenPos.y - handlerPos.y) * 50) <= 4;
  // HEDGE/BLITZ/ICE approach: the screener defender starts reacting
  // BEFORE the screen is physically set — when the screener is within
  // 12ft of the handler, the defender shows early to disrupt the screen
  // angle. Without this the coverage never fires because the slow-moving
  // screener takes seconds to reach 4ft (measured: screener stuck at
  // 9-20ft for 7+ seconds, HEDGE defender stood idle the entire time).
  // DROP is the exception: the drop defender sinks to the rim NOW, no
  // early show needed. SWITCH holds the pre-set matchup by design.
  const screenApproaching = screenPos !== null && screenPos !== undefined && handlerPos !== null && handlerPos !== undefined
    && Math.hypot((screenPos.x - handlerPos.x) * 94, (screenPos.y - handlerPos.y) * 50) <= 12;
  const earlyShowMode = screenDefense?.mode === 'HEDGE' || screenDefense?.mode === 'BLITZ' || screenDefense?.mode === 'ICE';
  // The coverage position persists through the SCREEN_USE/ADVANTAGE
  // stages even after the screener rolls off the handler — the trap/
  // hedge/wall must survive the roll (live geometry alone disengages
  // the trap the moment the screener passes 4ft, measured: BLITZ
  // trapper 14ft from the handler at SCREEN_USE). The plan's stage is
  // the authority; live distance only gates the PRE-set approach.
  const screenLive = plan?.stage === 'SCREEN_USE' || plan?.stage === 'ADVANTAGE';
  const isScreenDefender = screenDefense?.screenerDefender === defenderJersey
    && (screenPhysicallySet || (earlyShowMode && screenApproaching) || screenLive);
  if (isScreenDefender && screenPose && screen) {
    if (screenDefense.mode === 'DROP') {
      const dx = sense.rim.x - screenPose.x;
      const dy = sense.rim.y - screenPose.y;
      const length = Math.hypot(dx, dy) || 1;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(sense.rim.x - (dx / length) * nx(3)),
        y: clampCourt(sense.rim.y - (dy / length) * ny(3)),
        task: 'tag',
        slot: 'defend_tag',
        targetJersey: screen.jersey,
        movementRole: 'help_defender',
    };
    }
    // Show-and-recover: once the screener has left the screen point
    // (>8ft from the handler — popping to the arc or rolling), the
    // coverage show/trap/wall is over and the defender RECOVERS to the
    // screener. Without this the popper relocated to the arc wide open
    // while the screen defender stayed glued to the handler (measured:
    // popper open at the arc, screen defender 15ft away still at the
    // trap point), inflating catch-and-shoot efficiency and collapsing
    // possession length.
    const screenerLeftScreen = handlerPoseOf && Math.hypot((screenPose.x - handlerPoseOf.x) * 94, (screenPose.y - handlerPoseOf.y) * 50) > 8;
    if (screenerLeftScreen && handlerPoseOf) {
      // Recover to a normal defensive stance on the screener: between
      // them and the rim, ~4ft gap (the standard perimeter closeout).
      const rx = sense.rim.x - screenPose.x;
      const ry = sense.rim.y - screenPose.y;
      const rl = Math.hypot(rx, ry) || 1;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(screenPose.x + (rx / rl) * nx(4)),
        y: clampCourt(screenPose.y + (ry / rl) * ny(4)),
        task: 'deny',
        slot: 'defend_deny',
        targetJersey: screen.jersey,
        movementRole: 'perimeter_defender',
      };
    }
    if (screenDefense.mode === 'HEDGE' && screenPhysicallySet && handlerPoseOf) {
      // HEDGE after the screen is physically set: the show is OVER — the
      // hedge defender recovers to the SCREENER (not the handler). The
      // old fall-through let a HEDGE defender take the handler at
      // SCREEN_USE, stacking two on_ball_defend labels on one player
      // (the TRAP_LABEL_LEAK audit: 247 PNR_POP SCREEN_USE frames/game).
      const rx = sense.rim.x - screenPose.x;
      const ry = sense.rim.y - screenPose.y;
      const rl = Math.hypot(rx, ry) || 1;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(screenPose.x + (rx / rl) * nx(4)),
        y: clampCourt(screenPose.y + (ry / rl) * ny(4)),
        task: 'deny',
        slot: 'defend_deny',
        targetJersey: screen.jersey,
        movementRole: 'perimeter_defender',
      };
    }
    if (screenDefense.mode === 'HEDGE' && screenApproaching && !screenPhysicallySet && handlerPoseOf) {
      // HEDGE approach: shade toward the midpoint between handler and
      // screener - the defender shows early to disrupt the screen angle
      // without fully committing (a full switch). This is the "show"
      // position: 4ft above the screen toward the handler.
      const midX = (handlerPoseOf.x + screenPose.x) / 2;
      const midY = (handlerPoseOf.y + screenPose.y) / 2;
      const showX = midX + (handlerPoseOf.x - midX) * 0.3;
      const showY = midY + (handlerPoseOf.y - midY) * 0.3;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(showX),
        y: clampCourt(showY),
        task: 'deny',
        slot: 'defend_deny',
        targetJersey: sense.handler,
        movementRole: 'perimeter_defender',
      };
    }
    if (screenDefense.mode === 'BLITZ' && handlerPoseOf) {
      // BLITZ trap: the screener defender joins the on-ball defender AT
      // the handler — the two-man trap. The trapper stands on the
      // handler's screen side (~3.5ft), squeezing the ball between the
      // on-ball matchup and the screen. Engages on the approach (the
      // trap forms as the screen arrives, matching real show-and-trap).
      const sx = (screenPose.x - handlerPoseOf.x) * 94;
      const sy = (screenPose.y - handlerPoseOf.y) * 50;
      const sl = Math.hypot(sx, sy) || 1;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(handlerPoseOf.x + (sx / sl) * nx(3.5)),
        y: clampCourt(handlerPoseOf.y + (sy / sl) * ny(3.5)),
        task: 'on_ball_defend',
        slot: 'defend_ball',
        targetJersey: sense.handler,
        movementRole: 'on_ball_defender',
      };
    }
    if (screenDefense.mode === 'ICE' && handlerPoseOf) {
      // ICE: wall off the MIDDLE. The screener defender plants between
      // the handler and the middle corridor (toward the rim, biased to
      // the court center line), so the only escape is baseline. The
      // on-ball defender (see the isOnBall branch below) shades the
      // baseline side to complete the wall.
      const rx = sense.rim.x - handlerPoseOf.x;
      const ry = sense.rim.y - handlerPoseOf.y;
      const rl = Math.hypot(rx, ry) || 1;
      const midBiasX = rx / rl * 0.8 + Math.sign(0.5 - handlerPoseOf.x) * 0.2;
      const midBiasY = ry / rl * 0.8 + (0.5 - handlerPoseOf.y) * 0.4;
      const bl = Math.hypot(midBiasX, midBiasY) || 1;
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: clampCourt(handlerPoseOf.x + (midBiasX / bl) * nx(4.5)),
        y: clampCourt(handlerPoseOf.y + (midBiasY / bl) * ny(4.5)),
        task: 'deny',
        slot: 'defend_deny',
        targetJersey: sense.handler,
        movementRole: 'perimeter_defender',
      };
    }
    // NOTE: no SWITCH branch here. In a SWITCH the screener defender
    // TAKES THE HANDLER — the fallthrough below (on_ball_defend at the
    // handler) is exactly that. A dedicated branch here that sent the
    // screener defender back to the screener INVERTED the switch
    // (both defenders ended up guarding the screener, nobody on the
    // ball — the TRAP_LABEL_LEAK audit: 100+ frames/game in PNR sets).
    if (handlerPoseOf) return { jersey: defenderJersey, team: sense.defense, x: clampCourt(handlerPoseOf.x), y: clampCourt(handlerPoseOf.y), task: 'on_ball_defend', slot: 'defend_ball', targetJersey: sense.handler, movementRole: 'on_ball_defender' };
  }
  const isRimAction = attacker.task === 'cut';
  const currentPose = currentAttacker?.pose ?? null;
  const attackerAbility = sense.abilities[attacker.jersey] ?? null;
  const catchShoot = attackerAbility?.catchShoot ?? 0.5;
  const defenderAbility = sense.abilities[defenderJersey] ?? null;
  const defOnBall = defenderAbility?.onBallDefense ?? 0.5;
  const distToRim = Math.hypot(
    (sense.rim.x - attackerX) * 94,
    (sense.rim.y - attackerY) * 50,
  );
  // Three-point range ≈ beyond the arc (22ft corners, 23.75ft top).
  const inThreeRange = distToRim >= 21;
  // Strong side = within one swing pass (15ft of width) of the ball —
  // a corner spacer on the far side of the floor is weak side even when
  // ball and spacer share the same half (the old half-court test kept
  // corner spacers in deny forever).
  const strongSide = Math.abs(sense.ball.y - attackerY) * 50 <= 15;
  // Threat model: a spacer is denied when their OPEN three-point shot is
  // worth more than the help value their defender gives up by staying
  // tight. Open make rate comes from the same rate model the resolve
  // layer uses (base × corner/wing modifier × catch-shoot modifier), so
  // the deny/sag line is a consequence of the shooting math, not a
  // hand-tuned ability threshold.
  const zm3 = Math.abs(attackerY - (attackerY < 0.5 ? 0.1 : 0.9)) < 0.12
    ? (RESOLVE.zoneModifiers3pt['corner_L'] ?? 1.08)
    : (RESOLVE.zoneModifiers3pt['wing_L'] ?? 1.0);
  const open3Pts = Math.min(0.9, RESOLVE.baseRates.shot_make_3pt * zm3 * (1 + (catchShoot - 0.5) * 0.5)) * 3;
  const helpValue = (defenderAbility?.helpDefense ?? 0.5) * (1 + sense.paintDefenders * 0.3);
  // Chase detection: only a declared cut/drive is a chase target. The old
  // velocity heuristic (moving toward the rim at >5ft/s) misread spacers'
  // pocket drifts and side relocations as cuts — every spacer got chased
  // at 2.9ft and no one was ever open. Real defenders read intent.
  const isCutting = attacker.task === 'cut' || isRimAction;

  // ── coverage decision ──────────────────────────────────────────────────
  // The off-ball game: deny the shooter (贴防), chase the cutter (追防),
  // sag off the weak-side/low-threat spacer to help (协防留余地). The gap
  // and shade are re-evaluated every planning tick, so a swing pass flips
  // a sag into a deny in real time.
  let gapFt: number;
  let shadeX: number;
  let shadeY: number;
  let task: PlayerTask;
  let slot: RelationKind;
  // In-flight pass read: the receiver is about to catch — the defender
  // rotates onto them NOW (gap 3ft, shade toward the rim) instead of
  // waiting for the catch and arriving 17ft late (measured catch-and-
  // shoot with zero contest at Q1 11:45).
  const isPassTarget = sense.passTarget !== null && attacker.jersey === sense.passTarget;
  // NOTE: SWITCH的防守交接由effectiveMatchups层的matchup交换负责
  // (planSpatialTargets):handler的matchup已换成screenerDefender,
  // screener的matchup已换成原on-ball。这里不再做视觉转移——旧的
  // isOnBall SWITCH转移把"新matchup的handler defender"又转去守
  // screener,双重转移导致handler无人防守(审计SCREEN_BEAT_TOO_EASILY:
  // SWITCH后5人全在弱侧,on_ball_defend标签消失)。
  // ── 退防落位 (transition retreat) ─────────────────────────────────────
  // 真实篮球:进攻方还在后场推进时,防守人不是立即贴住对位人,而是
  // 先退到己方半场"油漆区锚点"与对位人之间的连线中点——这是阵地战
  // 落位(paint-first settle)的基础。球过半场后自动切换回正常盯人。
  // 旧逻辑在 TRANSITION_PUSH 全程直接追对位人,导致"防守人从一开始
  // 就只跟着进攻方跑、没有落位过程"(用户反馈)。
  const ballInBackcourt = sense.attackDirection === 1 ? sense.ball.x < 0.5 : sense.ball.x > 0.5;
  // First-principles transition retreat:
  // Whenever the ball is in the backcourt and not an inbound set in frontcourt,
  // ALL defenders MUST prioritize retreating to protect the defensive halfcourt and paint,
  // regardless of whether the offensive plan kind is tagged TRANSITION_PUSH or a set play!
  // Defensive settle (L9/L6 semantics): for the first ~5s of a halfcourt
  // possession the defense is still organizing — off-ball defenders shade
  // the passing lanes (gap 5ft) instead of hugging their matchup, so the
  // first kick-out is contested like real NBA (early catch-and-shoot ran
  // 60% of attempts; the settle window halves the clean early look).
  const possessionAgeSec = 24 - sense.shotClock;
  const defenseSettling = possessionAgeSec < 5 && sense.shotClock > 17;
  const retreating = ballInBackcourt && !sense.inbound;
  if (isOnBall) {
    if (retreating) {
      // 退防落位:先退向己方油漆区锚点,再随球推进连续过渡到盯人。
      // 几何信号(球越过己方底线的深度)驱动权重,间距上限防止放弃
      // 对位人——转换期不再漏空位/空篮。单一纯函数,见
      // transitionRetreatPoint。
      const defRimX = 1 - sense.rim.x;
      const defRimY = sense.rim.y;
      const ballDepthFt = sense.attackDirection === 1 ? sense.ball.x * 94 : (1 - sense.ball.x) * 94;
      const retreat = transitionRetreatPoint(attackerX, attackerY, defRimX, defRimY, ballDepthFt);
      return {
        jersey: defenderJersey,
        team: sense.defense,
        x: retreat.x,
        y: retreat.y,
        task: 'on_ball_defend',
        slot: 'defend_ball',
        targetJersey: sense.handler,
        movementRole: 'on_ball_defender',
      };
    }
    // L1 (reality law): on-ball pressure has a containment GRADIENT — real
    // NBA nearest-defender medians run 10.9ft (clock 24-18) → 6.0 (18-12) →
    // 5.2 (12-6) → 4.6 (6-0). The defense closes as the possession
    // organizes; a fixed 4ft gap from second zero produced a flat
    // 4.8/4.8/4.45/5.94 profile (measured seed 42) — no transition window.
    // Late-clock compression: past 16s of possession age the defense clamps
    // (real 6-0 remaining = 4.6ft) — the aged U-curve (measured 4.0-4.9ft
    // at age 12-18s from defender lag on handler probes) flattens back.
    const possessionAgeSec = 24 - sense.shotClock;
    const settleT = SPATIAL.onball_gap_settle_sec ?? 5;
    const earlyGap = SPATIAL.onball_gap_early_ft ?? 6.5;
    const setGap = SPATIAL.onball_gap_set_ft ?? 3.5;
    const settle = Math.max(0, Math.min(1, possessionAgeSec / settleT));
    const lateClamp = possessionAgeSec >= 16 ? Math.max(0, (possessionAgeSec - 16) / 8) * 2.0 : 0;
    gapFt = Math.max(2.8, earlyGap + (setGap - earlyGap) * settle - lateClamp);
    shadeX = sense.rim.x;
    shadeY = sense.rim.y;
    task = 'on_ball_defend';
    slot = 'defend_ball';
    // ── 绕过掩护 recover (DROP/非SWITCH模式) ────────────────────────
    // 掩护物理设置在on-ball defender与handler之间时,直线target
    // (handler+4ft朝rim)在screener背后——防守人被screener身体挡住,
    // 永远到不了handler(审计SCREEN_BEAT_TOO_EASILY: DROP模式
    // on-ball defender距handler 15ft,卡在掩护外侧)。
    // 真实防守: 从screener的"外侧"绕行——target改为screener外侧的
    // 追踪点(沿screener-rim方向+handler侧偏移),让防守人从边上挤过。
    if (screenPose && handlerPoseOf && screenDefense?.mode !== 'SWITCH') {
      // 防守人-掩护-持球人是否近似共线(screener挡在recover路径上)
      // 不要求screenPhysicallySet:SCREEN_USE/ADVANTAGE阶段screener
      // 已经开始pop/roll,物理距离>4ft,但on-ball defender的recover
      // 路径仍被screener挡住(审计:DROP模式on-ball defender卡在
      // 掩护外侧10-15ft,无法回到handler)。
      // 注意:attacker是进攻球员(handler),防守人的当前位置要从
      // sense.defensePlayers取——之前的实现误用了attackerX/Y
      // (handler位置),导致blocked永远不触发。
      const defPose = sense.defensePlayers.find((p) => p.jersey === defenderJersey)?.pose;
      if (defPose) {
        const dToHandler = Math.hypot((handlerPoseOf.x - defPose.x) * 94, (handlerPoseOf.y - defPose.y) * 50);
        const dToScreen = Math.hypot((screenPose.x - defPose.x) * 94, (screenPose.y - defPose.y) * 50);
        // screener在防守人前方(比handler近)且离防守人近(挡在路上)
        const blocked = dToScreen < dToHandler && dToScreen <= 10;
        if (blocked) {
        // 绕行点: 从screener旁侧(screen-rim法线方向)挤过,然后追handler
        const sx = (screenPose.x - handlerPoseOf.x) * 94;
        const sy = (screenPose.y - handlerPoseOf.y) * 50;
        const sl = Math.hypot(sx, sy) || 1;
        // 法线方向(垂直掩护方向)取防守人所在侧
        const nx2 = -sy / sl;
        const ny2 = sx / sl;
        // 防守人在screener哪一侧?取与防守人同侧的法线
        const side = ((defPose.x - screenPose.x) * nx2 + (defPose.y - screenPose.y) * ny2) > 0 ? 1 : -1;
        // 绕过点: screener旁侧2.5ft + handler方向4ft
        const bypassX = screenPose.x + nx2 * side * (2.5 / 94) + (sx / sl) * (1.5 / 94);
        const bypassY = screenPose.y + ny2 * side * (2.5 / 50) + (sy / sl) * (1.5 / 50);
        // 最终目标: bypass点与handler追踪点的混合(绕过掩护后回到handler)
        return {
          jersey: defenderJersey,
          team: sense.defense,
          x: clampCourt(bypassX),
          y: clampCourt(bypassY),
          task: 'on_ball_defend',
          slot: 'defend_ball',
          targetJersey: attacker.jersey,
          movementRole: 'on_ball_defender',
        };
        }
      }
    }
  } else if (!isOnBall && retreating) {
    // Off-ball retreat: same paint-first anchor for the other four — stand
    // between the DEFENSIVE basket and the assigned man, weighted toward
    // the defensive paint until the ball crosses. Same pure function as
    // the on-ball branch — one geometric model, two call sites.
    const ballDepthFt = sense.attackDirection === 1 ? sense.ball.x * 94 : (1 - sense.ball.x) * 94;
    const retreat = transitionRetreatPoint(attackerX, attackerY, 1 - sense.rim.x, sense.rim.y, ballDepthFt);
    return {
      jersey: defenderJersey,
      team: sense.defense,
      x: retreat.x,
      y: retreat.y,
      task: 'weak_side',
      slot: 'defend_weak',
      targetJersey: attacker.jersey,
      movementRole: 'perimeter_defender',
    };
  } else if (isPassTarget) {
    // Pre-rotate toward the receiver but NOT a full contest: a 3ft gap
    // made every pass into a 34%-failure gamble (measured 157 turnovers
    // in one game vs NBA ~14). 4.5ft shades the catch without turning
    // routine passes into steals — the pressure penalty scales off the
    // actual gap, so this lands near the real ~97% pass success.
    // The label must NOT be on_ball_defend: that action means "guarding
    // the holder", and the holder is still the passer while the ball is
    // in flight. Marking the receiver's defender on_ball_defend created
    // phantom double-teams (2 defenders labeled on_ball_defend within
    // 8ft of the handler — the trap audit read them as a BLITZ in
    // transition, ISO and HANDOFF sets where no trap exists).
    gapFt = 4.5;
    shadeX = sense.rim.x;
    shadeY = sense.rim.y;
    task = 'deny';
    slot = 'defend_deny';
  } else if (isCutting || distToRim <= 8) {
    // Chase/rim coverage: tight, between man and rim.
    gapFt = 2.8;
    shadeX = sense.rim.x;
    shadeY = sense.rim.y;
    task = 'tag';
    slot = 'defend_tag';
  } else if (inThreeRange) {
    if (open3Pts > 1.15 + helpValue * 0.12 && strongSide && defOnBall >= 0.45) {
      // DENY: shade the pass — stand between the man and the ball.
      // Better defenders deny tighter (gap 2.3-3.2ft by onBallDefense).
      gapFt = 2.8 + (0.5 - defOnBall) * 2;
      const awayX = attackerX - sense.ball.x;
      const awayY = attackerY - sense.ball.y;
      const awayLen = Math.hypot(awayX, awayY) || 1;
      shadeX = attackerX + awayX / awayLen;
      shadeY = attackerY + awayY / awayLen;
      task = 'deny';
      slot = 'defend_deny';
    } else {
      // SAG: give the spacer room, hold the help lane toward the rim with
      // a ball-side bias (协防). The gap SETTLES over the possession —
      // but only from a genuinely loose start in TRANSITION (defense
      // still rotating, early offense is real). After a dead ball the
      // defense is already set: the spacer starts tight (5.5ft, no free
      // corner threes at 3s) and loosens only slightly as the action
      // moves the ball.
      const inTransition = plan?.kind === 'TRANSITION_PUSH';
      const settle = (24 - sense.shotClock) / 24;
      // Corner threes are the highest-value look in the game (~40% make):
      // the corner defender holds tight (2.5ft, on the pass line) instead
      // of sinking to the elbow — a sagged corner shooter is a coverage
      // gift, not help defense. Non-corner spacers sag 3-4ft with the
      // ball-side bias so the help lane stays shaded.
      const isCornerAttacker = Math.abs(attackerY - 0.5) * 50 > 19.5;
      gapFt = isCornerAttacker
        ? 3.5
        : (inTransition ? 7.5 - settle * 3.0 : 5.0 + settle * 0.5)
          + Math.max(0, (0.5 - defOnBall)) * 1.2;
      if (isCornerAttacker) {
        // Corner defender shades the PASS line (between the ball and the
        // corner) at 2.5ft — a one-pass-away corner shot is contested the
        // moment the pass leaves.
        const awayX = attackerX - sense.ball.x;
        const awayY = attackerY - sense.ball.y;
        const awayLen = Math.hypot(awayX, awayY) || 1;
        shadeX = attackerX + awayX / awayLen;
        shadeY = attackerY + awayY / awayLen;
        task = 'deny';
        slot = 'defend_deny';
      } else {
        const rx = sense.rim.x - attackerX;
        const ry = sense.rim.y - attackerY;
        const rl = Math.hypot(rx, ry) || 1;
        const ballSide = Math.sign(sense.ball.y - attackerY);
        shadeX = attackerX + (rx / rl) + (ballSide !== 0 ? ballSide * 0.06 : 0);
        shadeY = attackerY + (ry / rl) + (ballSide !== 0 ? ballSide * 0.25 : 0);
        task = 'weak_side';
        slot = 'defend_weak';
        // Weak-side sag toward the rim anchor ("protect the nail"): the
        // far-side defender's job is rim FIRST, man second — but only
        // when his man is genuinely far from the rim. A pure man-relative
        // target parked all five defenders on the arc (measured: 81% of
        // settled halfcourt frames had ZERO paint presence); a flat 65%
        // anchor blend over-corrected into a 5-man wall that killed every
        // rim finish (11% rim share, 44% 3PA conceded). The sag weight
        // scales with the man's rim distance: a deep corner man keeps his
        // defender home (corner 3s stay contested), a wing/elbow man's
        // defender sags hard. Anchor = 7ft-ish rim-side point shaded
        // toward the man's side.
        // L5 (reality law): real paint touches draw 1-3 rim-area defenders
        // (median 2, p75 3) — the weak side keeps a body out of the collapse
        // (sim measured: median 4). Cap the deepest sag.
        const sag = Math.max(0, Math.min(0.42, (rl * 94 - 14) / 22));
        // Cap: the sag anchor may never be more than 10ft from the
        // assigned man. The old blend left weak-side defenders 16ft from
        // their matchup (measured: slot mate at 16ft open through an
        // entire ISO possession — the "loose man" the user flagged at
        // Q1 11:48). Real help defense sags 6-8ft, it does not abandon.
        const anchorX = sense.rim.x + (attackerX - sense.rim.x) * (1 - sag) * 0.6;
        const anchorY = 0.5 + (attackerY - 0.5) * (1 - sag * 0.5);
        const anchorDistToMan = Math.hypot((anchorX - attackerX) * 94, (anchorY - attackerY) * 50);
        const cappedX = anchorDistToMan > 10
          ? attackerX + (anchorX - attackerX) * (10 / anchorDistToMan)
          : anchorX;
        const cappedY = anchorDistToMan > 10
          ? attackerY + (anchorY - attackerY) * (10 / anchorDistToMan)
          : anchorY;
        if (sag >= 0.2) {
          return {
            jersey: defenderJersey,
            team: sense.defense,
            x: clampCourt(cappedX),
            y: clampCourt(cappedY),
            task,
            slot,
            targetJersey: attacker.jersey,
            movementRole: movementRoleForDefense(task, slot),
          };
        }
      }
    }
  } else {
    // Midrange: tight but not denying the pass.
    gapFt = 4.2;
    shadeX = sense.rim.x;
    shadeY = sense.rim.y;
    task = 'deny';
    slot = 'defend_deny';
  }
  const towardRimX = shadeX - attackerX;
  const towardRimY = shadeY - attackerY;
  const length = Math.hypot(towardRimX, towardRimY) || 1;
  let x = attackerX + (towardRimX / length) * nx(gapFt);
  let y = attackerY + (towardRimY / length) * ny(gapFt);
  // isScreenDefender branch); the original on-ball defender must NOT leave
  // the handler early — abandoning the ball for the screener is how the
  // handler ends up 30ft from any defender during the exchange.
  if (isOnBall && screen && screenPose && screen.action === 'screen') {
    const sx = (screenPose.x - attackerX) * 94;
    const sy = (screenPose.y - attackerY) * 50;
    const sl = Math.hypot(sx, sy) || 1;
    if (screenDefense?.mode === 'ICE') {
      // ICE: the on-ball defender shades the BASELINE side — body
      // between the handler and the sideline — so the middle wall
      // (screener defender) and the baseline shade leave only the
      // sideline escape. "拒绝底线" rendered as a real body line.
      const baselineX = sx / sl * nx(2.2) + (sense.rim.x - attackerX > 0 ? -nx(1.2) : nx(1.2));
      const baselineY = sy / sl * ny(2.2) + (attackerY > 0.5 ? -ny(1.6) : ny(1.6));
      x = attackerX + baselineX;
      y = attackerY + baselineY;
    } else {
      x = attackerX + (sx / sl) * nx(2.2) + (sense.rim.x - attackerX > 0 ? -nx(1.5) : nx(1.5));
      y = attackerY + (sy / sl) * ny(2.2);
    }
  }
  return { jersey: defenderJersey, team: sense.defense, x: clampCourt(x), y: clampCourt(y), task, slot, targetJersey: attacker.jersey, movementRole: movementRoleForDefense(task, slot) };
}
export function planSpatialTargets(sense: LiveCourtSense, plan: TeamPlan): SpatialPlan {
  return planTeamSpatial(plan, sense);
}

export function planTeamSpatial(plan: TeamPlan, sense: LiveCourtSense): SpatialPlan {
  const offense = plan.assignments.map((assignment) => {
    const rawTarget = desiredOffenseTarget(assignment, sense, plan, plan.matchups, plan.feedTargetJersey ?? null);
    const isSpacerRole = assignment.role === 'strong_corner' || assignment.role === 'weak_corner' || assignment.role === 'slot';
    const target = isSpacerRole
      ? clampToArc(rawTarget.x, rawTarget.y, sense.rim.x, sense.rim.y)
      : rawTarget;
    return {
      jersey: assignment.jersey,
      team: sense.offense,
      x: target.x,
      y: target.y,
      task: offenseTask(assignment),
      slot: assignment.role === 'handler' ? 'ball_handler_pocket' : 'space_perimeter',
      targetJersey: null,
      movementRole: movementRoleForOffense(assignment),
    };
  });
  // ── 独立防守决策 (per-defender live decision) ──────────────────────
  // 每个防守人每帧独立决策自己的站位,没有"分配对位"的概念:
  //   1. 最近者贴持球人 (on-ball)
  //   2. 其余防守人按"距离×威胁+接球轮转+掩护角色"贪心选定主防对象
  //   3. 未选中的防守人按协防模型落位 (弱侧收篮/油漆区)
  // 战术层的 plan.matchups 只服务于进攻战术选择,不再约束防守站位;
  // 转换/换人/传球飞行中的覆盖由几何自然涌现,不存在"无对位=漏人"。
  const offensePlayers = sense.offensePlayers;
  const handlerJersey = sense.handler;
  const handlerPose = offensePlayers.find((p) => p.jersey === handlerJersey)?.pose ?? null;
  // 1) On-ball defender: the closest defender to the ball.
  let onBallDefender: string | null = null;
  let onBallDist = Infinity;
  for (const d of sense.defensePlayers) {
    const dist = handlerPose
      ? Math.hypot((d.pose.x - handlerPose.x) * 94, (d.pose.y - handlerPose.y) * 50)
      : Infinity;
    if (dist < onBallDist) {
      onBallDist = dist;
      onBallDefender = d.jersey;
    }
  }
  // Dual-track defensive scheme resolution (Drop / Switch / Blitz)
  const schemeDecision = evaluateDefensiveScheme(sense, onBallDefender);
  const primaryCover = new Map<string, string>(); // attackerJersey -> defenderJersey
  if (onBallDefender !== null) primaryCover.set(handlerJersey, onBallDefender);
  // Greedy by ATTACKER urgency, not defender preference: the attacker
  // farthest from any defender is the most open, and must be covered
  // first — otherwise defenders cluster on the nearest threats and the
  // weak side leaks (the pre-fix 29ft open shooter).
  const remainingDefenders = sense.defensePlayers.filter((d) => d.jersey !== onBallDefender);
  const unattended = offensePlayers.filter((p) => p.jersey !== handlerJersey);
  const coverBy = new Map<string, string>(); // attackerJersey -> defenderJersey
  // Urgency of an attacker = how far they are from the nearest UNUSED
  // defender; pass targets and cutters get a boost (rotation priority).
  const urgencyOf = (attacker: (typeof offensePlayers)[number], pool: readonly (typeof sense.defensePlayers)[number][]): number => {
    const nearest = Math.min(
      ...pool.map((def) => Math.hypot((def.pose.x - attacker.pose.x) * 94, (def.pose.y - attacker.pose.y) * 50)),
    );
    let u = nearest;
    if (sense.passTarget === attacker.jersey) u += 6; // rotation priority
    const assignment = plan.assignments.find((a) => a.jersey === attacker.jersey);
    if (assignment?.role === 'screener' || assignment?.action === 'cut') u += 3;
    return u;
  };
  const pool = [...remainingDefenders];
  while (pool.length > 0 && unattended.length > 0) {
    // Pick the most-urgent unattended attacker.
    let bestAttacker: (typeof offensePlayers)[number] | null = null;
    let bestUrgency = -Infinity;
    for (const attacker of unattended) {
      if (coverBy.has(attacker.jersey)) continue;
      const u = urgencyOf(attacker, pool);
      if (u > bestUrgency) {
        bestUrgency = u;
        bestAttacker = attacker;
      }
    }
    if (bestAttacker === null) break;
    // Assign the defender closest to that attacker.
    let bestDef: (typeof sense.defensePlayers)[number] | null = null;
    let bestD = Infinity;
    for (const def of pool) {
      const d = Math.hypot((def.pose.x - bestAttacker.pose.x) * 94, (def.pose.y - bestAttacker.pose.y) * 50);
      if (d < bestD) {
        bestD = d;
        bestDef = def;
      }
    }
    if (bestDef === null) break;
    coverBy.set(bestAttacker.jersey, bestDef.jersey);
    pool.splice(pool.indexOf(bestDef), 1);
  }
  for (const [attacker, defender] of coverBy) primaryCover.set(attacker, defender);
  // 3) Build defensive targets: every defender gets a role by live decision.
  const defense = sense.defensePlayers.flatMap((d) => {
    // The on-ball defender guards the handler.
    if (d.jersey === onBallDefender && handlerPose) {
      const attacker: PlannedPlayer = {
        jersey: handlerJersey,
        team: sense.offense,
        x: handlerPose.x,
        y: handlerPose.y,
        task: 'ball_handler',
        slot: 'ball_handler_pocket',
        targetJersey: null,
        movementRole: 'ball_handler',
      };
      return [defenseTarget({ attacker, defenderJersey: d.jersey, sense, plan })];
    }
    // A defender with a primary target guards that attacker.
    const covered = [...primaryCover.entries()].find(([, def]) => def === d.jersey)?.[0];
    if (covered) {
      const pose = offensePlayers.find((p) => p.jersey === covered)?.pose;
      if (pose) {
        const attacker: PlannedPlayer = {
          jersey: covered,
          team: sense.offense,
          x: pose.x,
          y: pose.y,
          task: plan.assignments.find((a) => a.jersey === covered)?.action === 'cut' ? 'cut' : 'space',
          slot: 'defend_deny',
          targetJersey: null,
          movementRole: 'spacer',
        };
        return [defenseTarget({ attacker, defenderJersey: d.jersey, sense, plan })];
      }
    }
    // Otherwise: help defense — hold the closest unattended attacker's
    // pass lane while shading the ball (real help defends man+space, not
    // a rim anchor). Fallback: ball-side pocket.
    const rimX = sense.rim.x;
    const rimY = sense.rim.y;
    const helpAnchor = (() => {
      let best: (typeof offensePlayers)[number] | null = null;
      let bestD = Infinity;
      for (const op of offensePlayers) {
        if (op.jersey === handlerJersey) continue;
        if (primaryCover.has(op.jersey)) continue;
        const dd = Math.hypot((d.pose.x - op.pose.x) * 94, (d.pose.y - op.pose.y) * 50);
        if (dd < bestD) {
          bestD = dd;
          best = op;
        }
      }
      return best;
    })();
    // L5: a 50/50 man↔rim anchor put every free helper inside 15ft of the
    // rim during paint touches. Blend man-ward (0.65 man) so help holds the
    // LANE, not the rim itself.
    const anchorX = helpAnchor ? (helpAnchor.pose.x - rimX) * 0.65 + rimX : rimX + (sense.ball.x - rimX) * 0.15;
    const anchorY = helpAnchor ? (helpAnchor.pose.y - rimY) * 0.65 + rimY : rimY + (sense.ball.y - rimY) * 0.15;
    const helpPlayer: PlannedPlayer = {
      jersey: d.jersey,
      team: sense.defense,
      x: clampCourt(anchorX),
      y: clampCourt(anchorY),
      task: 'help',
      slot: 'defend_help',
      targetJersey: helpAnchor?.jersey ?? handlerJersey,
      movementRole: 'help_defender',
    };
    return [helpPlayer];
  });
  const players = solveTeamSeparation([...offense, ...defense]);
  return {
    players,
    routes: routesForOffense(plan.assignments, sense, players, plan.kind, plan.feedTargetJersey ?? null),
    onBallDefender,
  };
}

export function intentsFromSpatialPlan(args: {
  readonly spatial: SpatialPlan;
  readonly tactical: TeamPlan;
  readonly sense: LiveCourtSense;
}): Intent[] {
  const assignments = new Map(args.tactical.assignments.map((assignment) => [assignment.jersey, assignment]));
  return args.spatial.players.map((player) => {
    const assignment = assignments.get(player.jersey);
    const kind = assignment?.action ?? (player.task === 'on_ball_defend' ? 'pressure' : player.task === 'tag' ? 'tag' : player.task === 'weak_side' ? 'weak_side' : 'deny');
    return {
      jersey: player.jersey,
      team: player.team,
      kind,
      targetJersey: player.targetJersey,
      zone: zoneFromPoint(player.x, player.y, args.sense.offense, args.sense.baskets),
      task: player.task,
      score: 1,
      targetX: player.x,
      targetY: player.y,
      slot: player.slot,
      satisfied: false,
    };
  });
}
