/**
 * 体测 normalization (§1.2) — physical quantities rescaled to the 20..99
 * formula scale used by the ability layer.
 *
 *   H_n  = 20 + (H−175)/55×79        WT_n = 20 + (WT−70)/70×79
 *   VJ_n = 20 + (VJ−50)/60×79        WS_n = 20 + (WS/H−0.98)/0.14×79
 *
 * WS/H is clipped to [0.98, 1.12] at generation time; callers must never
 * feed out-of-range ratios (generation enforces the constraint).
 */
import { NORMALIZATION, PHYSICAL_RANGES } from './tables.js';
import type { NormalizedPhysical, Physical } from './types.js';

/** Clip a value into [min, max]. */
export function clip(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

/** Round-trip range check for generated physical values. */
export function physicalInRange(key: keyof typeof PHYSICAL_RANGES, value: number): boolean {
  const r = PHYSICAL_RANGES[key];
  return value >= r.min && value <= r.max;
}

/** §1.2 normalization formulas. */
export function normalizePhysical(p: Physical): NormalizedPhysical {
  const h = NORMALIZATION.H;
  const wt = NORMALIZATION.WT;
  const vj = NORMALIZATION.VJ;
  const ws = NORMALIZATION.WS;
  const H_n = h.base + ((p.H - h.low) / h.span) * h.scale;
  const WT_n = wt.base + ((p.WT - wt.low) / wt.span) * wt.scale;
  const VJ_n = vj.base + ((p.VJ - vj.low) / vj.span) * vj.scale;
  const WS_n = ws.base + ((p.WS / p.H - ws.lowRatio) / ws.spanRatio) * ws.scale;
  return { H_n, WT_n, VJ_n, WS_n };
}
