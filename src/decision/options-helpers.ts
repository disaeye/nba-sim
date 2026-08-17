/**
 * Tiny helper for ActionOption construction.
 * Extracted so tactics module can create variants without importing options.ts.
 */
import type { ActionKind } from '../decision/types.js';
import type { CourtZone } from '../state/types.js';
import type { PlayerTask } from '../court/alignment.js';

export interface ActionOption {
  readonly kind: ActionKind;
  readonly targetJersey: string | null;
  readonly zone: CourtZone;
  readonly task: PlayerTask;
  readonly baseScore: number;
}
