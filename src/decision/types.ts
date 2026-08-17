/**
 * Unified decision types — one decide() path for ball & off-ball agents.
 * Role bias and play goals are dynamic context, not separate code paths.
 */
import type { CourtZone, TeamId } from '../state/types.js';
import type { PlayerTask } from '../court/alignment.js';
import type { RelationKind } from '../court/relations.js';

export type ActionKind =
  | 'advance'
  | 'pass'
  | 'handoff'
  | 'drive'
  | 'shoot'
  | 'hold'
  | 'triple_threat'
  | 'back_to_basket'
  | 'pivot'
  | 'crossover'
  | 'pump_fake'
  | 'space'
  | 'cut'
  | 'screen'
  | 'relocate'
  | 'pressure'
  | 'contain'
  | 'deny'
  | 'help'
  | 'tag'
  | 'weak_side'
  | 'box_out'
  | 'idle';
export type HandlingActionKind =
  | 'triple_threat'
  | 'back_to_basket'
  | 'pivot'
  | 'crossover'
  | 'pump_fake';

export interface AgentPolicy {
  readonly jersey: string;
  readonly team: TeamId;
  readonly hasBall: boolean;
  /** Soft role label from lineup usage — bias only. */
  readonly roleBias:
    | 'creator'
    | 'screener'
    | 'spacer'
    | 'on_ball_def'
    | 'help_def'
    | 'weak_def'
    | 'neutral';
  readonly matchupJersey: string | null;
}

export interface WorldContext {
  readonly offense: TeamId;
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly gameClock: number;
  readonly shotClock: number;
  readonly scoreDiff: number;
  readonly ballHandler: string;
  readonly ballZone: CourtZone;
  /** Advisory play id (bonus only). */
  readonly playId: string | null;
  readonly period: number;
  /** Possession macro-phase for structural time model. */
  readonly possessionPhase: 'ADVANCE' | 'SETUP' | 'EXECUTE';
}
export interface ActionOption {
  readonly kind: ActionKind;
  readonly targetJersey: string | null;
  readonly zone: CourtZone;
  readonly task: PlayerTask;
  readonly baseScore: number;
}

export interface Intent {
  readonly jersey: string;
  readonly team: TeamId;
  readonly kind: ActionKind;
  readonly targetJersey: string | null;
  readonly zone: CourtZone;
  readonly task: PlayerTask;
  readonly score: number;
  readonly targetX: number;
  readonly targetY: number;
  readonly slot: RelationKind | null;
  readonly satisfied: boolean;
  /** P1.2 shot-method taxonomy for a shoot intent. */
  readonly shotType?: 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';
}

export interface DecisionBatch {
  readonly intents: readonly Intent[];
  readonly ballIntent: Intent;
}
