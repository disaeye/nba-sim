import { describe, it, expect } from 'vitest';
import {
  distFeet,
  isThreePointShot,
  isInPaint,
  shotValueAt,
  zoneFromPoint,
  rimNorm,
  buildCourtDrawSpec,
  nx,
  ny,
  tacticalSpot,
} from '../../src/court/geometry.js';
import { TIP_OFF_BASKETS } from '../../src/court/alignment.js';

describe('NBA court geometry (single source)', () => {
  it('rim is ~5.25 ft from baseline on each end', () => {
    const left = rimNorm('left');
    const right = rimNorm('right');
    expect(left.x).toBeCloseTo(5.25 / 94, 5);
    expect(right.x).toBeCloseTo(1 - 5.25 / 94, 5);
    expect(left.y).toBe(0.5);
  });

  it('corner three at 22 ft is three; under rim is two', () => {
    const rim = rimNorm('right');
    // Just outside corner distance near sideline
    const cornerX = rim.x - 22.5 / 94;
    const cornerY = 3 / 50;
    expect(isThreePointShot(cornerX, cornerY, rim)).toBe(true);
    expect(shotValueAt(rim.x - 0.02, 0.5, 'home', TIP_OFF_BASKETS)).toBe(2);
  });

  it('top of key beyond 23.75 ft arc is three for home attacking right', () => {
    const rim = rimNorm('right');
    const topKeyX = rim.x - 24.5 / 94;
    expect(isThreePointShot(topKeyX, 0.5, rim)).toBe(true);
    expect(isThreePointShot(rim.x - 10 / 94, 0.5, rim)).toBe(false);
  });

  it('paint contains free-throw line interior', () => {
    expect(isInPaint(1 - nx(10), 0.5, 'right')).toBe(true);
    expect(isInPaint(0.5, 0.5, 'right')).toBe(false);
  });

  it('zoneFromPoint near right rim is rim/paint', () => {
    const z = zoneFromPoint(0.95, 0.5, 'home', TIP_OFF_BASKETS);
    expect(['rim', 'paint', 'dunker_L', 'dunker_R']).toContain(z);
  });

  it('draw spec has both ends with three arc and rims', () => {
    const spec = buildCourtDrawSpec();
    expect(spec.left.threeArc.length).toBeGreaterThan(10);
    expect(spec.right.threeArc.length).toBeGreaterThan(10);
    expect(spec.left.rim.x).toBeLessThan(0.2);
    expect(spec.right.rim.x).toBeGreaterThan(0.8);
    expect(distFeet(0, 0, 1, 0)).toBeCloseTo(94, 5);
  });

  it('three-point corner lines run from the baseline to the exact arc join (no gap)', () => {
    const spec = buildCourtDrawSpec();
    for (const side of ['left', 'right'] as const) {
      const c = spec[side];
      // Corner segments: from the baseline (x=0/1) to the 23.75 arc's
      // crossing of the 3ft corner band: 5.25 + sqrt(23.75²−22²) ≈ 14.2ft.
      const joinFt = Math.sqrt(23.75 ** 2 - (25 - 3) ** 2);
      const joinX = side === 'left' ? c.rim.x + nx(joinFt) : c.rim.x - nx(joinFt);
      expect(c.threeCorners).toHaveLength(2);
      for (let i = 0; i < c.threeCorners.length; i++) {
        const seg = c.threeCorners[i]!;
        expect(seg.x1).toBeCloseTo(side === 'left' ? 0 : 1, 9);
        expect(seg.x2).toBeCloseTo(joinX, 9);
        // The arc polyline starts/ends exactly where the corner segments
        // end: left arc[0]↔bottom segment, arc[last]↔top; right mirrored.
        const arcEnd = side === 'left'
          ? (i === 0 ? c.threeArc[0]! : c.threeArc[c.threeArc.length - 1]!)
          : (i === 0 ? c.threeArc[c.threeArc.length - 1]! : c.threeArc[0]!);
        expect(arcEnd.x).toBeCloseTo(seg.x2, 9);
        expect(arcEnd.y).toBeCloseTo(seg.y1, 9);
      }
      // Corner line sits exactly 22ft from the basket center (perpendicular
      // distance at the rim's y-coordinate); its arc end is on the 23.75 arc.
      expect(distFeet(c.rim.x, c.threeCorners[0]!.y1, c.rim.x, c.rim.y)).toBeCloseTo(22, 6);
      expect(distFeet(c.threeCorners[0]!.x2, c.threeCorners[0]!.y1, c.rim.x, c.rim.y)).toBeCloseTo(23.75, 3);
    }
  });

  it('lane hash marks at 3/10/14/19 ft from the baseline, 2ft into the lane', () => {
    const spec = buildCourtDrawSpec();
    for (const side of ['left', 'right'] as const) {
      const marks = spec[side].laneHashMarks;
      expect(marks).toHaveLength(8); // 4 distances × 2 lane lines
      const xs = marks.map((m) => m.x1).filter((v, i, a) => a.indexOf(v) === i).sort();
      const expected = [3, 10, 14, 19].map((ft) => (side === 'left' ? nx(ft) : 1 - nx(ft))).sort();
      expect(xs).toHaveLength(4);
      for (let i = 0; i < 4; i++) expect(xs[i]).toBeCloseTo(expected[i]!, 9);
      // Marks extend 2ft into the lane from both lane lines.
      const laneLow = 0.5 - ny(16) / 2;
      const laneHigh = 0.5 + ny(16) / 2;
      for (const m of marks) {
        expect(Math.min(m.y1, m.y2)).toBeGreaterThanOrEqual(laneLow - 1e-9);
        expect(Math.max(m.y1, m.y2)).toBeLessThanOrEqual(laneHigh + 1e-9);
        const len = Math.hypot((m.x2 - m.x1) * 94, (m.y2 - m.y1) * 50);
        expect(len).toBeCloseTo(2, 5);
      }
    }
  });

  it('tactical spots derive from NBA geometry (FT line, rim, corners)', () => {
    const ball = tacticalSpot('ball', 'right');
    const elbow = tacticalSpot('elbow', 'right');
    const rim = tacticalSpot('rim', 'right');
    const corner = tacticalSpot('strong_corner', 'right');
    const ftX = 1 - nx(19);
    expect(elbow.x).toBeCloseTo(ftX, 5);
    expect(ball.x).toBeCloseTo(ftX - nx(2), 5);
    expect(ball.x).toBeGreaterThan(0.5);
    expect(rim.x).toBeCloseTo(rimNorm('right').x, 5);
    expect(corner.y).toBeGreaterThan(0.85);
    expect(tacticalSpot('ball', 'left').x).toBeLessThan(0.5);
  });
});
