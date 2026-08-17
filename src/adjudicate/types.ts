/**
 * Adjudication outputs — sole site for outcomes, score, possession, events.
 * docs/foundation/architecture.md §1.4
 */
import type { Event, GameState, TeamId } from '../state/types.js';
import type { BallMotionState } from '../court/ball-motion.js';
import type { EndReason } from '../possession/types.js';

export type EpisodeTransition =
  | { readonly kind: 'none' }
  | { readonly kind: 'continue_oreb' }
  | {
      readonly kind: 'end';
      readonly endReason: EndReason;
      readonly nextStartReason: string;
    };

export interface AdjudicateResult {
  readonly state: GameState;
  readonly ball: BallMotionState;
  readonly events: readonly Event[];
  readonly episode: EpisodeTransition;
  readonly pendingFt?: {
    readonly attempts: 0 | 1 | 2 | 3;
    readonly shooterId: string;
    readonly team: TeamId;
    readonly andOne: boolean;
  };
  /** Clear sticky ball intent after terminal / flight start. */
  readonly clearBallIntent: boolean;
  /** Force redecide next tick. */
  readonly forceDecision: boolean;
}
