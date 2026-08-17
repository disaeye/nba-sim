/**
 * simulateGame — dual-clock continuous kernel (foundation 0.4.0).
 * realClock always advances at TICK_DT; gameClock only in LIVE.
 * Position truth: poses + WorldSnapshot every tick. Events are semantic markers.
 */
import type { GameState, TeamId } from './state/types.js';
import { createInitialState } from './state/initial.js';
import type { GameInput as StateGameInput } from './state/types.js';

import { mulberry32 } from './rng/mulberry32.js';
import { transition } from './rules/driver.js';
import { distFeet } from './court/geometry.js';
import { CONTACT_FOUL_RADIUS_FT } from './adjudicate/adjudicate.js';
import { bindRoles } from './identity/bind.js';
import { capabilitiesForJersey } from './identity/roles.js';
import type { LineupCapability } from './identity/types.js';
import type { LineupPackage, RoleBinding } from './identity/types.js';

import { createEpisode } from './possession/episode.js';
import { selectPlay } from './possession/play-select.js';
import type { PossessionEpisode } from './possession/types.js';

import { bridgeCapabilities } from './playerdata/bridge.js';
import { chemistryEffects } from './playerdata/chemistry.js';
import { allocatePossessionShares } from './playerdata/possession.js';
import { moraleEffects } from './playerdata/morale.js';
import { OFFENSE_ROLES } from './playerdata/tables.js';
import { applyPeriodRest, fatigueReport, initStamina } from './stamina.js';

import type { RosterMaps, Player as BsPlayer } from './box-score.js';

import {
  PLAYS_CONFIG,
  MAX_ITERATIONS,
  emit,
  firstInLineup,
  periodRouterTrigger,
  resetForNewPeriod,
  otherTeam,
  applyDueSubs,
  pushPossessionLog,
  validateInput,
  buildGameResult,
} from './sim-utils.js';
import type {
  GameInput,
  GameResult,
  SubInstruction,
  PossessionEntry,
} from './sim-utils.js';
import { stepSimulationTick, TICK_DT, type TickContext, reconcileHasBall } from './sim-tick.js';
import type { WorldSnapshot } from './world/snapshot.js';
import type { PoseMap, PoseState } from './court/poses.js';
import { clamp01, makePose, resolveBodyContacts, setTarget, stepAllPoses, stepPose } from './court/poses.js';
import { perceiveLiveCourt } from './perception/live-court.js';
import { planSpatialTargets } from './spatial/team-planner.js';
import { loadMobilityConfig, setRuntimeSpeedOverrides, type MobilityConfig } from './court/mobility.js';
import { offenseAttacks } from './court/alignment.js';
import { ballHeld, ballDead, startPass, stepBall } from './court/ball-motion.js';
import {
  buildJumpBallAlignment,
  buildInboundAlignment,
  buildFtAlignment,
  buildTipReceiveAlignment,
} from './court/dead-setup.js';
import {
  buildLiveWorld,
  retargetFromLiveWorld,
} from './court/relations.js';
import { snapshotFromState } from './world/snapshot.js';

function deadBallPhase(phase: GameState['phase']): boolean {
  return phase === 'DEAD_MAKE' || phase === 'DEAD_OOB' || phase === 'DEAD_VIOLATION' || phase === 'DEAD_HELD';
}
/**
 * A dead ball ends the live possession's BALL state — but not the players'
 * tactical identities. The old hard clear (binding/teamPlan → null) turned
 * every dead ball into a tactical vacuum: players stood as statues until
 * the next LIVE decision rebuilt a plan from scratch. Real basketball:
 * the team keeps its offensive scheme through the dead ball; players walk
 * to their NEXT tactical positions while the ball is being retrieved.
 *
 * Soft clear: ball-specific state (intent, actions, inbound markers) is
 * dropped; the tactical context (binding, teamPlan skeleton, play) is
 * retained so the dead-ball walk and the inbound sprint both run toward
 * the NEXT possession's formation, not toward a void.
 */
function clearLiveTacticalContext(ctx: TickContext): TickContext {
  return {
    ...ctx,
    // Ball state: cleared (the ball is dead).
    forceDecision: false,
    stickyBallIntent: null,
    activeBallAction: null,
    screenEstablished: false,
    screenExecution: null,
    lastIntents: [],
    decisionCooldown: 0,
    handoffCooldownTicks: 0,
    recentHandoff: null,
    catchWindowTicks: 0,
    dwellTicksRemaining: 0,
    stickyScreenJersey: null,
    // Tactical state: RETAINED through the dead ball.
    // binding / teamPlan / playId / mode survive so the dead-ball walk
    // targets the next possession's formation. The incoming possession's
    // first decision will re-select if the team changed.
    // Ball-in-flight markers reset (the ball is dead, not being thrown).
    pendingInbound: null,
    pendingInboundTicks: 0,
    possessionPhase: 'ADVANCE',
    phaseTicksRemaining: 0,
  };
}

/** Inbound assignment: a non-creator takes the ball out (real NBA: the
 * point guard does not inbound), and the primary creator receives —
 *  the possession starts in the hands of the offense's engine. */
function inboundPair(
  state: GameState,
  team: TeamId,
  pkg: LineupPackage | undefined,
): { inbounder: string; receiver: string } {
  const lineup = team === 'home' ? state.lineups.home : state.lineups.away;
  // P6.3: the possession-share-ordered creator list (highest share first)
  // decides the inbound receiver — the engine's handler IS the highest
  // usage creator. Falls back to the usageProfile order.
  const ordered = state.creatorOrder[team];
  const creator = (ordered.length > 0 ? ordered : pkg?.usageProfile.creator ?? [])[0] ?? null;
  const receiver = creator && lineup.includes(creator)
    ? creator
    : (lineup.find((j) => j !== lineup[0]) ?? lineup[0]!);
  // The inbounder is whoever is CLOSEST to the end line (their own
  // baseline): in real basketball the nearest player grabs the ball and
  // throws it in, instead of a teammate sprinting 45ft from the far wing.
  // After a make the inbounding team's players are usually near their own
  // baseline anyway (they just defended/ran back), so this keeps the
  // dead-ball walk short and the inbound visually anchored at the line.
  const attack = offenseAttacks(team, state.baskets);
  const baselineX = attack === 'right' ? 1 : 0;
  const candidates = lineup.filter(
    (j) => j !== receiver && !(pkg?.usageProfile.creator.includes(j) ?? false),
  );
  let inbounder = lineup.find(
    (j) => j !== receiver && !(pkg?.usageProfile.creator.includes(j) ?? false),
  ) ?? lineup.find((j) => j !== receiver) ?? receiver;
  if (candidates.length > 0) {
    let best: string | null = null;
    let bestDist = Infinity;
    for (const j of candidates) {
      const pose = state.poses[j];
      if (!pose) continue;
      const d = Math.abs(pose.x - baselineX);
      if (d < bestDist) {
        bestDist = d;
        best = j;
      }
    }
    if (best) inbounder = best;
  }
  return { inbounder, receiver };
}
function applyLiveRetarget(
  state: GameState,
  binding: RoleBinding,
  offense: TeamId,
  mode: 'TRANSITION' | 'HALFCOURT',
  playId: string | null,
  ballHandler: string,
): GameState {
  const world = buildLiveWorld({
    poses: state.poses,
    ballX: state.ballMotion.x,
    ballY: state.ballMotion.y,
    holderId: state.ball.holderId,
    ballStatus: state.ballMotion.status,
    baskets: state.baskets,
    offense,
    mode,
    playId,
    binding,
    shotClock: state.clocks.shot,
    gameClock: state.clocks.game,
    offenseCoach: state.coach[offense],
  });
  const targets = retargetFromLiveWorld(
    world,
    state.lineups.home,
    state.lineups.away,
    ballHandler,
  );
  return applyTargetsFromAlignment(state, targets);
}

export type {
  GameInput,
  GameResult,
  Player,
  SubInstruction,
  PossessionEntry,
  TimelineEvent,
} from './sim-utils.js';
export type { WorldSnapshot };

function onCourtJerseys(state: GameState): ReadonlySet<string> {
  return new Set([...state.lineups.home, ...state.lineups.away]);
}

/**
 * P5.1: a substitution swaps jerseys but not poses — the incoming player
 * has no pose entry (dropping them from LIVE frames), and the outgoing
 * player's pose lingers (inflating the frame count). Reconcile the pose
 * map to the current lineups: drop off-court jerseys, add bench-side
 * spots for missing ones; the next dead-ball alignment walks them to
 * their real formation.
 */
function ensureAllPoses(state: GameState, input: GameInput): GameState {
  const allOnCourt = new Set([...state.lineups.home, ...state.lineups.away]);
  const poses: Record<string, PoseState> = {};
  let changed = false;
  for (const [jersey, pose] of Object.entries(state.poses)) {
    if (allOnCourt.has(jersey)) poses[jersey] = pose;
    else changed = true;
  }
  for (const jersey of allOnCourt) {
    if (poses[jersey]) continue;
    const team: TeamId = state.lineups.home.includes(jersey) ? 'home' : 'away';
    const lineup = team === 'home' ? state.lineups.home : state.lineups.away;
    const idx = Math.max(0, lineup.indexOf(jersey));
    const x = team === 'home' ? 0.04 : 0.96;
    poses[jersey] = makePose({
      jersey,
      team,
      x,
      y: 0.25 + 0.1 * (idx % 5),
      targetX: x,
      targetY: 0.25 + 0.1 * (idx % 5),
      action: 'idle',
    });
    changed = true;
  }
  return changed ? { ...state, poses } : state;
}

function applyTargetsFromAlignment(
  state: GameState,
  players: readonly {
    readonly jersey: string;    readonly x: number;
    readonly y: number;
    readonly task: string;
    readonly hasBall?: boolean;
    readonly team: TeamId;
    readonly movementRole?: import('./court/mobility.js').MovementRole;
  }[],
): GameState {
  const court = onCourtJerseys(state);
  const poses: Record<string, PoseState> = {};
  for (const j of court) {
    const prev = state.poses[j];
    if (prev) poses[j] = prev;
  }
  for (const p of players) {
    if (!court.has(p.jersey)) continue;
    const prev = poses[p.jersey];
    if (prev) {
      poses[p.jersey] = setTarget(prev, p.x, p.y, p.task, p.movementRole);
    } else {
      poses[p.jersey] = makePose({
        jersey: p.jersey,
        team: p.team,
        x: p.x,
        y: p.y,
        targetX: p.x,
        targetY: p.y,
        action: p.task,
        hasBall: p.hasBall,
        movementRole: p.movementRole,
      });
    }
  }
  for (const j of court) {
    if (poses[j]) continue;
    const team: TeamId = state.lineups.home.includes(j) ? 'home' : 'away';
    const lineup = team === 'home' ? state.lineups.home : state.lineups.away;
    const idx = Math.max(0, lineup.indexOf(j));
    const benchX = team === 'home' ? 0.04 : 0.96;
    const benchY = 0.25 + 0.1 * idx;
    poses[j] = makePose({
      jersey: j,
      team,
      x: benchX,
      y: benchY,
      targetX: benchX,
      targetY: benchY,
      action: 'idle',
    });
  }
  // Alignment is a player choreography only. Ball ownership and ball flight
  // are committed by live adjudication or explicit inbound/period-start
  // transitions. In particular, an FT alignment may mark the shooter with
  // `hasBall` for the formation, but that marker must not move a held ball
  // from the previous live player to the shooter before FT_START. The old
  // reconciliation made that implicit move, creating held→held full-court
  // jumps inside DEAD_MAKE/FT_SEQUENCE and a spectator frame where the
  // referee-side ball was silently reassigned.
  //
  // The pose map is still reconciled against the authoritative ball holder;
  // this keeps `hasBall` a projection of ballMotion rather than a second
  // ownership authority.
  const reconciled = reconcileHasBall(poses, state.ballMotion.holderId);
  return {
    ...state,
    poses: reconciled,
    ballMotion: state.ballMotion,
    ball: {
      holderId: state.ballMotion.holderId,
      status: state.ballMotion.status === 'held' ? 'held' : state.ball.status,
      zone: state.ball.zone,
    },
  };
}

const defaultDeadBallScale = loadMobilityConfig().physical.dead_ball_speed_scale;

// The lineup sub-loop simulates transition "dead time" (players jogging
// to inbound/FT spots) — not in-game performance. It uses a faster jog
// scale than the main tick's dead-ball walk (0.3) so players reach their
// spots within the maxTicks budget. Without this, lowering the main-tick
// dead-ball scale to 0.3 made every DEAD_MAKE/DEAD_VIOLATION stall the
// full 80-tick timeout (8s) because players couldn't walk across court.
const LINEUP_JOG_SCALE = 0.3;

function waitUntilArrivedOrTimeout(
  state: GameState,
  ctx: TickContext,
  snapshots: WorldSnapshot[],
  maxTicks: number,
  only?: readonly string[],
  speedScale: number = LINEUP_JOG_SCALE,
): { state: GameState; ctx: TickContext } {
  let s = state;
  for (let i = 0; i < maxTicks; i += 1) {
    let allArrived = true;
    if (only) {
      for (const jersey of only) {
        const p = s.poses[jersey];
        if (!p?.arrived) {
          allArrived = false;
          break;
        }
      }
    } else {
      for (const p of Object.values(s.poses)) {
        if (!p.arrived) {
          allArrived = false;
          break;
        }
      }
    }
    if (allArrived && i > 2) break;
    // Tactical retarget during the dead-ball walk: the offense keeps
    // moving toward its next formation while the ball is dead. Every
    // 5 ticks (0.5s), re-run the spatial plan for the offense so their
    // targets track the formation — not just the initial walk-to-spot.
    // The defense walks to the inbound alignment spots (already set)
    // and needs no live retarget.
    if (i % 5 === 0 && ctx.teamPlan && ctx.binding) {
      const sense = perceiveLiveCourt(s);
      if (sense) {
        const spatial = planSpatialTargets(sense, ctx.teamPlan);
        const next = { ...s.poses };
        for (const player of spatial.players) {
          const pose = next[player.jersey];
          if (!pose) continue;
          if (pose.team !== sense.offense) continue; // offense only
          next[player.jersey] = setTarget(pose, player.x, player.y, player.task, player.movementRole);
        }
        s = { ...s, poses: next };
      }
    }
    s = {
      ...s,
      realClock: Math.round((s.realClock + TICK_DT) * 10) / 10,
    };
    // The dead-ball setup loop uses the same walk scale as the regular
    // non-live tick. The previous 0.6 override made the setup path twice
    // as fast as its rendered DEAD_* frames, so the visual stream showed
    // 9-16 ft/s "walks" immediately after a made basket. A single scale
    // keeps the event choreography and frame truth on the same clock.
    const poses = stepAllPoses(s.poses, TICK_DT, undefined, defaultDeadBallScale);
    const br = stepBall({ ball: s.ballMotion, poses, dt: TICK_DT, allowIntercept: false });
    const st =
      br.ball.status === 'held'
        ? 'held'
        : br.ball.status === 'inbound'
          ? 'inbound'
          : br.ball.status === 'loose' || br.ball.status === 'pass' || br.ball.status === 'shot'
            ? 'loose'
            : 'dead';
    s = {
      ...s,
      poses,
      ballMotion: br.ball,
      ball: { holderId: br.ball.holderId, status: st, zone: s.ball.zone },
    };
    // stepBall may resolve pass_complete/loose_recovered mid-settle and shift
    // the holder; flags must follow ballMotion.holderId. Settle never fires
    // the decision window (PIN-T2).
    s = { ...s, poses: reconcileHasBall(s.poses, s.ballMotion.holderId) };
    snapshots.push(
      snapshotFromState({
        t_real: s.realClock,
        t_game: s.clocks.game,
        shotClock: s.clocks.shot,
        period: s.period,
        phase: s.phase,
        score: s.score,
        poses: s.poses,
        ball: s.ballMotion,
        // Dead-ball frames must still carry live stamina: without this
        // every settle frame rendered stm=-1 (11,884 of 40,882 frames in
        // the seed-42 export), making the spectator's fatigue display
        // blank through all dead balls and period breaks.
        stamina: ctx.stamina,
      }),
    );
  }
  return { state: s, ctx };
}

/** Advance the opening tap until a non-jumper legally controls the ball. */
function resolveOpeningTip(
  state: GameState,
  tapperId: string,
  receiverId: string,
  team: TeamId,
  snapshots: WorldSnapshot[],
  tipStamina: TickContext['stamina'],
): GameState {
  const from = state.poses[tapperId];
  const to = state.poses[receiverId];
  if (!from || !to) throw new Error(`resolveOpeningTip: missing tap participants`);
  // The outlet may already be moving toward its receive target. The tap is
  // passed to its current legal location; this keeps the transfer continuous
  // instead of making the player teleport to the future target.
  let s: GameState = {
    ...state,
    ballMotion: startPass({ from, to, team, realNow: state.realClock }),
    ball: { ...state.ball, holderId: null, status: 'loose' },
  };
  for (let i = 0; i < 20; i += 1) {
    s = {
      ...s,
      realClock: Math.round((s.realClock + TICK_DT) * 10) / 10,
      clocks: {
        ...s.clocks,
        game: Math.max(0, Math.round((s.clocks.game - TICK_DT) * 10) / 10),
      },
    };
    const poses = stepAllPoses(s.poses, TICK_DT, undefined, defaultDeadBallScale);
    const stepped = stepBall({ ball: s.ballMotion, poses, dt: TICK_DT, allowIntercept: false });
    s = {
      ...s,
      poses: reconcileHasBall(poses, stepped.ball.holderId),
      ballMotion: stepped.ball,
      ball: { ...s.ball, holderId: stepped.ball.holderId, status: stepped.ball.status === 'held' ? 'held' : 'loose' },
    };
    snapshots.push(snapshotFromState({
      t_real: s.realClock,
      t_game: s.clocks.game,
      shotClock: s.clocks.shot,
      period: s.period,
      phase: s.phase,
      score: s.score,
      poses: s.poses,
      ball: s.ballMotion,
      stamina: tipStamina,
    }));
    if (stepped.events.some((event) => event.kind === 'pass_complete')) {
      const receiver = s.poses[receiverId];
      if (!receiver) throw new Error(`resolveOpeningTip: receiver disappeared`);
      s = {
        ...s,
        ballMotion: ballHeld(receiver, team),
        ball: { ...s.ball, holderId: receiverId, status: 'held' },
      };
      return emit(s, 'POSSESSION_GAINED', [receiverId], {
        team,
        player_id: receiverId,
        source: 'jump_ball',
      });
    }
  }
  throw new Error('resolveOpeningTip: tap did not reach a non-jumper');
}

/**
 * Full game simulation under dual-clock continuous motion.
 */
export function simulateGame(input: GameInput): GameResult {
  validateInput(input);

  const homePackage = input.home.lineupPackages[0];
  const awayPackage = input.away.lineupPackages[0];
  if (homePackage === undefined || awayPackage === undefined) {
    throw new Error('simulateGame: missing lineup packages');
  }

  const stateInput: StateGameInput = {
    home: { id: 'home', starters: homePackage.players },
    away: { id: 'away', starters: awayPackage.players },
  };

  let state: GameState = createInitialState(stateInput);
  const rng = mulberry32(input.seed);
  let liveBinding: RoleBinding | null = null;

  const rosterMaps: RosterMaps = {
    home: new Map(input.home.roster.map((p) => [p.jersey, p as BsPlayer])),
    away: new Map(input.away.roster.map((p) => [p.jersey, p as BsPlayer])),
  };

  const packages: Record<TeamId, LineupPackage> = {
    home: homePackage,
    away: awayPackage,
  };
  const possessionLog: PossessionEntry[] = [];
  let possessionId = 0;
  let episode: PossessionEpisode | null = null;
  let possessionStartT = 0;
  let possessionStartPeriod = 1;
  let startReason = 'AFTER_JUMP_BALL';
  let endedPeriod = 0;
  let lastPossessionTeam: TeamId = 'home';
  /** P5.3 momentum tracker: recent scoring run per team (last 6 possessions). */
  let recentRun: { team: TeamId | null; points: number; possessions: number } = { team: null, points: 0, possessions: 0 };
  /** Phase before a momentum timeout — restored after TIMEOUT_END so the
   *  original dead-ball flow (DEAD_MAKE inbound, DEAD_FOUL FT, …) completes
   *  instead of being skipped (which orphaned the ball into a phantom rebound). */
  let preTimeoutPhase: GameState['phase'] | null = null;
  const snapshots: WorldSnapshot[] = [];
  let iterations = 0;

  // ── default rotation (10-man, staggered fives) ─────────────────────────
  // When the caller provides no scriptedSubs, derive a realistic pattern
  // from the two lineup packages: starters open each half, the bench unit
  // closes Q1/Q3 and opens Q2, starters return for the Q2/Q4 stretch and
  // close the game. Starters ≈ 27 min, bench ≈ 21 min. The old behavior
  // swapped all ten at halftime — every player played exactly 24 minutes
  // and the box score looked scripted.
  const defaultRotation: SubInstruction[] = [];
  const benchPkg = (team: TeamId) =>
    (team === 'home' ? input.home.lineupPackages : input.away.lineupPackages)[1];
  if (!input.scriptedSubs && benchPkg('home') && benchPkg('away')) {
    const stages = [
      { period: 1, clockMin: 4, incoming: 'bench' as const },
      { period: 2, clockMin: 6, incoming: 'starters' as const },
      { period: 3, clockMin: 4, incoming: 'bench' as const },
      { period: 4, clockMin: 5, incoming: 'starters' as const },
    ];
    for (const team of ['home', 'away'] as const) {
      const starters = (team === 'home' ? input.home.lineupPackages : input.away.lineupPackages)[0]!.players;
      const bench = benchPkg(team)!.players;
      for (const stage of stages) {
        const incoming = stage.incoming === 'starters' ? starters : bench;
        const outgoing = stage.incoming === 'starters' ? bench : starters;
        for (let i = 0; i < 5; i++) {
          const playerOut = outgoing[i];
          const playerIn = incoming[i];
          if (playerOut === undefined || playerIn === undefined) continue;
          defaultRotation.push({ period: stage.period, clockMin: stage.clockMin, team, playerOut, playerIn });
        }
      }
    }
  }
  const pendingSubs: SubInstruction[] = input.scriptedSubs ? [...input.scriptedSubs] : defaultRotation;

  // Keep the tactical package in sync with the on-court five after any
  // whole-unit substitution (bindRoles validates against package.players).
  const syncPackagesToLineups = (
    s: GameState,
  ): void => {
    for (const team of ['home', 'away'] as const) {
      const lineup = team === 'home' ? s.lineups.home : s.lineups.away;
      const pkgs = team === 'home' ? input.home.lineupPackages : input.away.lineupPackages;
      const match = pkgs.find((p) => p.players.every((j) => lineup.includes(j)));
      if (match) packages[team] = match;
    }
  };

  let tickCtx: TickContext = {
    packages,
    binding: null,
    mode: 'HALFCOURT',
    playId: null,
    rng,
    pendingFtAttempts: 0,
    ftAttemptTotal: 0,
    pendingFtShooter: null,
    pendingFtTeam: null,
    pendingFtAndOne: false,
    forceDecision: false,
    stickyBallIntent: null,
    activeBallAction: null,
    teamPlan: null,
    screenEstablished: false,
    screenExecution: null,
    lastIntents: [],
    lastEndReason: null,
    lastStartReason: null,
    dwellTicksRemaining: 0,
    watchdogStaleTicks: 0,
    watchdogSnapshotX: 0.5,
    watchdogSnapshotY: 0.5,
    lastDriveAt: -99,
    lastDriveHandler: null,
    decisionCooldown: 0,
    possessionPhase: 'ADVANCE',
    handoffCooldownTicks: 0,
    recentHandoff: null,
    phaseTicksRemaining: 0,
    stickyScreenJersey: null,
    lastHandlingAction: null,
    passChainSinceAttack: 0,
    catchWindowTicks: 0,
    handlingActionCooldownTicks: 0,
    pendingInbound: null,
    pendingInboundTicks: 0,
    stamina: initStamina(input),
  };
  state = emit(state, 'GAME_START', [], {});
  // Per-jersey talent overlay: roster abilities merged over the lineup
  // package's role-derived defaults. Resolve reads this for per-player
  // outcome rates; the coverage layer reads it for deny/sag decisions.
  {
    const abilities: Record<string, LineupCapability> = {};
    const playerData: Record<string, import('./playerdata/types.js').PlayerData> = {};
    const speedOverrides: Record<string, Record<string, number>> = {};
    for (const team of ['home', 'away'] as const) {
      const roster = team === 'home' ? input.home.roster : input.away.roster;
      const pkg = packages[team];
      for (const p of roster) {
        const fromPlayerData = p.playerData !== undefined ? bridgeCapabilities(p.playerData) : {};
        abilities[p.jersey] = {
          ...capabilitiesForJersey(pkg, p.jersey),
          ...fromPlayerData,
          ...(p.abilities ?? {}),
        };
        if (p.playerData !== undefined) {
          playerData[p.jersey] = p.playerData;
          // P4.1: derive per-player speed from physical SPD (20..99).
          // SPD 70 (league avg) → 1.0× the baseline; 90 (elite) → 1.15×;
          // 50 (plodder) → 0.88×. Applied to the burst actions so a
          // fast guard actually separates on the break and a slow big
          // doesn't chase wings down.
          const spd = p.playerData.physical.SPD;
          const factor = 0.7 + (spd / 99) * 0.5;
          speedOverrides[p.jersey] = {
            advance: Math.round(0.15 * factor * 100) / 100,
            drive: Math.round(0.2 * factor * 100) / 100,
            cut: Math.round(0.22 * factor * 100) / 100,
            relocate: Math.round(0.14 * factor * 100) / 100,
            crossover: Math.round(0.16 * factor * 100) / 100,
            on_ball_defend: Math.round(0.18 * factor * 100) / 100,
            help: Math.round(0.2 * factor * 100) / 100,
          };
        }
      }
    }
    setRuntimeSpeedOverrides(speedOverrides);
    // P6.1: chemistry effects per lineup — the five-man unit's role
    // assignment (doc offense/defense roles from lineupIdentity) yields
    // channel modifiers (±10%) the decision/resolve layers fold in.
    const chemistry: Record<TeamId, readonly import('./playerdata/types.js').ChemistryEffect[]> = { home: [], away: [] };
    const creatorOrder: Record<TeamId, readonly string[]> = { home: [], away: [] };
    for (const team of ['home', 'away'] as const) {
      const pkg = packages[team];
      const identity = pkg.lineupIdentity;
      if (!identity) continue;
      const offRoles = identity.assignments.flatMap((a) => a.roles.filter((r) => r in OFFENSE_ROLES)) as import('./playerdata/types.js').OffenseRoleId[];
      const defRoles = identity.assignments.flatMap((a) => a.roles.filter((r) => !(r in OFFENSE_ROLES))) as import('./playerdata/types.js').DefenseRoleId[];
      chemistry[team] = chemistryEffects(offRoles, defRoles);
      // P6.3: possession-share ordering — re-rank the unit's creator
      // priority by the §2.2 allocation so the highest-share creator is
      // the first receiver/binding candidate.
      if (identity.assignments.length > 0) {
        const budget = allocatePossessionShares(
          identity.assignments.map((a) => ({ role: a.roles[0] as import('./playerdata/types.js').OffenseRoleId, fit: 60 })),
        );
        const shareByJersey = new Map<string, number>();
        identity.assignments.forEach((a, i) => {
          const share = budget[i];
          if (share) shareByJersey.set(a.jersey, share.actual);
        });
        const creator = [...(pkg.usageProfile.creator ?? [])];
        creator.sort((a, b) => (shareByJersey.get(b) ?? 0) - (shareByJersey.get(a) ?? 0));
        creatorOrder[team] = creator;
      }
    }
    state = { ...state, abilities, playerData, coach: { home: input.coach?.home, away: input.coach?.away }, chemistry, creatorOrder };
    // P6.2: per-team morale exec multiplier — the season layer passes a
    // team morale 0..100; §9.3 bands fold an exec factor (0.8..1.05)
    // into every resolve via the stamina channel. Neutral 60 → 1.0.
    const moraleExec: Record<TeamId, number> = { home: 1, away: 1 };
    for (const team of ['home', 'away'] as const) {
      const morale = input.morale?.[team];
      if (morale !== undefined) {
        moraleExec[team] = moraleEffects(morale).exec;
      }
    }
    state = { ...state, moraleExec };
  }
  state = { ...state, phase: 'JUMP_BALL' };

  const homeJumper = firstInLineup(state, 'home');
  const awayJumper = firstInLineup(state, 'away');
  const jumpAlign = buildJumpBallAlignment({
    homeLineup: state.lineups.home,
    awayLineup: state.lineups.away,
    homeJumper,
    awayJumper,
  });
  state = applyTargetsFromAlignment(state, jumpAlign.players);
  // Place current = target for tip start (formation set).
  {
    const poses: Record<string, PoseState> = {};
    for (const [j, p] of Object.entries(state.poses)) {
      poses[j] = {
        ...p,
        x: p.targetX,
        y: p.targetY,
        arrived: true,
      };
    }
    state = { ...state, poses, ballMotion: ballDead({ x: 0.5, y: 0.5 }) };
  }
  snapshots.push(
    snapshotFromState({
      t_real: state.realClock,
      t_game: state.clocks.game,
      shotClock: state.clocks.shot,
      period: state.period,
      phase: state.phase,
      score: state.score,
      poses: state.poses,
      ball: state.ballMotion,
      stamina: tickCtx.stamina,
      lastEventType: 'GAME_START',
      lastEventSeq: state.seq,
    }),
  );

  // Hold formation briefly in real time, then tip.
  ({ state } = waitUntilArrivedOrTimeout(state, tickCtx, snapshots, 8));

  const winner: TeamId = rng.next() < 0.5 ? 'home' : 'away';
  const jumper = winner === 'home' ? homeJumper : awayJumper;
  const winLine = winner === 'home' ? state.lineups.home : state.lineups.away;
  const outlet = winLine.find((j) => j !== jumper) ?? jumper;

  state = emit(state, 'JUMP_BALL_TAP', [homeJumper, awayJumper], {
    home_jumper_id: homeJumper,
    away_jumper_id: awayJumper,
    tapping_team: winner,
    tapper_id: jumper,
  });
  // The outlet must receive in the winner's backcourt. Without this explicit
  // post-tip alignment, the receiver remains on the jump-circle spot; the
  // phase machine then classifies the possession as already frontcourt and
  // replaces the advance intent with a half-court hold.
  const tipReceive = buildTipReceiveAlignment({
    homeLineup: state.lineups.home,
    awayLineup: state.lineups.away,
    winner,
    controller: outlet,
    baskets: state.baskets,
  });
  state = resolveOpeningTip(state, jumper, outlet, winner, snapshots, tickCtx.stamina);
  const controllerSpot = tipReceive.players.find((player) => player.jersey === outlet);
  const receivedOutlet = state.poses[outlet];
  if (controllerSpot && receivedOutlet) {
    // Do not move the receiver before the tap is caught: the pass target is
    // its current circle position. Begin the backcourt run only after the
    // possession-gained transition, preserving both catch legality and
    // continuous motion.
    state = {
      ...state,
      poses: {
        ...state.poses,
        [outlet]: setTarget(receivedOutlet, controllerSpot.x, controllerSpot.y, controllerSpot.task),
      },
    };
  }
  // The game clock starts on the legal tap; the shot clock waits for control.
  state = { ...state, phase: 'LIVE' };
  lastPossessionTeam = winner;


  // Start LIVE possession
  state = { ...state, phase: 'LIVE' };
  episode = createEpisode(state, startReason, rng);
  episode.play = selectPlay(episode.mode, rng, PLAYS_CONFIG);
  liveBinding = bindRoles(packages[winner], episode.play);
  tickCtx = {
    ...tickCtx,
    binding: liveBinding,
    mode: episode.mode,
    playId: episode.play.id,
    stickyScreenJersey: null,
  };
  possessionStartT = state.clocks.game;
  possessionStartPeriod = state.clocks.period;

  state = applyLiveRetarget(
    state,
    liveBinding,
    winner,
    episode.mode,
    episode.play.id,
    outlet,
  );
  // The first live decision must start from the explicit tip-receive target.
  // applyLiveRetarget resolves half-court relations from the current circle
  // position; overriding only the outlet preserves legal catch continuity
  // while making the direction of the opening advance unambiguous.
  const liveOutlet = state.poses[outlet];
  if (controllerSpot && liveOutlet) {
    state = {
      ...state,
      poses: {
        ...state.poses,
        [outlet]: setTarget(liveOutlet, controllerSpot.x, controllerSpot.y, 'advance'),
      },
    };
  }

  // ── Main dual-clock loop ────────────────────────────────────────────────
  while (state.phase !== 'POST_GAME') {
    if (++iterations > MAX_ITERATIONS) {
      throw new Error(
        `simulateGame: exceeded ${MAX_ITERATIONS} iterations (phase=${state.phase}, game=${state.clocks.game}, shot=${state.clocks.shot}, period=${state.period}, score=${state.score.home}-${state.score.away}, t_real=${state.realClock}, ball=${state.ballMotion.status}, holder=${state.ballMotion.holderId}, pendingInbound=${tickCtx.pendingInbound ? 'yes' : 'no'}, possPhase=${tickCtx.possessionPhase}, cooldown=${tickCtx.decisionCooldown}, force=${tickCtx.forceDecision}, active=${tickCtx.activeBallAction?.kind ?? 'none'}, events=${state.events.slice(-8).map((event) => event.type).join(',')}) `,
      );
    }

    // LIVE continuous tick
    if (state.phase === 'LIVE') {
      if (state.clocks.game <= 0) {
        // A legal shot released before the buzzer must resolve before the
        // period boundary (the basket counts / the rebound ends the period).
        // Deferring the boundary keeps SHOT_RELEASE → SHOT_RESULT contiguous;
        // passes, by contrast, are truncated by the horn.
        if (state.ballMotion.status === 'shot') {
          const tick = stepSimulationTick(state, tickCtx);
          state = tick.state;
          tickCtx = tick.ctx;
          snapshots.push(tick.snapshot);
          continue;
        }
        // The horn truncates an in-flight pass (dead ball) — a pass must
        // never complete inside the next period with the wrong possession.
        if (state.ballMotion.status === 'pass') {
          state = {
            ...state,
            ballMotion: ballDead({ x: state.ballMotion.x, y: state.ballMotion.y }),
            ball: { ...state.ball, holderId: null, status: 'dead' },
          };
        }
        endedPeriod = state.clocks.period;
        state = emit(state, 'CLOCK_EXPIRY_ADJUDICATION', [], {
          clock: 'game',
          residual_seconds: 0,
        });
        // applyPeriodEnd advances state.period and sets DEAD_PERIOD_END
        state = emit(state, 'PERIOD_END', [], { period: endedPeriod });
        if (episode) {
          pushPossessionLog(
            possessionLog,
            possessionId++,
            episode,
            possessionStartT,
            possessionStartPeriod,
            state.clocks.game,
            'PERIOD_END',
          );
          episode = null;
          liveBinding = null;
        }
        // An inbound whose flight was truncated by the horn must not leak
        // into the next period: a stale pendingInbound freezes the shot
        // clock and suppresses decisions for the entire following period.
        tickCtx = { ...tickCtx, pendingInbound: null, pendingInboundTicks: 0 };
        const scoreDiff = state.score.home - state.score.away;
        const trigger = periodRouterTrigger(endedPeriod, scoreDiff);
        const trans = transition(state, trigger);
        if (trans === null) throw new Error(`no transition ${trigger}`);
        if (trans.phase === 'HALFTIME') {
          state = emit(state, 'HALFTIME', [], {});
        }
        for (const et of trans.emit_events) {
          if (et === 'GAME_END') {
            state = emit(state, 'GAME_END', [], {});
          }
        }
        // PERIOD_START is emitted when break ends (BREAK_END), not here —
        // except empty emit lists still land on PERIOD_BREAK/HALFTIME/POST_GAME.
        state = trans.phase === 'POST_GAME'
          ? {
            ...state,
            phase: trans.phase,
            ballMotion: ballDead({ x: state.ballMotion.x, y: state.ballMotion.y }),
            ball: { ...state.ball, holderId: null, status: 'dead' },
            poses: reconcileHasBall(state.poses, null),
          }
          : { ...state, phase: trans.phase };
        continue;
      }

      if (episode === null && state.possession.team && state.ballMotion.status === 'held' && state.ballMotion.holderId) {
        const team = state.possession.team;
        episode = createEpisode(state, startReason, rng);
        episode.play = selectPlay(episode.mode, rng, PLAYS_CONFIG);
        liveBinding = bindRoles(packages[team], episode.play);
        const handler = state.ball.holderId ?? firstInLineup(state, team);
        tickCtx = {
          ...tickCtx,
          binding: liveBinding,
          activeBallAction: null,
          teamPlan: null,
          screenEstablished: false,
          screenExecution: null,
          stickyScreenJersey: null,
          stickyBallIntent: null,
          lastHandlingAction: null,
          handlingActionCooldownTicks: 0,
          forceDecision: true,
          possessionPhase: 'ADVANCE',
          handoffCooldownTicks: 0,
          recentHandoff: null,
          phaseTicksRemaining: 0,
          dwellTicksRemaining: 0,
        };
        possessionStartT = state.clocks.game;
        possessionStartPeriod = state.clocks.period;
        state = applyLiveRetarget(
          state, liveBinding, team, episode.mode, episode.play.id, handler);
      }

      tickCtx = {
        ...tickCtx,
        binding: liveBinding,
        mode: episode?.mode ?? 'HALFCOURT',
        playId: episode?.play?.id ?? null,
      };

      // ── P5.4 intentional foul (foul game) ─────────────────────────────
      // The trailing team in the last 40s of the 4th (or OT) with a
      // possession deficit fouls deliberately to stop the clock. The
      // whistle targets the opponent's worst FT shooter. Only when the
      // trailing team is DEFENDING (the opponent holds the ball) — you
      // cannot foul your own possession.
      if (state.phase === 'LIVE' && state.clocks.period >= 4 && state.clocks.game <= 40) {
        const diff = state.score.home - state.score.away;
        const trailing = diff < 0 ? 'home' : diff > 0 ? 'away' : null;
        const offenseTeam = state.possession.team;
        if (trailing && offenseTeam && offenseTeam !== trailing && state.ballMotion.status === 'held' && state.ballMotion.holderId) {
          // Possession deficit: trailing by ≥8 with ≤40s, or ≥3 with
          // ≤20s — the extreme late-game scenario (the trailing team has
          // no realistic non-foul path back).
          const needsFoul = Math.abs(diff) >= 8 || (Math.abs(diff) >= 3 && state.clocks.game <= 20);
          if (needsFoul) {
            const holder = state.ballMotion.holderId;
            // The defender assigned to the holder commits the foul —
            // pick a defender from the DEFENDING team's lineup (anyone
            // but the holder's teammate is impossible: holder is offense).
            const defTeam: 'home' | 'away' = offenseTeam === 'home' ? 'away' : 'home';
            // A disqualified player cannot commit the intentional foul —
            // fouling with a 6-foul player on the floor would force an
            // immediate fouled-out sub and the strategy cycles the same
            // two disqualified players forever (measured: 12 consecutive
            // 1↔6 subs in the last 20s). Real coaches foul with the
            // player carrying the FEWEST fouls (the expendable one), and
            // rotate — pick the legal defender with the lowest foul count.
            // M2: the offender must also be the NEAREST legal defender to
            // the holder — an intentional foul is a physical reach, not a
            // telepathic whistle. The old lowest-foul pick could foul from
            // 40ft away (foul_contact_dist maxD measurements), which the
            // audit flags as a phantom foul.
            const holderPose = state.poses[holder];
            const legalDefenders = state.lineups[defTeam].filter(
              (j) => j !== holder && (state.fouls.players[j] ?? 0) < 6,
            );
            const distToHolder = (j: string): number => {
              const p = state.poses[j];
              const hp = holderPose;
              if (!p || !hp) return Infinity;
              return distFeet(p.x, p.y, hp.x, hp.y);
            };
            const offender = legalDefenders.length > 0
              ? legalDefenders.reduce((best, j) => {
                  const dBest = distToHolder(best);
                  const dJ = distToHolder(j);
                  if (dJ < dBest - 2) return j; // 2ft tie-break toward fewest fouls
                  if (Math.abs(dJ - dBest) <= 2 && (state.fouls.players[j] ?? 0) < (state.fouls.players[best] ?? 0)) return j;
                  return best;
                })
              : state.lineups[defTeam].find((j) => j !== holder) ?? state.lineups[defTeam][0];
            const offense = offenseTeam;
            const victim = holder;
            // M2 position gate: the whistle requires the chosen defender
            // within contact reach of the holder. A foul is a physical
            // act — a trailing team that cannot reach the ball cannot
            // foul it. The current coverage layer keeps defenders in
            // half-court setup during the late-game foul phase, so this
            // gate will suppress the hack strategy until M3 (defense
            // closing on the handler) lands — the honest tradeoff is
            // fewer phantom fouls now, full strategy once coverage closes.
            const nearestDist = offender ? distToHolder(offender) : Infinity;
            if (offender && victim && nearestDist <= CONTACT_FOUL_RADIUS_FT) {
              const bonus = offense === 'home' ? state.fouls.bonus.home : state.fouls.bonus.away;
              state = emit(state, 'FOUL', [offender, victim], {
                offender_id: offender,
                offender_team: offense === 'home' ? 'away' : 'home',
                victim_id: victim,
                foul_type: 'non_shooting',
                shooting: false,
                free_throws_awarded: bonus ? 2 : 0,
                note: 'intentional_foul',
              });
              // The whistle kills the ball: clear both holder sources so
              // the DEAD_FOUL flow owns the ball from here (I2: a held
              // ball must always have exactly one hasBall player).
              state = {
                ...state,
                ballMotion: { ...state.ballMotion, holderId: null, status: 'dead' },
                ball: { ...state.ball, holderId: null, status: 'dead' },
                phase: 'DEAD_FOUL',
              };
              if (bonus) {
                tickCtx = {
                  ...tickCtx,
                  pendingFtAttempts: 2,
                  ftAttemptTotal: 2,
                  pendingFtShooter: victim,
                  pendingFtTeam: offense,
                  pendingFtAndOne: false,
                };
              }
              tickCtx = { ...tickCtx, forceDecision: true, stickyBallIntent: null };
            }
          }
        }
      }

      const beforePhase = state.phase;
      const beforeScore = state.score.home + state.score.away;
      const tickBeforeEventCount = state.events.length;
      const tick = stepSimulationTick(state, tickCtx);
      state = tick.state;
      tickCtx = tick.ctx;
      snapshots.push(tick.snapshot);

      // Episode lifecycle from Adjudicate (architecture §1.5) — not event scanning.
      if (episode && tickCtx.lastEndReason && tickCtx.lastEndReason !== 'MISS_OREB_CONTINUE') {
        pushPossessionLog(
          possessionLog,
          possessionId++,
          episode,
          possessionStartT,
          possessionStartPeriod,
          state.clocks.game,
          tickCtx.lastEndReason,
        );
        lastPossessionTeam = state.possession.team ?? episode.team;
        if (tickCtx.lastStartReason) {
          startReason = tickCtx.lastStartReason;
        }
        episode = null;
        liveBinding = null;
        tickCtx = {
          ...tickCtx,
          stickyBallIntent: null,
          lastIntents: [],
          lastHandlingAction: null,
          handlingActionCooldownTicks: 0,
          activeBallAction: null,
          passChainSinceAttack: 0,
        };
      } else if (episode && tickCtx.lastEndReason === 'MISS_OREB_CONTINUE') {
        // OREB continue: same episode id, re-select advisory play
        if (episode.mode) {
          episode.play = selectPlay(episode.mode, rng, PLAYS_CONFIG);
          if (state.possession.team) {
            liveBinding = bindRoles(packages[state.possession.team], episode.play);
            tickCtx = {
              ...tickCtx,
              binding: liveBinding,
              playId: episode.play.id,
              forceDecision: true,
              stickyBallIntent: null,
              lastHandlingAction: null,
              handlingActionCooldownTicks: 0,
              activeBallAction: null,
            };
          }
        }
      }
      void tickBeforeEventCount;

      // ── P5.3 momentum run tracker + timeout trigger ────────────────────
      // After every ended possession, update the scoring-run state: the
      // last possessions' net points per team. A run of ≥8 unanswered
      // points triggers an automatic timeout for the trailing team
      // (real coaches stop the bleeding).
      if (episode === null && tickCtx.lastEndReason && tickCtx.lastEndReason !== 'MISS_OREB_CONTINUE' && beforePhase === 'LIVE') {
        const scorer = lastPossessionTeam;
        const pointsThisPossession = Math.max(0, state.score.home + state.score.away - beforeScore);
        if (recentRun.team === scorer) {
          recentRun = { team: scorer, points: recentRun.points + pointsThisPossession, possessions: recentRun.possessions + 1 };
        } else {
          recentRun = { team: scorer, points: pointsThisPossession, possessions: 1 };
        }
        if (recentRun.possessions >= 3 && recentRun.points >= 8 && state.clocks.game > 120) {
          // The TRAILING team calls the timeout (real coaches stop the
          // bleeding), not the team on the run. Their timeout slot is
          // consumed; the scoring team keeps theirs.
          const callingTeam = otherTeam(scorer);
          if (state.timeouts.remaining[callingTeam] <= 0) { recentRun = { team: null, points: 0, possessions: 0 }; }
          else {
            preTimeoutPhase = state.phase;
            state = emit(state, 'TIMEOUT_START', [], {
              team: callingTeam,
              reason: 'momentum',
            });
            state = { ...state, phase: 'TIMEOUT' };
            state = {
              ...state,
              timeouts: {
                remaining: {
                  ...state.timeouts.remaining,
                  [callingTeam]: state.timeouts.remaining[callingTeam] - 1,
                },
              },
            };
            recentRun = { team: null, points: 0, possessions: 0 };
          }
        }
      }

      void beforeScore;
      void beforePhase;

      continue;
    }

    // ── DEAD_FOUL → FT ────────────────────────────────────────────────────
    if (state.phase === 'DEAD_FOUL') {
      tickCtx = clearLiveTacticalContext(tickCtx);
      // P5.1: six personal fouls = disqualification. The dead-ball window
      // after a whistle is the legal substitution moment. A fouled-out
      // player is replaced by the first bench player of the same team
      // who is not already on court.
      // P5.2: a player with 4 fouls in a NON-final period is pulled for
      // protection (real coaches sit a 4-foul player in Q1-Q3); in the
      // 4th/OT they stay (foul trouble management is situational).
      // Scripted rotations opt out of BOTH automatic policies — the
      // caller owns the lineup (tests pin exact minute distributions).
      const autoManage = !input.scriptedSubs;
      for (const team of ['home', 'away'] as const) {
        const lineup = [...(team === 'home' ? state.lineups.home : state.lineups.away)];
        for (const jersey of lineup) {
          const foulCount = state.fouls.players[jersey] ?? 0;
          const protect = autoManage && foulCount === 4 && state.clocks.period <= 2;
          if (foulCount < 6 || !autoManage) {
            if (!(foulCount >= 6 && autoManage)) continue;
          }
          if (!autoManage) continue;
          const roster = team === 'home' ? input.home.roster : input.away.roster;
          const currentCourt = team === 'home' ? state.lineups.home : state.lineups.away;
          // Guard: only swap a player actually on court NOW (the lineup
          // snapshot may lag a prior substitution in the same dead-ball).
          if (!currentCourt.includes(jersey)) continue;
          // Bench pool must exclude players already on court AND players
          // who have fouled out (6+ personal fouls). The old filter only
          // checked the court, so a fouled-out player could be selected as
          // the replacement for another fouled-out teammate — an illegal
          // re-entry (measured: away subbed 12 out for 11 at 3663.8, then
          // re-inserted 12 for 20 at 3670.9 despite 12 having 6 fouls).
          // The illegal swap also churned poses (delete → bench recreate),
          // producing a 683ft/s teleport frame for jersey 12.
          const bench = roster
            .map((p) => p.jersey)
            .filter((j) => !currentCourt.includes(j) && (state.fouls.players[j] ?? 0) < 6);
          const replacement = bench[0];
          if (!replacement) continue; // no bench (5-man roster)
          state = emit(state, 'SUB', [jersey, replacement], {
            player_out_id: jersey,
            player_in_id: replacement,
            team,
            reason: foulCount >= 6 ? 'fouled_out' : 'foul_trouble',
          });
          // Ball ownership is handled inside applySub (the SUB event
          // fold): a live holder passes to the replacement, a dead-ball
          // holder is cleared.
          // Immutable lineup swap: build a NEW array. applySub already
          // validates the out-player is present and the in-player is not.
          const arr = team === 'home' ? state.lineups.home : state.lineups.away;
          const idx = arr.indexOf(jersey);
          if (idx >= 0) {
            const next = [...arr];
            next[idx] = replacement;
            state = {
              ...state,
              lineups: team === 'home'
                ? { ...state.lineups, home: next }
                : { ...state.lineups, away: next },
            };
          }
          state = emit(state, 'STATE_NOTE', [jersey], {
            player_id: jersey,
            note: foulCount >= 6 ? 'fouled_out' : 'foul_trouble_protected',
          });
        }
      }
      syncPackagesToLineups(state);
      state = applyDueSubs(state, pendingSubs);
      state = ensureAllPoses(state, input);
      if (tickCtx.pendingFtAttempts > 0 && tickCtx.pendingFtShooter) {
        const ftAlign = buildFtAlignment({
          homeLineup: state.lineups.home,
          awayLineup: state.lineups.away,
          shooterId: tickCtx.pendingFtShooter,
          shootingTeam: tickCtx.pendingFtTeam ?? 'home',
          baskets: state.baskets,
        });
        state = applyTargetsFromAlignment(state, ftAlign.players);
        // Players must physically reach their FT spots before the sequence
        // starts. 80 ticks = 8s worst case; the walk breaks early on arrival.
        ({ state } = waitUntilArrivedOrTimeout(state, tickCtx, snapshots, 80));
        // FT_START is emitted here — after the walk, not inside the LIVE
        // tick. Emitting it in the tick made applyEvent jump straight to
        // FT_SEQUENCE and skipped this alignment entirely, so free throws
        // were "shot" from wherever the player ended the previous play.
        state = emit(state, 'FT_START', [tickCtx.pendingFtShooter], {
          shooter_id: tickCtx.pendingFtShooter,
          team: tickCtx.pendingFtTeam,
          attempts: tickCtx.pendingFtAttempts,
        });
        state = { ...state, phase: 'FT_SEQUENCE' };
      } else {
        const inboundTeam = state.possession.team ?? lastPossessionTeam;
        const inboundPairRes = inboundPair(state, inboundTeam, packages[inboundTeam]);
        const inbounder = inboundPairRes.inbounder;
        const receiver = inboundPairRes.receiver;
        const ib = buildInboundAlignment({
          homeLineup: state.lineups.home,
          awayLineup: state.lineups.away,
          inboundTeam,
          inbounder,
          receiver,
          baskets: state.baskets,
          spot: 'baseline',
        });
        state = applyTargetsFromAlignment(state, ib.players);
        // The inbounder walks (not sprints) to the end line at dead-ball
        // speed — the old 2.0× scale made them sprint the court (18ft/s).
        // Other players walk simultaneously but are NOT waited on: they
        // keep moving into the inbound during the short flight, like a
        // real catch-and-advance.
        ({ state } = waitUntilArrivedOrTimeout(state, tickCtx, snapshots, 80, [inbounder]));
        // The inbound is a REAL pass: INBOUND_START puts the ball in the
        // inbounder's hands at the line; the LIVE tick then fires a short
        // pass to the receiver (INBOUND_TOUCH + POSSESSION_GAINED on the
        // catch), so the spectator sees the ball leave the baseline.
        const inbounderPose = state.poses[inbounder];
        if (!inbounderPose) throw new Error(`missing foul inbounder pose: ${inbounder}`);
        state = emit(state, 'INBOUND_START', [inbounder], {
          inbounder_id: inbounder,
          team: inboundTeam,
          spot_zone: 'baseline',
        });
        state = {
          ...state,
          ballMotion: { ...ballHeld(inbounderPose, inboundTeam), status: 'inbound' },
          ball: { ...state.ball, holderId: inbounder, status: 'inbound', zone: 'backcourt' },
        };
        if (process.env.SCDBG) { const g = globalThis as unknown as Record<string, string[]>; const arr = (g['__set'] ??= []); if (arr.length < 8) arr.push('g=' + state.clocks.game + ' ib=' + inbounder + ' phase=' + state.phase + ' last=' + (state.events[state.events.length - 1]?.type ?? 'none')); }
        tickCtx = { ...tickCtx, pendingInbound: { inbounder, receiver, team: inboundTeam }, pendingInboundTicks: 4, mode: 'TRANSITION', possessionPhase: 'ADVANCE', phaseTicksRemaining: 0, forceDecision: true };
        state = { ...state, phase: 'LIVE' };
        startReason = 'AFTER_INBOUND';
        lastPossessionTeam = inboundTeam;
      }
      continue;
    }

    if (state.phase === 'FT_SEQUENCE') {
      const tick = stepSimulationTick(state, tickCtx);
      state = tick.state;
      tickCtx = tick.ctx;
      snapshots.push(tick.snapshot);
      if (state.phase === 'LIVE') {
        state = ensureAllPoses(state, input);
        startReason = 'AFTER_MAKE';
        lastPossessionTeam = otherTeam(tickCtx.pendingFtTeam ?? lastPossessionTeam);
      }
      continue;
    }

    // ── other dead balls → inbound ────────────────────────────────────────
    if (deadBallPhase(state.phase)) {
      tickCtx = clearLiveTacticalContext(tickCtx);
      const before = state.phase;
      state = applyDueSubs(state, pendingSubs);
      state = ensureAllPoses(state, input);
      syncPackagesToLineups(state);
      if (before === 'DEAD_HELD') {
        state = { ...state, phase: 'JUMP_BALL' };
        continue;
      }
      const inboundTeam: TeamId = before === 'DEAD_MAKE'
        ? otherTeam(state.possession.team ?? lastPossessionTeam)
        : state.possession.team ?? 'home';
      const inboundPairRes = inboundPair(state, inboundTeam, packages[inboundTeam]);
      const inbounder = inboundPairRes.inbounder;
      const receiver = inboundPairRes.receiver;
      const alignment = buildInboundAlignment({
        homeLineup: state.lineups.home,
        awayLineup: state.lineups.away,
        inboundTeam,
        inbounder,
        receiver,
        baskets: state.baskets,
        spot: before === 'DEAD_MAKE' ? 'baseline' : 'sideline',
      });
      state = applyTargetsFromAlignment(state, alignment.players);
      ({ state } = waitUntilArrivedOrTimeout(state, tickCtx, snapshots, 80, [inbounder]));
      // Real-pass inbound (see DEAD_FOUL branch): the ball starts in the
      // inbounder's hands at the line; INBOUND_TOUCH/POSSESSION_GAINED
      // fire on the catch in the LIVE tick.
      const inbounderPose = state.poses[inbounder];
      if (!inbounderPose) throw new Error(`missing inbounder pose: ${inbounder}`);
      state = emit(state, 'INBOUND_START', [inbounder], {
        inbounder_id: inbounder,
        team: inboundTeam,
        spot_zone: before === 'DEAD_MAKE' ? 'baseline' : 'sideline',
      });
      state = {
        ...state,
        ballMotion: { ...ballHeld(inbounderPose, inboundTeam), status: 'inbound' },
        ball: { ...state.ball, holderId: inbounder, status: 'inbound', zone: 'backcourt' },
        phase: 'LIVE',
      };
      if (process.env.SCDBG) { const g = globalThis as unknown as Record<string, string[]>; const arr = (g['__set'] ??= []); if (arr.length < 8) arr.push('g=' + state.clocks.game + ' ib=' + inbounder + ' phase=' + state.phase + ' last=' + (state.events[state.events.length - 1]?.type ?? 'none')); }
        tickCtx = { ...tickCtx, pendingInbound: { inbounder, receiver, team: inboundTeam }, pendingInboundTicks: 4, mode: 'TRANSITION', possessionPhase: 'ADVANCE', phaseTicksRemaining: 0, forceDecision: true };
      startReason = before === 'DEAD_MAKE' ? 'AFTER_MAKE' : 'AFTER_INBOUND';
      lastPossessionTeam = inboundTeam;
      continue;
    }

    if (state.phase === 'JUMP_BALL') {
      // held-ball re-tip simplified
      const w: TeamId = rng.next() < 0.5 ? 'home' : 'away';
      const h = firstInLineup(state, w);
      state = emit(state, 'JUMP_BALL_TAP', [], { tapping_team: w, tapper_id: h });
      state = emit(state, 'POSSESSION_GAINED', [h], { team: w, player_id: h });
      state = { ...state, phase: 'LIVE' };
      lastPossessionTeam = w;
      startReason = 'AFTER_JUMP_BALL';
      continue;
    }

    if (state.phase === 'PERIOD_BREAK' || state.phase === 'HALFTIME') {
      // Rotation is handled by the staggered default/scripted subs at dead
      // balls; the halftime break only needs the package sync (the closing
      // five of Q2 keeps playing Q3's opening stretch).
      syncPackagesToLineups(state);
      // §8.3 quarter break +15 / halftime +30 for every player.
      tickCtx = { ...tickCtx, stamina: applyPeriodRest(tickCtx.stamina, state.phase === 'HALFTIME') };
      // Ball is dead during period breaks; bench swap at halftime invalidates
      // the prior holderId, so clear ballMotion before pushing break snapshots.
      state = {
        ...state,
        // Break frames are dead-ball frames. Reconcile poses at the same
        // boundary so no player continues to advertise `hasBall` after the
        // holder is cleared; otherwise the spectator stream contains a
        // dead ball with a phantom live owner for the entire break.
        poses: reconcileHasBall(state.poses, null),
        ballMotion: ballDead({ x: 0.5, y: 0.5 }),
        ball: { holderId: null, status: 'dead', zone: state.ball.zone },
      };
      if (state.phase !== 'POST_GAME') {
        for (let i = 0; i < 20; i++) {
          state = {
            ...state,
            realClock: Math.round((state.realClock + TICK_DT) * 10) / 10,
          };
          snapshots.push(
            snapshotFromState({
              t_real: state.realClock,
              t_game: state.clocks.game,
              shotClock: state.clocks.shot,
              period: state.period,
              phase: state.phase,
              score: state.score,
              poses: state.poses,
              ball: state.ballMotion,
              // Break frames carry the rested stamina (applyPeriodRest
              // already ran above) instead of rendering stm=-1.
              stamina: tickCtx.stamina,
            }),
          );
        }
      }
      // state.period already advanced by applyPeriodEnd to the upcoming period.
      // A GAME_END transition is terminal: do not run reset/alignment/inbound
      // choreography after POST_GAME, which otherwise moves players for several
      // seconds after the horn and exposes them as impossible teleports.
      if (state.phase === 'POST_GAME') continue;
      // Rebuild a legal inbound formation before the next live tick; pruning
      // new lineups to the center would stack all ten players at mid-court.
      const nextPeriod = state.period;
      state = resetForNewPeriod(state);
      const newTeam = otherTeam(lastPossessionTeam);
      const inboundPairRes = inboundPair(state, newTeam, packages[newTeam]);
      const inbounder = inboundPairRes.inbounder;
      const receiver = inboundPairRes.receiver;
      const alignment = buildInboundAlignment({
        homeLineup: state.lineups.home,
        awayLineup: state.lineups.away,
        inboundTeam: newTeam,
        inbounder,
        receiver,
        baskets: state.baskets,
        spot: 'sideline',
      });
      state = applyTargetsFromAlignment(state, alignment.players);
      ({ state } = waitUntilArrivedOrTimeout(state, tickCtx, snapshots, 30));

      const trans = transition(state, 'BREAK_END');
      if (trans) {
        for (const et of trans.emit_events) {
          if (et === 'PERIOD_START') {
            state = emit(state, 'PERIOD_START', [], { period: nextPeriod });
          }
        }
        state = { ...state, phase: trans.phase };
      } else {
        state = emit(state, 'PERIOD_START', [], { period: nextPeriod });
        state = { ...state, phase: 'LIVE' };
      }

      const receiverPose = state.poses[receiver];
      if (!receiverPose) throw new Error(`missing period-start receiver pose: ${receiver}`);
      state = {
        ...state,
        possession: { team: newTeam },
        ballMotion: ballHeld(receiverPose, newTeam),
        ball: { holderId: receiver, status: 'held', zone: 'backcourt' },
        phase: 'LIVE',
      };
      state = emit(state, 'POSSESSION_GAINED', [receiver], {
        team: newTeam,
        player_id: receiver,
      });
      startReason = 'AFTER_INBOUND';
      lastPossessionTeam = newTeam;
      continue;
    }

    if (state.phase === 'OVERTIME_SETUP') {
      state = resetForNewPeriod(state);
      const trans = transition(state, 'BREAK_END');
      if (trans) {
        for (const et of trans.emit_events) {
          if (et === 'PERIOD_START') {
            state = emit(state, 'PERIOD_START', [], { period: state.clocks.period });
          }
        }
        state = { ...state, phase: trans.phase };
      }
      continue;
    }

    if (state.phase === 'TIMEOUT') {
      state = emit(state, 'TIMEOUT_END', [], {});
      state = applyDueSubs(state, pendingSubs);
      state = ensureAllPoses(state, input);
      syncPackagesToLineups(state);
      // Restore the pre-timeout phase: a momentum timeout is called from
      // inside a dead-ball flow (DEAD_MAKE/DEAD_FOUL/…). The original
      // inbound/FT setup must run AFTER the timeout, not be skipped —
      // resuming to LIVE orphans the ball (phantom rebound). Only a
      // timeout called from true LIVE play (none currently) resumes LIVE.
      const resume = preTimeoutPhase ?? 'LIVE';
      preTimeoutPhase = null;
      state = { ...state, phase: resume };
      continue;
    }

    if (state.phase === 'DEAD_PERIOD_END') {
      const scoreDiff = state.score.home - state.score.away;
      const trigger = periodRouterTrigger(endedPeriod || state.period, scoreDiff);
      const trans = transition(state, trigger);
      if (trans === null) throw new Error(`no transition ${trigger}`);
      if (trans.phase === 'HALFTIME') state = emit(state, 'HALFTIME', [], {});
      for (const et of trans.emit_events) {
        if (et === 'GAME_END') state = emit(state, 'GAME_END', [], {});
        if (et === 'PERIOD_START') {
          state = emit(state, 'PERIOD_START', [], { period: (endedPeriod || state.period) + 1 });
        }
      }
      state = {
        ...state,
        phase: trans.phase,
        ...(trans.phase === 'POST_GAME'
          ? {
            ballMotion: ballDead({ x: state.ballMotion.x, y: state.ballMotion.y }),
            ball: { ...state.ball, holderId: null, status: 'dead' as const },
            poses: reconcileHasBall(state.poses, null),
          }
          : {}),
      };
      continue;
    }

    state = emit(state, 'GAME_END', [], {});
    state = {
      ...state,
      phase: 'POST_GAME',
      ballMotion: ballDead({ x: state.ballMotion.x, y: state.ballMotion.y }),
      ball: { ...state.ball, holderId: null, status: 'dead' },
      poses: reconcileHasBall(state.poses, null),
    };
  }
  const finalSnapshot = snapshotFromState({
    t_real: state.realClock,
    t_game: state.clocks.game,
    shotClock: state.clocks.shot,
    period: state.period,
    phase: state.phase,
    score: state.score,
    poses: state.poses,
    ball: state.ballMotion,
    stamina: tickCtx.stamina,
    lastEventType: 'GAME_END',
    lastEventSeq: state.seq,
  });
  const lastSnapshot = snapshots[snapshots.length - 1];
  if (lastSnapshot && lastSnapshot.t_real === finalSnapshot.t_real) {
    snapshots[snapshots.length - 1] = finalSnapshot;
  } else {
    snapshots.push(finalSnapshot);
  }

  const result = buildGameResult(state, input, possessionLog, rosterMaps);
  return { ...result, snapshots, stamina_report: fatigueReport(tickCtx.stamina) };
}
