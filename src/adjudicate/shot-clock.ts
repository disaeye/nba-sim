/**
 * Single shot-clock reset table (architecture §1.4).
 * Handlers must NOT scatter reset logic.
 */
export type ShotClockResetReason =
  | 'defensive_rebound'
  | 'steal'
  | 'turnover'
  | 'inbound'
  | 'jump_ball'
  | 'offensive_rebound'
  | 'oob_defense_frontcourt'
  | 'shot_clock_violation_new_poss'
  | 'possession_gained';

export function shotClockAfter(reason: ShotClockResetReason, previousShot = 24): number {
  switch (reason) {
    case 'offensive_rebound':
    case 'oob_defense_frontcourt':
      return 14;
    case 'defensive_rebound':
    case 'steal':
    case 'turnover':
    case 'inbound':
    case 'jump_ball':
    case 'shot_clock_violation_new_poss':
    case 'possession_gained':
      return 24;
    default:
      return previousShot;
  }
}
