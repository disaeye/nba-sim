/**
 * Expected-value decision core (foundation 0.9.0).
 *
 * The handler decision is a score over options — shoot / drive / pass to
 * each teammate / hold — where every option is priced as expected points
 * from the SAME rate model the resolve layer uses. Behaviours that used to
 * be threshold chains (`shotClock <= 3 → shoot`, `open >= 3 → shoot`,
 * `paintDefenders >= 2 → pass`) now emerge from the numbers:
 *
 *  - a wide-open catch-and-shoot three has higher EV than holding → early
 *    offence appears without a SETUP gate;
 *  - with 3s left, HOLD's value collapses to ~0 (no time to create) →
 *    the late-clock heave appears without a clock threshold;
 *  - a big catching the ball at the arc has a terrible shot EV and a great
 *    give-back EV → the ball returns to the guard without a role rule.
 *
 * Purely functional: no RNG, no events, no state mutation.
 */
import type { LiveCourtSense } from '../perception/live-court.js';
import { zoneFromPoint, shotValueAt } from '../court/geometry.js';
import { loadResolveConfig, makeResolveContext } from '../resolve/index.js';
import { shotAbilityModifier } from '../resolve/checks.js';

const RESOLVE = makeResolveContext(loadResolveConfig());

function defenderDistance(sense: LiveCourtSense, x: number, y: number): number {
  let min = Infinity;
  for (const d of sense.defensePlayers) {
    const dd = Math.hypot((d.pose.x - x) * 94, (d.pose.y - y) * 50);
    if (dd < min) min = dd;
  }
  return Number.isFinite(min) ? min : 10;
}

/**
 * Expected points for a shot taken now from `zone` with `shotValue`,
 * contested by a defender at `defenderDistFt`, with an explicit shot
 * method. Includes the offensive-rebound tail. Mirrors resolveShot's
 * rate model (minus blocks).
 */
export function expectedShotPoints(
  sense: LiveCourtSense,
  shooterId: string,
  zone: string,
  shotValue: 2 | 3,
  defenderDistFt: number,
): number {
  return expectedShotPointsWithType(sense, shooterId, zone, shotValue, defenderDistFt, 'other');
}

/** Shot-type-aware pricing — the one-model twin of resolveShot. */
function expectedShotPointsWithType(
  sense: LiveCourtSense,
  shooterId: string,
  zone: string,
  shotValue: 2 | 3,
  defenderDistFt: number,
  shotType: 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other',
): number {
  const abilities = sense.abilities[shooterId] ?? null;
  const typeRates = RESOLVE.shotTypeRates ?? null;
  const base = typeRates !== null
    ? (shotValue === 3
        ? (shotType === 'catch_shoot' ? typeRates.catch_shoot_3pt : typeRates.pull_up_3pt)
        : shotType === 'catch_shoot'
          ? typeRates.catch_shoot_2pt
          : shotType === 'pull_up'
            ? typeRates.pull_up_2pt
            : shotType === 'post'
              ? typeRates.post_2pt
              : shotType === 'drive_finish'
                ? typeRates.drive_finish_2pt
                : typeRates.other_2pt)
    : (shotValue === 2 ? RESOLVE.baseRates.shot_make_2pt : RESOLVE.baseRates.shot_make_3pt);
  const zm = shotValue === 3
    ? (RESOLVE.zoneModifiers3pt[zone] ?? 1.0)
    : (RESOLVE.zoneModifiers[zone] ?? 0.85);
  // P1.1 continuous contest — mirrors resolveShot's jump-shot curve.
  // The EV layer uses the jump-shot curve for all shots; the resolve
  // layer applies a lighter curve for contact finishes. This asymmetry
  // is intentional: the EV layer should be conservative to avoid
  // overvaluing rim attempts and stalling the handler.
  const contest = defenderDistFt >= 6 ? 1 : 0.55 + 0.45 / (1 + Math.exp(-(defenderDistFt - 3.0) * 0.9));
  const mod = shotAbilityModifier(abilities, shotValue, zone);
  // §8.4 fatigue pricing — mirrors resolve's multiplier.
  const fatigue = sense.staminaFactors?.[shooterId] ?? 1;
  // ── space-pressure discount (three-point pricing) ────────────────────
  // A three is not a free 1.14 ppp: every defender near the arc squeezes
  // the look, and the offense cannot always FIND a clean wing/corner
  // shot. Without this the EV core treats every possession as a wide-open
  // three contest and the shot profile collapses to 90% threes. The
  // discount scales with how many defenders sit within 14ft of the
  // shooter AND how close the nearest one is — a guarded perimeter look
  // prices below a drive, an unguarded one still prices as the best
  // option on the floor.
  let spacePressure = 1;
  if (shotValue === 3) {
    let nearDefenders = 0;
    let nearest = Infinity;
    const shooterPose = sense.offensePlayers.find((p) => p.jersey === shooterId)?.pose;
    if (shooterPose) {
      for (const d of sense.defensePlayers) {
        const dd = Math.hypot((d.pose.x - shooterPose.x) * 94, (d.pose.y - shooterPose.y) * 50);
        if (dd < nearest) nearest = dd;
        if (dd <= 14) nearDefenders += 1;
      }
      // Reduced from 0.14 to 0.10 per defender — the old penalty killed
      // too many threes (21-28% 3pt rate vs NBA 35-40%). NBA teams shoot
      // threes through moderate closeout pressure; only a defender within
      // 4ft (contest range) materially drops the make rate.
      const crowd = Math.min(3, nearDefenders) * 0.10;
      const nearestPenalty = nearest < 4 ? 0.18 : 0;
      spacePressure = Math.max(0.70, 1 - crowd - nearestPenalty);
    }
  }
  // ── deep-range penalty ───────────────────────────────────────────────
  // A three from 30ft is NOT the same shot as one from 23.75ft: NBA
  // 28ft+ attempts convert at ~28% vs ~36% at the arc. Without this the
  // EV layer priced every three identically and the offense happily
  // jacked from 35-40ft (measured 47% of threes beyond the arc, avg 7ft
  // deep, max 16.5ft). Depth is measured against the arc (23.75ft top,
  // 22ft corner) — corner shots at y≈0.05 are not penalized.
  const shooterPose = sense.offensePlayers.find((p) => p.jersey === shooterId)?.pose;
  let depthFactor = 1;
  if (shotValue === 3 && shooterPose) {
    const rimDist = Math.hypot(
      (shooterPose.x - sense.rim.x) * 94,
      (shooterPose.y - sense.rim.y) * 50,
    );
    const corner = Math.abs(shooterPose.y - 0.5) * 50 > 19;
    const lineDist = corner ? 22 : 23.75;
    const beyond = rimDist - lineDist;
    if (beyond > 0.5) depthFactor = Math.max(0.55, 1 - beyond * 0.035);
  }
  // Dead zone penalty: 2-point shots from 20-23ft (just inside the arc,
  // top of the key) are the WORST shot in basketball — too far for efficiency,
  // not far enough for the extra point. NBA players either step back for 3
  // or drive; they don't shoot from 22ft for 2. Without this penalty the
  // engine's halfcourt handler parked at the strike pocket (22ft) shot
  // long-twos constantly (measured: 46% of all attempts in the 5-14ft+
  // long-mid band, NBA 8-15%).
  if (shotValue === 2 && shooterPose) {
    const deadRimDist = Math.hypot(
      (shooterPose.x - sense.rim.x) * 94,
      (shooterPose.y - sense.rim.y) * 50,
    );
    if (deadRimDist >= 20 && deadRimDist < 23.7) {
      depthFactor = Math.min(depthFactor, 0.55);
    }
  }
  const makeRate = Math.min(0.92, base * zm * contest * mod * fatigue * spacePressure * depthFactor);
  const tail = (1 - makeRate) * RESOLVE.baseRates.offensive_rebound_rate * 1.1;
  return makeRate * shotValue + tail;
}

/** Expected points for driving from the handler's current spot. */
export function expectedDrivePoints(sense: LiveCourtSense, handlerId: string): number {
  const handler = sense.offensePlayers.find((p) => p.jersey === handlerId);
  if (!handler) return 0;
  const abilities = sense.abilities[handlerId] ?? null;
  const pressure = Math.max(0, Math.min(0.45, (6 - sense.onBallDistanceFt) / 6 * 0.6));
  const handlerMod = 1 + ((abilities?.handleSecurity ?? 0.5) - 0.5) * 0.5;
  const defId = sense.onBallDefender;
  const defAb = defId ? (sense.abilities[defId] ?? null) : null;
  const defMod = 1 - ((defAb?.onBallDefense ?? 0.5) - 0.5) * 0.4;
  // §8.4 fatigue pricing for the handler.
  const fatigue = sense.staminaFactors?.[handlerId] ?? 1;
  // Distance decay: a drive from 22ft is not the same play as a drive
  // from 12ft. The success of BEATING the defender decays with the
  // distance to the rim — the handler must cover more ground while
  // protecting the ball. 0.45 base at ≤12ft, −0.35 per 10ft beyond.
  const dRimFt = Math.hypot((handler.pose.x - sense.rim.x) * 94, (handler.pose.y - sense.rim.y) * 50);
  const distanceMod = dRimFt <= 12 ? 1 : Math.max(0.6, 1 - (dRimFt - 12) / 10 * 0.15);
  const success = Math.max(0, Math.min(1, 0.45 * distanceMod * (1 - pressure) * handlerMod * defMod * fatigue));
  const fouled = Math.min(0.25, RESOLVE.baseRates.foul_on_drive_rate * (1 + pressure));
  const failure = Math.max(0, 1 - success - fouled);
  // Successful finish: a TIGHT rim attempt — the beaten defender recovers
  // into the play, so the contest is close (1.5ft → 0.7), not the 2.5ft
  // used for regular drives. The old 2.5ft contest inflated drive EV to
  // ~1.27ppp and made "just drive" the answer to everything. P1.2: the
  // finish is a drive_finish, priced with its own base rate (0.62).
  const finish = expectedShotPointsWithType(sense, handlerId, 'rim', 2, 1.5, 'drive_finish');
  // Fouled: two free throws (and-one tail folded into the finish share).
  const evFoul = fouled * 2 * RESOLVE.baseRates.ft_make;
  // Failed drive: 30% lost ball, 70% contested finish at reduced quality.
  const evBadFinish = failure * 0.7 * expectedShotPointsWithType(sense, handlerId, 'rim', 2, 1, 'drive_finish') * 0.8;
  // A drive takes real time to reach the rim (~1.5s of ball movement).
  // With little shot clock left the drive cannot finish before the horn,
  // so its value collapses toward the rushed-finish share — this is what
  // makes a mid-range pull-up the correct late-clock choice instead of a
  // doomed sprint into a packed paint.
  const driveWindow = Math.max(0, sense.shotClock - 1.5);
  const timeFactor = driveWindow >= 2 ? 1 : Math.max(0, driveWindow / 2);
  return (success * finish + evFoul + evBadFinish) * (0.4 + 0.6 * timeFactor);
}

/**
 * Expected points for passing to `receiverId` (they act on the catch).
 *
 * P2.5 two-step lookahead: the old pricing assumed the receiver shoots
 * (90%) or attacks the closeout (10%) and stopped there — that made a
 * swing pass to a NON-shooter worthless, so possessions never moved the
 * ball through a big or a connector. The two-step model recurses ONE
 * level: the receiver's own best EV (shoot / drive / pass-to-open-mate)
 * is evaluated at their catch position, discounted by the defense's
 * adjustment time. One recursion only — deeper search is not worth the
 * cost in this architecture.
 */
export function expectedPassPoints(sense: LiveCourtSense, passerId: string, receiverId: string, depth: number = 0): number {
  if (depth >= 1) {
    // Recursion floor: at depth 1 the receiver is a pure shooter/driver —
    // no further chain exploration. This bounds the recursion at two
    // levels (the P2.5 contract) and prevents A→B→A cycles.
    return expectedPassPointsLeaf(sense, passerId, receiverId);
  }
  const passer = sense.offensePlayers.find((p) => p.jersey === passerId);
  const receiver = sense.offensePlayers.find((p) => p.jersey === receiverId);
  if (!passer || !receiver) return 0;
  const dist = Math.hypot(
    (passer.pose.x - receiver.pose.x) * 94,
    (passer.pose.y - receiver.pose.y) * 50,
  );
  // A pass is not free: real pass completion is ~0.92-0.94 in the NBA,
  // but the decision layer must also price DEFENSIVE ROTATION — every
  // defender near the passing lane or the receiver closes during the
  // flight. The old 0.88 base with only a distance discount made passes
  // strictly better than shooting (1.04ppp vs 0.9ppp), producing 9.8
  // passes/possession. The 0.82 base + receiver pressure factor prices
  // the real risk that a nearby defender deflects or picks off the pass.
  const passSuccess = Math.max(0.55, 0.82 - (dist / 94) * 0.12);
  // Passer pressure: a handler being hounded (defender ≤4ft) cannot
  // casually fire skip passes — real guards pass out of pressure with
  // lower success, and the decision layer must price that (measured
  // 79 bad-pass turnovers/game after defense started contesting).
  const passerPressure = sense.onBallDistanceFt < 4 ? 0.6 : 1;
  // Hot-potato return guard: passing straight back to the player who
  // just passed to you is a dead-end swing in real basketball — the
  // defense has not been moved, and the return pass gives the original
  // passer no new advantage (measured 140+ <0.7s catch→re-pass chains
  // and 4-in-a-row 2↔1 ping-pong per game). The return is only priced
  // at a steep discount unless the passer is genuinely open (≥6ft).
  // Hot-potato return guard: passing straight back to the player who
  // just passed to you is a dead-end swing in real basketball — the
  // defense has not been moved, and the return pass gives the original
  // passer no new advantage (measured 140+ <0.7s catch→re-pass chains
  // and 4-in-a-row 2↔1 ping-pong per game). BUT the guard must not kill
  // legitimate swing passes: a wing→corner→wing sequence IS a real NBA
  // possession-mover, and a blanket discount starved corner shooters
  // (3PT rate fell 28→20% while the return guard was fully on). The
  // penalty is mild: only the immediate whip-back to a NON-open passer
  // is discounted.
  // Side-overload read: count defenders within 14ft of the passer (the
  // strong-side load). A receiver on the OPPOSITE side of the floor from
  // that crowd inherits the rotation lag — the defense must shift 12-20ft
  // to contest, and the pass is worth more. The measured defect: 64% of
  // overloaded-situation passes went INTO the crowd (chance = 50%) — no
  // live side-overload game existed. This read makes the swing pass to
  // the naked side the correct decision when the load commits.
  const overloadSide = (() => {
    let count = 0;
    let sumY = 0;
    for (const d of sense.defensePlayers) {
      if (Math.hypot((d.pose.x - passer.pose.x) * 94, (d.pose.y - passer.pose.y) * 50) <= 14) {
        count += 1;
        sumY += d.pose.y;
        }
      }
    if (count < 3) return null;
    return sumY / count > passer.pose.y ? 'high' : 'low';
  })();
  const weakSideBonus = overloadSide !== null
    && ((overloadSide === 'high' && receiver.pose.y < passer.pose.y - 0.08)
      || (overloadSide === 'low' && receiver.pose.y > passer.pose.y + 0.08))
    ? 1.35
    : 1;
  // Help-counter read: if a defender is COMMITTED to help (help/tag action
  // near the ball) and that same defender is the receiver's nearest man, the
  // receiver is the rotation's abandoned man — the designed counter. NBA
  // offenses hit the helper's man 30-40% when the rotation commits; measured
  // here before this term: 5%.
  const helperAbandonBonus = (() => {
    for (const d of sense.defensePlayers) {
      if (d.pose.action !== 'help' && d.pose.action !== 'tag') continue;
      const dToBall = Math.hypot((d.pose.x - passer.pose.x) * 94, (d.pose.y - passer.pose.y) * 50);
      if (dToBall > 12) continue;
      const dToRecv = Math.hypot((d.pose.x - receiver.pose.x) * 94, (d.pose.y - receiver.pose.y) * 50);
      if (dToRecv < 10 && dToRecv < dToBall) return 1.3;
    }
    return 1;
  })();
  const returnPassPenalty = sense.lastPasserId === receiverId
    ? (defenderDistance(sense, receiver.pose.x, receiver.pose.y) >= 6 ? 0.9 : 0.6)
    : 1;
  // §8.4 fatigue pricing for the passer.
  const passerFatigue = sense.staminaFactors?.[passerId] ?? 1;
  // Receiver pressure: a defender near the receiver closes DURING the
  // pass flight — the old closeout (dist - 3) gave the receiver too
  // much credit (a defender at 6ft was priced as 3ft away, but by the
  // catch they are at 2-3ft). The tighter -2 offset reflects that
  // real closeout speed covers 3-4ft during a 0.4s pass flight.
  const receiverDefDist = defenderDistance(sense, receiver.pose.x, receiver.pose.y);
  // Corner shooters are the highest-value catch target in basketball —
  // the 0.85 pressure penalty for a <5ft defender double-taxed the
  // corner pass (pressure penalty × tight closeout), starving corner
  // shooters (measured: corner 3PA = 0 per game across 5 seeds even
  // with 27% open-corner frames). The 0.85 is for non-corner receivers;
  // a corner look at 3-4ft is still the best shot on the floor.
  const cornerReceiver = Math.abs(receiver.pose.y - 0.5) * 50 > 19
    && zoneFromPoint(receiver.pose.x, receiver.pose.y, sense.offense, sense.baskets).startsWith('corner');
  const receiverPressure = receiverDefDist < 5 && !cornerReceiver ? 0.85 : 1;
  const zone = zoneFromPoint(receiver.pose.x, receiver.pose.y, sense.offense, sense.baskets);
  const shotValue = shotValueAt(receiver.pose.x, receiver.pose.y, sense.offense, sense.baskets);
  // The receiver's contest on the catch: the defender closes ~2ft during
  // the flight, BUT the compression must respect the actual gap — a
  // shooter with 6ft of air catches with ~4ft of air (a real look), a
  // shooter at 4ft catches at ~3ft (contested but not dead). The old
  // unbounded `max(2, dist-2)` priced every receiver at 2.2ft, which
  // killed passes to open corner shooters (measured: 809 open-corner
  // frames/game with the ball 35ft away, but only 6 catches — the pass
  // EV never picked them).
  const closeout = Math.max(3, receiverDefDist - 2);
  // P2.5: the receiver's continuation EV — their own best option on the
  // catch. For a non-shooter catching away from the rim this is a cheap
  // give-back pass to an open mate, which prices the swing through the
  // big as a REAL chain step instead of a dead end.
  const catchShootEv = expectedShotPoints(sense, receiverId, zone, shotValue, closeout);
  const driveEv = expectedDrivePoints(sense, receiverId);
  // One-level recursion: the receiver's best immediate option plus a
  // discounted "keep the chain alive" value (best open mate pass).
  let bestMatePassEv = 0;
  for (const mate of sense.offensePlayers) {
    if (mate.jersey === receiverId || mate.jersey === passerId) continue;
    const mateEv = expectedPassPoints(sense, receiverId, mate.jersey, depth + 1);
    if (mateEv > bestMatePassEv) bestMatePassEv = mateEv;
  }
  // The continuation captures the receiver's best option on the catch:
  // shoot, drive, or ONE give-back pass (discounted 0.6 for defense
  // re-positioning). The old code added bestMatePassEv again (0.25
  // weight), double-counting chain value and inflating every pass EV
  // by ~25%, which produced 9.8 passes/possession vs NBA ~3.5. The
  // single continuation term is the correct price.
  const continuation = Math.max(catchShootEv, driveEv, bestMatePassEv * 0.6);
  return passSuccess * passerFatigue * passerPressure * receiverPressure * returnPassPenalty * weakSideBonus * helperAbandonBonus * continuation;
}

/** Depth-1 leaf: receiver shoots or drives, no chain continuation. */
function expectedPassPointsLeaf(sense: LiveCourtSense, passerId: string, receiverId: string): number {
  const passer = sense.offensePlayers.find((p) => p.jersey === passerId);
  const receiver = sense.offensePlayers.find((p) => p.jersey === receiverId);
  if (!passer || !receiver) return 0;
  const dist = Math.hypot(
    (passer.pose.x - receiver.pose.x) * 94,
    (passer.pose.y - receiver.pose.y) * 50,
  );
  const passSuccess = Math.max(0, 0.88 - (dist / 94) * 0.12);
  const passerPressure = sense.onBallDistanceFt < 4 ? 0.6 : 1;
  const passerFatigue = sense.staminaFactors?.[passerId] ?? 1;
  const zone = zoneFromPoint(receiver.pose.x, receiver.pose.y, sense.offense, sense.baskets);
  const shotValue = shotValueAt(receiver.pose.x, receiver.pose.y, sense.offense, sense.baskets);
  const closeout = Math.max(2, defenderDistance(sense, receiver.pose.x, receiver.pose.y) - 3);
  const catchShootEv = expectedShotPoints(sense, receiverId, zone, shotValue, closeout);
  const driveEv = expectedDrivePoints(sense, receiverId);
  return passSuccess * passerFatigue * passerPressure * (catchShootEv * 0.85 + driveEv * 0.15);
}

export type HandlerDecision =
  | { readonly kind: 'shoot' }
  | { readonly kind: 'drive' }
  | { readonly kind: 'pass'; readonly targetJersey: string }
  | { readonly kind: 'handoff'; readonly targetJersey: string }
  | { readonly kind: 'triple_threat' }
  | { readonly kind: 'back_to_basket' }
  | { readonly kind: 'pivot' }
  | { readonly kind: 'crossover' }
  | { readonly kind: 'relocate' }
  | { readonly kind: 'advance' }
  | { readonly kind: 'pump_fake' };

export function decideHandlerAction(
  sense: LiveCourtSense,
  handlerId: string,
  possessionPhase: 'ADVANCE' | 'SETUP' | 'EXECUTE',
  stage: string = 'SET',
  mode: 'TRANSITION' | 'HALFCOURT' = 'HALFCOURT',
  catchWindowTicks: number = 0,
  tacticKind: string | null = null,
  coverage: 'DROP' | 'SWITCH' | 'BLITZ' | 'HEDGE' | 'ICE' | null = null,
  executeCooldownTicks: number = 0,
  screenExecutionPhase: 'APPROACH' | 'SET' | 'USE' | 'EXIT' | null = null,
  screenTargetId: string | null = null,
  recentHandoff: { readonly giverId: string; readonly receiverId: string } | null = null,
  activeDrive: boolean = false,
  lastHandlingAction: import('./types.js').HandlingActionKind | null = null,
  handlingActionCooldownTicks: number = 0,
  passChainSinceAttack: number = 0,
): HandlerDecision {
  const handler = sense.offensePlayers.find((p) => p.jersey === handlerId);
  if (!handler) return { kind: 'relocate' };

  const attack = sense.attack;
  const inFrontcourt = attack === 'right' ? handler.pose.x >= 0.5 : handler.pose.x <= 0.5;
  // Backcourt advance with an outlet: the hard 'advance' return made the
  // handler dribble 50ft alone every possession (measured: 2 early passes
  // per GAME; zero fast-break shots). Real transition basketball moves the
  // ball ahead — a lane-runner 10+ft ahead of the ball with daylight gets
  // the outlet. The pass keeps the advance otherwise.
  if (possessionPhase === 'ADVANCE' && !inFrontcourt) {
    let outlet: string | null = null;
    let bestAhead = 10;
    for (const mate of sense.offensePlayers) {
      if (mate.jersey === handlerId) continue;
      const aheadFt = (sense.rim.x - handler.pose.x) * (sense.rim.x > 0.5 ? 94 : -94)
        - (sense.rim.x - mate.pose.x) * (sense.rim.x > 0.5 ? 94 : -94);
      if (aheadFt < bestAhead) continue;
      const closestDef = sense.defensePlayers.reduce(
        (best, d) => Math.min(best, Math.hypot((d.pose.x - mate.pose.x) * 94, (d.pose.y - mate.pose.y) * 50)),
        Infinity,
      );
      if (closestDef >= 8) {
        bestAhead = aheadFt;
        outlet = mate.jersey;
      }
    }
    if (outlet !== null) return { kind: 'pass', targetJersey: outlet };
    return { kind: 'advance' };
  }
  // Drive continuation: a live drive inside 14ft is past the point of
  // no return — the argmax re-roll (pull-up EV, kick-out EV) killed every
  // such drive at 12-14ft (measured: 0 of 200+ drives ever finished; the
  // pull-up abort at <=18ft always won). Inside 14 the driver FINISHES
  // (the paint-arrival fact resolves the attempt) or hits the real wall
  // (2+ paint defenders → kick-out from the adjudicator's finish logic).
  if (activeDrive) {
    const rimDist0 = Math.hypot(
      (handler.pose.x - sense.rim.x) * 94,
      (handler.pose.y - sense.rim.y) * 50,
    );
    if (rimDist0 < 17 && sense.shotClock > 2) {
      // Pocket exception: a committed drive with the roller sprinting to the
      // rim and his defender BEHIND him is the lob/pocket window — the single
      // highest-value read in basketball. The drive continuation used to
      // return blindly here, which is why rolls were fed on 1-4% of screens
      // (NBA 40-55%): the handler drove past a wide-open roller every time.
      // Gate: roller within 8ft of the rim, his nearest defender on the
      // rim-far side (behind the play), handler still outside 4ft (has room
      // to deliver a lead pass).
      const roller = screenTargetId !== null
        ? sense.offensePlayers.find((p) => p.jersey === screenTargetId)
        : undefined;
      if (roller && rimDist0 > 4) {
        const rollerRim = Math.hypot(
          (roller.pose.x - sense.rim.x) * 94,
          (roller.pose.y - sense.rim.y) * 50,
        );
        if (rollerRim <= 8) {
          const rimX = sense.rim.x * 94;
          const rimY = sense.rim.y * 50;
          const rx = roller.pose.x * 94;
          const ry = roller.pose.y * 50;
          const toRimX = rimX - rx;
          const toRimY = rimY - ry;
          const rimLen = Math.hypot(toRimX, toRimY) || 1;
          const covered = sense.defensePlayers.some((d) => {
            const dx = d.pose.x * 94 - rx;
            const dy = d.pose.y * 50 - ry;
            // defender between roller and rim (rim-side) and within 6ft
            const along = (dx * toRimX + dy * toRimY) / rimLen;
            return along > 0.5 && Math.hypot(dx, dy) < 6;
          });
          if (!covered) return { kind: 'pass', targetJersey: roller.jersey };
        }
      }
      return { kind: 'drive' };
    }
  }
  // A PNR action has two physical beats. Until the screen is actually used,
  //  a pass to an arbitrary spacer abandons the roll or pop before it
  // develops. The screener itself remains a legal pocket-pass target for a
  // blitz or an early advantage read.
  const pnrScreenStillSet = (tacticKind === 'PNR_ROLL' || tacticKind === 'PNR_POP')
    && screenExecutionPhase === 'SET';


  // Handler rim distance — used by shot classification, advance gate,
  // and rim priority. Computed early so all downstream logic can use it.
  const handlerRimDist = Math.hypot(
    (handler.pose.x - sense.rim.x) * 94,
    (handler.pose.y - sense.rim.y) * 50,
  );
  const zone = zoneFromPoint(handler.pose.x, handler.pose.y, sense.offense, sense.baskets);
  const shotValue = shotValueAt(handler.pose.x, handler.pose.y, sense.offense, sense.baskets);
  // P1.2 decision-time shot classification: a catch-window shot is
  // catch_shoot (the defender is still arriving), anything else is a
  // pull-up — pull-up threes are materially harder (0.33 vs 0.38 base),
  // which is the main brake on "just shoot threes". The same
  // classification flows through the Intent → startShot → resolveShot
  // chain (one-model discipline).
  const shotTypeForHandler: 'catch_shoot' | 'pull_up' | 'drive_finish' | 'other' =
    handlerRimDist <= 4 ? 'drive_finish'
    : sense.catchWindowSeconds > 0 ? 'catch_shoot' : 'pull_up';
  // A live handler on the configured arc with six feet of separation has
  // a real pull-up three, even before the formal EXECUTE window. This is
  // narrower than a generic catch-window exception: range and gap must
  // both be physically true.
  const onArc = handlerRimDist >= 21 && handlerRimDist <= 25;
  const genuinelyOpenThree = shotValue === 3 && onArc && sense.onBallDistanceFt >= 6;
  // Early-offense hurry penalty: with >16s left the defense is still
  // settling (large measured gaps are retreating defenders, not open
  // looks) and the offense is unorganized. Halfcourt possessions HARD
  // gate the early shot (0.7 discount still lost argmax against elite
  // shooters); only a genuine catch-and-shoot window may fire early.
  // TRANSITION possessions skip the penalty entirely — a fast break is
  // SUPPOSED to shoot early (the defense is retreating, not settled).
  const earlyGate = mode === 'TRANSITION' || sense.catchWindowSeconds > 0 || genuinelyOpenThree
    ? 1
    : sense.shotClock > 16
      ? 0
      : 1;
  // Structural gate: a halfcourt possession must not end in a shot
  // before the set forms — the SETUP→EXECUTE transition (shot ≤13) is
  // the only legal shot window. Both ADVANCE and SETUP are gated; only
  // EXECUTE (or a genuine catch-and-shoot window, or TRANSITION) may
  // shoot early. A 9s jack from the top is a rhythm collapse.
  // ISO is an isolation play — the handler attacks immediately from the
 // start. Gating the shot/drive to EXECUTE made the ISO handler stand
 // at 15ft for 7+ seconds doing nothing (measured: A20 froze at 13-15ft
 // for the entire SET phase). For ISO, skip the early gate so the
 // handler can drive or shoot as soon as they read the matchup.
  const instantAttack = (tacticKind === 'ISO' || tacticKind === 'HANDOFF' || tacticKind === 'DRIVE_KICK') && handlerRimDist < 35;
  const atRimNow = handlerRimDist <= 5;
  // advanceGate: during halfcourt SETUP (shot > 13), the handler may
  // not shoot/drive to prevent a rhythm collapse — EXCEPT:
  //   1. atRimNow: a layup is always correct
  //   2. instantAttack: ISO/HANDOFF/DRIVE_KICK probe at 0.2 rate
  //   3. screenUsed: once a screen is physically used (SCREEN_USE/
  //      ADVANTAGE), the tactic has executed — the handler read the
  //      coverage and must attack. Without this, holdDead=true +
  //      advanceGate=0 created a forced pass chain (6+ passes in 6s
  //      with no shot). A moderate 0.3 discount keeps possession
  //      length realistic while breaking the pass chain.
  const screenUsed = stage === 'SCREEN_USE' || stage === 'ADVANTAGE';
  // advanceGate: during halfcourt SETUP (shot > 13), the handler may
  // not shoot/drive to prevent a rhythm collapse — EXCEPT:
  //   1. atRimNow: a layup is always correct
  //   2. instantAttack: ISO/HANDOFF/DRIVE_KICK probe at 0.2 rate
  //   3. screenUsed: once a screen is physically used (SCREEN_USE/
  //      ADVANTAGE), the tactic has executed — the handler read the
  //      coverage and must attack. Without this, holdDead=true +
  //      advanceGate=0 created a forced pass chain (6+ passes in 6s
  //      with no shot). A moderate 0.3 discount keeps possession
  // Chain break exception: once the pass chain has run ≥2 exchanges with
  // no attack, the possession is stalling — the set gates exist to make
  // the offense organize BEFORE attacking, not to forbid attacking while
  // the ball circles forever (measured: a POST_UP stuck in SET produced
  // a 19-pass chain over 16s because shoot/drive were gated to 0 and the
  // pass was the only legal option). Open the gate progressively so a
  // stalling chain resolves into an attack.
  const chainStall = passChainSinceAttack >= 2 ? Math.min(1, 0.3 * (passChainSinceAttack - 1)) : 0;
  const advanceGate = possessionPhase !== 'EXECUTE' && mode !== 'TRANSITION' && !atRimNow && !genuinelyOpenThree
    ? instantAttack
      ? sense.shotClock > 13 ? Math.max(0.2, chainStall) : 1
      : screenUsed
        ? sense.shotClock > 13 ? Math.max(0.1, chainStall) : 1
        : sense.shotClock > 13 ? chainStall : 1
    : 1;
  // the shot is up they are on the shooter (≤2.5ft). Beyond 8ft the
  // look is genuinely open.
  // The ≤2.5ft compression is ONLY valid during the catch-and-shoot
  // window — the closeout flight began when the pass left, so the
  // defender IS still arriving and the effective contest is tighter
  // than the static gap. For a held-ball pull-up the defender's gap is
  // the truth: a defender parked 4ft away is 4ft away, not 2.5ft
  // (the old blanket compression priced every pull-up at 2.5ft,
  // killing the pull-up three entirely: 0 pull-up 3PA/game vs NBA
  // ~40% of threes).
  const closeoutDist = sense.catchWindowSeconds > 0
    ? (sense.onBallDistanceFt > 8 ? sense.onBallDistanceFt : Math.min(sense.onBallDistanceFt, 2.5))
    : sense.onBallDistanceFt;
  // Catch-and-shoot window: right after a catch the defender is still
  // arriving (their closeout flight began when the pass left), so the
  // look is genuinely better for the first ~0.4s. This is what makes a
  // swing pass to an open corner an actual shot, not the start of
  // another pass chain.
  const windowedCloseout = sense.catchWindowSeconds > 0
    ? Math.max(closeoutDist, 5)
    : closeoutDist;
  // Structural brake on contested pull-up threes: a defender within 5ft
  // makes the pull-up a bad shot regardless of EV math (NBA pull-up 3P%
  // against a close defender ≈ 28%). The catch-window shot keeps its
  // genuine openness; the non-window pull-up against pressure is priced
  // down hard so the handler looks for the drive/pass instead.
  // The threshold must match the engine's ACTUAL on-ball gap: the
  // adjustOnBallDefender blend parks the defender at ~4ft by design, so
  // a <5ft gate makes EVERY pull-up three eat the 0.55 penalty and the
  // pull-up three disappears entirely (measured: 0 pull-up 3PA/game vs
  // NBA ~40% of threes are pull-ups). A 4ft gap is a contestable but
  // real look; only ≤3.2ft (a true hug) is the brick-wall zone.
  const pullUpPressurePenalty = shotValue === 3 && sense.catchWindowSeconds <= 0 && sense.onBallDistanceFt < 3.2
    ? 0.55
    : 1;
  // ── execution-time discipline ────────────────────────────────────────
  // A PNR that JUST used the screen (SCREEN_USE) is not a shot moment —
  // the handler must read the coverage and let the roll/pop develop.
  // The old code let the handler shoot immediately at the strike pocket,
  // collapsing possessions to 6-8s. The ADVANTAGE stage (the coverage
  // read happened, the advantage is real) is the shot moment.
  const executionDiscipline = stage === 'SCREEN_USE' && sense.catchWindowSeconds <= 0 && handlerRimDist > 6 && !screenUsed
    ? 0.6
    : 1;
  // Perimeter range used by both the EV gate and the catch-window guide.
  const openThreeRange = shotValue === 3 && handlerRimDist <= (Math.abs(handler.pose.y - 0.5) * 50 > 19 ? 23 : 25);
  // EXECUTE cooldown: the first 2s after EXECUTE begins the set is still
  // developing — the shot is forced to wait (except a genuine
  // catch-and-shoot window). This is the structural time model's
  // "execute" commitment. Hard gate: the shoot option is REMOVED, not
  // discounted — a discounted shot still wins argmax against a low hold.
  const executeCooldownActive = executeCooldownTicks > 0 && sense.catchWindowSeconds <= 0 && !genuinelyOpenThree;
  // Deep pull-up gate: no pull-up threes from beyond ~4ft past the arc
  // (real NBA: 28ft+ pull-ups are ~2% of attempts). The catch-and-shoot
  // window keeps its spot only when the receiver is actually on the arc.
  const handlerPose = sense.offensePlayers.find((p) => p.jersey === handlerId)?.pose;
  let deepPullupBlock = 1;
  if (shotValue === 3 && handlerPose && shotTypeForHandler !== 'catch_shoot') {
    const hRimDist = Math.hypot(
      (handlerPose.x - sense.rim.x) * 94,
      (handlerPose.y - sense.rim.y) * 50,
    );
    const hCorner = Math.abs(handlerPose.y - 0.5) * 50 > 19.5;
    const hLine = hCorner ? 22 : 23.75;
    if (hRimDist - hLine > 4.5) deepPullupBlock = 0;
  }
  const deepCatchBlock = shotValue === 3 && shotTypeForHandler === 'catch_shoot' && !openThreeRange ? 0 : 1;
  // Early-clock discipline: a non-catch-window shot in the first ~4s of a
  // halfcourt possession is a rushed look (NBA: ~15% of attempts come
  // before 20s on the shot clock). The mild discount keeps the argmax
  // honest about using the possession instead of jacking the first
  // semi-open look (measured: mean possession length 11.6s vs the NBA
  // 13.5-15.5 acceptance band).
  const earlyClockRush = sense.shotClock > 20 && sense.catchWindowSeconds <= 0 && mode === 'HALFCOURT' ? 0.85 : 1;
  const evShoot = expectedShotPointsWithType(sense, handlerId, zone, shotValue, windowedCloseout, shotTypeForHandler) * earlyGate * advanceGate * deepPullupBlock * deepCatchBlock * pullUpPressurePenalty * executionDiscipline * earlyClockRush * (executeCooldownActive ? 0 : 1);
  // The pocket-pass window: while the screen is still SET at SCREEN_USE, the
  // forced drive used to bypass the roll man entirely (measured: rolls fed on
  // 4% of screens vs NBA 40-55% — the defense never paid for dropping). When
  // the roller has a REAL window (within 14ft of the rim with his nearest
  // defender 5+ft away — the defense in rotation), the pocket feed is the
  // designed read and competes with the drive instead of being skipped.
  const rollerWindow = screenTargetId !== null
    ? (() => {
      const roller = sense.offensePlayers.find((p) => p.jersey === screenTargetId);
      if (!roller) return false;
      const mateRim = Math.hypot(
        (roller.pose.x - sense.rim.x) * 94,
        (roller.pose.y - sense.rim.y) * 50,
      );
      const rollerDefDist = sense.defensePlayers.reduce(
        (best, d) => Math.min(best, Math.hypot((d.pose.x - roller.pose.x) * 94, (d.pose.y - roller.pose.y) * 50)),
        Infinity,
      );
      // 3ft gate: the roll man's defender typically tags at 2-3ft — the
      // pocket pass is still the designed read whenever the roller has a
      // step (the 5ft gate never opened; rolls fed on 1-4% of screens).
      return mateRim <= 14 && rollerDefDist >= 3;
    })()
    : false;
  const pnrScreenDeveloping = pnrScreenStillSet && stage === 'SCREEN_USE' && sense.shotClock > 4 && handlerRimDist > 6 && !rollerWindow;
  if (pnrScreenDeveloping && coverage !== 'BLITZ') return { kind: 'drive' };
  const evDrive = expectedDrivePoints(sense, handlerId) * earlyGate * advanceGate;

  let bestPassEv = 0;
  let screenPassEv = 0;
  let bestTarget: string | null = null;
  for (const mate of sense.offensePlayers) {
    if (mate.jersey === handlerId) continue;
    let ev = expectedPassPoints(sense, handlerId, mate.jersey);
    // PNR roll/pop feed: after the screen is used, the screener rolling
    // to the rim (or popping to the line) is a designed target — the
    // defense is in rotation, the pocket pass is high-value. Without
    // this the handler never fed the roll (81% shot share off screens).
    if ((tacticKind === 'PNR_ROLL' || tacticKind === 'PNR_POP') && (stage === 'ADVANTAGE' || stage === 'SCREEN_USE')) {
      const mateRim = Math.hypot(
        (mate.pose.x - sense.rim.x) * 94,
        (mate.pose.y - sense.rim.y) * 50,
      );
      // Roll man near the rim (≤14ft) gets the pocket-pass bonus; the
      // pop man beyond the arc gets the kick-out bonus. The pocket bonus
      // scales with the roller's window — a wide-open roller is the best
      // pass in basketball (measured before this scaling: rolls fed on
      // only 4% of screens, NBA 40-55%).
      if (mateRim <= 14) {
        const rollerDefDist = sense.defensePlayers.reduce(
          (best, d) => Math.min(best, Math.hypot((d.pose.x - mate.pose.x) * 94, (d.pose.y - mate.pose.y) * 50)),
          Infinity,
        );
        ev *= rollerDefDist >= 5 ? 1.75 : 1.25;
      } else if (mateRim >= 21) ev *= 1.15;
    }
    // Designed-feed families: the plan's named second target (the curler /
    // post hub) is the family's PURPOSE. A SEALED POST is priced by position,
    // not openness — entry passes into tight coverage complete ~90% in the
    // NBA (the post man's body IS the advantage); openness-based EV called
    // every entry a turnover risk and the post never got fed (0/57). The
    // curler off a pin is priced by his separation only when the pin has
    // actually separated him.
    if (tacticKind === 'POST_UP' && screenTargetId !== null && mate.jersey === screenTargetId) {
      const mateRim0 = Math.hypot(
        (mate.pose.x - sense.rim.x) * 94,
        (mate.pose.y - sense.rim.y) * 50,
      );
      // Entry viable when the hub is sealed inside 14ft; the pass bonus is
      // the designed continuation, defender proximity notwithstanding.
      if (mateRim0 <= 14) ev = Math.max(ev, 0.95);
    }
    if (tacticKind === 'OFF_BALL_SCREEN' && screenTargetId !== null && mate.jersey === screenTargetId) {
      ev *= stage === 'SCREEN_USE' || stage === 'ADVANTAGE' ? 1.6 : 1.15;
    }
    // HANDOFF: when the screener approaches the handler (≤6ft), the pass
    // to them is the designed action — the handoff exchange. Without this
    // bonus the handler never passes to the approaching screener and the
    // "handoff" never happens (measured: handler waits 3s then drives
    // alone, no exchange occurs).
    if (tacticKind === 'HANDOFF' && (stage === 'SET' || stage === 'SCREEN_APPROACH')) {
      const hp = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose;
      if (hp) {
        const mateDist = Math.hypot((mate.pose.x - hp.x) * 94, (mate.pose.y - hp.y) * 50);
        if (mateDist <= 6) ev *= 2.5;
      }
    }
    if (mate.jersey === screenTargetId) screenPassEv = ev;
    if (ev > bestPassEv) {
      bestPassEv = ev;
      bestTarget = mate.jersey;
    }
  }
  // A DHO is a physical exchange, not a normal swing pass. Once the
  // screener is within handoff distance, a pressured handler must complete
  // the exchange even when a distant spacer has the larger raw pass EV; the
  // handoff's downhill continuation is the tactical value being priced.
  const handoffMate = tacticKind === 'HANDOFF'
    ? sense.offensePlayers.find((mate) => mate.jersey !== handlerId && mate.jersey === screenTargetId)
      ?? sense.offensePlayers.find((mate) => mate.jersey !== handlerId && Math.hypot((mate.pose.x - handler.pose.x) * 94, (mate.pose.y - handler.pose.y) * 50) <= 6)
    : null;
  const handoffDistance = handoffMate
    ? Math.hypot((handoffMate.pose.x - handler.pose.x) * 94, (handoffMate.pose.y - handler.pose.y) * 50)
    : Infinity;
  const handoffExchange = handoffMate
    && handoffDistance <= 6
    && sense.onBallDistanceFt < 6
    && handlerRimDist > 18
    // A handoff is a SET action. Once the exchange has completed, the
    // receiver is in the downhill/read beat; allowing another handoff from
    // ADVANTAGE recreates a stationary pass-around instead of an attack.
    && stage === 'SET'
    && !(recentHandoff
      && recentHandoff.giverId === handoffMate.jersey
      && recentHandoff.receiverId === handlerId);
  if (handoffExchange) return { kind: 'handoff', targetJersey: handoffMate.jersey };
  const openish = evShoot >= 1.1;
  // Hold's own value: the best action the handler can take WITHOUT giving
  // up the ball (shoot/drive only). Including bestPassEv here made hold
  // inherit the pass's value as its own — hold(EV) = passEV + org - 0.03
  // always beat pass(EV) = passEV × 0.93, so the handler NEVER passed to
  // a wide-open teammate (measured: 16ft-open slot mate never fed through
  // an entire 14s ISO possession; the handler relocated until forced shot).
  const bestNow = Math.max(evShoot, evDrive);
  const orgValue = openish
    ? 0
    : instantAttack
      ? stage === 'SET' ? 0.02 : 0.01
      : stage === 'SCREEN_APPROACH'
        ? 0.11
        : stage === 'SET'
          ? 0.06
          : 0.04;
  const timeDecay = sense.shotClock > 15 ? 1 : Math.max(0, (sense.shotClock - 4) / 10);

  // ── P2.1 tendency priors ──────────────────────────────────────────────
  // The EV core prices what the SITUATION is worth; the tendency layer
  // prices what THIS PLAYER will do. A T3=90 gunner and a T3=10 floor
  // general face the same shot but value it differently — that is the
  // entire player-identity lever. Priors are multiplicative modifiers
  // around neutral 50 (1.0), clamped to [0.7, 1.3]:
  //   T3/TMID/TDRIVE/TPOST (sum 100) modulate shoot/drive share by zone
  //   PASS1ST modulates the pass option
  //   TAKEOVER raises aggression in clutch (P2.4)
  //   PUSH raises transition advance value (ADVANCE phase)
  const tend = sense.tendencies?.[handlerId] ?? null;
  const prior = (key: 'T3' | 'TMID' | 'TDRIVE' | 'TPOST' | 'PASS1ST' | 'TAKEOVER' | 'PUSH', fallback: number = 1): number => {
    if (!tend) return fallback;
    const value = tend[key];
    if (value === undefined) return fallback;
    return Math.max(0.7, Math.min(1.3, 1 + (value - 50) / 100));
  };
  const t3Prior = prior('T3');
  const tmidPrior = prior('TMID');
  const tdrivePrior = prior('TDRIVE');
  const tpostPrior = prior('TPOST');
  const passPrior = prior('PASS1ST');
  const takeoverPrior = prior('TAKEOVER');
  const pushPrior = prior('PUSH');

  // ── P2.4 situation layer ──────────────────────────────────────────────
  // Score × time × period strategy, folded into the same option EVs.
  // Late-game trailing: threes worth more; leading: hold worth more
  // (milking the clock). Clutch (≤2 min, score diff ≤5): TAKEOVER
  // aggression. 2-for-1 window (≤40s left in period, shot clock ≤14):
  // slight pass-now bias.
  const scoreDiff = sense.scoreDiff ?? 0;
  const gameClock = sense.gameClock;
  const period = sense.period ?? 1;
  const quarterEnd = period <= 4 ? 720 : 300;
  const twoForOne = quarterEnd - gameClock <= 40 && sense.shotClock <= 14;
  const clutch = gameClock <= 120 && Math.abs(scoreDiff) <= 5;
  const trailing3 = scoreDiff <= -3;
  const leading3 = scoreDiff >= 3;
  // 追分博弈: 落后时3分获得战术性加成(需要快速追分),2分不享受
  // 同等加成——否则2分更高的base rate永远压过3分,落后方会一直
  // 投中距离(审计NO_CLUTCH_3PT: 落后9分投16ft中距离,落后6分投
  // 21ft的2分,全场只有3/26关键球是3分)。NBA落后≥3分时3分出手
  // 占比从~38%升到~55%。
  const situationShoot = trailing3
    ? (shotValue === 3 ? 1.22 : 0.98)
    : leading3
      ? (shotValue === 3 ? 0.94 : 1.02)
      : 1;
  const situationHold = leading3 ? 1.1 : trailing3 ? 0.85 : 1;
  const situationPass = twoForOne ? 1.06 : 1;
  const clutchFactor = clutch ? takeoverPrior : 1;

  const rimZone = zone === 'rim' || zone === 'dunker_L' || zone === 'dunker_R' || zone === 'paint';
  // A pull-up mid-range from the strike pocket is the lowest-value look
  // in modern basketball — the pocket exists to launch the ACTION
  // (screen/drive/pass), not to shoot. Only a catch-window or late-clock
  // (≤8s) mid shot is a real option. HANDOFF is stricter: after the
  // exchange the handler should turn the corner or reach the rim, not
  // stop at 19–23ft with a defender attached. A pull-up remains legal
  // only after a genuine downhill probe has brought the handler inside
  const pocketPullup = shotValue === 2 && !rimZone && sense.catchWindowSeconds <= 0 && sense.shotClock > 8;
  const handoffPullupOutsideLane = tacticKind === 'HANDOFF' && pocketPullup && handlerRimDist > 18;
  const pocketPullupPenalty = pocketPullup ? 0.95 : 1;
  // A DHO can produce a legitimate elbow pull-up when the defender goes
  // under, but not a 19–23ft stop while the handler is still attached to
  // the defender. The next action is turn-the-corner drive, re-screen, or
  // pass—not a bailout jumper. This is an action-level brake, not a shot
  // distribution target: the shot option is unavailable in that state.
  const handoffPullupBlock = handoffPullupOutsideLane && sense.onBallDistanceFt < 6 ? 0 : 1;
  // ── tactic-driven shot-profile guide (P3.3 precursor) ────────────────
  // The argmax alone swings the shot profile to extremes (90% threes or
  // 70% drives depending on tiny rate differences). Real offenses run
  // SETS with a designed shot outcome: PNR hunts the pocket pull-up or
  // the roll, ISO hunts the mid-post, POST hunts the paint. The tactic
  // guides the SHARE of shots, the EV still prices the exact look.
  let tacticShootGuide = 1;
  let tacticDriveGuide = 1;
  switch (tacticKind) {
    case 'PNR_ROLL':
    case 'PNR_POP':
      // The screen creates the lane: driving off the screen is the
      // designed attack (not a 81%-shot jack). The shooter guide stays
      // near neutral so the pull-up remains a real option under DROP,
      // not the only option.
      tacticDriveGuide = 1.0; // drive and kick-out 3 should compete equally
      tacticShootGuide = 1.0;
      break;
    case 'ISO':
      tacticDriveGuide = 1.05;
      tacticShootGuide = 1.25; // ISO mid-post pull-ups are the designed look
      break;
    case 'POST_UP':
      tacticDriveGuide = 1.15; // seal → paint touch
      tacticShootGuide = 0.85;
      break;
    case 'DRIVE_KICK':
      tacticDriveGuide = 1.2;
      tacticShootGuide = 0.9;
      break;
    case 'OFF_BALL_SCREEN':
      tacticShootGuide = 1.15; // curl → catch-shoot
      tacticDriveGuide = 0.9;
      break;
    default:
      break;
  }
  // A wide-open catch-and-shoot three is the highest-EV shot in basketball,
  // but the catch window must not bless an impossible release point. A
  // receiver still 4+ feet beyond the arc is jogging into the pass, not set
  // for a normal rhythm three; wait for them to arrive or swing the ball.
  const openThree = openThreeRange && (sense.catchWindowSeconds > 0 || sense.onBallDistanceFt > 6);
  const shootGuide = openThree ? Math.max(1, tacticShootGuide) : tacticShootGuide;
  // An open three is the best shot in basketball for ANY NBA player — even a
  // low-T3 tendency player shoots ~36% on a wide-open catch-and-shoot. The
  // t3Prior remains a tendency modifier for contested looks, but it should
  // not kill an open three.
  const effectiveT3Prior = openThree ? Math.max(0.9, t3Prior) : t3Prior;
  // Rim priority: a player WITH the ball inside 4ft of the rim should
  // almost always shoot (layup/dunk). NBA rim-adjacent FG% is 65-75%,
  // making it the highest-EV play on the floor. Without this boost the
  // rim EV competes with pass-out EV and players repeatedly pass from
  // point-blank range instead of finishing (measured: A19 at 1ft
  // passed 4 times instead of shooting).
  const atRim = handlerRimDist <= 4 && shotValue === 2;
  const rimPriority = atRim ? 1.4 : 1;
  const evShootFinal = evShoot * (shotValue === 3 ? effectiveT3Prior : rimZone ? tdrivePrior : tmidPrior) * situationShoot * clutchFactor * pocketPullupPenalty * handoffPullupBlock * shootGuide * rimPriority;
  const evDriveFinal = evDrive * tdrivePrior * clutchFactor * tacticDriveGuide;
  // ── P3.3 coverage reads ──────────────────────────────────────────────
  // The handler READS the defense's screen coverage and prices the
  // options accordingly (real PNR reads):
  //   BLITZ: the trap takes away the pull-up and the drive; the short
  //     roll/pocket pass is the answer — pass EV up.
  //   ICE: the baseline drive is walled off; the pull-up and the
  //     cross-screen (switch-hunt) are the answers — drive down, shoot up.
  //   DROP: the big sinks; the pull-up mid/three is the correct attack —
  //     shoot up, drive down (no paint to attack).
  //   HEDGE: balanced; slight pass-up (the roller is temporarily free).
  //   SWITCH: the mismatch is the hunt — drive at the new matchup.
  let coverageShoot = 1;
  let coverageDrive = 1;
  let coveragePass = 1;
  switch (coverage) {
    case 'BLITZ':
      coverageShoot = 0.8;
      coverageDrive = 0.7;
      coveragePass = 1.25;
      break;
    case 'ICE':
      coverageShoot = 1.15;
      coverageDrive = 0.75;
      break;
    case 'DROP':
      coverageShoot = 1.2;
      coverageDrive = 0.85;
      break;
    case 'HEDGE':
      coveragePass = 1.1;
      coverageDrive = 0.95;
      break;
    case 'SWITCH':
      coverageDrive = 1.15;
      coverageShoot = 0.9;
      break;
    default:
      break;
  }
  const evShootCovered = evShootFinal * coverageShoot;
  const evDriveCovered = evDriveFinal * coverageDrive;
  // P2.5 chain tax: the two-step lookahead raised pass EV (correct for
  // real chain value), but it also made pass the default answer — every
  // possession became a pass chain. The defense re-positions on every
  // catch, so each extra pass carries a real opportunity cost; the 0.93
  // tax trims the excess without freezing ball movement. During a
  // catch-and-shoot window the handler JUST received the ball — the
  // catch window exists to let them shoot the open look, not to
  // immediately re-pass. The pass EV is priced at a steep 0.2 multiplier
  // during the window so only a truly open teammate (where the pass
  // value vastly exceeds shooting) triggers a re-pass; otherwise the
  // handler shoots, drives, or holds until the window expires and the
  // 2.8s dwell enforces real possession time (measured 8.8
  // passes/possession vs NBA ~3.5 without this).
  const catchWindowPassPenalty = sense.catchWindowSeconds > 0 ? 0.4 : 1;
  // During SCREEN_USE the handler just used the screen — the defense is
  // in rotation and the advantage is LIVE. An immediate pass wastes the
  // screen advantage (the roller/pop man hasn't settled, the defense
  // hasn't committed). A moderate penalty suppresses the pass chain
  // (measured: 5+ passes in 5s during PNR_POP SCREEN_USE) while keeping
  // a genuinely open kick-out viable.
  const screenUsePassPenalty = stage === 'SCREEN_USE' && possessionPhase !== 'EXECUTE' && !rollerWindow ? 0.5 : 1;
  // The catch window exists to SHOOT the open look: a receiver who just
  // caught with the closeout still in flight must not re-pass the advantage
  // away (measured: endless drive_kick chains — 6+ one-touch passes per
  // possession ending in shot-clock violations). When the window is live
  // and the catch look is real (any three, or any two inside the paint),
  // the pass option is OFF — shoot or attack the closeout.
  const windowRimLook = sense.catchWindowSeconds > 0 && handlerRimDist <= 5 && sense.onBallDistanceFt >= 4;
  const openWindowShoot = sense.catchWindowSeconds > 0 && (openThree || windowRimLook);
  const windowPassFactor = openWindowShoot ? 0.25 : 1;
  // Pass-chain decay: a called play survives ball reversals, but a chain
  // of passes with NO attacking action is a stalled possession. Real NBA
  // offenses average ~2.4 passes before a shot attempt; ball reversal is
  // tactical, a 4th pass without anyone attacking is dribbling in circles.
  // Each chain step past the second cuts the pass option hard — the EV
  // must prefer an attack (even a contested one) over passing forever.
  const chainPassFactor = passChainSinceAttack >= 2
    ? Math.max(0.25, 1 - (passChainSinceAttack - 1) * 0.35)
    : 1;
  const evPassFinal = bestPassEv * windowPassFactor * chainPassFactor * passPrior * situationPass * 0.93 * catchWindowPassPenalty * screenUsePassPenalty;
  const evScreenPassFinal = screenPassEv * windowPassFactor * chainPassFactor * passPrior * situationPass * 0.93 * catchWindowPassPenalty * screenUsePassPenalty;
  const evPassCovered = evPassFinal * coveragePass;
  const evScreenPassCovered = evScreenPassFinal * coveragePass;
  // Ball-handling fundamentals are real beats, not spectator labels. They
  // are selected only while the ball is held, outside a live drive/screen
  // completion, and never repeat back-to-back. A beat consumes time and
  // then returns to the same EV read: triple-threat is the neutral perimeter
  // stance, back-to-basket is a near-paint seal, pivot is a tight catch read,
  // and crossover is the first change-of-direction attack.
  if (handlingActionCooldownTicks <= 0 && lastHandlingAction === null
    && !activeDrive
    && possessionPhase === 'EXECUTE'
    && mode === 'HALFCOURT'
    && stage !== 'ADVANTAGE'
    && stage !== 'SCREEN_USE') {
    const nearPaint = handlerRimDist <= 12;
    const tight = sense.onBallDistanceFt < 4;
    if (nearPaint) return { kind: 'back_to_basket' };
    if (tight) return { kind: 'pivot' };
    if (evDriveCovered >= evShootCovered * 0.9) return { kind: 'crossover' };
    if (catchWindowTicks > 0 || sense.onBallDistanceFt >= 4) return { kind: 'triple_threat' };
  }

  // A screen that is PHYSICALLY SET is a commitment: the handler must
  // attack (drive/pass/shoot) or the screener's defender recovers for
  // free. Holding at the screen for seconds — the "everyone stands
  // around the ball" look — was the max of a hold EV that was priced
  // nearly equal to the best action. In ADVANTAGE the hold option is
  // REMOVED (hard zero), so the PNR always develops.
  const holdDead = stage === 'ADVANTAGE' || stage === 'SCREEN_USE';
  // ── 领先压时间(节奏博弈) ──────────────────────────────────────────
  // NBA: 末节领先≥6分的球队,每次进攻会用满24s(平均出手时shot
  // clock 16-18s,比落后方晚3-4s)。当前模型hold的EV只比出手高10%
  // (situationHold 1.1),而出手EV(~1.0)远高于hold的衰减值,领先方
  // 根本不会压时间(审计NO_TEMPO:领先方出手shot clock 14.8s vs
  // 落后方12.9s,差距仅1.9s)。
  // 机制: 末节后半段(gameClock<300)领先≥6分时,hold获得额外价值——
  // 每消耗1s shot clock,时间价值≈0.035分(一次进攻≈0.85分/24s)。
  // 这等价于"晚出手的期望收益",让argmax在shot clock 18s时选hold
  // 而非出手,比赛末段的节奏差异真实浮现。
  const milkingClock = leading3
    && gameClock <= 300
    && sense.shotClock > 8
    ? (sense.shotClock - 8) * 0.055
    : 0;
  const evHoldFinal = holdDead ? -1 : (bestNow + orgValue) * timeDecay - 0.03 + milkingClock;

  // ── P6.1 chemistry channels ──────────────────────────────────────────
  // The five-man unit's chemistry (synergy/clash/duplicate-class rules)
  // shifts option values ±10%: dime_cut synergy prices the pass higher,
  // paint_clog clash prices the drive lower, iso_space synergy opens the
  // isolation look. Fold the active effects for the OFFENSE unit.
  const chemEffects = sense.chemistry?.[sense.offense] ?? [];
  let chemShoot = 1;
  let chemDrive = 1;
  let chemPass = 1;
  for (const effect of chemEffects) {
    if (effect.channel === 'dime_cut' || effect.channel === 'cut_target' || effect.channel === 'relief_target') {
      chemPass *= effect.modifier;
    } else if (effect.channel === 'paint_clog') {
      chemDrive *= effect.modifier;
    } else if (effect.channel === 'iso_space' || effect.channel === 'spacer_open') {
      chemShoot *= effect.modifier;
    } else if (effect.channel === 'initiation_eff') {
      chemShoot *= effect.modifier;
      chemDrive *= effect.modifier;
    }
  }
  const handoffDownhillBlock = tacticKind === 'HANDOFF' && handlerRimDist > 18 && sense.onBallDistanceFt < 6;
  // A BLITZ's first release is the screener. The ordinary best-pass search
  // can prefer a spacer because it prices the post-catch continuation; while
  // the physical screen is still set, omitting that spacer option used to
  // omit ALL passes instead. With shoot/drive gated at zero, the default
  // array tie-break then returned a zero-EV shot from the top of the arc.
  const passOption = pnrScreenStillSet
    ? screenTargetId !== null && screenPassEv > 0 && (coverage === 'BLITZ' || bestTarget === screenTargetId)
      ? { kind: 'pass' as const, ev: evScreenPassCovered * chemPass, target: screenTargetId }
      : null
    : bestTarget
      ? { kind: 'pass' as const, ev: evPassCovered * chemPass, target: bestTarget }
      : null;
  const options: Array<{ kind: 'shoot' | 'drive' | 'pass' | 'hold' | 'pump_fake'; ev: number; target?: string }> = [
    { kind: 'shoot', ev: handoffDownhillBlock ? -1 : evShootCovered * chemShoot },
    { kind: 'drive', ev: handoffDownhillBlock ? Math.max(evDriveCovered * chemDrive, 0.8) : evDriveCovered * chemDrive },
    ...(passOption ? [passOption] : []),
    { kind: 'hold', ev: evHoldFinal * situationHold },
    // P3.7 pump fake: when the defense is in a hard closeout posture (the
    // on-ball defender within 3ft but NOT blocking the lane), a pump fake
    // converts the closeout into a drive. Valued at a fraction of the
    // drive EV — the fake itself is not a shot, it creates the next one.
    ...(sense.onBallDistanceFt < 3 && evDriveCovered > 0.5
      ? [{ kind: 'pump_fake' as const, ev: evDriveCovered * 0.6 }]
      : []),
  ];
  // ── Late-clock urgency (the heave law) ────────────────────────────────
  // With <=4s on the shot clock, an NBA offense ALWAYS rises for an attempt:
  // the alternatives (hold/organize) score zero at the horn. The EV gates
  // (early/advance/execute-cooldown) were built for shot-quality discipline
  // and, applied at the buzzer, produced silent late clocks: only 3 attempts
  // in the 0-4s bucket and passes that DECELERATED under pressure (3.5s →
  // 4.5s gaps, measured). At <=4s the shot goes up from wherever the ball is.
  if (sense.shotClock <= 3) {
    const urgencyShootEv = Math.max(evShootCovered * chemShoot, 0.35);
    const urgencyDriveEv = evDriveCovered * chemDrive;
    return urgencyShootEv >= urgencyDriveEv ? { kind: 'shoot' } : { kind: 'drive' };
  }
  let best = options[0]!;
  for (const o of options) {
    if (o.ev > best.ev) best = o;
  }
  switch (best.kind) {
    case 'shoot':
      return { kind: 'shoot' };
    case 'drive':
      return { kind: 'drive' };
    case 'pass':
      return { kind: 'pass', targetJersey: best.target ?? '' };
    case 'hold':
      return { kind: 'relocate' };
    case 'pump_fake':
      return { kind: 'pump_fake' };
  }
}

