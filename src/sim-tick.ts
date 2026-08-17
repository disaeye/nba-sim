/**
 * One 0.1s simulation tick — single-axis causal chain:
 *   clocks → decide(intents) → step world → completion facts → adjudicate
 * docs/foundation/architecture.md
 *
 * v0.6.0 — dwell model + progress watchdog (anti-hot-potato, anti-hold-freeze).
 * stepBall no longer pre-commits steals (ball-motion.ts single-point-commit fix).
 */
import type { GameState, TeamId, Event, EventType, Phase } from './state/types.js';
import { applyEvent } from './state/apply.js';
import { stepAllPoses, setTarget, makePose, type PoseMap, type PoseState } from './court/poses.js';
import { rimNorm, distFeet, isInPaint } from './court/geometry.js';
import type { BasketSide } from './court/alignment.js';
import { applyLocalPerception } from './perception/local-react.js';
import {
  stepBall,
  ballLoose,
  ballHeld,
  startPass,
  type BallMotionState,
} from './court/ball-motion.js';
import { loadMobilityConfig, isValidMovementRole } from './court/mobility.js';
import { perceiveLiveCourt } from './perception/live-court.js';
import { planSpatialTargets } from './spatial/team-planner.js';
import { snapshotFromState, type WorldSnapshot } from './world/snapshot.js';
import {
  loadResolveConfig,
  makeResolveContext,
  resolveFt,
} from './resolve/index.js';
import type { Rng } from './rng/types.js';
import type { RoleBinding, LineupPackage } from './identity/types.js';
import type { TacticalSnapshot } from './world/snapshot.js';
import { runDecisionStep } from './decision/step.js';
import type { Intent } from './decision/types.js';
import type { TeamPlan } from './tactics/team-plan.js';
import type { ActiveBallAction } from './actions/lifecycle.js';
import { startBallAction, startsPhysicalBallAction, actionInWindup, progressBallAction, handlingActionFinished, isHandlingAction } from './actions/lifecycle.js';
import { bindRoles } from './identity/bind.js';
import { selectPlay } from './possession/play-select.js';
import { PLAYS_CONFIG } from './sim-utils.js';
import {
  factsFromBallStep,
  factPaintArrival,
  factShotClockExpired,
  factGameClockExpired,
  factScreenSet,
  factScreenUsed,
} from './completion/index.js';
import type { CompletionFact } from './completion/types.js';
import { adjudicateFact, factsFromBallIntent } from './adjudicate/index.js';
import type { EndReason } from './possession/types.js';
import { consumptionRateFor, detectSubRequests, staminaFactor, tickStamina } from './stamina.js';
import type { LiveStamina } from './stamina.js';

const MOBILITY = loadMobilityConfig();
const RESOLVE = makeResolveContext(loadResolveConfig());
export const TICK_DT = MOBILITY.tick_seconds;
/** Memoized tactical renders keyed by the (immutable) TeamPlan reference. */
const tacticalStaticCache = new WeakMap<TeamPlan, TacticalSnapshot>();
/** Minimum ticks (0.1s each) a new holder must wait before redeciding.
 *  12 ticks = 1.2s — anti-hot-potato; NBA catch→release typical: 1–3s.
 *  Note: the catch window (4 ticks) supersedes this — the first post-catch
 *  decision runs inside the window, and a hold decision sets the 40-tick
 *  cooldown, so the dwell only binds when no window decision fires. */
export const DWELL_MIN_TICKS = 28; // 2.8s minimum catch-to-release


export type PossessionPhase = 'ADVANCE' | 'SETUP' | 'EXECUTE';

export interface TickContext {
  readonly packages: Record<TeamId, LineupPackage>;
  readonly binding: RoleBinding | null;
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly playId: string | null;
  readonly rng: Rng;
  pendingFtAttempts: number;
  /** Original FT total (pendingFtAttempts decrements per attempt). */
  ftAttemptTotal: number;
  pendingFtShooter: string | null;
  pendingFtTeam: TeamId | null;
  pendingFtAndOne: boolean;
  forceDecision: boolean;
  /** Legacy intent field pending full lifecycle cutover. */
  stickyBallIntent: Intent | null;
  /** Current physical ball action; semantic begins are emitted once per id. */
  activeBallAction: ActiveBallAction | null;
  /** Coordinated five-player plan, continuously re-evaluated from perception. */
  teamPlan: TeamPlan | null;
  /** Compatibility flag; derived from screenExecution.phase. */
  screenEstablished: boolean;
  /** Physical pick-and-roll execution; null outside a live PNR. */
  readonly screenExecution: {
    readonly screenerId: string;
    readonly handlerId: string;
    readonly defenderId: string | null;
    readonly phase: 'APPROACH' | 'SET' | 'USE' | 'EXIT';
    readonly anchorX: number;
    readonly anchorY: number;
    readonly handlerStartX: number;
    readonly handlerStartY: number;
  } | null;
  /** All intents from the latest coordinated plan. */
  lastIntents: readonly Intent[];
  /** Last episode end reason surfaced this tick (for simulate possession_log). */
  lastEndReason: EndReason | null;
  lastStartReason: string | null;
  /** Anti-hot-potato: remaining ticks before new holder may redecide. */
  dwellTicksRemaining: number;
  /** Progress watchdog: ticks since last meaningful position change. */
  watchdogStaleTicks: number;
  watchdogSnapshotX: number;
  readonly watchdogSnapshotY: number;
  /** Real-time of the handler's last DRIVE fact — a bounded memory so the
   *  drive continuation survives replan bookkeeping that clears the
   *  lifecycle object mid-drive (recovery/changedAfterBallFacts rebuilds). */
  lastDriveAt: number;
  lastDriveHandler: string | null;
  /**
   * Decision throttle: after committing to a running action (relocate /
   * hold), the handler does not re-evaluate for this many ticks — the
   * tactic gets execution time instead of a 0.1s re-roll. Set to 0 on
   * shoot/drive/pass (immediate execution).
   */
  decisionCooldown: number;
  /**
   * Possession macro-phase — structural time model (foundation 0.6.0).
   *
   * ADVANCE: ball handler must advance past half-court. Only advance + safe
   *   forward passes allowed. Off-ball players move to formation spots.
   *   Auto-transitions to SETUP after crossing + settle ticks.
   *
   * SETUP: team settles into half-court set. Screen setting, off-ball
   *   relocations. No shoot/drive yet — probe and create.
   *   Auto-transitions to EXECUTE after setup ticks.
   *
   * EXECUTE: full offensive options. Drive, shoot, all passes. Late-clock
   *   pressure applies normally.
   *
   * TRANSITION/OREB start → skip to SETUP (ball already in frontcourt).
   */
  possessionPhase: PossessionPhase;
  /** Ticks remaining before the same pair may exchange another DHO. */
  handoffCooldownTicks: number;
  /** Last successful DHO pair, retained until a third-player action breaks it. */
  recentHandoff: { readonly giverId: string; readonly receiverId: string } | null;
  /** Ticks remaining in current phase before auto-advance. */
  phaseTicksRemaining: number;
  stickyScreenJersey: string | null;
  /** Most recent first-class handling beat, exposed for tactical continuity. */
  lastHandlingAction: import('./decision/types.js').HandlingActionKind | null;
  /** Cooldown preventing an immediate repeat of the same handling beat. */
  handlingActionCooldownTicks: number;
  /** Ticks remaining in the catch-and-shoot window after a reception. */
  catchWindowTicks: number;
  /** Consecutive same-team passes with NO attacking action (drive/shoot/
   *  handoff) between them — the pass-chain guard. Resets on any attack,
   *  turnover, or possession change. A called play survives ball reversals;
   *  the CHAIN length is what the EV layer must price, so a third pass in
   *  a row without an attack is a stalled possession, not a probing one. */
  passChainSinceAttack: number;
  /** Live inbound in flight: the inbounder holds at the line, then a real
   *  short pass reaches the receiver (INBOUND_TOUCH/POSSESSION_GAINED on
   *  the catch). Null outside the inbound window. */
  pendingInbound: { readonly inbounder: string; readonly receiver: string; readonly team: TeamId } | null;
  /** Ticks the inbounder holds the ball at the line before passing. */
  pendingInboundTicks: number;
  /** Per-player stamina (§8 of docs/playerdata-design.md). */
  stamina: import('./stamina.js').StaminaMap;
}

export interface TickResult {
  readonly state: GameState;
  readonly snapshot: WorldSnapshot;
  readonly ctx: TickContext;
}

function emitSemantic(
  state: GameState,
  type: EventType,
  actors: readonly string[],
  payload: Record<string, unknown>,
): GameState {
  const event: Event = {
    type,
    t_game: state.clocks.game,
    t_real: state.realClock,
    seq: state.seq + 1,
    actors: [...actors],
    payload,
    clocks: { game: state.clocks.game, shot: state.clocks.shot },
    score: { ...state.score },
  };
  return applyEvent(state, event);
}

function syncBallCatalog(state: GameState, ball: BallMotionState): GameState {
  const status = ball.status;
  return {
    ...state,
    ballMotion: ball,
    ball: {
      holderId: ball.holderId,
      status,
      zone: state.ball.zone,
    },
  };
}

export function reconcileHasBall(poses: PoseMap, holderId: string | null): PoseMap {
  const next: Record<string, (typeof poses)[string]> = {};
  for (const [j, p] of Object.entries(poses)) {
    next[j] = { ...p, hasBall: holderId !== null && j === holderId };
  }
  return next;
}

function withPoses(state: GameState, poses: PoseMap): GameState {
  return {
    ...state,
    poses: reconcileHasBall(poses, state.ballMotion.holderId),
  };
}
/**
 * A decision is made after this tick's movement pass. Without pinning a
 * newly-started terminal action here, the holder keeps integrating the
 * previous relocate/drive target throughout the wind-up; a catch-and-shoot
 * can visibly drift ten feet before the release is adjudicated. Passing and
 * handoff gathers are also planted, while drives retain their live steering.
 */
function pinHolderForBallAction(
 state: GameState,
 holderId: string | null,
 kind: ActiveBallAction['kind'],
 intent?: Intent,
): GameState {
 if (holderId === null) return state;
 const pose = state.poses[holderId];
 if (!pose) return state;
 const isTerminal = kind === 'shoot' || kind === 'pass' || kind === 'handoff';
 const isHandling = isHandlingAction(kind);
 if (!isTerminal && !isHandling) return state;
 return withPoses(state, {
  ...state.poses,
  [holderId]: {
   ...pose,
   targetX: isHandling && intent ? intent.targetX : pose.x,
   targetY: isHandling && intent ? intent.targetY : pose.y,
   vx: isHandling && kind === 'crossover' ? pose.vx : 0,
   vy: isHandling && kind === 'crossover' ? pose.vy : 0,
   arrived: !(isHandling && kind === 'crossover'),
   // Handling beats remain visible as their own physical labels. Terminal
   // actions keep the existing gather/follow-through semantics.
   action: kind,
  },
 });
}


/**
 * A late catch/rebound re-plan can replace the label after the movement pass.
 * Keep the rendered label truthful: a defender more than 18ft from the
 * basket is rotating/tagging, not yet providing active rim help.
 */
function normalizeFinalLiveLabels(state: GameState): GameState {
  if (state.phase !== 'LIVE' || state.ballMotion.status !== 'held') return state;
  const sense = perceiveLiveCourt(state);
  if (!sense) return state;
  let next: Record<string, PoseState> | null = null;
  for (const pose of Object.values(state.poses)) {
    if (pose.action !== 'help' || pose.team !== sense.defense) continue;
    if (distFeet(pose.x, pose.y, sense.rim.x, sense.rim.y) <= 18) continue;
    next ??= { ...state.poses };
    next[pose.jersey] = setTarget(pose, pose.targetX, pose.targetY, 'tag', pose.movementRole);
  }
  return next === null ? state : withPoses(state, next);
}


function separatePayloadPlayers(
  raw: unknown[],
): void {
  const parsed: { i: number; team: string; x: number; y: number }[] = [];
  for (let i = 0; i < raw.length; i++) {
    const row = raw[i];
    if (typeof row !== 'object' || row === null) continue;
    const r = row as Record<string, unknown>;
    const x = typeof r.x === 'number' ? r.x : undefined;
    const y = typeof r.y === 'number' ? r.y : undefined;
    const team = typeof r.team === 'string' ? r.team : undefined;
    if (x === undefined || y === undefined || team === undefined) continue;
    parsed.push({ i, team, x, y });
  }
  for (const tm of ['home', 'away'] as const) {
    const group = parsed.filter((p) => p.team === tm);
    for (let pass = 0; pass < 3; pass++) {
      for (let a = 0; a < group.length; a++) {
        for (let b = a + 1; b < group.length; b++) {
          const pa = group[a]!;
          const pb = group[b]!;
          const dx = pb.x - pa.x;
          const dy = pb.y - pa.y;
          const d01 = Math.hypot(dx, dy);
          const dFt = d01 * 94;
          if (dFt >= 3.5) continue;
          // Identical coordinates: nudge apart so separatePayloadPlayers
          // never creates a zero-distance stack.
          if (d01 < 1e-9) {
            pa.y = Math.max(0.02, Math.min(0.98, pa.y - 0.5 / 94));
            pb.y = Math.max(0.02, Math.min(0.98, pb.y + 0.5 / 94));
            continue;
          }
          const push = (3.5 / 94 - d01) / 2;
          const nx = dx / d01;
          const ny = dy / d01;
          pa.x = Math.max(0.02, Math.min(0.98, pa.x - push * nx));
          pa.y = Math.max(0.02, Math.min(0.98, pa.y - push * ny));
          pb.x = Math.max(0.02, Math.min(0.98, pb.x + push * nx));
          pb.y = Math.max(0.02, Math.min(0.98, pb.y + push * ny));
          const ra = raw[pa.i] as Record<string, unknown>;
          const rb = raw[pb.i] as Record<string, unknown>;
          ra['x'] = pa.x;
          ra['y'] = pa.y;
          rb['x'] = pb.x;
          rb['y'] = pb.y;
        }
      }
    }
  }
}

function applyRetarget(
  state: GameState,
  retargetPayload: Record<string, unknown>,
): GameState {
  let poses = { ...state.poses };
  const rawPlayers = retargetPayload['players'];
  if (Array.isArray(rawPlayers)) {
    separatePayloadPlayers(rawPlayers);
    const holderId = state.ballMotion.holderId;
    const inFlightReceiver = state.ballMotion.status === 'pass' ? state.ballMotion.receiverId : null;
    for (const row of rawPlayers) {
      if (typeof row !== 'object' || row === null) continue;
      const r = row as Record<string, unknown>;
      if (typeof r.jersey !== 'string') continue;
      const prev = poses[r.jersey];
      if (!prev) continue;
      // The in-flight receiver must NOT be re-targeted: the pass aims at
      // their flight-start spot, and a mid-flight retarget walks them
      // off the catch point (the pass is dropped).
      if (r.jersey === inFlightReceiver) continue;
      const rawTx = typeof r.x === 'number' ? r.x : prev.targetX;
      const rawTy = typeof r.y === 'number' ? r.y : prev.targetY;
      let action = typeof r.task === 'string' ? r.task : prev.action;
      const role = typeof r.movementRole === 'string' ? r.movementRole : prev.movementRole;
      if (
        r.jersey === holderId
        && (action === 'on_ball_defend' || action === 'deny' || action === 'help' || action === 'tag' || action === 'weak_side' || action === 'box_out')
      ) {
        action = 'ball_handler';
      }
      // Use the ABSOLUTE formation target. stepPose already limits
      // per-frame displacement to maxSpeed×dt (≈0.8ft/tick for a space
      // action), so a 40ft relocation is a walk, not a teleport — the
      // old MAX_STEP_FT clamp made every retarget a moving target that
      // the spacer could never catch (decision re-rolls every tick when
      // the handler drives, so the clamp point kept receding). Players
      // arrive at their spots over a few seconds of walking.
      poses[r.jersey] = setTarget(prev, rawTx, rawTy, action, isValidMovementRole(role) ? role : prev.movementRole);
    }
  }
  return withPoses(state, poses);
}

function runFacts(
  state: GameState,
  ball: BallMotionState,
  facts: readonly CompletionFact[],
  ctx: TickContext,
): { state: GameState; ball: BallMotionState; ctx: TickContext; lastType: string | null; lastSeq: number | null } {
  let s = state;
  let b = ball;
  let c = ctx;
  let lastType: string | null = null;
  let lastSeq: number | null = null;

  for (const fact of facts) {
    const adj = adjudicateFact(s, b, fact, c.rng, c.stickyBallIntent, c.stamina);
    s = adj.state;
    b = adj.ball;
    s = syncBallCatalog(s, b);
    if (fact.kind === 'ScreenSet') {
      // A screen can be discovered at the end of the same tick in which the
      // handler's previous decision started a pass wind-up. If that pass is
      // headed to an ordinary spacer, letting the wind-up finish turns the
      // newly established PNR into a premature swing before SCREEN_USE.
      // The physical pass has not started yet (a set fact requires a held
      // ball), so cancel only this incompatible pending pass; a pocket pass
      // to the screener remains a legal blitz/read.
      const pendingSwing = c.activeBallAction?.kind === 'pass'
        && c.activeBallAction.jersey === fact.ballHandlerId
        && c.activeBallAction.intent.targetJersey !== fact.screenerId
        && c.activeBallAction.stage !== 'COMPLETED'
        && c.activeBallAction.stage !== 'ABORTED';
      c = {
        ...c,
        activeBallAction: pendingSwing ? null : c.activeBallAction,
        stickyBallIntent: pendingSwing ? null : c.stickyBallIntent,
        screenEstablished: true,
        screenExecution: {
          screenerId: fact.screenerId,
          handlerId: fact.ballHandlerId,
          defenderId: fact.defenderId,
          phase: 'SET',
          anchorX: fact.anchorX,
          anchorY: fact.anchorY,
          handlerStartX: fact.handlerX,
          handlerStartY: fact.handlerY,
        },
        teamPlan: c.teamPlan ? { ...c.teamPlan, screenActive: true } : c.teamPlan,
      };
    }
    if (fact.kind === 'IntentDriveBegun') {
      c = { ...c, lastDriveAt: s.realClock, lastDriveHandler: fact.ballHandlerId };
    }
    s = withPoses(s, s.poses);
    if (adj.clearBallIntent) {
      c = { ...c, stickyBallIntent: null };
      c = { ...c, activeBallAction: null };
    }
    if (adj.forceDecision) {
      c = { ...c, forceDecision: true };
    }
    if (adj.pendingFt && adj.pendingFt.attempts > 0) {
      c = {
        ...c,
        pendingFtAttempts: adj.pendingFt.attempts,
        ftAttemptTotal: adj.pendingFt.attempts,
        pendingFtShooter: adj.pendingFt.shooterId,
        pendingFtTeam: adj.pendingFt.team,
        pendingFtAndOne: adj.pendingFt.andOne,
      };
    }
    if (adj.episode.kind === 'end') {
      c = {
        ...c,
        lastEndReason: adj.episode.endReason,
        lastStartReason: adj.episode.nextStartReason,
      };
    } else if (adj.episode.kind === 'continue_oreb') {
      c = {
        ...c,
        lastEndReason: 'MISS_OREB_CONTINUE',
        lastStartReason: 'AFTER_OREB_CONTINUE',
      };
    }
    if (adj.events.length > 0) {
      lastSeq = s.seq;
      lastType = s.events[s.events.length - 1]?.type ?? null;
    }
    // Update pose snapshot for any ball-receipt fact (new handler)
    if (
      adj.events.length > 0 &&
      (fact.kind === 'BallArrivedAtReceiver' ||
        fact.kind === 'HandoffExchange' ||
        fact.kind === 'PassIntercepted' ||
        fact.kind === 'HolderStripped' ||
        fact.kind === 'LooseRecovered')
    ) {
      const holder = s.ballMotion.holderId;
      if (holder && s.poses[holder]) {
        c = {
          ...c,
          watchdogStaleTicks: 0,
          watchdogSnapshotX: s.poses[holder]!.x,
          watchdogSnapshotY: s.poses[holder]!.y,
        };
      }
    }
    // Start dwell timer on ball receipt. The dwell is an anti-hot-potato
    // minimum, but it must never exceed the remaining shot clock — a catch
    // with 2s left is an immediate shot in the NBA, not a 2.8s freeze.
    if (fact.kind === 'HandoffExchange' && adj.events.some((event) => event.type === 'HANDOFF')) {
      c = {
        ...c,
        possessionPhase: 'EXECUTE',
        phaseTicksRemaining: 0,
        handoffCooldownTicks: 12,
        recentHandoff: { giverId: fact.giverId, receiverId: fact.receiverId },
      };
    }
    if (fact.kind === 'BallArrivedAtReceiver' || fact.kind === 'HandoffExchange') {
      c = {
        ...c,
        dwellTicksRemaining: Math.max(
          2,
          Math.min(DWELL_MIN_TICKS, Math.round(s.clocks.shot * 10) - 8),
        ),
      };
    }
  }
  return { state: s, ball: b, ctx: c, lastType, lastSeq };
}

/** The handler's live drive lifecycle, if any — the decision layer reads
 *  it to continue a drive inside 14ft instead of re-argmaxing it away. */
function liveDriveFor(state: GameState, ctx: TickContext): { readonly jersey: string } | null {
  const active = ctx.activeBallAction;
  const holder = state.ballMotion.holderId;
  if (!holder) return null;
  if (active && active.kind === 'drive' && active.jersey === holder
    && (active.stage === 'INITIATED' || active.stage === 'PROGRESSING')) {
    return { jersey: holder };
  }
  // Bounded memory: replan bookkeeping clears the lifecycle mid-drive; the
  // continuation still applies for 1.6s (the physical window of a live
  // drive). Beyond that the drive is over — no resurrection, no loop.
  if (ctx.lastDriveHandler === holder && state.realClock - ctx.lastDriveAt <= 1.6) {
    return { jersey: holder };
  }
  return null;
}

function applyLiveDecision(
  state: GameState,
  ctx: TickContext,
): { state: GameState; ctx: TickContext; lastType: string | null; lastSeq: number | null } {
  if (!ctx.binding || state.possession.team === null) {
    return { state, ctx, lastType: null, lastSeq: null };
  }
  const result = runDecisionStep({
    state,
    packages: ctx.packages,
    binding: ctx.binding,
    possessionPhase: ctx.possessionPhase,
    mode: ctx.mode,
    previousPlan: ctx.teamPlan,
    screenReady: ctx.screenEstablished,
    rng: ctx.rng,
    screenExecution: ctx.screenExecution,
    catchWindowTicks: ctx.catchWindowTicks ?? 0,
    stamina: ctx.stamina,
    phaseTicksRemaining: ctx.phaseTicksRemaining,
    playId: ctx.playId,
    handoffGuard: ctx.handoffCooldownTicks > 0 ? ctx.recentHandoff : null,
    lastHandlingAction: ctx.lastHandlingAction,
    handlingActionCooldownTicks: ctx.handlingActionCooldownTicks,
    committedExchange: ctx.activeBallAction
      && (ctx.activeBallAction.kind === 'pass' || ctx.activeBallAction.kind === 'handoff')
      && ctx.activeBallAction.stage !== 'COMPLETED'
      && ctx.activeBallAction.stage !== 'ABORTED'
      ? {
        fromJersey: ctx.activeBallAction.jersey,
        toJersey: ctx.activeBallAction.intent.targetJersey ?? '',
      }
      : null,
    driveCommitment: liveDriveFor(state, ctx),
    passChainSinceAttack: ctx.passChainSinceAttack,
  });

  const active = ctx.activeBallAction;
  const holderId = state.ballMotion.holderId;
  let s = applyRetarget(state, result.retargetPayload);
  const holder = s.ballMotion.holderId;
  // Replans still run while an action winds up so defenders/off-ball players
  // can react, but they must not reclaim the terminal holder's body target.
  if (active && active.jersey === holder && active.stage !== 'COMPLETED' && active.stage !== 'ABORTED') {
    s = pinHolderForBallAction(s, holder, active.kind, active.intent);
  }
  // Advance is also a terminal physical action: its target must survive a
  // decision re-plan until the handler reaches the next incremental lane.
  // Without this guard, the planner's transition target is written over the
  // advance target in the same tick that emits ADVANCE_BACKCOURT.
  if (active?.kind === 'advance' && active.jersey === holder
    && active.stage !== 'COMPLETED' && active.stage !== 'ABORTED') {
    const current = s.poses[holder];
    const prior = state.poses[holder];
    if (current && prior && prior.action === 'advance' && prior.targetX !== prior.x) {
      s = withPoses(s, { ...s.poses, [holder]: setTarget(current, prior.targetX, prior.targetY, 'advance', prior.movementRole) });
    }
  }
  let lastType: string | null = null;
  let lastSeq: number | null = null;
  // paint. A fresh pass may still interrupt at any point for a kick-out, but
  // allowing a shoot decision to cancel a drive from 30–50ft produced a
  // physically incoherent sequence: DRIVE followed by a long two before the
  // handler had covered even half the lane. NBA handlers can abort into a
  // pull-up from the elbow, not from the starting pocket.
  const activeHandlerPose = active && holder ? s.poses[holder] : null;
  const attackBasket = s.possession.team === 'home' ? s.baskets.away : s.baskets.home;
  const activeHandlerRimDistance = activeHandlerPose
    ? distFeet(activeHandlerPose.x, activeHandlerPose.y, rimNorm(attackBasket).x, rimNorm(attackBasket).y)
    : Infinity;
  // Inside 14ft a committed drive is past the point of no return: the
  // pull-up abort (shoot override) is OFF — the paint-arrival fact or the
  // adjudicator's wall-read resolves the drive. This mirrors the
  // decision layer's continuation gate; without it the override still
  // killed every deep drive at 12-14ft (0 rim finishes across 200+ drives).
  const canAbortDriveForPullUp = active?.kind !== 'drive'
    || (activeHandlerRimDistance <= 18 && activeHandlerRimDistance >= 17);
  // A pass/handoff commitment is a CALL: once the exchange is in flight
  // (INITIATED/PROGRESSING), the passer cannot re-argmax the target every
  // 0.1s. The old overrideSticky let a later re-read replace the pass
  // target mid-windup (measured: pass>12 → pass>13 → pass>11 across three
  // consecutive decisions while the ball never moved; the final throw went
  // to a fourth man). Real players commit the pass at the release decision.
  // A drive keeps its pass-kickout abort (the wall read is a genuine
  // in-flight development), and a shoot abort stays subject to the
  // pull-up distance gate below.
  const overrideSticky = active
    && active.jersey === holder
    && active.stage === 'PROGRESSING'
    && active.kind === 'drive'
    && (result.ballIntent.kind === 'pass'
      || (result.ballIntent.kind === 'shoot' && canAbortDriveForPullUp));
  const guardBrokenByDifferentHandler = ctx.recentHandoff !== null
    && holderId !== ctx.recentHandoff.giverId
    && holderId !== ctx.recentHandoff.receiverId;
  const handlingActive = active !== null
    && active.jersey === holder
    && isHandlingAction(active.kind)
    && active.stage !== 'COMPLETED'
    && active.stage !== 'ABORTED';
  const plannedIntent = handlingActive
    ? active.intent
    : !overrideSticky
      && active
      && active.jersey === holder
      && active.stage !== 'COMPLETED'
      && active.stage !== 'ABORTED'
      ? active.intent
      : result.ballIntent;
  const stickyIntent = active
    && active.jersey === holderId
    && active.stage !== 'COMPLETED'
    && active.stage !== 'ABORTED'
    ? active.intent
    : result.ballIntent;
  let c: TickContext = {
    ...ctx,
    teamPlan: result.plan,
    lastIntents: result.intents,
    stickyBallIntent: stickyIntent,
    recentHandoff: guardBrokenByDifferentHandler ? null : ctx.recentHandoff,
    handoffCooldownTicks: guardBrokenByDifferentHandler ? 0 : ctx.handoffCooldownTicks,
    handlingActionCooldownTicks: Math.max(0, ctx.handlingActionCooldownTicks - 1),
    forceDecision: false,
  };
  const sameAction = active && active.jersey === holder && active.kind === plannedIntent.kind;
  const canExecute = sameAction && active.stage === 'INITIATED' && !actionInWindup(active, s.realClock);
  if (sameAction && active.stage === 'INITIATED' && actionInWindup(active, s.realClock)) {
    c = { ...c, activeBallAction: progressBallAction(active, s.realClock) };
  } else if (canExecute || startsPhysicalBallAction(c.activeBallAction, plannedIntent, holder)) {
    const nextAction = canExecute ? progressBallAction(active, s.realClock) : startBallAction(plannedIntent, s.realClock);
    c = { ...c, activeBallAction: nextAction };
    // An attacking action (drive/shoot/handoff) resets the pass chain:
    // the possession is progressing toward the called play's finish, so
    // the chain guard must re-arm from zero. A pass keeps the chain.
    if (nextAction.kind === 'drive' || nextAction.kind === 'shoot' || nextAction.kind === 'handoff') {
      c = { ...c, passChainSinceAttack: 0 };
    }
    if (!canExecute && actionInWindup(nextAction, s.realClock)) {
      s = pinHolderForBallAction(s, holder, nextAction.kind, nextAction.intent);
      return { state: s, ctx: c, lastType, lastSeq };
    }
    if (isHandlingAction(nextAction.kind)) {
      const handling = nextAction.kind;
      c = {
        ...c,
        lastHandlingAction: handling,
        handlingActionCooldownTicks: Math.round((nextAction.windupSeconds + nextAction.recoverySeconds) / TICK_DT * 10),
        activeBallAction: nextAction,
        decisionCooldown: 0,
      };
      s = pinHolderForBallAction(s, holder, nextAction.kind, nextAction.intent);
      return { state: s, ctx: c, lastType, lastSeq };
    }
    const facts = factsFromBallIntent(s, plannedIntent, result.intents);
    if (facts.length > 0) {
      const out = runFacts(s, s.ballMotion, facts, c);
      s = syncBallCatalog({ ...out.state, ballMotion: out.ball }, out.ball);
      c = out.ctx;
      lastType = out.lastType;
      lastSeq = out.lastSeq;
    }
  }
  // A screen intent is a physical commitment, not a one-tick suggestion.
  // When the decision cooldown expires while the screener is still walking,
  // the re-plan must retain the same sticky jersey until SCREEN_SET. The old
  // code assigned null whenever `c.stickyScreenJersey` was already set: the
  // guard suppressed discovery of a new intent, then the assignment erased
  // the old one. That left the plan in SCREEN_APPROACH for another cooldown
  // (4s) with no factScreenSet check, producing 8-9s screen approaches.
  const plannedScreen = result.intents.find((intent) =>
    intent.kind === 'screen' && intent.team === (s.possession.team ?? 'home'));
  const retainedScreen = c.stickyScreenJersey
    && c.stickyScreenJersey !== holder
    && plannedScreen?.jersey === c.stickyScreenJersey
    ? c.stickyScreenJersey
    : null;
  const nextScreen = c.possessionPhase === 'ADVANCE' || c.screenExecution
    ? null
    : retainedScreen ?? (plannedScreen && plannedScreen.jersey !== holder ? plannedScreen.jersey : null);
  c = { ...c, stickyScreenJersey: nextScreen };
  return { state: s, ctx: c, lastType, lastSeq };
}

/**
 * Advance the world by one tick (default 0.1s).
 */
export function stepSimulationTick(
  state: GameState,
  ctx: TickContext,
): TickResult {
  const dt = TICK_DT;
  let s: GameState = {
    ...state,
    realClock: Math.round((state.realClock + dt) * 10) / 10,
  };
  let c: TickContext = {
    ...ctx,
    lastEndReason: null,
    lastStartReason: null,
  };

  if (s.phase === 'LIVE') {
    s = {
      ...s,
      clocks: {
        period: s.clocks.period,
        game: Math.max(0, Math.round((s.clocks.game - dt) * 10) / 10),
        // The 24s clock does not run while the inbound is live (the
        // inbounder holds at the line, then the ball flies to the receiver);
        // it starts on the catch, like real basketball.
        shot: c.pendingInbound
          ? s.clocks.shot
          : Math.max(0, Math.round((s.clocks.shot - dt) * 10) / 10),
      },
    };
  }

  // ── Possession phase machine — structural time model ───────────────────
  // Each macro-phase enforces realistic time commitment:
  //   ADVANCE → SETUP after crossing half-court + settle
  //   SETUP → EXECUTE after setup ticks expire
  //   EXECUTE persists until possession ends
  if (s.phase === 'LIVE' && c.possessionPhase !== 'EXECUTE') {
    // v0.7.2: Use HOLDER's actual position, not ball.zone label.
    // ball.zone can be stale (set at event time, not updated each tick).
    const holder = s.ballMotion.holderId;
    const holderPose = holder ? s.poses[holder] : null;
    const attackSide = s.possession.team === 'home' ? s.baskets.away : s.baskets.home;
    const inFrontcourt = holderPose
      ? (attackSide === 'right' ? holderPose.x >= 0.5 : holderPose.x <= 0.5)
      : (s.ball.zone !== 'backcourt' && s.ball.zone !== null);
    if (c.possessionPhase === 'ADVANCE' && inFrontcourt) {
      // Crossing half-court is a macro-phase boundary: the halfcourt set
      // must be organized NOW, not after the advance decision's 4s
      // cooldown expires. Without forceDecision the kind stays
      // TRANSITION_PUSH for 3-4s after the handler is already in the
      // frontcourt (measured: 43% of live ticks are TRANSITION_PUSH, half
      // of which are post-crossing phantom transition where the handler
      // stands at 24-25ft doing nothing). Forcing the re-plan selects the
      // real halfcourt tactic (PNR/ISO/HANDOFF) immediately.
      // Don't interrupt a live shoot windup: the phase transition is
      // deferred until the shot completes. Starting a new tactical plan
      // mid-windup would override the shooter's pinned position.
      const inShootWindup = c.activeBallAction?.kind === 'shoot'
        && c.activeBallAction.stage !== 'COMPLETED'
        && c.activeBallAction.stage !== 'ABORTED';
      if (!inShootWindup) {
        c = { ...c, possessionPhase: 'SETUP', forceDecision: true, decisionCooldown: 0, mode: 'HALFCOURT' };
      }
      // P3.x: organization is TIME-gated for halfcourt possessions —
      // but the gate must scale with the tactic's complexity. Screen
      // tactics (PNR family) need the screen to physically form, so
      // they organize until shot ≤13 (~11s). Non-screen tactics (ISO,
      // HANDOFF, DRIVE_KICK) are instant-attack sets — the handler
      // reads the defence and attacks in 2-4s, not 11s. Without this
      // shorter gate, ISO/HANDOFF handlers froze at 15-25ft for 6+
      // seconds during SET (measured: A20 stuck at 15ft for the entire
      // shot 18.8→12.8 window). TRANSITION possessions skip the gate
      // entirely — the break shoots as soon as the advance reaches the
      // frontcourt.
      const organized = s.clocks.shot <= 13;
      const ready = c.mode === 'TRANSITION'
        ? true
        : organized;
      if (ready) {
        // Enter EXECUTE with a 3s execution cooldown for halfcourt sets;
        // transition keeps no cooldown (the break must finish).
        c = { ...c, possessionPhase: 'EXECUTE', forceDecision: true, phaseTicksRemaining: c.mode === 'TRANSITION' ? 0 : 30 };
      }
    }
  }

  // Live re-target every tick. During ADVANCE the holder's movement target
  // is a physical commitment, not a formation suggestion: a re-plan after
  // an advance event must not replace it with the current position or a
  // half-court relocate target before the ball has crossed midcourt. The
  // old pass-through treated every `c.possessionPhase === ADVANCE` holder
  // as advance but still used `player.x/player.y` (the planner target),
  // which turned a newly emitted ADVANCE_BACKCOURT into a pass/space stop.
  // Keep the existing advance target while a held advance action is active;
  // a pass or turnover still exits this branch through ball-state changes.
  if (s.phase === 'LIVE' && c.teamPlan && c.binding
    && (s.ballMotion.status === 'held' || s.ballMotion.status === 'pass' || s.ballMotion.status === 'inbound')
    // during advance, lane runners need continuous retargets while the handler
    // dribbles up (the freeze kept all five in their inbound backcourt spots
    // for the entire 50ft advance — 0 outlet passes, 0 fast breaks); during a
    // DRIVE, off-ball spacers must clear the drive lane (the freeze kept them
    // walking INTO the path with their defenders — 47% of drive-side mates,
    // measured). The holder's own target is retained by the advance guard
    // below, so only the teammates' targets change here.
    // Off-ball retarget runs for ALL ball states except an in-windup shot
    // (the shooter's planted position is sacred). The old allow-list
    // (null/advance/drive) froze lane runners during passes — the inbound
    // catch is a pass completion, so the entire transition sprint after
    // a made basket was frozen (measured: mates at x=0.50 through the
    // whole post-catch window).
    && (c.activeBallAction === null || c.activeBallAction.kind !== 'shoot')) {
    const sense = perceiveLiveCourt(s);
    if (sense) {
      const spatial = planSpatialTargets(sense, c.teamPlan);
      const nextPoses = { ...s.poses };
      for (const player of spatial.players) {
        const pose = nextPoses[player.jersey];
        if (!pose) continue;
        // An in-flight receiver is pinned to the point used to launch the
        // pass; moving that target mid-flight turns a legal catch into a miss.
        if (s.ballMotion.status === 'pass' && player.jersey === s.ballMotion.receiverId) continue;
        const isActiveHolderAction = player.jersey === s.ballMotion.holderId
          && c.activeBallAction !== null
          && c.activeBallAction.jersey === player.jersey
          && c.activeBallAction.stage !== 'COMPLETED'
          && c.activeBallAction.stage !== 'ABORTED';
        const isAdvanceHandler = player.jersey === s.ballMotion.holderId && c.possessionPhase === 'ADVANCE';
        const retainAdvanceTarget = isAdvanceHandler && pose.action === 'advance'
          && pose.targetX !== pose.x;
        nextPoses[player.jersey] = isActiveHolderAction
          ? pose
          : retainAdvanceTarget
            ? pose
            : isAdvanceHandler
              ? setTarget(pose, player.x, player.y, 'advance', player.movementRole)
              : setTarget(pose, player.x, player.y, player.task, player.movementRole);
      }
      s = withPoses(s, nextPoses);
    }
  }

  // A loose-ball contest is a temporary recovery assignment. Once a
  // recoverer owns the ball, the next decision must replace every contest
  // label immediately; otherwise the holder can be rendered box-outing while
  // Loose ball: a live ball with no holder. Two distinct basketball
  // situations collapse into this state, discriminated by
  // ballMotion.reboundOffenseTeam:
  //   • Rebound (reboundOffenseTeam !== null): a missed shot. Players near
  //     the rim box out and chase the ball; perimeter players release to
  //     transition lanes. Box-out is ONLY valid here, near the rim.
  //   • Scramble (reboundOffenseTeam === null): a strip, deflection, or
  //     loose ball after a turnover. The nearest 1-2 players dive for the
  //     ball; everyone else spaces out or fills a lane. Nobody box-outs a
  //     ball that was never shot.
  // The old code set EVERY player's target to the ball and labelled them
  // box_out regardless — 5 defenders boxing out a turnover 30ft from the
  // rim (completely inverted: 91% of loose frames had 4+ box-outs, but
  // only 27% of shot-flight frames did).
  //
  // SHOT FLIGHT is also a rebound-preparation window: NBA bigs start
  // boxing out the moment the shot leaves the shooter's hands, not when
  // the ball bounces off the rim. The audit measured 67/145 rebounds with
  // zero box-outs at the rim while the ball was in flight — the shot
  // status left every player in their pre-shot assignment until the ball
  // hit, so the loose-ball transition suddenly stamped box_out on players
  // who had been standing in deny/space posture. Extending the rebound
  // preparation to shot flight makes the box-out read as a deliberate
  // early action instead of a teleported label.
  if (s.phase === 'LIVE'
    && (s.ballMotion.status === 'loose' || s.ballMotion.status === 'shot')
    && s.ballMotion.shooterId !== null) {
    const isRebound = s.ballMotion.reboundOffenseTeam != null || s.ballMotion.status === 'shot';
    const ballX = s.ballMotion.x;
    const ballY = s.ballMotion.y;
    const attackSide: BasketSide = s.possession.team === 'home' ? s.baskets.away : s.baskets.home;
    const rim = rimNorm(attackSide);
    const posesList = Object.values(s.poses);
    const chaseRadius = isRebound ? 16 : 20;
    const next: Record<string, PoseState> = {};
    for (const team of ['home', 'away'] as const) {
      const sorted = posesList.filter((p) => p.team === team)
      const chasers = sorted.slice(0, 2);
      for (const p of sorted) {
        if (chasers.includes(p)) {
          next[p.jersey] = setTarget(p, ballX, ballY, 'cut');
        } else if (isRebound) {
          const nearRim = distFeet(p.x, p.y, rim.x, rim.y) <= 16;
          next[p.jersey] = nearRim
            ? setTarget(p, p.targetX, p.targetY, 'box_out')
            : setTarget(p, p.targetX, p.targetY, 'space');
        } else {
          next[p.jersey] = setTarget(p, p.targetX, p.targetY, p.action === 'box_out' ? 'space' : p.action);
        }
      }
    }
    if (Object.keys(next).length > 0) s = withPoses(s, { ...s.poses, ...next });
  }

  // Dead-ball walk speed: during any non-live phase, players walk to their
  // spots rather than sprint at full 21 ft/s. JUMP_BALL is live-adjacent
  // (players are set), keep at full scale.
  const isLivePhase = s.phase === 'LIVE' || s.phase === 'JUMP_BALL';
  const tickSpeedScale = isLivePhase ? 1 : 0.3;
  // Zone-active flag: when the committed plan runs a 2-3 zone, the
  // local-react help adjustments must NOT overwrite the zone slots
  // (they pull zone defenders toward the ball handler — the audit found
  // zone defenders shuttling between slots, 72-77% of frames collapsed).
  const zoneActive = c.teamPlan?.screenDefense?.zone === 'ZONE_2_3';
  s = withPoses(s, stepAllPoses(applyLocalPerception(s.poses, s, zoneActive), dt, MOBILITY, tickSpeedScale));
  if (s.phase === 'LIVE' && s.ballMotion.status === 'held' && c.activeBallAction?.kind === 'advance' && s.ballMotion.holderId) {
    const holder = s.poses[s.ballMotion.holderId];
    const attackSide = s.possession.team === 'home' ? s.baskets.away : s.baskets.home;
    const crossedHalf = holder && (attackSide === 'right' ? holder.x >= 0.5 : holder.x <= 0.5);
    if (crossedHalf) c = { ...c, activeBallAction: null };
  }
  if (s.phase === 'LIVE' && c.dwellTicksRemaining > 0) {
    c = { ...c, dwellTicksRemaining: c.dwellTicksRemaining - 1 };
  }
  if (s.phase === 'LIVE' && c.handoffCooldownTicks > 0) {
    const remaining = c.handoffCooldownTicks - 1;
    c = {
      ...c,
      handoffCooldownTicks: remaining,
      recentHandoff: remaining > 0 ? c.recentHandoff : null,
    };
  }
  if (s.phase === 'LIVE' && c.decisionCooldown > 0) {
    c = { ...c, decisionCooldown: c.decisionCooldown - 1 };
  }
  if (s.phase === 'LIVE' && c.phaseTicksRemaining > 0) {
    c = { ...c, phaseTicksRemaining: c.phaseTicksRemaining - 1 };
  }
  if (s.phase === 'LIVE' && c.activeBallAction && isHandlingAction(c.activeBallAction.kind)
    && handlingActionFinished(c.activeBallAction, s.realClock)) {
    c = { ...c, activeBallAction: null, forceDecision: true };
  }
  // ── 体力 (§8): on-court consumption by role, bench rest, sub requests ──
  if (s.phase === 'LIVE') {
    const onCourt = [...s.lineups.home, ...s.lineups.away];
    const rates: Record<string, number> = {};
    const planAssignments = c.teamPlan?.assignments ?? [];
    for (const jersey of onCourt) {
      const assignment = planAssignments.find((a) => a.jersey === jersey);
      const team = jersey === s.lineups.home.find((j) => j === jersey) ? 'home' : 'away';
      rates[jersey] = consumptionRateFor(jersey, assignment?.role, c.packages[team], s.playerData);
    }
    const transitionHandler = c.mode === 'TRANSITION' && c.teamPlan
      ? c.teamPlan.assignments.find((a) => a.role === 'handler')?.jersey ?? null
      : null;
    c = { ...c, stamina: tickStamina({ stamina: c.stamina, dt, rates, transitionHandler, onCourt }) };
    // §8.6 automatic substitution requests (one per crossing).
    const teamOf: Record<string, 'home' | 'away'> = {};
    for (const j of s.lineups.home) teamOf[j] = 'home';
    for (const j of s.lineups.away) teamOf[j] = 'away';
    const requests = detectSubRequests(c.stamina, onCourt, teamOf);
    if (requests.length > 0) {
      const flagged: Record<string, LiveStamina> = { ...c.stamina };
      for (const req of requests) {
        const st = flagged[req.jersey];
        if (!st) continue;
        flagged[req.jersey] = req.reason === 'stamina_forced_sub'
          ? { ...st, forcedRequested: true }
          : { ...st, subRequested: true };
        s = emitSemantic(s, 'STATE_NOTE', [req.jersey], {
          player_id: req.jersey,
          team: req.team,
          stm: Math.round(req.stm * 10) / 10,
          note: req.reason,
        });
      }
      c = { ...c, stamina: flagged };
    }
  }
  if (c.catchWindowTicks > 0) {
    c = { ...c, catchWindowTicks: c.catchWindowTicks - 1 };
  }
  // Live inbound: the inbounder holds the ball at the line for a beat
  // (visible pause), then the inbound is a real short pass — the ball
  // leaves the baseline and flies to the receiver like any pass.
  if (s.phase === 'LIVE' && c.pendingInbound) {
    if (c.pendingInboundTicks > 0) {
      c = { ...c, pendingInboundTicks: c.pendingInboundTicks - 1 };
    } else if ((s.ballMotion.status === 'held' || s.ballMotion.status === 'inbound')
      && s.ballMotion.holderId === c.pendingInbound.inbounder) {
      const from = s.poses[c.pendingInbound.inbounder];
      const toPose = s.poses[c.pendingInbound.receiver];
      const team = s.possession.team ?? 'home';
      if (from && toPose) {
        // Aim at the receiver's station spot, not their current pose: the
        // receiver keeps walking toward the spot during the flight, so a
        // mid-walk aim point would sail past them (pass_missed storm).
        const to = { ...toPose, x: toPose.targetX, y: toPose.targetY };
        // Inbounds are thrown hard: cap the flight at 0.7s so the dead-ball
        // window stays short and the spectator sees a crisp line-drive.
        s = { ...s, ballMotion: { ...startPass({ from, to, team, realNow: s.realClock, inbound: true }), flightDuration: Math.min(0.7, Math.max(0.35, Math.hypot((to.x - from.x) * 94, (to.y - from.y) * 50) / 45)) } };
        s = emitSemantic(s, 'PASS', [c.pendingInbound.inbounder, c.pendingInbound.receiver], {
          passer_id: c.pendingInbound.inbounder,
          receiver_id: c.pendingInbound.receiver,
          note: 'flight_start',
        });
      }
    }
  }
  if (s.phase === 'LIVE' && c.activeBallAction?.kind === 'hold' && c.dwellTicksRemaining <= 0) {
    c = { ...c, activeBallAction: null };
  }

  let lastType: string | null = null;
  let lastSeq: number | null = null;
  // Seq BEFORE this tick's emissions — the delta after all decision/adjudication
  // passes is the complete same-tick event list (SHOT_RESULT+MADE_BASKET_DEAD,
  // REBOUND+LOOSE_BALL_RECOVER, INBOUND_TOUCH+POSSESSION_GAINED collapse when
  // only the last seq is exposed).
  const seqBeforeTick = s.seq;

  // CROSS_HALF is a position fact, not a decision artifact: the holder
  // crosses mid-court exactly once per possession. The old emission
  // depended on an IntentAdvance decision landing on the crossing tick,
  // which a 0.1s sampler can miss entirely (measured 0 CROSS_HALF in a
  // full game) — the PlayWalker's ADVANCE_BACKCOURT step then never
  // advanced. The ball.zone flip guards once-per-possession emission.
  if (s.phase === 'LIVE' && s.ballMotion.status === 'held' && s.ballMotion.holderId && c.possessionPhase === 'ADVANCE') {
    const holder = s.poses[s.ballMotion.holderId];
    const attackSide = s.possession.team === 'home' ? s.baskets.away : s.baskets.home;
    const crossedHalf = holder !== undefined && holder !== null
      && (attackSide === 'right' ? holder.x >= 0.5 : holder.x <= 0.5);
    if (crossedHalf && s.ball.zone !== 'frontcourt_center') {
      s = emitSemantic(s, 'CROSS_HALF', [s.ballMotion.holderId], {
        ballHandlerId: s.ballMotion.holderId,
      });
      s = { ...s, ball: { ...s.ball, zone: 'frontcourt_center' } };
      lastSeq = s.seq;
      lastType = 'CROSS_HALF';
    }
    if (crossedHalf && c.activeBallAction?.kind === 'advance') {
      c = { ...c, activeBallAction: null };
    }
  }

  // A physical ball action started inside the catch window can outlive the
  // window (shoot windup 0.4s vs a 4-tick window). The dwell timer would
  const pendingAction = c.activeBallAction;
  const stuckWindupDone = pendingAction !== null
    && pendingAction.stage === 'INITIATED'
    && pendingAction.jersey === s.ballMotion.holderId
    && !actionInWindup(pendingAction, s.realClock);
  const shouldDecide = s.phase === 'LIVE' &&
    c.binding !== null &&
    s.possession.team !== null &&
    s.ballMotion.status === 'held' &&
    // forceDecision (catch, screen set/used, phase transitions) must
    // bypass BOTH the dwell pause and the cooldown — otherwise a catch
    // waits 0.4s of dwell + up to 4s of cooldown before the defense
    // re-allocates, and the new handler shoots wide open (measured:
    // catch-and-shoot at t=46.6 with the closest defender 17ft away).
    (c.dwellTicksRemaining <= 0 || c.catchWindowTicks > 0 || stuckWindupDone || c.forceDecision) &&
    (c.decisionCooldown <= 0 || c.forceDecision) &&
    // The inbound is a fixed choreography: the inbounder holds at the
    // line, then the pending-inbound pass fires. No free decisions in
    // that window — otherwise the decision layer races the inbound pass
    // and the receiver/ball-flight bookkeeping diverges.
    c.pendingInbound === null;

  if (shouldDecide && c.binding && s.possession.team) {
    const teamBefore = s.possession.team;
    const out = applyLiveDecision(s, c);
    s = out.state;
    c = out.ctx;
    // The first decision pass emits its own semantic events (SHOT_RELEASE,
    // DRIVE, PASS start, ...) via runFacts — those must land on this tick's
    // snapshot lastEventSeq, or the frame audit can never locate the event
    // (it previously saw 5 of ~200 shot releases). The team-change
    // re-decision below already captured out2; the main pass did not.
    if (out.lastSeq !== null) {
      lastSeq = out.lastSeq;
      lastType = out.lastType;
    }
    // Decision throttle: a committed running action (relocate/hold) gets
    // ~4s of execution time before the next evaluation — the tactic is
    // allowed to develop instead of being re-rolled every 0.1s. Terminal
    // actions (shoot/drive/pass) reset the cooldown so they execute
    // immediately. P3.7: a pump_fake decision also resets the cooldown —
    // the fake is a beat that baits the closeout; the NEXT evaluation
    // (with the defender committed) prices the drive higher.
    const committed = out.ctx.teamPlan?.assignments.find(
      (a) => a.jersey === s.ballMotion.holderId && (a.action === 'relocate' || a.action === 'hold'),
    );
    const faked = out.ctx.teamPlan?.assignments.find(
      (a) => a.jersey === s.ballMotion.holderId && a.action === 'pump_fake',
    );
    // A physical terminal action owns the next few tenths of a second.  A
    // re-plan can change the visible assignment to relocate/hold while a
    // pass or handoff is still in its windup; applying the 4s committed-plan
    // cooldown here leaves the ball held with an INITIATED action for several
    // seconds before it finally releases.  Preserve the action lifecycle and
    // let it complete before throttling a later relocate/hold decision.
    const physicalActionInProgress = out.ctx.activeBallAction !== null
      && out.ctx.activeBallAction.jersey === s.ballMotion.holderId
      && out.ctx.activeBallAction.stage !== 'COMPLETED'
      && out.ctx.activeBallAction.stage !== 'ABORTED'
      && (out.ctx.activeBallAction.kind === 'pass'
        || out.ctx.activeBallAction.kind === 'handoff'
        || out.ctx.activeBallAction.kind === 'drive'
        || out.ctx.activeBallAction.kind === 'shoot'
        || out.ctx.activeBallAction.kind === 'advance');
    c = { ...c, decisionCooldown: physicalActionInProgress ? 0 : committed && !faked ? 40 : 0 };
    if (s.phase === 'LIVE' && s.possession.team && s.possession.team !== teamBefore) {
      const newTeam = s.possession.team;
      const play = selectPlay('TRANSITION', c.rng, PLAYS_CONFIG);
      const binding = bindRoles(c.packages[newTeam], play);
      c = {
        ...c,
        binding,
        mode: 'TRANSITION',
        playId: play.id,
        forceDecision: true,
        stickyBallIntent: null,
        dwellTicksRemaining: 0,
        decisionCooldown: 0,
        activeBallAction: null,
        teamPlan: null,
        screenEstablished: false,
        handoffCooldownTicks: 0,
        recentHandoff: null,
        possessionPhase: 'ADVANCE' as const,
        phaseTicksRemaining: 0,
      };
      const out2 = applyLiveDecision(s, c);
      s = out2.state;
      c = out2.ctx;
      if (out2.lastSeq !== null) {
        lastSeq = out2.lastSeq;
        lastType = out2.lastType;
      }
    }
  }

  // Continuous motion completion: stepBall commits deterministic receipts
  // (pass_complete, shot_arrived); holder changes are synced immediately.
  const holderBefore = s.ballMotion.holderId;
  const teamBeforeBall = s.possession.team;
  const ballBeforeStep = s.ballMotion;
  let replannedAfterBallChange = false;
  const interceptTick = Math.round(s.realClock * 10) % 5 === 0;
  const ballStep = stepBall({
    ball: s.ballMotion,
    poses: s.poses,
    dt,
    // The inbound flight is sacred: no intercept/strip candidates while
    // the ball is being thrown in from the line (defenders may not reach
    // over), otherwise every inbound spawns reach-in foul attempts.
    allowIntercept: s.phase === 'LIVE' && interceptTick && c.pendingInbound === null,
  });
  s = syncBallCatalog(s, ballStep.ball);
  s = withPoses(s, s.poses);

  if (ballStep.events.length > 0) {
    const ballFacts = factsFromBallStep(ballStep.events, s, ballBeforeStep);
    const out = runFacts(s, s.ballMotion, ballFacts, c);
    s = out.state;
    c = out.ctx;
    // A fresh catch opens the catch-and-shoot window: for the next few
    // ticks the handler's look is priced at open (the closeout is still
    // in flight), so swing passes become real shots instead of the
    // start of another pass chain.
    const arrived = ballFacts.some((fact) => fact.kind === 'BallArrivedAtReceiver');
    // Pass-chain accounting: every completed same-team pass without an
    // attacking action in between lengthens the chain. The EV layer prices
    // the chain so a third consecutive pass must beat an attack to win —
    // real offenses reverse the ball once or twice, then somebody attacks.
    if (arrived) {
      c = { ...c, passChainSinceAttack: c.passChainSinceAttack + 1 };
    }
    // Catch-and-shoot window only for live passes; an inbound reception
    // (pendingInbound set) is not an open look — the defense is set.
    if (arrived && !c.pendingInbound) {
      c = { ...c, catchWindowTicks: 4 };
    }
    // The inbound completes on the catch: the receiver touching the ball
    // IS the INBOUND_TOUCH, and possession is confirmed with
    // POSSESSION_GAINED (the dead-ball branch no longer emits them). A
    // FAILED inbound flight (missed/out-of-bounds/intercepted) clears the
    // marker too — otherwise the 24s clock stays frozen while the loose
    // ball is contested and possessions stretch to absurd lengths.
    if (c.pendingInbound && (arrived
      || ballFacts.some((fact) => fact.kind === 'PassMissed'
        || fact.kind === 'PassOutOfBounds'
        || fact.kind === 'PassIntercepted'))) {
      if (arrived) {
        const { inbounder, receiver } = c.pendingInbound;
        // A fresh possession starts a fresh 24s clock: the inbound catch
        // resets shot (real rule; the previous possession's residual shot
        // time must NOT carry into the new one — it made possessions
        // start at shot≈11 and cut the half-court organization window).
        s = {
          ...s,
          clocks: { ...s.clocks, shot: 24 },
        };
        s = emitSemantic(s, 'INBOUND_TOUCH', [inbounder, receiver], {
          inbounder_id: inbounder,
          receiver_id: receiver,
        });
        s = emitSemantic(s, 'POSSESSION_GAINED', [receiver], {
          team: c.pendingInbound.team,
          player_id: receiver,
        });
        lastSeq = s.seq;
        lastType = 'POSSESSION_GAINED';
      }
      c = { ...c, pendingInbound: null, pendingInboundTicks: 0 };
      if (process.env.SCDBG) {
        const g = globalThis as unknown as Record<string, number>;
        g['__cleared'] = (g['__cleared'] ?? 0) + 1;
        g['__cleared_arrived'] = (g['__cleared_arrived'] ?? 0) + (arrived ? 1 : 0);
      }
    }
    if (out.lastSeq !== null) {
      lastSeq = out.lastSeq;
      lastType = out.lastType;
    }
  }
  // A live recovery can begin a new possession even when the prior tactical
  // binding is still present. This is common after an offensive rebound or a
  // same-team loose-ball recovery: the old plan describes the previous
  // holder, while the new holder must receive a fresh present-tense read.
  if (s.phase === 'LIVE' && s.ballMotion.status === 'held' && s.ballMotion.holderId && c.binding === null && s.possession.team) {
    const recoveryTeam = s.possession.team;
    const recoveryPlay = selectPlay('TRANSITION', c.rng, PLAYS_CONFIG);
    c = {
      ...c,
      binding: bindRoles(c.packages[recoveryTeam], recoveryPlay),
      mode: 'TRANSITION',
      playId: recoveryPlay.id,
      forceDecision: true,
      possessionPhase: 'ADVANCE',
      phaseTicksRemaining: 0,
      decisionCooldown: 0,
      activeBallAction: null,
      stickyBallIntent: null,
      lastHandlingAction: null,
      handlingActionCooldownTicks: 0,
      stickyScreenJersey: null,
      screenExecution: null,
      teamPlan: null,
      screenEstablished: false,
    };
    const recoveryDecision = applyLiveDecision(s, c);
    s = recoveryDecision.state;
    c = recoveryDecision.ctx;
    if (recoveryDecision.lastSeq !== null) {
      lastSeq = recoveryDecision.lastSeq;
      lastType = recoveryDecision.lastType;
    }
  }
  if (s.phase === 'LIVE' && c.screenExecution?.phase === 'SET') {
    const used = factScreenUsed(s, c.screenExecution);
    if (used) {
      const out = runFacts(s, s.ballMotion, [used], c);
      s = out.state;
      const usedPlan = out.ctx.teamPlan;
      const nextPlan = usedPlan
        ? {
          ...usedPlan,
          stage: 'SCREEN_USE' as const,
          assignments: usedPlan.assignments.map((assignment) => assignment.role !== 'screener'
            ? assignment
            : usedPlan.kind === 'PNR_POP'
              ? { ...assignment, action: 'relocate' as const }
              : usedPlan.kind === 'PNR_ROLL'
                ? { ...assignment, action: 'cut' as const }
                : assignment),
        }
        : null;
      const routedPlan = nextPlan
        ? (() => {
          const sense = perceiveLiveCourt(s);
          if (!sense) return nextPlan;
          const spatial = planSpatialTargets(sense, nextPlan);
          return { ...nextPlan, routes: spatial.routes };
        })()
        : null;
      c = {
        ...out.ctx,
        screenExecution: out.ctx.screenExecution ? { ...out.ctx.screenExecution, phase: 'USE' } : null,
        teamPlan: routedPlan,
      };
      if (out.lastSeq !== null) { lastSeq = out.lastSeq; lastType = out.lastType; }
    }
  }
  if (s.phase === 'LIVE' && c.stickyScreenJersey && !c.screenExecution) {
    const screenFact = factScreenSet(s, c.stickyScreenJersey);
    if (screenFact) {
      const out = runFacts(s, s.ballMotion, [screenFact], c);
      s = out.state;
      c = out.ctx;
      if (out.lastSeq !== null) { lastSeq = out.lastSeq; lastType = out.lastType; }
      c = { ...c, stickyScreenJersey: null };
    }
  }
  if (s.phase === 'LIVE' && s.ballMotion.status === 'held') {
    const paint = factPaintArrival(s, c.stickyBallIntent ?? c.activeBallAction?.intent ?? null);
    if (paint) {
      const out = runFacts(s, s.ballMotion, [paint], c);
      s = out.state;
      c = out.ctx;
      if (out.lastSeq !== null) {
        lastSeq = out.lastSeq;
        lastType = out.lastType;
      }
    }
  }
  const possChanged = s.possession.team !== teamBeforeBall;
  const holderChanged = s.ballMotion.status === 'held'
    && s.ballMotion.holderId !== null
    && s.ballMotion.holderId !== holderBefore;
  if (s.phase === 'LIVE' && s.possession.team && c.packages[s.possession.team] && (possChanged || holderChanged) && !replannedAfterBallChange) {
    const newTeam = s.possession.team;
    const possessionChanged = possChanged;
    if (!possessionChanged) {
      // A SAME-TEAM pass completion is a read INSIDE the called play —
      // not a new possession. The call (playId/mode/binding), the phase
      // machine (possessionPhase), and the team's tactical skeleton
      // (teamPlan, screen lifecycle) all survive the exchange: real NBA
      // teams run a called set through 4-6 ball reversals; only the
      // ball-specific state resets (the new holder gets a fresh read,
      // the hot-potato dwell re-arms). The old branch treated every
      // catch as a turnover-shaped replan: mode forced to TRANSITION,
      // possessionPhase thrown back to ADVANCE, teamPlan null — the
      // call died on its own first pass and the next selection drew a
      // RANDOM tactic once shot ≤13 (measured: 21.9% of consecutive
      // live ticks flipped the five-man assignment signature, median
      // stable run 0.2s; the rendered "execution lines" redrew a new
      // wish every decision).
      c = {
        ...c,
        stickyBallIntent: null,
        decisionCooldown: 0,
        forceDecision: true,
        dwellTicksRemaining: DWELL_MIN_TICKS,
        activeBallAction: null,
        catchWindowTicks: 4,
        lastHandlingAction: null,
        handlingActionCooldownTicks: 0,
        handoffCooldownTicks: 0,
      };
    } else {
      const mode: 'TRANSITION' | 'HALFCOURT' = lastType === 'STEAL' ? 'TRANSITION' : c.mode;
      const play = selectPlay(mode, c.rng, PLAYS_CONFIG);
      const binding = bindRoles(c.packages[newTeam], play);
      c = {
        ...c,
        binding,
        mode,
        playId: play.id,
        forceDecision: true,
        stickyBallIntent: null,
        stickyScreenJersey: null,
        screenExecution: null,
        possessionPhase: 'ADVANCE' as const,
        phaseTicksRemaining: 0,
        decisionCooldown: 0,
        activeBallAction: null,
        teamPlan: null,
        screenEstablished: false,
        handoffCooldownTicks: 0,
        recentHandoff: null,
        passChainSinceAttack: 0,
      };
    }
    const out3 = applyLiveDecision(s, c);
    s = out3.state;
    c = out3.ctx;
    if (out3.lastSeq !== null) {
      lastSeq = out3.lastSeq;
      lastType = out3.lastType;
    }
    replannedAfterBallChange = true;
  }

  // FT sequence over real time — FT_START is emitted by the simulate
  // DEAD_FOUL branch after the players walk to their spots.
  if (s.phase === 'FT_SEQUENCE' && c.pendingFtShooter && c.pendingFtAttempts > 0) {
    if (Math.round(s.realClock * 10) % 8 === 0) {
      const shooterStm = c.stamina[c.pendingFtShooter]?.stm ?? 100;
      const shooterAbilities = s.abilities[c.pendingFtShooter] ?? null;
      const shooterData = s.playerData[c.pendingFtShooter] ?? null;
      const made = resolveFt(c.rng, RESOLVE, {
        staminaModifier: staminaFactor(shooterStm),
        ftAbility: shooterData ? shooterData.ability.FT / 99 : undefined,
        catchShoot: shooterAbilities?.catchShoot,
      }).made;
      // Canonical FT sequence requires attempt_number to advance 1..N.
      // The old hardcoded 1 made every attempt of a 2/3-shot trip report
      // as attempt 1, violating the FT_START→(ATTEMPT→RESULT)+ contract.
      const attemptNumber = c.ftAttemptTotal - c.pendingFtAttempts + 1;
      s = emitSemantic(s, 'FT_ATTEMPT', [c.pendingFtShooter], {
        shooter_id: c.pendingFtShooter,
        attempt_number: attemptNumber,
      });
      s = emitSemantic(s, 'FT_RESULT', [c.pendingFtShooter], {
        shooter_id: c.pendingFtShooter,
        attempt_number: attemptNumber,
        made,
      });
      c.pendingFtAttempts -= 1;
      lastSeq = s.seq;
      lastType = 'FT_RESULT';
      if (c.pendingFtAttempts <= 0) {
        s = emitSemantic(s, 'FT_SEQUENCE_END', [], { next_phase: 'LIVE' });
        const ftTeam = c.pendingFtTeam;
        c.pendingFtShooter = null;
        c.pendingFtTeam = null;
        c.pendingFtAndOne = false;
        c.forceDecision = true;
        c.stickyBallIntent = null;
        if (made) {
          s = { ...s, phase: 'DEAD_MAKE' };
          c.dwellTicksRemaining = 0;
        } else {
          s = { ...s, phase: 'LIVE' };
          if (ftTeam) {
            const other: TeamId = ftTeam === 'home' ? 'away' : 'home';
            const attack = ftTeam === 'home' ? s.baskets.away : s.baskets.home;
            const rim = rimNorm(attack);
            s = {
              ...s,
              possession: { team: other },
              ballMotion: ballLoose({ x: rim.x, y: rim.y }, null, other),
            };
            s = syncBallCatalog(s, s.ballMotion);
            s = withPoses(s, s.poses);
          }
          c.dwellTicksRemaining = 0;
        }
      }
    }
  }

  // Shot clock expiry via completion predicate. The 24s clock does not
  // run during the inbound (it starts on the catch), so an inbound after
  // a make — where the shot clock may still read 0 from the previous
  // possession — must not fire a violation while the ball is being
  // thrown in from the line.
  if (s.phase === 'LIVE' && c.pendingInbound === null) {
    const scv = factShotClockExpired(s);
    if (scv) {
      const out = runFacts(s, s.ballMotion, [scv], c);
      s = out.state;
      c = out.ctx;
      if (out.lastSeq !== null) {
        lastSeq = out.lastSeq;
        lastType = out.lastType;
      }
    }
  }

  // Game clock expiry fact (simulate still owns period router)
  if (s.phase === 'LIVE') {
    const gce = factGameClockExpired(s);
    if (gce) {
      // Leave PERIOD_END emission to simulate outer loop; just mark
      lastType = lastType ?? 'CLOCK_EXPIRY_ADJUDICATION';
    }
  }

  // The tactical planner owns matchup geometry. The local perception layer
  // mirrors the same defender between decisions; do not run a second
  // post-adjudication on-ball snap here, because it overwrites a deny/help
  // target with a handler-relative target and makes off-ball assignments
  // visually lie about who is actually guarding whom.
  // Defense positioning is owned entirely by planSpatialTargets → defenseTarget,
  // which places each defender between their assigned attacker and the basket.
  // The per-tick "defense tracking" override that used to live here dragged
  // every defender toward the ball handler (30% blend), collapsing all five
  // into a vertical line and destroying the matchup structure.

  // The tactical render is stable between decisions (plans are immutable);
  // memoizing it per plan reference removes ~32k allocations per game of
  // the same object. Only the activeAction's elapsed clock changes per tick.
  let tactical: TacticalSnapshot | undefined;
  if (c.teamPlan) {
    let base = tacticalStaticCache.get(c.teamPlan);
    if (!base) {
      base = {
        kind: c.teamPlan.kind,
        systemId: c.teamPlan.systemId,
        stage: c.teamPlan.stage,
        offense: c.teamPlan.offense,
        handler: c.teamPlan.handler,
        assignments: c.teamPlan.assignments.map((assignment) => {
          const route = c.teamPlan?.routes?.[assignment.jersey];
          return {
            jersey: assignment.jersey,
            role: assignment.role,
            action: assignment.action,
            targetJersey: assignment.targetJersey,
            lane: assignment.lane,
            ...(route ? { route } : {}),
          };
        }),
        feedTargetJersey: c.teamPlan.feedTargetJersey ?? null,
        ...(c.teamPlan.screenDefense ? { screenDefense: c.teamPlan.screenDefense } : {}),
      };
      tacticalStaticCache.set(c.teamPlan, base);
    }
    tactical = c.activeBallAction
      ? {
          ...base,
          activeAction: {
            jersey: c.activeBallAction.jersey,
            kind: c.activeBallAction.kind,
            stage: c.activeBallAction.stage,
            windupSeconds: c.activeBallAction.windupSeconds,
            recoverySeconds: c.activeBallAction.recoverySeconds,
            elapsedSeconds: Math.max(0, s.realClock - c.activeBallAction.startedAt),
          },
        }
      : base;
  }

  // P5.x safety net: a substitution that landed mid-sequence (SUB emitted
  // inside a LIVE tick by a rotation helper) can leave the pose map out
  // of sync with the lineups — an incoming player missing a pose (8-9
  // player frame) or an outgoing player's pose lingering (11+). Reconcile
  // before the snapshot so LIVE frames always carry exactly the on-court
  // ten. Missing jerseys get a bench-side placeholder; the next dead-ball
  // alignment walks them into formation.
  if (s.phase === 'LIVE') {
    const court = new Set([...s.lineups.home, ...s.lineups.away]);
    let posesDirty = false;
    for (const jersey of Object.keys(s.poses)) {
      if (!court.has(jersey)) posesDirty = true;
    }
    for (const jersey of court) {
      if (!s.poses[jersey]) posesDirty = true;
    }
    if (posesDirty) {
      const reconciled: Record<string, PoseState> = {};
      for (const [jersey, pose] of Object.entries(s.poses)) {
        if (court.has(jersey)) reconciled[jersey] = pose;
      }
      for (const jersey of court) {
        if (reconciled[jersey]) continue;
        const team: TeamId = s.lineups.home.includes(jersey) ? 'home' : 'away';
        const idx = Math.max(0, (team === 'home' ? s.lineups.home : s.lineups.away).indexOf(jersey));
        const x = team === 'home' ? 0.04 : 0.96;
        reconciled[jersey] = makePose({
          jersey,
          team,
          x,
          y: 0.25 + 0.1 * (idx % 5),
          targetX: x,
          targetY: 0.25 + 0.1 * (idx % 5),
          action: 'idle',
        });
      }
      s = withPoses(s, reconciled);
    }
  }

  // Ball facts can trigger a re-plan after movement; normalize the final
  // frame label once more before it becomes the public position truth.
  s = normalizeFinalLiveLabels(s);

  const snapshot = snapshotFromState({
    t_real: s.realClock,
    t_game: s.clocks.game,
    shotClock: s.clocks.shot,
    period: s.period,
    phase: s.phase,
    tactical,
    score: s.score,
    poses: s.poses,
    ball: s.ballMotion,
    stamina: c.stamina,
    lastEventSeq: lastSeq,
    lastEventType: lastType,
    tickEventSeqs: s.seq > seqBeforeTick
      ? Array.from({ length: s.seq - seqBeforeTick }, (_, i) => seqBeforeTick + 1 + i)
      : [],
  });

  return { state: s, snapshot, ctx: c };
}

export type { Phase };
