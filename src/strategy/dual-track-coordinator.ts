/**
 * Dual-Track Action Coordinator:
 * Manages off-ball secondary actions (Pin-down, Flare screen, Corner shake)
 * concurrently while the primary ball-action (PNR / DHO / ISO) is executing.
 */
import type { LiveCourtSense } from '../perception/live-court.js';
import type { TacticalRole } from './play-graph.js';

export type WeaksideActionType = 'PIN_DOWN' | 'FLARE_SCREEN' | 'CORNER_SHAKE' | 'CLEAR_OUT';

export interface WeaksideActionState {
  readonly type: WeaksideActionType;
  readonly screenerId: string | null;
  readonly cutterId: string | null;
  readonly activeTicks: number;
}

/**
 * Assigns dynamic off-ball roles so weak-side players execute active motion
 * (pin-downs, flare screens, back-door cuts, corner shakes) concurrently
 * while the primary ball-action is executing.
 */
export function coordinateWeaksideMotion(
  sense: LiveCourtSense,
  weaksideJerseys: readonly string[],
): WeaksideActionState {
  if (weaksideJerseys.length === 0) {
    return {
      type: 'CLEAR_OUT',
      screenerId: null,
      cutterId: null,
      activeTicks: 0,
    };
  }

  if (weaksideJerseys.length === 1) {
    return {
      type: 'CORNER_SHAKE',
      screenerId: null,
      cutterId: weaksideJerseys[0] ?? null,
      activeTicks: 0,
    };
  }

  // With 2+ weakside players, coordinate an off-ball tandem action.
  // Tactical selection based on game phase and positioning:
  // - When handler drives into paint: weakside lifts / flares to clear corner help.
  // - When ball is static / probing at perimeter: pin-down to free a shooter.
  const handler = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose;
  const rim = sense.rim;
  const handlerDistToRimFt = handler
    ? Math.hypot((handler.x - rim.x) * 94, (handler.y - rim.y) * 50)
    : 25;

  const [p1, p2] = weaksideJerseys;
  // Find who is deeper in the corner (cutter) vs at the wing/slot (screener)
  const p1Pose = sense.offensePlayers.find((p) => p.jersey === p1)?.pose;
  const p2Pose = sense.offensePlayers.find((p) => p.jersey === p2)?.pose;
  
  let screenerId = p1 ?? null;
  let cutterId = p2 ?? null;

  if (p1Pose && p2Pose) {
    const p1DistToRim = Math.hypot((p1Pose.x - rim.x) * 94, (p1Pose.y - rim.y) * 50);
    const p2DistToRim = Math.hypot((p2Pose.x - rim.x) * 94, (p2Pose.y - rim.y) * 50);
    // The player further from baseline / closer to slot sets the pin-down
    if (p1DistToRim < p2DistToRim) {
      screenerId = p2 ?? null;
      cutterId = p1 ?? null;
    }
  }

  // If handler is driving (rim attack), initiate FLARE_SCREEN or CORNER_SHAKE to lift help defender
  if (handlerDistToRimFt < 16) {
    return {
      type: 'FLARE_SCREEN',
      screenerId,
      cutterId,
      activeTicks: 0,
    };
  }

  return {
    type: 'PIN_DOWN',
    screenerId,
    cutterId,
    activeTicks: 0,
  };
}
