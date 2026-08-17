/**
 * Bridge — layered PlayerData → the engine's `LineupCapability` (0..1)
 * talent overlay. This is the fusion point: the kernel keeps consuming
 * capabilities; rosters that carry `playerData` get their capabilities
 * derived from the 24-ability / tendency / awareness truths.
 *
 * The mapping table is an engine-facing interpretation (⌛): each capability
 * dimension is a weighted blend of the design doc's layer-1 quantities,
 * normalized ÷99 to keep the existing calibration (0.5 = neutral).
 */
import { effectiveAbility } from './aggregate.js';
import type { LineupCapability } from '../identity/types.js';
import type { AbilityKey, AwarenessKey, PlayerData, TendencyKey } from './types.js';

type BridgeSource = AbilityKey | AwarenessKey | TendencyKey | 'SPD' | 'LAT';

export interface BridgeWeight {
  readonly source: BridgeSource;
  readonly weight: number;
}

/** Dimension weights — all rows sum to 1. */
export const BRIDGE_WEIGHTS: Readonly<Record<keyof LineupCapability, readonly BridgeWeight[]>> = {
  creation: [
    { source: 'HND', weight: 0.45 },
    { source: 'PNR', weight: 0.3 },
    { source: 'PASM', weight: 0.25 },
  ],
  pullUp: [
    { source: 'OD3', weight: 0.5 },
    { source: 'PUM', weight: 0.5 },
  ],
  catchShoot: [
    { source: 'CS3', weight: 0.55 },
    { source: 'CSM', weight: 0.3 },
    { source: 'FT', weight: 0.15 },
  ],
  rimFinishing: [
    { source: 'FINC', weight: 0.35 },
    { source: 'FINS', weight: 0.3 },
    { source: 'DUNK', weight: 0.2 },
    { source: 'FINW', weight: 0.15 },
  ],
  passing: [
    { source: 'PASS', weight: 0.5 },
    { source: 'PASM', weight: 0.25 },
    { source: 'POCK', weight: 0.25 },
  ],
  screening: [{ source: 'SCRN', weight: 1.0 }],
  rolling: [
    { source: 'FINC', weight: 0.4 },
    { source: 'DUNK', weight: 0.3 },
    { source: 'ORB', weight: 0.3 },
  ],
  popping: [
    { source: 'CSM', weight: 0.6 },
    { source: 'CS3', weight: 0.4 },
  ],
  postPlay: [
    { source: 'POST', weight: 0.7 },
    { source: 'FINC', weight: 0.3 },
  ],
  cutting: [
    { source: 'FINS', weight: 0.4 },
    { source: 'SPC', weight: 0.3 },
    { source: 'SPD', weight: 0.3 },
  ],
  handleSecurity: [{ source: 'HND', weight: 1.0 }],
  transition: [
    { source: 'SPD', weight: 0.5 },
    { source: 'PUSH', weight: 0.3 },
    { source: 'FINS', weight: 0.2 },
  ],
  onBallDefense: [
    { source: 'POBD', weight: 0.5 },
    { source: 'NAVS', weight: 0.25 },
    { source: 'STL', weight: 0.25 },
  ],
  helpDefense: [
    { source: 'DEFR', weight: 0.45 },
    { source: 'SWCH', weight: 0.25 },
    { source: 'RIM', weight: 0.15 },
    { source: 'BOX', weight: 0.15 },
  ],
};

/** Resolve one bridge source to its 0..1 unit value. */
export function bridgeSourceValue(data: PlayerData, source: BridgeSource): number {
  if (source === 'SPD' || source === 'LAT') return data.physical[source] / 99;
  if (source in data.ability) return effectiveAbility(data, source as AbilityKey) / 99;
  if (source in data.awareness) return data.awareness[source as AwarenessKey] / 99;
  if (source in data.tendency) return data.tendency[source as TendencyKey] / 99;
  throw new Error(`bridgeSourceValue: unknown source ${String(source)}`);
}

/** Full capability overlay derived from layered player data. */
export function bridgeCapabilities(data: PlayerData): LineupCapability {
  const out: Record<string, number> = {};
  for (const dim of Object.keys(BRIDGE_WEIGHTS) as (keyof LineupCapability)[]) {
    let value = 0;
    for (const w of BRIDGE_WEIGHTS[dim]) {
      value += w.weight * bridgeSourceValue(data, w.source);
    }
    out[dim] = Math.min(1, Math.max(0, value));
  }
  return out as unknown as LineupCapability;
}
