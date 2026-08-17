/**
 * 士气系统 (§9) — MOR gauge 0..100, base 60, weekly regression toward base.
 * Events settle instantly; regression runs once per rest day, never on game
 * days. Outputs are multiplicative performance/training modifiers only —
 * morale NEVER mutates ability/tendency/awareness truths (§9.4 design
 * boundary: truth changes come only from chapters 3-4).
 */
import { MORALE } from './tables.js';
import { clip } from './normalize.js';
import { validateTendencies } from './growth.js';
import type { MoraleEffects, MoraleEventId, TendencySet } from './types.js';

/** Clamp a morale value to 0..100. */
export function clampMorale(value: number): number {
  return clip(value, 0, 100);
}

/** Apply one §9.2 event instantly. */
export function applyMoraleEvent(morale: number, event: MoraleEventId): number {
  const delta = MORALE.events[event];
  if (delta === undefined) throw new Error(`applyMoraleEvent: unknown event ${event}`);
  return clampMorale(morale + delta);
}

/** §9.1 weekly regression: 10% of the gap toward base 60 (rest days only). */
export function weeklyRegression(morale: number): number {
  return clampMorale(MORALE.base + (morale - MORALE.base) * (1 - MORALE.weeklyRegression));
}

/** §9.3 output bands. */
export function moraleEffects(morale: number): MoraleEffects {
  const v = clampMorale(morale);
  for (const band of MORALE.bands) {
    if (v >= band.morMin) {
      return {
        exec: band.exec,
        train: band.train,
        takeoverErosion: band.takeoverErosion,
        demandsTrade: band.demandsTrade,
      };
    }
  }
  const last = MORALE.bands[MORALE.bands.length - 1]!;
  return { exec: last.exec, train: last.train, takeoverErosion: 0, demandsTrade: last.demandsTrade };
}

/**
 * 倾向侵蚀 (§9.3, 10~24 band): TAKEOVER −2/month. This is the ONLY morale
 * path that touches a tendency; it is a monthly settlement, not a per-event
 * mutation, and it re-runs the §1.4 validation afterwards.
 */
export function applyTakeoverErosion(tendency: TendencySet, months: number): TendencySet {
  if (months <= 0) return tendency;
  const out = { ...tendency };
  out.TAKEOVER = clip(out.TAKEOVER - 2 * months, 0, 99);
  return validateTendencies(out);
}
