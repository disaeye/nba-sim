/**
 * Possession types — config/plays.json shape + episode lifecycle.
 *
 * config/plays.json (validated by config/schemas/plays.schema.json and
 * cross-checked by scripts/check-foundation.mjs) is the sole authority.
 *
 * The live offense engine is DecisionKernel (src/decision). Plays here are
 * advisory: selectPlay picks a playId; buildTeamPlan binds the tactic
 * kind to that play's family (PLAY_ID_KIND) so the plan and the playbook
 * agree during the halfcourt phase. There is no per-step play walker —
 * the play is a kind/role hint, not a choreographed sequence; the stage
 * machine (possessionPhase) owns the time model.
 */
import type { TeamId } from '../state/types.js';
import type { OffenseSlot } from '../identity/types.js';

export type PossessionMode = 'TRANSITION' | 'HALFCOURT';

export interface PlayStep {
  readonly action: string;
  readonly actor_slot: OffenseSlot;
  readonly duration_id: string;
  readonly on_success: string;
  readonly on_fail: string;
}

export interface Play {
  readonly id: string;
  readonly mode: PossessionMode;
  readonly slots: readonly OffenseSlot[];
  readonly steps: readonly PlayStep[];
  /** Selection weight (default 1). Higher = selected more often. This is
   *  the calibration lever that shapes the tactic MIX toward NBA reality:
   *  PNR families run most, ISO/POST are change-of-pace, not defaults. */
  readonly weight?: number;
}

export interface PlaysConfig {
  readonly foundation_version: string;
  readonly modes: readonly PossessionMode[];
  readonly plays: readonly Play[];
}

export type StartReason = string;

export type EndReason =
  | 'MAKE'
  | 'MISS_DREB'
  | 'MISS_OREB_CONTINUE'
  | 'TURNOVER'
  | 'SHOOTING_FOUL'
  | 'AND_ONE'
  | 'NON_SHOOTING_FOUL'
  | 'SHOT_CLOCK_VIOLATION'
  | 'PERIOD_END'
  | 'UNKNOWN';

export interface PossessionEpisode {
  readonly team: TeamId;
  readonly startReason: StartReason;
  readonly mode: PossessionMode;
  play: Play | null;
  ended: boolean;
  endReason: EndReason | null;
}

export type PossessionErrorCode = 'POSSESSION_NO_TEAM';

export class PossessionError extends Error {
  readonly code: PossessionErrorCode;
  constructor(code: PossessionErrorCode, message: string) {
    super(message);
    this.code = code;
    this.name = 'PossessionError';
  }
}
