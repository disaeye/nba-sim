/**
 * Displacement guard — the single source of truth for "how far can a body
 * move in one frame, measured in real feet."
 *
 * Court coordinates are normalized anisotropically:
 *   x ∈ [0,1] spans the 94 ft length
 *   y ∈ [0,1] spans the 50 ft width
 *
 * The old in-line clamp in stepAllPoses compared raw normalized deltas with
 * a single 94 ft scale (`hypot(dx, dy)` vs `(25*dt)/94`), which silently let
 * pure y-axis motion travel 94/50 ≈ 1.88× the intended ceiling. Every
 * displacement check must go through here so the anisotropy is applied once,
 * consistently.
 *
 * Design notes
 * ------------
 *  • `frameFeet` is the only correct distance metric for basketball motion.
 *  • `clampFrameDisplacement` preserves the direction of motion and only
 *    shortens it — never rewrites a target or zeroes velocity bookkeeping.
 *    Callers compose the clamped (x, y) into whatever pose record they hold.
 *  • NBA observed sprint ceiling ≈ 22 ft/s (John Wall/D-Rose burst tier);
 *    MAX_FRAME_FTPS = 24 leaves a small numerical margin above the engine's
 *    fastest action profile (relocate ≈ 0.22 court/s ≈ 20.7 ft/s) without
 *    admitting the 28-33 ft/s teleport frames the collision resolver used
 *    to produce.
 */
import { clamp01 } from './poses.js';

/** Court length in feet (baseline → baseline). */
export const COURT_LENGTH_FT = 94;
/** Court width in feet (sideline → sideline). */
export const COURT_WIDTH_FT = 50;

/**
 * Absolute per-frame sprint ceiling. The engine's fastest scripted action
 * (relocate, 0.22 court/s ≈ 20.7 ft/s) sits below this; anything above is a
 * solver artifact (stacked collision push, reset bypass) and is clamped.
 */
export const MAX_FRAME_FTPS = 24;

/**
 * Real-feet distance between two normalized court points. This is the
 * metric every speed/teleport decision must use — never raw `hypot(dx, dy)`.
 */
export function frameFeet(
  ax: number, ay: number,
  bx: number, by: number,
): number {
  const dx = (bx - ax) * COURT_LENGTH_FT;
  const dy = (by - ay) * COURT_WIDTH_FT;
  return Math.hypot(dx, dy);
}

/**
 * Clamp the displacement from `prev` to `next` so the resulting point is no
 * farther than `maxFtps * dt` real feet away, preserving direction. Returns
 * the (possibly shortened) next coordinates, clamped to [0,1]².
 *
 * When `dt <= 0` (a pure collision-resolve / poses-test call) the function is
 * a no-op: there is no time interval over which to enforce a speed limit, and
 * the contact resolver legitimately needs to push bodies apart in a single
 * settled frame.
 */
export function clampFrameDisplacement(
  prevX: number, prevY: number,
  nextX: number, nextY: number,
  dt: number,
  maxFtps: number = MAX_FRAME_FTPS,
): { x: number; y: number } {
  if (dt <= 0) return { x: clamp01(nextX), y: clamp01(nextY) };
  const dx = nextX - prevX;
  const dy = nextY - prevY;
  const maxMoveFt = maxFtps * dt;
  // Per-axis ceiling derived from the same real-feet budget — this is the
  // bound that was missing: a pure y-axis move used to be judged on the 94 ft
  // scale and overshoot by 1.88×.
  const maxX = (maxMoveFt / COURT_LENGTH_FT);
  const maxY = (maxMoveFt / COURT_WIDTH_FT);
  const clampedX = clamp01(prevX + Math.max(-maxX, Math.min(maxX, dx)));
  const clampedY = clamp01(prevY + Math.max(-maxY, Math.min(maxY, dy)));
  // Final isotropic guard: if a diagonal move slipped through the per-axis
  // bound (it can't exceed it, but the resultant may still top maxMoveFt),
  // shorten along the direction vector.
  const distFt = frameFeet(prevX, prevY, clampedX, clampedY);
  if (distFt > maxMoveFt && distFt > 1e-9) {
    const ratio = maxMoveFt / distFt;
    return {
      x: clamp01(prevX + (clampedX - prevX) * ratio),
      y: clamp01(prevY + (clampedY - prevY) * ratio),
    };
  }
  return { x: clampedX, y: clampedY };
}
