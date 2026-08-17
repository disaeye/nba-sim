import type { LineupPackage, RoleBinding } from '../identity/types.js';
import type { GameState, TeamId } from '../state/types.js';
import type { ScreenExecution, TeamPlan } from '../tactics/team-plan.js';
import { buildTeamPlan } from '../tactics/team-plan.js';
import { perceiveLiveCourt } from '../perception/live-court.js';
import { planSpatialTargets, intentsFromSpatialPlan } from '../spatial/team-planner.js';
import { staminaFactor } from '../stamina.js';
import type { Intent } from './types.js';
import { retargetPayloadFromResolved } from '../court/relations.js';
import type { Rng } from '../rng/types.js';

export interface CoordinatedDecision {
  readonly plan: TeamPlan | null;
  readonly intents: readonly Intent[];
  readonly ballIntent: Intent;
  readonly retargetPayload: Record<string, unknown>;
  readonly onBallDefender: string | null;
}

export interface CoordinatedDecisionResult extends CoordinatedDecision {
  readonly events: readonly [];
  readonly possessionEnded: false;
  readonly endReason: 'UNKNOWN';
}

export interface DecisionStepInput {
  readonly state: GameState;
  readonly packages: Record<TeamId, LineupPackage>;
  readonly binding: RoleBinding;
  readonly possessionPhase: 'ADVANCE' | 'SETUP' | 'EXECUTE';
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly previousPlan?: TeamPlan | null;
  readonly screenReady?: boolean;
  readonly catchWindowTicks?: number;
  readonly screenExecution?: ScreenExecution | null;
  readonly rng?: Rng | null;
  /** Ticks since EXECUTE began — gates the immediate shot (P3.x). */
  readonly phaseTicksRemaining?: number;
  /** Per-player stamina map — folded into the sense as exec factors. */
  readonly stamina?: Readonly<Record<string, { readonly max: number; readonly stm: number }>> | null;
  /** Recent DHO pair guard; blocks an immediate reverse exchange. */
  readonly handoffGuard?: { readonly giverId: string; readonly receiverId: string } | null;
  /** Active play id — the play's family becomes the tactic while active. */
  readonly playId?: string | null;
  /** Recent first-class handling beat, if any. */
  readonly lastHandlingAction?: import('./types.js').HandlingActionKind | null;
  /** Ticks before another handling beat may be selected. */
  readonly handlingActionCooldownTicks?: number;
  /** Committed pass/handoff in flight — plan display must not re-argmax. */
  readonly committedExchange?: { readonly fromJersey: string; readonly toJersey: string } | null;
  /** Live drive lifecycle for the handler (continuation gate). */
  readonly driveCommitment?: { readonly jersey: string } | null;
  /** Consecutive passes without an attacking action in between. */
  readonly passChainSinceAttack?: number;
}

function emptyResult(): CoordinatedDecisionResult {
  const ballIntent: Intent = {
    jersey: '', team: 'home', kind: 'idle', targetJersey: null,
    zone: 'frontcourt_center', task: 'idle', score: 0,
    targetX: 0.5, targetY: 0.5, slot: null, satisfied: true,
  };
  return {
    plan: null,
    intents: [],
    ballIntent,
    retargetPayload: { context: 'dead', offense: null, players: [] },
    onBallDefender: null,
    events: [],
    possessionEnded: false,
    endReason: 'UNKNOWN',
  };
}

/** One coordinated live decision: choose one team branch, then solve all targets. */
export function runDecisionStep(input: DecisionStepInput): CoordinatedDecisionResult {
  const sense = perceiveLiveCourt(input.state);
  if (!sense) return emptyResult();
  // §8.4: fold per-player stamina exec factors into the sense so the EV
  // prices fatigue exactly like resolve will resolve it. P6.2: the
  // team morale exec multiplier rides the same channel.
  const staminaSense = input.stamina
    ? {
        ...sense,
        staminaFactors: Object.fromEntries(
          Object.entries(input.stamina).map(([jersey, st]) => {
            const team = input.state.lineups.home.includes(jersey) ? 'home' : 'away';
            const morale = input.state.moraleExec[team] ?? 1;
            return [jersey, staminaFactor(st.stm) * morale];
          }),
        ),
      }
    : sense;
  const catchWindowTicks = input.catchWindowTicks ?? 0;
  const catchSense = catchWindowTicks > 0
    ? { ...staminaSense, catchWindowSeconds: catchWindowTicks / 10 }
    : staminaSense;
  // P1.2: decision-time shot-method classification — a catch-window shot
  // is catch_shoot, a drive-intent handler is drive_finish, a post
  // assignment is post, everything else from the handler's pocket is
  // pull_up. This must MATCH the adjudicate-side classification
  // (factsFromBallIntent + startShot) so the EV prices the same base
  // rate the resolve will use (one-model discipline).
  const classification: Record<string, 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other'> = {};
  for (const player of catchSense.offensePlayers) {
    if (player.jersey !== catchSense.handler) continue;
    if (catchWindowTicks > 0) classification[player.jersey] = 'catch_shoot';
  }
  // P2.1: fold the raw tendency profile into the sense so the EV layer
  // can price option priors per player (T3 shooter takes threes, PASS1ST
  // point guard passes first). Absent playerData → no prior (neutral).
  const tendencies: Record<string, Partial<Record<'T3' | 'TMID' | 'TDRIVE' | 'TPOST' | 'PASS1ST' | 'GAMBLE' | 'FOUL' | 'TAKEOVER' | 'PUSH', number>>> = {};
  for (const [jersey, data] of Object.entries(input.state.playerData)) {
    tendencies[jersey] = data.tendency;
  }
  const tendSense = Object.keys(tendencies).length > 0
    ? { ...catchSense, tendencies }
    : catchSense;
  // P2.2: awareness-gated perception noise — the "read" layer.
  // A low-OFFR handler misreads the floor: open teammates may be missed
  // (dropped from the pass pool) and a merely-guarded look may look open.
  // This is the source of "decision errors" — the player picks a worse
  // option because their SENSE of the floor is worse, not because the EV
  // math is wrong. DEFR/SPC feed the defense side (P2.3) and spacing.
  const handlerData = input.state.playerData[sense.handler] ?? null;
  const offr = handlerData ? handlerData.awareness.OFFR / 99 : 0.5;
  const noiseSense = (() => {
    if (offr >= 0.7) return tendSense; // high-IQ: trust the perception
    const misreadProb = (0.7 - offr) * 0.5; // 0..0.2 chance to misread
    const rng = input.rng ?? null;
    if (rng === null || misreadProb <= 0) return tendSense;
    if (rng.next() >= misreadProb) return tendSense;
    // Misread: drop a random open teammate from the perceived pool.
    const pool = [...tendSense.openTeammates];
    if (pool.length === 0) return tendSense;
    const drop = pool[Math.floor(rng.next() * pool.length)]!;
    const perceived = tendSense.openTeammates.filter((j) => j !== drop);
    return { ...tendSense, openTeammates: perceived };
  })();
  const lineup = input.state.lineups[sense.offense];
  const lineupPackage = input.packages[sense.offense];
  const defenseLineupPackage = input.packages[sense.defense];
  const plan = buildTeamPlan({
    sense: Object.keys(classification).length > 0
      ? { ...noiseSense, shotTypeForDecision: classification }
      : noiseSense,
    lineup,
    lineupPackage,
    defenseLineupPackage,
    binding: input.binding,
    possessionPhase: input.possessionPhase,
    mode: input.mode,
    previous: input.previousPlan ?? null,
    screenReady: input.screenReady === true,
    rng: input.rng ?? null,
    screenExecution: input.screenExecution ?? null,
    executeCooldownTicks: input.phaseTicksRemaining ?? 0,
    playId: input.playId ?? null,
    handoffGuard: input.handoffGuard ?? null,
    lastHandlingAction: input.lastHandlingAction ?? null,
    driveCommitment: input.driveCommitment ?? null,
    passChainSinceAttack: input.passChainSinceAttack ?? 0,
    committedExchange: input.committedExchange ?? null,
  });
  const spatial = planSpatialTargets(sense, plan);
  const intents = intentsFromSpatialPlan({ spatial, tactical: plan, sense });
  const ballIntent = intents.find((intent) => intent.jersey === sense.handler) ?? intents[0];
  if (!ballIntent) return emptyResult();
  // The tactical plan is the public continuity object. Keep the route on the
  // same plan that drives the retarget payload so the snapshot and the
  // physical targets describe one decision, rather than two geometries.
  const planWithRoutes: TeamPlan = { ...plan, routes: spatial.routes };
  return {
    plan: planWithRoutes,
    intents,
    ballIntent,
    retargetPayload: retargetPayloadFromResolved(spatial.players.map((player) => ({
      ...player,
      hasBall: player.jersey === sense.handler,
      satisfied: false,
      zone: intents.find((intent) => intent.jersey === player.jersey)?.zone ?? 'frontcourt_center',
    }))),
    onBallDefender: spatial.onBallDefender,
    events: [],
    possessionEnded: false,
    endReason: 'UNKNOWN',
  };
}
