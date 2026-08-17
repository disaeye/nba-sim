/**
 * NBA court geometry — single source for zone, 2/3, rim, and draw paths.
 * Coordinates: x ∈ [0,1] along length (94 ft), y ∈ [0,1] along width (50 ft).
 * Tip-off: home basket LEFT (x→0), away basket RIGHT (x→1).
 */
import type { CourtZone, TeamId } from '../state/types.js';
import type { Baskets, BasketSide } from './alignment.js';
import geometryJson from '../../config/court-geometry.json' with { type: 'json' };

export interface CourtGeometryFt {
  readonly length_ft: number;
  readonly width_ft: number;
  readonly rim_from_baseline_ft: number;
  readonly rim_radius_ft: number;
  readonly backboard_from_baseline_ft: number;
  readonly backboard_width_ft: number;
  readonly lane_width_ft: number;
  readonly lane_length_ft: number;
  readonly free_throw_circle_radius_ft: number;
  readonly center_circle_radius_ft: number;
  readonly restricted_area_radius_ft: number;
  readonly three_point_arc_ft: number;
  readonly three_point_corner_ft: number;
  readonly three_point_corner_from_sideline_ft: number;
}

export interface CourtPoint {
  readonly x: number;
  readonly y: number;
}

export interface CourtRect {
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

/** A straight line segment in normalized court coordinates. */
export interface CourtSegment {
  readonly x1: number;
  readonly y1: number;
  readonly x2: number;
  readonly y2: number;
}

/** Serializable draw primitives for any canvas (normalized 0–1). */
export interface CourtDrawSpec {
  readonly length_ft: number;
  readonly width_ft: number;
  readonly boundary: CourtRect;
  readonly halfLine: CourtSegment;
  readonly centerCircle: { readonly cx: number; readonly cy: number; readonly r: number };
  readonly left: {
    readonly rim: CourtPoint;
    readonly rimR: number;
    readonly backboard: CourtSegment;
    readonly lane: CourtRect;
    readonly ftLine: CourtSegment;
    readonly ftCircle: { readonly cx: number; readonly cy: number; readonly r: number };
    readonly restricted: { readonly cx: number; readonly cy: number; readonly r: number };
    readonly threeArc: readonly CourtPoint[];
    readonly threeCorners: readonly CourtSegment[];
    /** NBA lane hash marks: 3/10/14/19 ft from the baseline, 2ft into the lane. */
    readonly laneHashMarks: readonly CourtSegment[];
  };
  readonly right: CourtDrawSpec['left'];
}

const G = geometryJson as CourtGeometryFt;

export function loadCourtGeometryFt(): CourtGeometryFt {
  return G;
}

export function nx(ft: number): number {
  return ft / G.length_ft;
}

export function ny(ft: number): number {
  return ft / G.width_ft;
}

/** Euclidean distance in feet between two normalized points. */
export function distFeet(ax: number, ay: number, bx: number, by: number): number {
  const dx = (bx - ax) * G.length_ft;
  const dy = (by - ay) * G.width_ft;
  return Math.hypot(dx, dy);
}

export function rimNorm(side: BasketSide): CourtPoint {
  if (side === 'left') {
    return { x: nx(G.rim_from_baseline_ft), y: 0.5 };
  }
  return { x: 1 - nx(G.rim_from_baseline_ft), y: 0.5 };
}

export type TacticalSlot =
  | 'ball'
  | 'strong_wing'
  | 'weak_wing'
  | 'strong_corner'
  | 'weak_corner'
  | 'elbow'
  | 'paint'
  | 'rim'
  | 'trail'
  | 'slot_strong'
  | 'slot_weak';

export interface TacticalSpot extends CourtPoint {
  readonly zone: CourtZone;
}

/** Half-court role anchors from NBA ft constants; strong side = higher y. */
export function tacticalSpot(slot: TacticalSlot, attack: BasketSide): TacticalSpot {
  const rim = rimNorm(attack);
  const laneL = nx(G.lane_length_ft);
  const ftX = attack === 'right' ? 1 - laneL : laneL;
  const midX = 0.5;
  const ballX = attack === 'right' ? ftX - nx(2) : ftX + nx(2);
  const elbowY = 0.5 + ny(G.lane_width_ft) / 2 - ny(1);
  const cornerYStrong = 1 - ny(G.three_point_corner_from_sideline_ft + 0.5);
  const cornerYWeak = ny(G.three_point_corner_from_sideline_ft + 0.5);
  const wingDist = nx(G.three_point_arc_ft - 1);
  const wingStrong =
    attack === 'right'
      ? { x: rim.x - wingDist * 0.55, y: 0.5 + ny(14) }
      : { x: rim.x + wingDist * 0.55, y: 0.5 + ny(14) };
  const wingWeak =
    attack === 'right'
      ? { x: rim.x - wingDist * 0.55, y: 0.5 - ny(14) }
      : { x: rim.x + wingDist * 0.55, y: 0.5 - ny(14) };
  const cornerStrong =
    attack === 'right'
      ? { x: rim.x - nx(G.three_point_corner_ft - 2), y: cornerYStrong }
      : { x: rim.x + nx(G.three_point_corner_ft - 2), y: cornerYStrong };
  const cornerWeak =
    attack === 'right'
      ? { x: rim.x - nx(G.three_point_corner_ft - 2), y: cornerYWeak }
      : { x: rim.x + nx(G.three_point_corner_ft - 2), y: cornerYWeak };
  const paint = {
    x: attack === 'right' ? 1 - nx(8) : nx(8),
    y: 0.5,
  };
  const trail = {
    x: attack === 'right' ? midX - nx(4) : midX + nx(4),
    y: 0.5,
  };
  const slotStrong = {
    x: (ballX + wingStrong.x) / 2,
    y: (0.5 + wingStrong.y) / 2,
  };
  const slotWeak = {
    x: (ballX + wingWeak.x) / 2,
    y: (0.5 + wingWeak.y) / 2,
  };

  const table: Record<TacticalSlot, TacticalSpot> = {
    ball: { x: ballX, y: 0.5, zone: 'frontcourt_center' },
    elbow: {
      x: ftX,
      y: attack === 'right' ? elbowY : 1 - elbowY,
      zone: attack === 'right' ? 'elbow_L' : 'elbow_R',
    },
    strong_wing: { ...wingStrong, zone: attack === 'right' ? 'wing_L' : 'wing_R' },
    weak_wing: { ...wingWeak, zone: attack === 'right' ? 'wing_R' : 'wing_L' },
    strong_corner: {
      ...cornerStrong,
      zone: attack === 'right' ? 'corner_L' : 'corner_R',
    },
    weak_corner: {
      ...cornerWeak,
      zone: attack === 'right' ? 'corner_R' : 'corner_L',
    },
    paint: { ...paint, zone: 'paint' },
    rim: { x: rim.x, y: rim.y, zone: 'rim' },
    trail: { ...trail, zone: 'backcourt' },
    slot_strong: { ...slotStrong, zone: attack === 'right' ? 'slot_L' : 'slot_R' },
    slot_weak: { ...slotWeak, zone: attack === 'right' ? 'slot_R' : 'slot_L' },
  };
  return table[slot];
}

export function rimForTeamBasket(baskets: Baskets, team: TeamId): CourtPoint {
  return rimNorm(baskets[team]);
}

/** Basket the offense is attacking. */
export function attackRim(offense: TeamId, baskets: Baskets): CourtPoint {
  const side: BasketSide = offense === 'home' ? baskets.away : baskets.home;
  return rimNorm(side);
}

export function isInPaint(x: number, y: number, attackSide: BasketSide): boolean {
  const laneW = ny(G.lane_width_ft);
  const laneL = nx(G.lane_length_ft);
  const y0 = 0.5 - laneW / 2;
  const y1 = 0.5 + laneW / 2;
  if (y < y0 || y > y1) return false;
  if (attackSide === 'right') {
    return x >= 1 - laneL && x <= 1;
  }
  return x >= 0 && x <= laneL;
}

/**
 * NBA 3PT: arc 23.75 ft from rim center; corners 22 ft.
 * Corner three uses the straight sideline segment.
 */
export function isThreePointShot(x: number, y: number, rim: CourtPoint): boolean {
  const d = distFeet(x, y, rim.x, rim.y);
  const cornerBand = ny(G.three_point_corner_from_sideline_ft);
  const inCornerStrip = y <= cornerBand || y >= 1 - cornerBand;
  if (inCornerStrip) {
    return d >= G.three_point_corner_ft - 1e-6;
  }
  return d >= G.three_point_arc_ft - 1e-6;
}

export function shotValueAt(x: number, y: number, offense: TeamId, baskets: Baskets): 2 | 3 {
  const rim = attackRim(offense, baskets);
  return isThreePointShot(x, y, rim) ? 3 : 2;
}

export function isFrontcourt(x: number, attackSide: BasketSide): boolean {
  if (attackSide === 'right') return x >= 0.5;
  return x <= 0.5;
}

/**
 * Map continuous position to foundation CourtZone label (same enum as config).
 */
export function zoneFromPoint(
  x: number,
  y: number,
  offense: TeamId,
  baskets: Baskets,
): CourtZone {
  const attackSide: BasketSide = offense === 'home' ? baskets.away : baskets.home;
  const rim = rimNorm(attackSide);
  const dRim = distFeet(x, y, rim.x, rim.y);

  if (dRim <= 4) return 'rim';
  if (isInPaint(x, y, attackSide)) {
    if (y < 0.42) return attackSide === 'right' ? 'dunker_R' : 'dunker_L';
    if (y > 0.58) return attackSide === 'right' ? 'dunker_L' : 'dunker_R';
    return 'paint';
  }
  if (!isFrontcourt(x, attackSide)) return 'backcourt';

  const cornerBand = ny(G.three_point_corner_from_sideline_ft + 5);
  if (y <= cornerBand) return attackSide === 'right' ? 'corner_R' : 'corner_L';
  if (y >= 1 - cornerBand) return attackSide === 'right' ? 'corner_L' : 'corner_R';

  const ftX =
    attackSide === 'right'
      ? 1 - nx(G.lane_length_ft)
      : nx(G.lane_length_ft);
  const nearElbow = Math.abs(x - ftX) < nx(4) && Math.abs(y - 0.5) < ny(10);
  if (nearElbow) {
    if (y < 0.5) return attackSide === 'right' ? 'elbow_R' : 'elbow_L';
    return attackSide === 'right' ? 'elbow_L' : 'elbow_R';
  }

  if (Math.abs(y - 0.5) < ny(6) && dRim > 14 && dRim < 28) return 'frontcourt_center';

  if (y < 0.5) {
    return dRim < 18
      ? attackSide === 'right'
        ? 'slot_R'
        : 'slot_L'
      : attackSide === 'right'
        ? 'wing_R'
        : 'wing_L';
  }
  return dRim < 18
    ? attackSide === 'right'
      ? 'slot_L'
      : 'slot_R'
    : attackSide === 'right'
      ? 'wing_L'
      : 'wing_R';
}

/**
 * Distance from the rim (along the baseline axis) to where the 23.75ft
 * three-point arc crosses the 3ft corner band:
 *   sqrt(23.75² − (25 − 3)²) = 8.947ft
 * The NBA corner lines are straight, 22ft from the basket center (3ft from
 * the sideline), and meet the arc exactly at this point.
 */
function threeCornerJoinFt(): number {
  const halfWidth = G.width_ft / 2;
  return Math.sqrt(
    G.three_point_arc_ft ** 2 - (halfWidth - G.three_point_corner_from_sideline_ft) ** 2,
  );
}

function threeArcPoints(rim: CourtPoint, side: BasketSide): CourtPoint[] {
  const pts: CourtPoint[] = [];
  const rArc = G.three_point_arc_ft;
  // Sweep the arc between the two exact corner joins (angles of the
  // straight-segment endpoints), so the polyline meets the corner lines
  // without a seam. The old ±117° sweep + clip left the arc's first kept
  // point up to ~2ft past the join.
  const joinFt = threeCornerJoinFt();
  const halfSpan = Math.atan2(G.width_ft / 2 - G.three_point_corner_from_sideline_ft, joinFt);
  const start = side === 'left' ? -halfSpan : Math.PI - halfSpan;
  const end = side === 'left' ? halfSpan : Math.PI + halfSpan;
  const steps = 48;
  for (let i = 0; i <= steps; i++) {
    const a = start + ((end - start) * i) / steps;
    const xf = rim.x * G.length_ft + rArc * Math.cos(a);
    const yf = rim.y * G.width_ft + rArc * Math.sin(a);
    pts.push({ x: xf / G.length_ft, y: yf / G.width_ft });
  }
  return pts;
}

function sideDraw(side: BasketSide): CourtDrawSpec['left'] {
  const rim = rimNorm(side);
  const laneL = nx(G.lane_length_ft);
  const laneW = ny(G.lane_width_ft);
  const bbX =
    side === 'left' ? nx(G.backboard_from_baseline_ft) : 1 - nx(G.backboard_from_baseline_ft);
  const bbHalf = ny(G.backboard_width_ft) / 2;
  const ftX = side === 'left' ? laneL : 1 - laneL;
  const lane: CourtRect =
    side === 'left'
      ? { x: 0, y: 0.5 - laneW / 2, w: laneL, h: laneW }
      : { x: 1 - laneL, y: 0.5 - laneW / 2, w: laneL, h: laneW };

  const cornerY = ny(G.three_point_corner_from_sideline_ft);
  // Corner straight segments: from the baseline to the exact arc join
  // (22ft from the basket center, 3ft from the sideline). The old
  // `cornerDist * 0.15` guess stopped at 8.5ft — a 5.7ft unpainted gap
  // before the arc.
  const cornerJoin = nx(threeCornerJoinFt());
  const threeCorners =
    side === 'left'
      ? [
          {
            x1: 0,
            y1: cornerY,
            x2: rim.x + cornerJoin,
            y2: cornerY,
          },
          {
            x1: 0,
            y1: 1 - cornerY,
            x2: rim.x + cornerJoin,
            y2: 1 - cornerY,
          },
        ]
      : [
          {
            x1: 1,
            y1: cornerY,
            x2: rim.x - cornerJoin,
            y2: cornerY,
          },
          {
            x1: 1,
            y1: 1 - cornerY,
            x2: rim.x - cornerJoin,
            y2: 1 - cornerY,
          },
        ];

  // NBA lane hash marks: 3, 10, 14, 19 ft from the baseline on both lane
  // lines, each extending 2ft into the lane.
  const laneLow = 0.5 - laneW / 2;
  const laneHigh = 0.5 + laneW / 2;
  const hashLen = ny(2);
  const laneHashMarks = [3, 10, 14, 19].flatMap((ft) => {
    const x = side === 'left' ? nx(ft) : 1 - nx(ft);
    return [
      { x1: x, y1: laneLow, x2: x, y2: laneLow + hashLen },
      { x1: x, y1: laneHigh - hashLen, x2: x, y2: laneHigh },
    ];
  });

  return {
    rim,
    rimR: nx(G.rim_radius_ft),
    backboard: { x1: bbX, y1: 0.5 - bbHalf, x2: bbX, y2: 0.5 + bbHalf },
    lane,
    ftLine: { x1: ftX, y1: 0.5 - laneW / 2, x2: ftX, y2: 0.5 + laneW / 2 },
    ftCircle: {
      cx: ftX,
      cy: 0.5,
      r: ny(G.free_throw_circle_radius_ft),
    },
    restricted: {
      cx: rim.x,
      cy: rim.y,
      r: nx(G.restricted_area_radius_ft),
    },
    threeArc: threeArcPoints(rim, side),
    threeCorners,
    laneHashMarks,
  };
}

/** Full draw spec for spectator / any renderer (normalized coords). */
export function buildCourtDrawSpec(): CourtDrawSpec {
  return {
    length_ft: G.length_ft,
    width_ft: G.width_ft,
    boundary: { x: 0, y: 0, w: 1, h: 1 },
    halfLine: { x1: 0.5, y1: 0, x2: 0.5, y2: 1 },
    centerCircle: { cx: 0.5, cy: 0.5, r: ny(G.center_circle_radius_ft) },
    left: sideDraw('left'),
    right: sideDraw('right'),
  };
}

/** JSON-serializable for export package (browser has no TS import). */
export function courtDrawSpecJson(): CourtDrawSpec {
  return buildCourtDrawSpec();
}
