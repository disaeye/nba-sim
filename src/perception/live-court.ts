import type { BasketSide } from '../court/alignment.js';
import { attackRim, distFeet, zoneFromPoint } from '../court/geometry.js';
import type { PoseState } from '../court/poses.js';
import type { LineupCapability } from '../identity/types.js';
import type { GameState, TeamId } from '../state/types.js';

export type PerceivedCourtZone =
  | 'backcourt'
  | 'frontcourt_center'
  | 'slot_L'
  | 'slot_R'
  | 'wing_L'
  | 'wing_R'
  | 'corner_L'
  | 'corner_R'
  | 'elbow_L'
  | 'elbow_R'
  | 'paint'
  | 'dunker_L'
  | 'dunker_R'
  | 'rim';

export interface PlayerSense {
  readonly jersey: string;
  readonly team: TeamId;
  readonly pose: PoseState;
  readonly distanceToBallFt: number;
  readonly distanceToRimFt: number;
}

export interface LiveCourtSense {
  readonly offense: TeamId;
  readonly defense: TeamId;
  readonly handler: string;
  readonly attack: BasketSide;
  readonly baskets: { readonly home: BasketSide; readonly away: BasketSide };
  readonly attackDirection: -1 | 1;
  readonly rim: { readonly x: number; readonly y: number };
  readonly ball: { readonly x: number; readonly y: number };
  readonly ballZone: PerceivedCourtZone;
  readonly shotClock: number;
  readonly gameClock: number;
  /** P2.4 score margin (offense − defense), for situation pricing. */
  readonly scoreDiff: number;
  /** P2.4 period number, for end-of-quarter clocks. */
  readonly period: number;
  readonly offensePlayers: readonly PlayerSense[];
  readonly defensePlayers: readonly PlayerSense[];
  /** Per-jersey talent overlay (roster abilities merged over defaults). */
  readonly abilities: Readonly<Record<string, LineupCapability>>;
  readonly onBallDefender: string | null;
  readonly onBallDistanceFt: number;
  readonly paintDefenders: number;
  readonly openTeammates: readonly string[];
  /** Catch-and-shoot window: seconds since the handler caught the ball. */
  readonly catchWindowSeconds: number;
  /** In-flight pass receiver (ball.status === 'pass'): defenders read the
   *  pass and start rotating BEFORE the catch — the target gets on-ball
   *  attention while the ball is still in the air. */
  readonly passTarget: string | null;
  /** The player who passed the ball to the current handler (the previous
   *  PASS flight's passerId). Guards the "hot potato" return pass: NBA
   *  handlers do not whip the ball straight back to the passer they just
   *  received from unless the passer is genuinely open — the decision
   *  layer prices that return lower. Null when the handler did not
   *  receive via a live pass (rebound, inbound, steal). */
  readonly lastPasserId?: string | null;
  /** P1.2 per-jersey shot-method classification at decision time. */
  readonly shotTypeForDecision?: Readonly<Record<string, 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other'>>;
  /** P2.1 per-jersey tendency profile (T3/TMID/TDRIVE/TPOST/PASS1ST/GAMBLE/FOUL/TAKEOVER/PUSH), 0..99. */
  readonly tendencies?: Readonly<Record<string, Readonly<Partial<Record<'T3' | 'TMID' | 'TDRIVE' | 'TPOST' | 'PASS1ST' | 'GAMBLE' | 'FOUL' | 'TAKEOVER' | 'PUSH', number>>>>>;
  /** P3.1 raw player data per jersey (ability/awareness/physical). */
  readonly playerData?: Readonly<Record<string, import('../playerdata/types.js').PlayerData>>;
  /** P5.5 coach identity per team (scheme/pace biases). */
  readonly coach?: Readonly<Record<TeamId, import('../sim-utils.js').CoachProfile | undefined>>;
  /** P6.1 active chemistry effects per team lineup (channel modifiers). */
  readonly chemistry?: Readonly<Record<TeamId, readonly import('../playerdata/types.js').ChemistryEffect[]>>;
  /** P6.2 per-team morale exec multiplier (§9.3 band). */
  readonly moraleExec?: Readonly<Record<TeamId, number>>;
  /** §8.4 exec factor per jersey (fatigue pricing); absent = 1. */
  readonly staminaFactors?: Readonly<Record<string, number>>;
}

function otherTeam(team: TeamId): TeamId {
  return team === 'home' ? 'away' : 'home';
}

function sensePlayer(
  pose: PoseState,
  ball: { readonly x: number; readonly y: number },
  rim: { readonly x: number; readonly y: number },
): PlayerSense {
  return {
    jersey: pose.jersey,
    team: pose.team,
    pose,
    distanceToBallFt: distFeet(pose.x, pose.y, ball.x, ball.y),
    distanceToRimFt: distFeet(pose.x, pose.y, rim.x, rim.y),
  };
}

/**
 * Pure present-tense perception. It derives identity from the authoritative
 * held ball when available, otherwise from the catalog holder while callers
 * finish synchronizing a receipt in the same tick.
 */
export function perceiveLiveCourt(state: GameState): LiveCourtSense | null {
  const offense = state.possession.team;
  const handler = state.ballMotion.status === 'held'
    ? state.ballMotion.holderId
    : state.ballMotion.status === 'pass'
      ? state.ballMotion.passerId ?? state.ball.holderId
      : state.ballMotion.status === 'inbound'
        ? state.ballMotion.holderId ?? state.ball.holderId
        : state.ball.status === 'held'
          ? state.ball.holderId
          : null;
  if (!offense || !handler) return null;
  const handlerPose = state.poses[handler];
  if (!handlerPose || handlerPose.team !== offense) return null;
  const defense = otherTeam(offense);
  const attack = offense === 'home' ? state.baskets.away : state.baskets.home;
  const rim = attackRim(offense, state.baskets);
  const ball = state.ballMotion.status === 'held'
    ? { x: state.ballMotion.x, y: state.ballMotion.y }
    : { x: handlerPose.x, y: handlerPose.y };
  const offensePlayers = Object.values(state.poses)
    .filter((pose) => pose.team === offense)
    .map((pose) => sensePlayer(pose, ball, rim));
  const defensePlayers = Object.values(state.poses)
    .filter((pose) => pose.team === defense)
    .map((pose) => sensePlayer(pose, ball, rim));

  let onBallDefender: string | null = null;
  let onBallDistanceFt = Infinity;
  let paintDefenders = 0;
  for (const defender of defensePlayers) {
    if (defender.distanceToBallFt < onBallDistanceFt) {
      onBallDistanceFt = defender.distanceToBallFt;
      onBallDefender = defender.jersey;
    }
    if (defender.distanceToRimFt <= 10) paintDefenders += 1;
  }

  const openTeammates = offensePlayers
    .filter((player) => player.jersey !== handler)
    .filter((mate) => {
      const closestDefender = defensePlayers.reduce(
        (closest, defender) => Math.min(closest, distFeet(mate.pose.x, mate.pose.y, defender.pose.x, defender.pose.y)),
        Infinity,
      );
      return closestDefender >= 6;
    })
    .map((player) => player.jersey);

  return {
    offense,
    defense,
    handler,
    attack,
    baskets: state.baskets,
    attackDirection: attack === 'right' ? 1 : -1,
    rim,
    ball,
    ballZone: zoneFromPoint(ball.x, ball.y, offense, state.baskets),
    shotClock: state.clocks.shot,
    gameClock: state.clocks.game,
    scoreDiff: state.score.home - state.score.away,
    period: state.clocks.period,
    offensePlayers,
    defensePlayers,
    abilities: state.abilities,
    playerData: state.playerData,
    coach: state.coach,
    chemistry: state.chemistry,
    moraleExec: state.moraleExec,
    onBallDefender,
    onBallDistanceFt,
    paintDefenders,
    openTeammates,
    catchWindowSeconds: 0,
    passTarget: state.ballMotion.status === 'pass' ? state.ballMotion.receiverId : null,
    // The previous pass flight's passerId — the player the current holder
    // just received from. ballMotion retains passerId only while the ball
    // is in flight, so the return-pass guard needs the holder-relative
    // derivation: when the ball is HELD, the receiver of the last flight
    // is the current holder, and the last flight's passerId is the
    // previous passer.
    lastPasserId: state.ballMotion.status === 'held'
      ? state.ballMotion.passerId ?? null
      : null,
  };
}
