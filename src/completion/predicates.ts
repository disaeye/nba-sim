/**
 * Pure completion predicates — no RNG, no events.
 * docs/foundation/architecture.md §1.3
 */
import type { GameState } from '../state/types.js';
import type { BallStepEvent, BallMotionState } from '../court/ball-motion.js';
import { isInPaint, attackRim, distFeet } from '../court/geometry.js';
import type { CompletionFact } from './types.js';

export function factsFromBallStep(
  bevs: readonly BallStepEvent[],
  state: GameState,
  flight: BallMotionState = state.ballMotion,
): CompletionFact[] {
  const facts: CompletionFact[] = [];
  for (const ev of bevs) {
    if (ev.kind === 'pass_complete') {
      facts.push({
        kind: 'BallArrivedAtReceiver',
        receiverId: ev.receiverId,
        passerId: flight.passerId ?? '?',
        team: flight.team ?? state.possession.team ?? 'home',
        via: 'pass',
      });
    } else if (ev.kind === 'pass_out_of_bounds') {
      facts.push({
        kind: 'PassOutOfBounds',
        passerId: flight.passerId ?? '?',
        receiverId: ev.receiverId,
        team: flight.team ?? state.possession.team ?? 'home',
        x: ev.x,
        y: ev.y,
      });
    } else if (ev.kind === 'pass_missed') {
      facts.push({
        kind: 'PassMissed',
        passerId: flight.passerId ?? '?',
        receiverId: ev.receiverId,
        team: flight.team ?? state.possession.team ?? 'home',
      });
    } else if (ev.kind === 'pass_intercepted') {
      facts.push({
        kind: 'PassIntercepted',
        stealerId: ev.stealerId,
        victimId: ev.victimId,
      });
    } else if (ev.kind === 'shot_arrived') {
      facts.push({
        kind: 'ShotArrivedAtRim',
        shooterId: ev.shooterId,
        shotValue: ev.shotValue,
        zone: flight.zone ?? 'rim',
        x: flight.toX,
        y: flight.toY,
        assisterId: flight.assisterId,
        shotType: ev.shotType ?? flight.shotType,
      });
    } else if (ev.kind === 'strip') {
      facts.push({
        kind: 'HolderStripped',
        stealerId: ev.stealerId,
        victimId: ev.victimId,
      });
    } else if (ev.kind === 'loose_recovered') {
      facts.push({
        kind: 'LooseRecovered',
        recovererId: ev.recovererId,
        team: ev.team,
      });
    }
  }
  return facts;
}

/** Drive termination: ball handler with drive intent inside/near paint. */
export function factPaintArrival(
  state: GameState,
  sticky: { readonly jersey: string; readonly kind: string } | null,
): CompletionFact | null {
  if (state.phase !== 'LIVE') return null;
  if (state.ballMotion.status !== 'held') return null;
  const holder = state.ballMotion.holderId;
  if (!holder) return null;
  if (!sticky || sticky.jersey !== holder) return null;
  if (sticky.kind !== 'drive') return null;

  const offense = state.possession.team;
  if (!offense) return null;
  const pose = state.poses[holder];
  if (!pose) return null;
  const rim = attackRim(offense, state.baskets);
  // Paint arrival means AT THE RIM: the old isInPaint clause fired the
  // moment a wing drive crossed the lane boundary 14-16ft out (the lane is
  // 19ft deep), resolving every drive into a 12-14ft "forced shot" long
  // before the driver reached the basket (measured: drive-end shots
  // clustered 12-14ft, 0 finishes under 10ft). The arrival fact now
  // requires genuine rim proximity (8ft); lane presence alone is not
  // a finishing position.
  const nearRim = distFeet(pose.x, pose.y, rim.x, rim.y) <= 8;
  if (!nearRim) return null;
  return {
    kind: 'PlayerArrivedInPaint',
    ballHandlerId: holder,
    offense,
  };
}

export function factShotClockExpired(state: GameState): CompletionFact | null {
  if (state.phase !== 'LIVE') return null;
  if (state.clocks.shot > 0) return null;
  if (state.clocks.game <= 0) return null;
  // A shot already in flight owns the possession; the result (make/miss) is
  // pending. A pass in flight is mid-air — the horn truncates it as a dead
  // ball, but that is adjudicated from a held/loose state, not from flight.
  if (state.ballMotion.status === 'shot' || state.ballMotion.status === 'pass') return null;
  const team = state.possession.team ?? 'home';
  return { kind: 'ShotClockExpired', team };
}

export function factGameClockExpired(state: GameState): CompletionFact | null {
  if (state.phase !== 'LIVE') return null;
  if (state.clocks.game > 0) return null;
  return { kind: 'GameClockExpired' };
}

/** Handler has used the set screen when it crosses beyond the screen anchor. */
export function factScreenUsed(
  state: import('../state/types.js').GameState,
  screen: {
    readonly screenerId: string;
    readonly handlerId: string;
    readonly anchorX: number;
    readonly anchorY: number;
    readonly handlerStartX: number;
    readonly handlerStartY: number;
  } | null,
): import('./types.js').CompletionFact | null {
  if (state.phase !== 'LIVE' || !screen || state.ballMotion.holderId !== screen.handlerId) return null;
  const handler = state.poses[screen.handlerId];
  const screener = state.poses[screen.screenerId];
  if (!handler || !screener) return null;
  const movedX = (handler.x - screen.handlerStartX) * 94;
  const movedY = (handler.y - screen.handlerStartY) * 50;
  const moved = Math.hypot(movedX, movedY);
  const initialAnchorDistance = Math.hypot(
    (screen.handlerStartX - screen.anchorX) * 94,
    (screen.handlerStartY - screen.anchorY) * 50,
  );
  const currentAnchorDistance = Math.hypot(
    (handler.x - screen.anchorX) * 94,
    (handler.y - screen.anchorY) * 50,
  );
  // Screen use is a movement through the screen, not the pre-existing
  // handler-to-anchor spacing at the moment the screener arrives.
  const clearedScreen = currentAnchorDistance >= initialAnchorDistance + 1.5;
  return moved >= 3 && clearedScreen
    ? { kind: 'ScreenUsed', screenerId: screen.screenerId, ballHandlerId: screen.handlerId }
    : null;
}

/** Screen physical set: screener has arrived near the handler. */
export function factScreenSet(
  state: import('../state/types.js').GameState,
  stickyScreenJersey: string | null,
): import('./types.js').CompletionFact | null {
  if (state.phase !== 'LIVE') return null;
  if (!stickyScreenJersey) return null;
  const screener = state.poses[stickyScreenJersey];
  if (!screener) return null;
  const handler = state.ballMotion.holderId;
  if (stickyScreenJersey === handler) return null;
  if (!handler) return null;
  const handlerPose = state.poses[handler];
  if (!handlerPose || handlerPose.team !== screener.team) return null;
  const handlerTeam = state.possession.team;
  if (!handlerTeam || handlerTeam !== screener.team) return null;
  const defenderCandidates = Object.values(state.poses).filter(
    (pose) => pose.team !== screener.team && pose.jersey !== handler,
  );
  let onBallDefender = defenderCandidates[0];
  let nearestDefenderDistance = Infinity;
  for (const defender of defenderCandidates) {
    const distance = Math.hypot(
      (defender.x - handlerPose.x) * 94,
      (defender.y - handlerPose.y) * 50,
    );
    if (distance < nearestDefenderDistance) {
      nearestDefenderDistance = distance;
      onBallDefender = defender;
    }
  }
  // The proximity circle alone is not a screen: an approaching screener can
  // pass within four feet on the wrong side of the handler and still trigger
  // SCREEN_SET before the body has actually occupied the defender's path.
  // A legal screen can be declared during the final body-entry step (up to
  // 5.5ft marker-to-handler distance) when the screener is already between
  // the handler and defender and no farther from the defender than the
  // handler-side gap. This avoids a moving-target delay while retaining the
  // defender-facing geometry as the causal gate.
  if (nearestDefenderDistance > 8) return null;
  const dFt = Math.hypot(
    (screener.x - handlerPose.x) * 94,
    (screener.y - handlerPose.y) * 50,
  );
  if (dFt > 5.5) return null;
  if (onBallDefender) {
    const handlerToScreen = {
      x: (screener.x - handlerPose.x) * 94,
      y: (screener.y - handlerPose.y) * 50,
    };
    const handlerToDefender = {
      x: (onBallDefender.x - handlerPose.x) * 94,
      y: (onBallDefender.y - handlerPose.y) * 50,
    };
    const sideDot = handlerToScreen.x * handlerToDefender.x
      + handlerToScreen.y * handlerToDefender.y;
    const defenderGap = Math.hypot(handlerToDefender.x, handlerToDefender.y);
    const screenToDefender = Math.hypot(
      (screener.x - onBallDefender.x) * 94,
      (screener.y - onBallDefender.y) * 50,
    );
    // The screen should occupy the defender-facing side of the handler.
    // Body-contact spacing can push the marker slightly past the matchup,
    // so accept a short tolerance but reject an orthogonal/wrong-side pass.
    if (sideDot <= 0 || screenToDefender > dFt + 1.5) return null;
    if (dFt > defenderGap + 2.5) return null;
  }
  return {
    kind: 'ScreenSet',
    screenerId: stickyScreenJersey,
    ballHandlerId: handler,
    defenderId: onBallDefender?.jersey ?? null,
    separationBonus: Math.max(0, 1 - dFt / 10),
    anchorX: screener.x,
    anchorY: screener.y,
    handlerX: handlerPose.x,
    handlerY: handlerPose.y,
  };
}
