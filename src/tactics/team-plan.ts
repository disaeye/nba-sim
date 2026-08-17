import type { LineupPackage, RoleBinding } from '../identity/types.js';
import { resolveLineupIdentity, capabilitiesForJersey } from '../identity/roles.js';
import type { ActionKind } from '../decision/types.js';
import type { TeamId } from '../state/types.js';
import type { LiveCourtSense } from '../perception/live-court.js';
import type { Rng } from '../rng/types.js';
import { weighted } from '../rng/helpers.js';
import { familyForSystem, systemForKind, type TacticalSystemId } from './system-select.js';
import { TACTICAL_SYSTEMS, systemFitness } from './systems.js';
import { decideHandlerAction } from '../decision/expected-value.js';

/**
 * Real-time matchup fit for a tactic: how favourable the CURRENT on-court
 * matchups are for each action family. Positive = advantage. This is what
 * makes the tactic mix an emergent property of the floor — a weak defender
 * on the screener makes PNR attractive, a dominated creator matchup makes
 * ISO attractive, an open paint makes DRIVE_KICK attractive.
 */
function matchupFit(
  kind: TeamPlanKind,
  sense: LiveCourtSense,
  matchups: Readonly<Record<string, string>>,
  screener: string,
): number {
  const handlerAb = sense.abilities[sense.handler] ?? null;
  const defId = matchups[sense.handler] ?? null;
  const defAb = defId ? (sense.abilities[defId] ?? null) : null;
  const handlerGap = (handlerAb?.creation ?? 0.5) - (defAb?.onBallDefense ?? 0.5);
  const screenerDefId = matchups[screener] ?? null;
  const screenerDefAb = screenerDefId ? (sense.abilities[screenerDefId] ?? null) : null;
  const screenerGap = 0.5 - (screenerDefAb?.onBallDefense ?? 0.5);
  const rimOpen = sense.paintDefenders <= 1 ? 0.3 : 0;
  const openShooters = sense.openTeammates.length * 0.22;
  switch (kind) {
    case 'ISO':
      return handlerGap * 1.2 + (handlerAb?.pullUp ?? 0.5) * 0.4;
    case 'PNR_ROLL':
    case 'PNR_POP':
      // P3.x: the screener-gap term is genuinely matchup-driven (weak
      // screener defender → PNR), but creation must not be a flat bonus —
      // it made PNR's fit always-positive and the mix collapsed to it.
      return handlerGap * 0.6 + Math.max(0, screenerGap) + rimOpen + (handlerAb?.creation ?? 0.5) * 0.15;
    case 'DRIVE_KICK':
      return Math.max(0, 1 - sense.paintDefenders * 0.35) * 0.8 + openShooters
        + (handlerAb?.rimFinishing ?? 0.5) * 0.3 + handlerGap * 0.5;
    case 'POST_UP':
      return ((sense.abilities[screener]?.postPlay ?? 0.5) - (screenerDefAb?.postPlay ?? 0.5)) * 1.2 + rimOpen;
    case 'OFF_BALL_SCREEN':
      return openShooters + (handlerAb?.passing ?? 0.5) * 0.5 + handlerGap * 0.3;
    case 'HANDOFF':
      return handlerGap * 0.8 + (handlerAb?.creation ?? 0.5) * 0.3;
    default:
      return 0;
  }
}

// ─── tactic catalog ─────────────────────────────────────────────────────────

export type TeamPlanKind =
  | 'TRANSITION_PUSH'
  | 'PNR_ROLL'
  | 'PNR_POP'
  | 'DRIVE_KICK'
  | 'POST_UP'
  | 'OFF_BALL_SCREEN'
  | 'ISO'
  | 'HANDOFF';

/** Back-compat aliases — older code may still check for 'PNR'. */
export type TeamPlanKindLegacy = 'PNR' | 'ISO' | 'BROKEN_PLAY';

export type TeamPlanStage = 'ADVANCE' | 'SET' | 'SCREEN_APPROACH' | 'SCREEN_USE' | 'ADVANTAGE' | 'TERMINAL';
export type TeamRole = 'handler' | 'screener' | 'strong_corner' | 'weak_corner' | 'slot';
export type ScreenCoverage = 'DROP' | 'SWITCH' | 'BLITZ' | 'HEDGE' | 'ICE';

export interface ScreenDefensePlan {
  readonly mode: ScreenCoverage;
  readonly onBallDefender: string | null;
  readonly screenerDefender: string | null;
  readonly switchesAtUse: boolean;
  /** P3.6 defensive scheme — MAN (default) or 2-3 ZONE. */
  readonly zone: 'MAN' | 'ZONE_2_3';
}

export interface TeamAssignment {
  readonly jersey: string;
  readonly role: TeamRole;
  readonly action: ActionKind;
  readonly targetJersey: string | null;
  readonly lane: 'middle' | 'strong' | 'weak' | 'rim';
}

/** Compact, normalized waypoints for one offensive movement. */
export interface TacticalRoute {
  readonly kind: string;
  readonly points: readonly { readonly x: number; readonly y: number }[];
}

export interface TeamPlan {
  readonly kind: TeamPlanKind;
  /** Tactical system that selected this plan's action family. */
  readonly systemId: TacticalSystemId;
  readonly stage: TeamPlanStage;
  readonly offense: TeamId;
  readonly handler: string;
  readonly assignments: readonly TeamAssignment[];
  readonly matchups: Readonly<Record<string, string>>;
  readonly screenDefense: ScreenDefensePlan | null;
  /** Tactical screen lifecycle retained after the set fact for snapshot/audit. */
  readonly screenActive?: boolean;
  /** Coverage chosen for this physical PNR; stable until the possession changes. */
  readonly committedScreenCoverage?: ScreenCoverage | null;
  /** True only after the SETUP placeholder is replaced by EXECUTE selection. */
  readonly selected?: boolean;
  /** Designed FEED target for the family's second action: the curling shooter
   * (OFF_BALL_SCREEN) or the sealed post hub (POST_UP). */
  readonly feedTargetJersey?: string | null;
  /** Action-specific spatial routes generated with the current live read. */
  readonly routes?: Readonly<Record<string, TacticalRoute>>;
}
export interface ScreenExecution {
  readonly screenerId: string;
  readonly handlerId: string;
  readonly defenderId: string | null;
  readonly phase: 'APPROACH' | 'SET' | 'USE' | 'EXIT';
  readonly anchorX: number;
  readonly anchorY: number;
  readonly handlerStartX: number;
  readonly handlerStartY: number;
}

// ─── helpers ────────────────────────────────────────────────────────────────

function remainingLineup(lineup: readonly string[], excluded: readonly string[]): string[] {
  const used = new Set(excluded);
  return lineup.filter((jersey) => !used.has(jersey));
}

/** Zones from which a shot is a three-pointer. */
const THREE_ZONES: ReadonlySet<string> = new Set([
  'wing_L', 'wing_R', 'corner_L', 'corner_R', 'slot_L', 'slot_R', 'frontcourt_center',
]);
const MID_ZONES: ReadonlySet<string> = new Set(['elbow_L', 'elbow_R']);

function isHandlerOpen(sense: LiveCourtSense): boolean {
  return sense.onBallDistanceFt >= 3;
}

/** Does this tactic use a physical screen (ball screen or off-ball pin)? */
function isScreenTactic(kind: TeamPlanKind): boolean {
  return kind === 'PNR_ROLL' || kind === 'PNR_POP' || kind === 'OFF_BALL_SCREEN';
}

// ─── defense ────────────────────────────────────────────────────────────────

function chooseScreenDefense(
  sense: LiveCourtSense,
  matchups: Readonly<Record<string, string>>,
  screener: string,
  kind: TeamPlanKind,
): ScreenDefensePlan {
  const onBallDefender = matchups[sense.handler] ?? sense.onBallDefender;
  const screenerDefender = matchups[screener] ?? null;
  const handlerDistance = sense.onBallDistanceFt;
  const screenerPose = sense.offensePlayers.find((player) => player.jersey === screener)?.pose;
  const rimDistance = screenerPose ? Math.hypot((sense.rim.x - screenerPose.x) * 94, (sense.rim.y - screenerPose.y) * 50) : 18;
  // DROP is only declared once the screener defender can actually reach the
  // rim-side help lane during the current physical screen. Before that point
  // the coverage remains a conservative switch/contain state, so snapshots
  // cannot advertise a drop while the defender is still attached to the
  // perimeter matchup.
  const switchBias = kind === 'PNR_POP' ? 6 : 5.5;
  const screenerDefenderPose = screenerDefender
    ? sense.defensePlayers.find((player) => player.jersey === screenerDefender)?.pose
    : undefined;
  const defenderRimDistance = screenerDefenderPose
    ? Math.hypot((sense.rim.x - screenerDefenderPose.x) * 94, (sense.rim.y - screenerDefenderPose.y) * 50)
    : Infinity;
  const canPhysicallyDrop = defenderRimDistance <= 20;
  // P3.1 coverage catalog — the choice is defensive-identity driven:
  //   DROP: screener defender sinks to the rim (soft coverage); chosen
  //     when the rim protector is close and the handler is not an elite
  //     pull-up shooter.
  //   BLITZ: both defenders trap the handler; chosen against weak
  //     handleSecurity — the risk (open roller) is worth forcing the
  //     ball out of the creator's hands.
  //   HEDGE: screener defender steps out and recovers; balanced default
  //     when the screener can pop.
  //   ICE: force the handler baseline (away from the screen); used
  //     against strong pull-up shooters going downhill.
  //   SWITCH: everyone switches; used when the defense is switchable
  //     (small lineup, no rim protector to drop).
  const handlerData = sense.playerData?.[sense.handler] ?? null;
  const screenerData = screener ? (sense.playerData?.[screener] ?? null) : null;
  const handlerPullup = sense.abilities[sense.handler]?.pullUp ?? 0.5;
  const handlerHandle = sense.abilities[sense.handler]?.handleSecurity ?? 0.5;
  const screenerCanPop = kind === 'PNR_POP' || (screenerData ? screenerData.ability.CS3 / 99 > 0.55 : false);
  // P5.5 coach biases shift the thresholds: a blitz-happy coach traps
  // below the neutral handle bar; a zone coach slides the scheme toward
  // the zone flag earlier.
  const defCoach = sense.coach?.[sense.defense] ?? undefined;
  const blitzBias = defCoach?.blitzBias ?? 0.5;
  const blitzThreshold = 0.45 + (blitzBias - 0.5) * 0.2; // 0.35..0.55
  // ICE targets strong pull-up creators going downhill — the pullUp
  // capability IS the definition (elite pull-up guards like Morant get
  // ICE'd regardless of raw three-point volume). An earlier T3>55
  // tendency refinement made the path depend on playerData presence and
  // on generated rosters never having both high T3 and high pullUp
  // (measured: zero ICE frames across 7 seeds even with 0.8+ pullUp
  // handlers). The pick-and-roll handler's pullUp alone gates ICE now.
  const iceThreat = handlerPullup >= 0.75;
  let mode: ScreenCoverage;
  if (handlerHandle < blitzThreshold) {
    // BLITZ: trap a weak-handle creator anywhere on the floor — the
    // risk (open roller) is worth forcing the ball out of their hands.
    mode = 'BLITZ';
  } else if (iceThreat) {
    // ICE: wall the middle for an elite pull-up threat. Called on BOTH
    // high and deep screens (real ICE runs against the scorer, not the
    // screener depth) — the old drop-range gate tied it to the screener
    // being ≤18ft from the rim and made it structurally rare.
    mode = 'ICE';
  } else if (handlerDistance <= switchBias && rimDistance <= 18 && canPhysicallyDrop) {
    // In drop range: soft coverage — the rim protector sinks.
    mode = 'DROP';
  } else if (screenerCanPop) {
    mode = 'HEDGE';
  } else {
    mode = 'SWITCH';
  }
  // P3.6 zone scheme: a defense with a weak rim protector and no
  // switchable wings (all defenders slow) may sit in a 2-3 zone to
  // protect the paint against a drive-heavy offense. The zone flag is a
  // scheme label the spatial layer renders as zone slots. PNR sets are
  // excluded — a screen against a zone is not a pick-and-roll (the
  // screen's defender does not switch/trap), and the ScreenSet audit
  // expects a MAN coverage exposure.
  let zone: 'MAN' | 'ZONE_2_3' = 'MAN';
  // 持球战术(HANDOFF/ISO/DRIVE_KICK)同样排除:zone的5人站slot不追踪
  // 持球人,handoff后receiver直接无人防守(审计SCREEN_BEAT_TOO_EASILY:
  // HANDOFF ADVANTAGE时handler距最近防守人18ft,on-ball defender在
  // zone slot上不动)。Zone只适合"球在传导"的流动进攻,不适合明确的
  // 持球人单打/手递手。
  const isBallCarryTactic = kind === 'HANDOFF' || kind === 'ISO' || kind === 'DRIVE_KICK' || kind === 'POST_UP';
  if (!isScreenTactic(kind) && !isBallCarryTactic) {
    const rimProtector = sense.defensePlayers.some((d) => {
      const ab = sense.abilities[d.jersey] ?? null;
      return (ab?.helpDefense ?? 0.5) >= 0.75;
    });
    const switchable = sense.defensePlayers.every((d) => {
      const ab = sense.abilities[d.jersey] ?? null;
      return (ab?.onBallDefense ?? 0.5) >= 0.6;
    });
    // Coach zone bias: a zone-heavy coach plays the 2-3 regardless of
    // personnel (Syracuse/Miami heritage) — the bias shifts the structural
    // trigger so scheme identity expresses in the frame stream. A man
    // coach (bias ≤ 0.2) never zones. (The old trigger ignored zoneBias
    // entirely — the coach field was validated on input and never read,
    // so scheme identity was dead config.)
    const zoneBias = defCoach?.zoneBias ?? 0.5;
    const structuralZone = !rimProtector && !switchable && sense.paintDefenders <= 1 && sense.shotClock <= 18;
    if (structuralZone || zoneBias >= 0.7) {
      zone = 'ZONE_2_3';
    }
    if (zoneBias <= 0.2) zone = 'MAN';
  }
  return { mode, onBallDefender, screenerDefender, switchesAtUse: mode === 'SWITCH' || mode === 'HEDGE', zone };
}

type MatchupRole = 'creator' | 'screener' | 'spacer';

function roleForBinding(jersey: string, binding: RoleBinding): MatchupRole {
  if (jersey === binding.screener) return 'screener';
  if (jersey === binding.spacer_strong || jersey === binding.spacer_weak) return 'spacer';
  return 'creator';
}

function roleForPackage(jersey: string, lineupPackage: LineupPackage): MatchupRole {
  if (lineupPackage.usageProfile.screener.includes(jersey)) return 'screener';
  if (lineupPackage.usageProfile.spacer.includes(jersey)) return 'spacer';
  return 'creator';
}

function chooseInitialMatchups(
  sense: LiveCourtSense,
  binding: RoleBinding,
  defensePackage: LineupPackage | null,
): Record<string, string> {
  // Constrain the matchup by role first, then use live distance among
  // compatible candidates. Bigs guard bigs; perimeter players guard
  // perimeter players while preserving the current court geometry.
  const available = [...sense.defensePlayers];
  const offenseOrder = [
    sense.handler,
    binding.primary_creator,
    binding.secondary_creator,
    binding.screener,
    binding.spacer_strong,
    binding.spacer_weak,
  ];
  const result: Record<string, string> = {};
  const seenAttackers = new Set<string>();
  // Size-sorted assignment: NBA defenses match bigs on bigs. The old
  // usage-role buckets (creator/screener/spacer) describe OFFENSE usage, not
  // body size — a 225cm 'spacer' center guarded opposing wings while the
  // 209cm 'screener' PF took their center (measured height mismatch stayed
  // 12.9-13.1cm ≈ random pairing). With heights available, both sides sort by
  // height and pair rank-to-rank (distance breaks ties); without heights the
  // role buckets still apply.
  const heightOf = (jersey: string): number | null => {
    const data = sense.playerData?.[jersey];
    return data?.physical.H ?? null;
  };
  const offenseHeights = sense.offensePlayers
    .map((p) => ({ jersey: p.jersey, h: heightOf(p.jersey) }))
    .filter((x): x is { jersey: string; h: number } => x.h !== null);
  const defenseHeights = sense.defensePlayers
    .map((p) => ({ jersey: p.jersey, h: heightOf(p.jersey) }))
    .filter((x): x is { jersey: string; h: number } => x.h !== null);
  if (offenseHeights.length >= 5 && defenseHeights.length >= 5) {
    const sortedOffense = [...offenseHeights].sort((a, b) => a.h - b.h);
    const sortedDefense = [...defenseHeights].sort((a, b) => a.h - b.h);
    for (let i = 0; i < 5; i += 1) {
      result[sortedOffense[i]!.jersey] = sortedDefense[i]!.jersey;
    }
    return result;
  }
  for (const attackerJersey of offenseOrder) {
    if (seenAttackers.has(attackerJersey)) continue;
    seenAttackers.add(attackerJersey);
    const attacker = sense.offensePlayers.find((player) => player.jersey === attackerJersey);
    if (!attacker || available.length === 0) continue;
    const role = roleForBinding(attackerJersey, binding);
    const compatible = defensePackage
      ? available.filter((defender) => roleForPackage(defender.jersey, defensePackage) === role)
      : [];
    // The handler gets the NEAREST defender regardless of role compatibility.
    // In transition, the lineup role of the nearest defender (e.g. spacer)
    // may differ from the handler's role (creator), but matching them to
    // a farther compatible defender leaves the handler 11ft open while a
    // spacer defender 6ft away is assigned to a weak-side attacker.
    // All other matchups still use role filtering for stable big-on-big /
    // perimeter-on-perimeter assignments.
    const candidates = attackerJersey === sense.handler ? available : compatible.length > 0 ? compatible : available;
    let best = candidates[0]!;
    let bestDistance = Infinity;
    for (const defender of candidates) {
      const dx = (defender.pose.x - attacker.pose.x) * 94;
      const dy = (defender.pose.y - attacker.pose.y) * 50;
      const distance = Math.hypot(dx, dy);
      if (distance < bestDistance) {
        bestDistance = distance;
        best = defender;
      }
    }
    result[attackerJersey] = best.jersey;
    const index = available.findIndex((defender) => defender.jersey === best.jersey);
    if (index >= 0) available.splice(index, 1);
  }
  return result;
}

function stableMatchups(
  sense: LiveCourtSense,
  previous: TeamPlan | null,
  binding: RoleBinding,
  defensePackage: LineupPackage | null,
): Readonly<Record<string, string>> {
  const defenders = new Set(sense.defensePlayers.map((player) => player.jersey));
  const prior = previous?.matchups;
  if (prior) {
    const values = Object.values(prior);
    // A prior matchup set is only reusable when the HANDLER is unchanged:
    // a pass that moves the ball to a new handler (5→4) must re-allocate
    // the on-ball defender, otherwise the new handler's man stays glued
    // to their old spot and the handler shoots wide open (measured 14ft
    // of daylight on catch-and-shoot threes).
    const priorHandler = previous?.handler;
    if (values.length === sense.offensePlayers.length
      && new Set(values).size === values.length
      && values.every((jersey) => defenders.has(jersey))
      && priorHandler === sense.handler) {
      return prior;
    }
  }
  return chooseInitialMatchups(sense, binding, defensePackage);
}

// ─── tactic selection ───────────────────────────────────────────────────────

// ─── per-tactic handler terminal action ─────────────────────────────────────

function actionForScreener(
  stage: TeamPlanStage,
  kind: TeamPlanKind,
  screenExecution: ScreenExecution | null,
): ActionKind {
  switch (kind) {
    case 'PNR_ROLL':
      // Set screen, then roll hard to the rim
      if (stage === 'SCREEN_APPROACH' || (stage === 'SCREEN_USE' && screenExecution?.phase === 'SET')) return 'screen';
      if (stage === 'SCREEN_USE' || stage === 'ADVANTAGE') return 'cut';
      return 'space';

    case 'PNR_POP':
      // Set screen, then pop to the three-point line
      // Once the screen is used, the popper must immediately relocate to
      // the perimeter. The physical screen lifecycle is authoritative;
      // retaining `screen` here after USE makes the big keep walking through
      // the handler instead of creating the designed kick-out window.
      if (stage === 'SCREEN_APPROACH' || (stage === 'SCREEN_USE' && screenExecution?.phase === 'SET')) return 'screen';
      if (stage === 'SCREEN_USE' || stage === 'ADVANTAGE') return 'relocate'; // pop = relocate to perimeter
      return 'space';

    case 'POST_UP':
      // Seal in the post, then back down (stay put to receive entry pass)
      if (stage === 'SCREEN_APPROACH' || stage === 'SET') return 'relocate'; // get to post position
      if (stage === 'SCREEN_USE' || stage === 'ADVANTAGE') return 'hold'; // sealed, waiting for entry / backing down
      return 'space';

    case 'OFF_BALL_SCREEN':
      // Set an off-ball screen for a spacer
      if (stage === 'SCREEN_APPROACH') return 'screen';
      if (stage === 'SCREEN_USE' || stage === 'ADVANTAGE') return 'space'; // after setting, space out
      return 'space';

    case 'DRIVE_KICK':
      // Weak-side screen or just space
      return 'space';

    case 'HANDOFF':
      // The handoff giver must approach the handler for the exchange.
      // During SETUP (SET stage) and SCREEN_APPROACH, the screener moves
      // toward the handler — without this the handoff never forms (the
      // giver parked 15ft away as a spacer and the "handoff" became a
      // generic corner pass). After the exchange (ADVANTAGE), space out.
      if (stage === 'SCREEN_APPROACH' || stage === 'SET') return 'relocate';
      return 'space';

    case 'ISO':
    default:
      return 'space';
  }
}

// ─── per-tactic spacer actions ──────────────────────────────────────────────

function actionForSpacer(
  role: 'strong_corner' | 'weak_corner' | 'slot',
  stage: TeamPlanStage,
  kind: TeamPlanKind,
  sense: LiveCourtSense,
  jersey: string,
): ActionKind {
  // OFF_BALL_SCREEN: the strong_corner spacer curls off the screen
  if (kind === 'OFF_BALL_SCREEN' && role === 'strong_corner') {
    if (stage === 'SCREEN_USE' || stage === 'ADVANTAGE') return 'cut'; // curl to open spot
    return 'relocate';
  }
  // POST_UP: weak side stays spread for kick-out
  if (kind === 'POST_UP') return 'space';

  // ── P3.5 off-ball read & react ───────────────────────────────────────
  // The spacer READS their defender's coverage instead of mechanically
  // holding spacing:
  //   - denied (defender ≤4ft, between me and the ball): backdoor — the
  //     deny overplays the pass, the rim cut is open.
  //   - sagged (defender ≥8ft): hold the pocket and stay a live
  //     catch-and-shoot target.
  //   - neutral: hold spacing; relocate when the advantage is created.
  const pose = sense.offensePlayers.find((p) => p.jersey === jersey)?.pose;
  if (pose) {
    // A backdoor cut that has already reached the rim is over: an unreceived
    // cut must not linger in 'cut' for 10-20s (measured: 21s cut runs where
    // the cutter stood at the rim corner, defender attached, waiting for a
    // pass the handler never made). Once inside 8ft of the rim the cutter
    // exits to spacing — the read resets instead of looping the cut.
    const poseRimDist = Math.hypot(
      (pose.x - sense.rim.x) * 94,
      (pose.y - sense.rim.y) * 50,
    );
    const nearRimAlready = poseRimDist <= 8;
    let closestDef = Infinity;
    for (const d of sense.defensePlayers) {
      const dd = Math.hypot((d.pose.x - pose.x) * 94, (d.pose.y - pose.y) * 50);
      if (dd < closestDef) closestDef = dd;
    }
    const ballToMe = Math.hypot((sense.ball.x - pose.x) * 94, (sense.ball.y - pose.y) * 50);
    // Is the defender between ball and me (deny posture)?
    let denied = false;
    if (!nearRimAlready) {
      for (const d of sense.defensePlayers) {
        const dToBall = Math.hypot((d.pose.x - sense.ball.x) * 94, (d.pose.y - sense.ball.y) * 50);
        const dToMe = Math.hypot((d.pose.x - pose.x) * 94, (d.pose.y - pose.y) * 50);
        // Defender closer to the pass line than the receiver, tight to me.
        // Defender must be VERY tight (≤3ft) AND between ball and me.
        // The old ≤4ft threshold fired constantly because weak-side sag
        // at 5ft + natural defensive positioning registered as "denied",
        // turning every spacer into a perpetual cutter.
        if (closestDef <= 3 && dToBall < ballToMe && dToMe <= closestDef + 0.5) {
          denied = true;
          break;
        }
      }
    }
    if (denied && stage !== 'SCREEN_APPROACH') return 'cut'; // backdoor
    // Sagged defender (≥6ft): hold spacing and stay a catch-shoot target.
    // Lowered from 8ft to 6ft so the 5ft weak-side sag doesn't trigger
    // constant repositioning.
    if (closestDef >= 6) return 'space'; // sagged: stay live
  }
  // Default: hold spacing, relocate when advantage is created
  if (stage === 'ADVANTAGE') return 'relocate';
  return 'space';
}

// ─── main plan builder ──────────────────────────────────────────────────────

/**
 * Selects one compatible five-player branch from present-tense perception.
 * The tactic kind is selected once per possession (sticky) and then the
 * stage machine advances through formation → execution → terminal.
 */
export function buildTeamPlan(args: {
  readonly sense: LiveCourtSense;
  readonly lineup: readonly string[];
  readonly lineupPackage: LineupPackage;
  readonly defenseLineupPackage?: LineupPackage | null;
  readonly binding: RoleBinding;
  readonly possessionPhase: 'ADVANCE' | 'SETUP' | 'EXECUTE';
  readonly mode: 'TRANSITION' | 'HALFCOURT';
  readonly previous: TeamPlan | null;
  readonly screenReady: boolean;
  /** Optional fixed target retained while a PNR pop/roll develops. */
  readonly screenExecution?: ScreenExecution | null;
  readonly handoffGuard?: { readonly giverId: string; readonly receiverId: string } | null;
  readonly rng?: Rng | null;
  readonly catchWindowTicks?: number;
  /** Ticks since EXECUTE began — gates the immediate shot (P3.x). */
  readonly executeCooldownTicks?: number;
  /** Active play id — the play's family becomes the tactic while active. */
  readonly playId?: string | null;
  /** Most recent first-class handling beat. */
  readonly lastHandlingAction?: import('../decision/types.js').HandlingActionKind | null;
  /** Cooldown preventing immediate repeat of a handling beat. */
  readonly handlingActionCooldownTicks?: number;
  /** Committed pass/handoff in flight: the plan must keep displaying the
   * promised receiver until the ball is released — a re-read that argmaxes
   * a different target every 0.1s makes the drawn execution line lie. */
  readonly committedExchange?: { readonly fromJersey: string; readonly toJersey: string } | null;
  /** Live drive lifecycle for the handler: the re-plan must keep the
   * drive action (commitment) instead of re-argmaxing into a pass/hold. */
  readonly driveCommitment?: { readonly jersey: string } | null;
  /** Consecutive passes without an attack — the EV layer prices the chain. */
  readonly passChainSinceAttack?: number;
}): TeamPlan {
  const {
    sense,
    lineup,
    lineupPackage,
    defenseLineupPackage = null,
    binding,
    possessionPhase,
    mode,
    previous,
    screenReady,
    screenExecution = null,
    handoffGuard = null,
    rng = null,
    catchWindowTicks = 0,
    executeCooldownTicks = 0,
    playId = null,
    lastHandlingAction = null,
    handlingActionCooldownTicks = 0,
    committedExchange = null,
    driveCommitment = null,
    passChainSinceAttack = 0,
  } = args;
  const handler = sense.handler;
  const handlerRole = roleForBinding(handler, binding);
  const frontcourt = sense.attackDirection === 1 ? sense.ball.x >= 0.5 : sense.ball.x <= 0.5;

  // Resolve role jerseys
  // Resolve role jerseys. A binding may name a BENCHED jersey: staggered
  // substitutions create mixed lineups that match no lineup package, so
  // syncPackagesToLineups cannot refresh the binding and the old unit's
  // screener stays referenced while sitting on the bench (measured: a
  // PNR_ROLL SCREEN_APPROACH stalled 12s — screener 18 off-court, lineup
  // 16,12,13,14,15, SCREEN_SET never fired). Any binding slot not in the
  // current lineup falls back to the best on-court substitute: the
  // package's role lists first, then lineup order.
  const onCourt = (jersey: string): boolean => lineup.includes(jersey);
  const resolveSlot = (bound: string | undefined, fallbacks: readonly string[], taken: readonly string[]): string => {
    if (bound && onCourt(bound) && !taken.includes(bound)) return bound;
    for (const candidate of fallbacks) {
      if (onCourt(candidate) && !taken.includes(candidate) && candidate !== handler) return candidate;
    }
    return remainingLineup(lineup, [handler, ...taken])[0] ?? handler;
  };
  const usage = lineupPackage.usageProfile;
  const screener = resolveSlot(binding.screener, [...usage.screener, ...usage.creator], [handler]);
  const candidates = remainingLineup(lineup, [handler, screener]);
  const strongCorner = resolveSlot(binding.spacer_strong, usage.spacer, [handler, screener]);
  const weakCorner = resolveSlot(binding.spacer_weak, usage.spacer, [handler, screener, strongCorner]);
  const slotPlayer = remainingLineup(lineup, [handler, screener, strongCorner, weakCorner])[0] ?? handler;
  // Matchups are needed by the tactic fit (matchup advantages) and by the
  // coverage layer, so resolve them before the kind selection.
  const matchups = stableMatchups(sense, previous, binding, defenseLineupPackage);

  // ── tactic kind selection ─────────────────────────────────────────────
  // The halfcourt system is chosen by ADAPTIVE FIT, not a weighted draw:
  //   fit = systemFitness(role/capability baseline) + matchupFit(real-time
  //   matchups — who guards whom, is the rim open, are shooters open).
  // A lineup draws ISO when its creator owns the matchup, PNR when the
  // screener's defender is weak, DRIVE_KICK when the paint is open — the
  // tactic MIX is an emergent property of the matchups on the floor, not
  // a fixed table of weights.
  let kind: TeamPlanKind;
  let systemId: TacticalSystemId;
  let selected = false;
  // The playbook's family is the tactic WHEN a halfcourt set is running —
  // but ADVANCE stays TRANSITION_PUSH (the break is the break). The
  // old dual selection made OFF_BALL_SCREEN plans flip stage every 0.1s
  // against a pnr play; binding the kind to the active play keeps the
  // plan and the playbook in agreement for the halfcourt phase.
  const PLAY_ID_KIND: Record<string, TeamPlanKind> = {
    pnr_high: 'PNR_ROLL',
    early_drag: 'PNR_POP',
    handoff_wing: 'HANDOFF',
    iso_clear: 'ISO',
    transition_push: 'TRANSITION_PUSH',
    pin_down_flare: 'OFF_BALL_SCREEN',
    post_isolation: 'POST_UP',
  };
  const hasHalfCourtTactic = previous?.selected === true;
  if (possessionPhase === 'ADVANCE' || !frontcourt) {
    kind = 'TRANSITION_PUSH';
    systemId = 'PUSH_PACE';
  } else if (playId && playId !== 'transition_push' && PLAY_ID_KIND[playId]) {
    kind = PLAY_ID_KIND[playId]!;
    systemId = systemForKind(kind);
    selected = true;
  } else if (hasHalfCourtTactic) {
    // Sticky: keep the possession's chosen tactic across passes and ticks.
    kind = previous.kind;
    systemId = previous.systemId;
    selected = true;
  } else if (possessionPhase === 'EXECUTE' && rng) {
    // Soft selection: fit → weight → one draw. The weights come from the
    // LIVE matchups every possession (a weak screener defender raises PNR,
    // a dominated creator matchup raises ISO), so the tactic mix is an
    // emergent property of the floor — but low-fit tactics keep a floor
    // weight so the offense is not perfectly predictable.
    const identity = resolveLineupIdentity(lineupPackage, sense.playerData, sense.abilities);
    const roles = identity.assignments.flatMap((assignment) => assignment.roles);
    const caps = identity.assignments.map((assignment) => assignment.capabilities);
    const candidates: Array<{ id: TacticalSystemId; fit: number }> = [];
    for (const spec of TACTICAL_SYSTEMS) {
      if (spec.id === 'PUSH_PACE') continue; // transition only
      const fit = systemFitness(spec, roles, caps)
        + matchupFit(spec.families[0] ?? 'ISO', sense, matchups, screener);
      candidates.push({ id: spec.id, fit });
    }
    // P3.x: fit FILTERS, uniform draw selects. Systems below the fit
    // floor (a genuinely bad matchup, e.g. POST_UP with no post threat)
    // are excluded; every viable system then draws equally. This keeps
    // the mix diverse (real teams run 5+ families per game) while fit
    // still stops the truly wrong calls. The floor is low on purpose —
    // most families are playable, and the EV layer + coverage reads make
    // the actual quality of the look.
    const fitFloor = 0.9; // systemFitness baseline is 1.0; below 0.9 = clearly wrong
    const pool = candidates.filter((c) => c.fit >= fitFloor);
    const viable = pool.length > 0 ? pool : candidates.slice(0, 3);
    const chosen = viable[Math.floor(rng.next() * viable.length)]!;
    systemId = chosen.id;
    kind = familyForSystem(systemId);
    selected = true;
  } else {
    // SETUP before tactic selection: placeholder formation derived from
    // the lineup's role identity (screener-present → PNR, else ISO).
    systemId = systemForKind(lineupPackage.usageProfile.screener.length > 0 ? 'PNR_ROLL' : 'ISO');
    kind = lineupPackage.usageProfile.screener.length > 0 ? 'PNR_ROLL' : 'ISO';
  }
  // ── stage resolution ──────────────────────────────────────────────────
  // The STAGE MACHINE (possessionPhase) is the authoritative time model:
  // ADVANCE → SETUP (organize ≥10s) → EXECUTE (action + 3s cooldown).
  // Stage derives ONLY from possessionPhase + screenReady, never from
  // the play step — deriving stage elsewhere risks the early-shot gates
  // (shoot/drive EV=0 until EXECUTE) desynchronizing from the hold
  // lockout (ADVANTAGE/SCREEN_USE → hold=-1), which deadlocks the
  // handler into pass-only.
  // Designed feed target: OFF_BALL_SCREEN frees its best catch-and-shoot
  // spacer (the curler); POST_UP feeds its best post hub. Sticky per
  // possession via previous plan.
  const feedTargetJersey = (() => {
    if (kind !== 'OFF_BALL_SCREEN' && kind !== 'POST_UP') return null;
    if (previous?.feedTargetJersey && lineup.includes(previous.feedTargetJersey)
      && previous.feedTargetJersey !== handler && previous.kind === kind) {
      return previous.feedTargetJersey;
    }
    let best: string | null = null;
    let bestScore = -Infinity;
    for (const mate of [strongCorner, weakCorner, slotPlayer, screener]) {
      if (!mate || mate === handler) continue;
      const data = sense.playerData?.[mate];
      const score = kind === 'POST_UP'
        ? (data?.ability.POST ?? 50) + (sense.abilities[mate]?.postPlay ?? 0.5) * 30
        : (data?.ability.CS3 ?? 50) + (sense.abilities[mate]?.catchShoot ?? 0.5) * 30;
      if (score > bestScore) { bestScore = score; best = mate; }
    }
    return best;
  })();
  // POST seal readiness: a post seal has no SCREEN_SET fact (the seal forms
  // 15ft from the handler, not on the ball). Read it geometrically — the
  // feed target sealed inside 10ft with a defender attached means the post
  // position is established and the entry window is open.
  const postSealed = (() => {
    if (kind !== 'POST_UP' || !feedTargetJersey) return screenReady;
    const target = sense.offensePlayers.find((p) => p.jersey === feedTargetJersey);
    if (!target) return screenReady;
    const rimDist = Math.hypot(
      (target.pose.x - sense.rim.x) * 94,
      (target.pose.y - sense.rim.y) * 50,
    );
    if (rimDist > 10) return screenReady;
    return sense.defensePlayers.some((d) =>
      Math.hypot((d.pose.x - target.pose.x) * 94, (d.pose.y - target.pose.y) * 50) <= 6,
    ) || screenReady;
  })();
  let stage: TeamPlanStage;
  // Frontcourt gate: the designed-feed families' stage machine must not
  // advance while the ball is still coming up the floor (measured: OFF_BALL
  // possessions reached SCREEN_USE with the handler 65ft from the rim — the
  // pin window burned before the set existed).
  const feedReady = frontcourt || (kind !== 'OFF_BALL_SCREEN' && kind !== 'POST_UP');
  if (kind === 'TRANSITION_PUSH') {
    stage = 'ADVANCE';
  } else if (possessionPhase === 'SETUP') {
    // Screen tactics organize toward the screen (approach/use). Non-screen
    // tactics (ISO/HANDOFF/DRIVE_KICK) organize in a neutral SET — there
    // is no screen to approach, so SCREEN_APPROACH is semantically wrong.
    const ready = kind === 'POST_UP' ? postSealed : screenReady;
    stage = isScreenTactic(kind) ? (ready && feedReady ? 'SCREEN_USE' : 'SCREEN_APPROACH') : 'SET';
  } else if (possessionPhase === 'EXECUTE') {
    if (isScreenTactic(kind)) {
      const ready = kind === 'POST_UP' ? postSealed : screenReady;
      stage = ready && feedReady ? 'ADVANTAGE' : 'SCREEN_APPROACH';
    } else {
      // Non-screen tactics (ISO/HANDOFF/DRIVE_KICK) execute as soon as
      // EXECUTE begins. The old `previous.stage` flip alternated
      // ADVANTAGE/SET every tick (hold deadlock on one, open on the
      // next), flashing the stage label on the board.
      stage = 'ADVANTAGE';
    }
  } else {
    stage = 'SET';
  }

  // ── actions per role ──────────────────────────────────────────────────
  // The handler's action is an expected-value decision (shot/drive/pass/
  // hold scored as expected points from the resolve rate model) — no
  // clock thresholds, no openness gates, no stage switches. The pass
  // target is the best-EV receiver from the same core.
  const screenerAction = actionForScreener(stage, kind, screenExecution);
  // P3.3: the coverage is resolved BEFORE the handler decides so the
  // decision can READ it — under a BLITZ the handler attacks the roller,
  // under ICE the drive is baseline-constrained, under DROP the pull-up
  // is the answer. The screen intent is inferred from the plan kind
  // (PNR family runs a screen) + the previous plan's lifecycle — the
  // current assignments are not needed (and cannot be, the decision
  // depends on the coverage).
  const hasScreenAssignment = kind === 'PNR_ROLL' || kind === 'PNR_POP' || kind === 'OFF_BALL_SCREEN' || screenerAction === 'screen';
  const screenActive = screenExecution !== null || hasScreenAssignment || previous?.screenActive === true;
  // P3.6: the zone scheme is a DEFENSE-WIDE decision, independent of
  // whether a screen is active — a 2-3 zone is a possession-level scheme.
  // chooseScreenDefense returns the zone flag for both cases; the
  const screenDefense = chooseScreenDefense(sense, matchups, screener, kind);
  const committedScreenCoverage = previous?.committedScreenCoverage
    ?? (screenActive && isScreenTactic(kind) ? screenDefense.mode : null);
  // The committed coverage's mode is the displayed truth: switchesAtUse
  // must follow it, not the latest re-read (which may have drifted toward
  // DROP while the committed plan still advertises SWITCH — a SWITCH
  const committedDefense = committedScreenCoverage
    ? {
        ...screenDefense,
        mode: committedScreenCoverage,
        switchesAtUse: committedScreenCoverage === 'SWITCH' || committedScreenCoverage === 'HEDGE',
      }
    : screenDefense;
  const effectiveScreenDefense = screenActive
    ? committedDefense
    : screenDefense;
  const decision = decideHandlerAction(sense, handler, possessionPhase, stage, mode, catchWindowTicks, kind, screenActive ? (committedScreenCoverage ?? screenDefense.mode) : null, executeCooldownTicks, screenExecution?.phase ?? null, feedTargetJersey ?? screener, handoffGuard, driveCommitment?.jersey === handler, lastHandlingAction, handlingActionCooldownTicks, passChainSinceAttack);
  const exchangeFrom = committedExchange?.fromJersey === handler ? committedExchange : null;
  const handlerAction: ActionKind = decision.kind === 'pass'
    ? 'pass'
    : decision.kind === 'handoff'
      ? 'handoff'
      : decision.kind;
  const passTarget = handlerAction === 'pass' || handlerAction === 'handoff'
    ? (exchangeFrom
      ? exchangeFrom.toJersey
      : decision.kind === 'pass' || decision.kind === 'handoff' ? decision.targetJersey : null)
    : null;

  const assignments: TeamAssignment[] = [
    {
      jersey: handler,
      role: 'handler',
      action: handlerAction,
      targetJersey: handlerAction === 'pass' || handlerAction === 'handoff' ? passTarget : null,
      lane: stage === 'ADVANCE' ? 'middle' : 'strong',
    },
    {
      jersey: screener,
      role: 'screener',
      action: screenerAction,
      targetJersey: kind === 'PNR_ROLL' && stage === 'SCREEN_APPROACH' ? handler
        : kind === 'POST_UP' ? handler : null,
      lane: screenerAction === 'cut' ? 'rim' : 'strong',
    },
    {
      jersey: strongCorner,
      role: 'strong_corner',
      action: actionForSpacer('strong_corner', stage, kind, sense, strongCorner),
      targetJersey: null,
      lane: 'strong',
    },
    {
      jersey: weakCorner,
      role: 'weak_corner',
      action: actionForSpacer('weak_corner', stage, kind, sense, weakCorner),
      targetJersey: null,
      lane: 'weak',
    },
    {
      jersey: slotPlayer,
      role: 'slot',
      action: actionForSpacer('slot', stage, kind, sense, slotPlayer),
      targetJersey: null,
      lane: 'weak',
    },
  ];


  return { kind, systemId, stage, offense: sense.offense, handler, assignments, matchups, screenDefense: effectiveScreenDefense, screenActive, committedScreenCoverage, selected, feedTargetJersey };
}
