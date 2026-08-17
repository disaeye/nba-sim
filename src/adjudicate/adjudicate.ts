/**
 * Adjudicate — sole resolve + event + possession + shot-clock gateway.
 * docs/foundation/architecture.md §1.4
 */
import type { GameState, TeamId } from '../state/types.js';
import type { Rng } from '../rng/types.js';
import type { CompletionFact } from '../completion/types.js';
import type { Intent } from '../decision/types.js';
import {
  loadResolveConfig,
  makeResolveContext,
  resolveDrive,
  resolveHandoff,
  resolvePass,
  resolveRebound,
  resolveShot,
  shotAbilityModifier,
  resolveSteal,
} from '../resolve/index.js';
import {
  ballDead,
  ballHeld,
  ballLoose,
  startPass,
  startShot,
  type BallMotionState,
} from '../court/ball-motion.js';
import { shotValueAt, zoneFromPoint, rimNorm } from '../court/geometry.js';
import { offenseAttacks } from '../court/alignment.js';
import { loadMobilityConfig } from '../court/mobility.js';
import { setTarget } from '../court/poses.js';
import { staminaFactor } from '../stamina.js';
import { emit } from './emit.js';
import { shotClockAfter } from './shot-clock.js';
import type { AdjudicateResult, EpisodeTransition } from './types.js';
import type { EndReason } from '../possession/types.js';

const RESOLVE = makeResolveContext(loadResolveConfig());
const MOBILITY = loadMobilityConfig();

/**
 * Maximum distance (ft) between victim and nearest defender for a whistle
 * to be legal. A foul is a contact event — the defender must be within
 * reach. Used to gate every FOUL draw (shooting at release, reach-in on
 * strip, drive contact) so position truth backs the call.
 */
export const CONTACT_FOUL_RADIUS_FT = 6;

function otherTeam(t: TeamId): TeamId {
  return t === 'home' ? 'away' : 'home';
}
import type { PoseMap } from '../court/poses.js';
import { distFeet } from '../court/geometry.js';

function findNearestDefenderDist(
  poses: PoseMap,
  offense: TeamId,
  x: number, y: number,
): number {
  let best = Infinity;
  for (const p of Object.values(poses)) {
    if (p.team === offense) continue;
    const d = distFeet(x, y, p.x, p.y);
    if (d < best) best = d;
  }
  return best === Infinity ? 10 : best;
}

function findNearestDefenderId(
  poses: PoseMap,
  offense: TeamId,
  x: number, y: number,
): string | null {
  let best = Infinity;
  let id: string | null = null;
  for (const p of Object.values(poses)) {
    if (p.team === offense) continue;
    const d = distFeet(x, y, p.x, p.y);
    if (d < best) { best = d; id = p.jersey; }
  }
  return id;
}

/**
 * Re-derive an un-consumed shooting foul for a shooter: the most recent
 * shooting FOUL on them that was followed by a SHOT_RELEASE (a real shot
 * foul — the drive-foul miss path also emits shooting:true but has no
 * release) and has no SHOT_RESULT after it yet. Emitted at release
 * (whistle at contact); consumed when the shot lands (or is killed by a
 * block). Returns the provisional 2/3 FT count, or null.
 */
function pendingShotFoulFromEvents(state: GameState, shooterId: string): 2 | 3 | null {
  const events = state.events;
  for (let i = events.length - 1; i >= 0; i -= 1) {
    const e = events[i]!;
    if (e.type !== 'FOUL') continue;
    if (e.payload['victim_id'] !== shooterId || e.payload['shooting'] !== true) continue;
    let hadRelease = false;
    let consumedByFt = false;
    let consumed = false;
    // Single pass over the tail — no slice allocation (this runs at every
    // shot arrival and clock-expiry tick; copying the event tail made a
    // game O(N²) in allocations).
    for (let j = i + 1; j < events.length; j += 1) {
      const e2 = events[j]!;
      if (e2.type === 'SHOT_RELEASE' && e2.payload['shooter_id'] === shooterId) hadRelease = true;
      else if (e2.type === 'FT_START' && e2.payload['shooter_id'] === shooterId) consumedByFt = true;
      else if (e2.type === 'SHOT_RESULT' && e2.payload['shooter_id'] === shooterId) consumed = true;
    }
    // A drive foul (no shot) must never be treated as a pending shot foul;
    // a foul already settled by its own FT sequence is consumed — a later
    // unrelated shot by the same player must not inherit it.
    if (!hadRelease || consumedByFt) continue;
    if (consumed) return null;
    return e.payload['free_throws_awarded'] === 3 ? 3 : 2;
  }
  return null;
}


function emptyResult(state: GameState, ball: BallMotionState): AdjudicateResult {
  return {
    state,
    ball,
    events: [],
    episode: { kind: 'none' },
    clearBallIntent: false,
    forceDecision: false,
  };
}

function endEpisode(endReason: EndReason, nextStartReason: string): EpisodeTransition {
  return { kind: 'end', endReason, nextStartReason };
}

function withShotClock(state: GameState, shot: number): GameState {
  return {
    ...state,
    clocks: { ...state.clocks, shot },
  };
}

function withPossession(state: GameState, team: TeamId): GameState {
  return { ...state, possession: { team } };
}

/**
 * Apply one completion fact. ONE resolve site. ONE event emission path.
 */
export function adjudicateFact(
  state: GameState,
  ball: BallMotionState,
  fact: CompletionFact,
  rng: Rng,
  stickyBallIntent: Intent | null = null,
  stamina?: Readonly<Record<string, { readonly max: number; stm: number }>>,
): AdjudicateResult {
  const staminaFactorOf = (jersey: string): number => {
    const st = stamina?.[jersey];
    const base = st ? staminaFactor(st.stm) : 1;
    // P6.2: team morale exec multiplier rides the same factor.
    const team = state.lineups.home.includes(jersey) ? 'home' : 'away';
    const morale = state.moraleExec[team] ?? 1;
    return base * morale;
  };
  const beforeSeq = state.seq;
  let s = state;
  let b = ball;
  let episode: EpisodeTransition = { kind: 'none' };
  let clearBallIntent = false;
  let forceDecision = false;
  let pendingFt: AdjudicateResult['pendingFt'];

  const finish = (): AdjudicateResult => {
    const events = s.events.filter((e) => e.seq > beforeSeq);
    return { state: s, ball: b, events, episode, pendingFt, clearBallIntent, forceDecision };
  };

  switch (fact.kind) {
    case 'IntentPassStarted': {
      const from = s.poses[fact.passerId];
      const to = s.poses[fact.receiverId];
      if (!from || b.status !== 'held') return emptyResult(s, b);
      if (!to) {
        s = emit(s, 'OOB', [fact.passerId], {
          team_causing: fact.team,
          player_id: fact.passerId,
          receiver_id: fact.receiverId,
          x: from.x,
          y: from.y,
        });
        b = ballLoose({ x: from.x, y: from.y }, null, otherTeam(fact.team));
        s = withPossession(s, otherTeam(fact.team));
        s = withShotClock(s, shotClockAfter('turnover'));
        episode = endEpisode('TURNOVER', 'AFTER_OOB');
        clearBallIntent = true;
        forceDecision = true;
        return finish();
      }
      const receiverPressure = findNearestDefenderDist(s.poses, fact.team, to.x, to.y);
      // Pass-lane geometry: a defender standing ON the ball→receiver segment
      // can deflect/intercept regardless of where the receiver's man is. The
      // lane is only contested where the defender can actually reach it —
      // the middle 15-85% of the segment (an on-ball defender hugging the
      // passer is doing his job, not jumping the lane).
      const lanePenalty = (() => {
        const ax = from.x * 94, ay = from.y * 50, bx = to.x * 94, by = to.y * 50;
        const dx = bx - ax, dy = by - ay;
        const len2 = dx * dx + dy * dy;
        if (len2 < 1) return 0;
        let worst = 0;
        for (const defender of Object.values(s.poses)) {
          if (defender.team === fact.team) continue;
          const px = defender.x * 94, py = defender.y * 50;
          const t = ((px - ax) * dx + (py - ay) * dy) / len2;
          if (t < 0.15 || t > 0.85) continue; // passer-side / receiver-side presence isn't a lane jump
          const perp = Math.abs((px - ax) * dy - (py - ay) * dx) / Math.sqrt(len2);
          if (perp > 3) continue; // ≥3ft off the line: a real window, not a wall
          // 0ft off the line at mid-segment = full deflection threat.
          worst = Math.max(worst, (3 - perp) / 3);
        }
        return worst * 0.10; // ≤10pp failure add-on for a dead-center lane jumper
      })();
      const pressurePenalty = Math.max(0, Math.min(0.08, (6 - receiverPressure) / 6 * 0.08)) + lanePenalty;
      const passerAbilities = s.abilities[fact.passerId] ?? null;
      const receiverAbilities = s.abilities[fact.receiverId] ?? null;
      const passDistFt = Math.hypot((to.x - from.x) * 94, (to.y - from.y) * 50);
      const launch = resolvePass(rng, RESOLVE, {
        pressurePenalty,
        staminaModifier: staminaFactorOf(fact.passerId),
        passerAbility: passerAbilities?.passing,
        receiverCatch: receiverAbilities?.catchShoot,
        distanceFt: passDistFt,
      });
      if (!launch.success) {
        s = emit(s, 'TURNOVER', [fact.passerId], {
          player_id: fact.passerId,
          team: fact.team,
          turnover_type: 'bad_pass',
        });
        b = ballLoose({ x: from.x, y: from.y }, null, otherTeam(fact.team));
        s = withPossession(s, otherTeam(fact.team));
        s = withShotClock(s, shotClockAfter('turnover'));
        episode = endEpisode('TURNOVER', 'LIVE_BALL_TURNOVER_RECOVER');
        clearBallIntent = true;
        forceDecision = true;
        return finish();
      }
      b = startPass({ from, to, team: fact.team, passerId: fact.passerId });
      s = emit(s, 'PASS', [fact.passerId, fact.receiverId], {
        passer_id: fact.passerId,
        receiver_id: fact.receiverId,
        note: 'flight_start',
      });
      s = {
        ...s,
        poses: {
          ...s.poses,
          [to.jersey]: setTarget(to, to.x, to.y, 'pass_receive'),
        },
      };
      // Re-allocate the defense DURING the flight (real defenders read
      // the pass and start rotating before the catch) — otherwise the
      // receiver catches and shoots with the old on-ball defender still
      // 17ft away denying a different man (measured at Q1 11:45).
      forceDecision = true;
      clearBallIntent = true;
      return finish();
    }

    case 'PassOutOfBounds': {
      s = emit(s, 'OOB', [fact.passerId], {
        team_causing: fact.team,
        player_id: fact.passerId,
        receiver_id: fact.receiverId,
        x: fact.x,
        y: fact.y,
      });
      b = ballLoose({ x: fact.x, y: fact.y }, null, otherTeam(fact.team));
      episode = endEpisode('TURNOVER', 'AFTER_OOB');
      clearBallIntent = true;
      forceDecision = true;
      return finish();
    }
    case 'PassMissed': {
      s = emit(s, 'TURNOVER', [fact.passerId], {
        player_id: fact.passerId,
        team: fact.team,
        receiver_id: fact.receiverId,
        turnover_type: 'pass_receiver_missed',
      });
      s = withPossession(s, otherTeam(fact.team));
      s = withShotClock(s, shotClockAfter('turnover'));
      episode = endEpisode('TURNOVER', 'LIVE_BALL_TURNOVER_RECOVER');
      clearBallIntent = true;
      forceDecision = true;
      return finish();
    }
    case 'BallArrivedAtReceiver': {
      const recv = s.poses[fact.receiverId];
      if (!recv) return emptyResult(s, b);
      const passType = b.passType ?? null;
      // Retain the passer on the held ball: the perception layer reads it
      // as lastPasserId for the return-pass (hot potato) guard.
      b = ballHeld(recv, fact.team, null, fact.passerId);
      s = emit(s, 'PASS', [fact.passerId, fact.receiverId], {
        passer_id: fact.passerId,
        receiver_id: fact.receiverId,
        note: 'flight_complete',
        ...(passType ? { pass_type: passType } : {}),
      });
      s = {
        ...s,
        ball: { ...s.ball, holderId: fact.receiverId, status: 'held' },
      };
      forceDecision = true;
      clearBallIntent = true;
      return finish();
    }

    case 'HandoffExchange': {
      const ok = resolveHandoff(rng, RESOLVE);
      if (!ok.success) {
        s = emit(s, 'TURNOVER', [fact.giverId], {
          player_id: fact.giverId,
          team: fact.team,
          turnover_type: 'handoff',
        });
        const g = s.poses[fact.giverId];
        if (g) b = ballLoose({ x: g.x, y: g.y }, null, otherTeam(fact.team));
        s = withPossession(s, otherTeam(fact.team));
        s = withShotClock(s, shotClockAfter('turnover'));
        episode = endEpisode('TURNOVER', 'LIVE_BALL_TURNOVER_RECOVER');
        clearBallIntent = true;
        forceDecision = true;
        return finish();
      }
      const recv = s.poses[fact.receiverId];
      s = emit(s, 'HANDOFF', [fact.giverId, fact.receiverId], {
        giver_id: fact.giverId,
        receiver_id: fact.receiverId,
      });
      if (recv) b = ballHeld(recv, fact.team, null, fact.giverId);
      forceDecision = true;
      clearBallIntent = true;
      return finish();
    }

    case 'PassIntercepted':
    case 'HolderStripped': {
      const stealerPose = s.poses[fact.stealerId];
      const victimPose = s.poses[fact.victimId];
      const contactDistance = stealerPose && victimPose
        ? distFeet(stealerPose.x, stealerPose.y, victimPose.x, victimPose.y)
        : Infinity;
      // A mid-pass interception is a hand-to-ball play, not a teleporting
      // turnover. The candidate gate is measured against the flight segment,
      // but the victim can keep running while the ball travels; cap the final
      // award to a reachable contest window and otherwise let the pass finish.
      // The cap must match the audit's STEAL_OUT_OF_REACH gate (6ft): a
      // longer allowance produced steals with the stealer 6.3ft from the
      // victim, which the reality gate flags as an error.
      const maxStealDistance = fact.kind === 'PassIntercepted' ? 6 : CONTACT_FOUL_RADIUS_FT;
      if (contactDistance > maxStealDistance) return emptyResult(s, b);
      const stealerAbilities = s.abilities[fact.stealerId] ?? null;
      const stealerData = s.playerData[fact.stealerId] ?? null;
      const steal = resolveSteal(rng, RESOLVE, {
        playerModifier: (1 + ((stealerAbilities?.onBallDefense ?? 0.5) - 0.5) * 0.8) * staminaFactorOf(fact.stealerId),
        stlAbility: stealerData ? stealerData.ability.STL / 99 : undefined,
        gambleTendency: stealerData ? stealerData.tendency.GAMBLE : undefined,
      });
      if (!steal.success) {
        if (fact.kind === 'HolderStripped') {
          const inReach = contactDistance <= CONTACT_FOUL_RADIUS_FT;
          const reachIn = inReach && rng.next() < RESOLVE.baseRates.foul_on_drive_rate * 0.25;
          if (reachIn) {
            const offenderId = s.poses[fact.stealerId] ? fact.stealerId : fact.victimId;
            const offenderTeam = otherTeam(s.possession.team ?? 'home');
            s = emit(s, 'FOUL', [offenderId, fact.victimId], {
              offender_id: offenderId,
              offender_team: offenderTeam,
              victim_id: fact.victimId,
              foul_type: 'reach_in',
              shooting: false,
              free_throws_awarded: 0,
            });
          }
        }
        return emptyResult(s, b);
      }
      const stealer = s.poses[fact.stealerId];
      const victim = fact.victimId;
      if (!stealer) return emptyResult(s, b);
      s = emit(s, 'STEAL', [fact.stealerId, victim], {
        stealer_id: fact.stealerId,
        victim_id: victim,
        mid_pass: fact.kind === 'PassIntercepted',
        strip: fact.kind === 'HolderStripped',
      });
      s = emit(s, 'TURNOVER', [victim], {
        player_id: victim,
        team: otherTeam(stealer.team),
        stealer_id: fact.stealerId,
        turnover_type: 'steal',
      });
      b = ballHeld(stealer, stealer.team);
      s = withPossession(s, stealer.team);
      s = withShotClock(s, shotClockAfter('steal'));
      s = { ...s, phase: 'LIVE' };
      episode = endEpisode('TURNOVER', 'STEAL');
      clearBallIntent = true;
      forceDecision = true;
      return finish();
    }

    case 'IntentShotReleased': {
      if (b.status !== 'held' || !b.holderId) return emptyResult(s, b);
      const shooter = s.poses[fact.shooterId];
      if (!shooter) return emptyResult(s, b);
      const offense = s.possession.team;
      if (!offense) return emptyResult(s, b);
      const right = offenseAttacks(offense, s.baskets) === 'right';
      const rim = rimNorm(right ? 'right' : 'left');
      // Flight time by distance (real ballistics: 3PT ≈ 1.35s, midrange
      // ≈ 0.7s, rim ≈ 0.4s) instead of a distance-blind random draw that
      // made every shot a 0.55–1.05s laser.
      const dFt = Math.hypot((rim.x - shooter.x) * 94, (rim.y - shooter.y) * 50);
      const flight = Math.min(
        MOBILITY.ball.shot_flight_max,
        Math.max(MOBILITY.ball.shot_flight_min, dFt / (MOBILITY.ball.shot_flight_speed * 94)),
      );
      b = startShot({
        from: shooter,
        rim,
        shotValue: fact.shotValue,
        team: offense,
        flightDuration: flight,
        zone: fact.zone,
        shotType: fact.shotType ?? (fact.assisterId ? 'catch_shoot' : 'pull_up'),
        assisterId: fact.assisterId,
        realNow: s.realClock,
      });
      const def = otherTeam(offense);
      // Shooting foul decided at RELEASE — the whistle sounds at contact,
      // not when the ball lands. The old arrival-time whistle left the
      // foul event 1.4s late (shooter had already relocated) and pinned
      // the "fouler" far from the shot. The and-one/FT count is settled
      // at ShotArrivedAtRim, which re-derives this foul from the stream.
      // M2: position gate — a whistle requires a defender within contact
      // reach of the shooter at release. Without this, an open 3 with the
      // nearest defender 20ft away could still draw a shooting foul
      // (measured: foul_contact_dist over-6ft on 32/53 fouls in seed 42).
      const releaseDefenderDist = findNearestDefenderDist(s.poses, offense, shooter.x, shooter.y);
      const shootingFoul = releaseDefenderDist <= CONTACT_FOUL_RADIUS_FT
        && rng.next() < RESOLVE.baseRates.foul_on_drive_rate * 0.55;
      if (shootingFoul) {
        const offenderId = findNearestDefenderId(s.poses, offense, shooter.x, shooter.y)
          ?? s.lineups[def][0]
          ?? fact.shooterId;
        s = emit(s, 'FOUL', [offenderId, fact.shooterId], {
          offender_id: offenderId,
          offender_team: def,
          victim_id: fact.shooterId,
          foul_type: 'shooting',
          shooting: true,
          // Provisional: 2/3 for the miss case; a made shot awards 1
          // (and-one) at arrival.
          free_throws_awarded: fact.shotValue === 3 ? 3 : 2,
        });
      }
      const payload: Record<string, unknown> = {
        shooter_id: fact.shooterId,
        shot_value: fact.shotValue,
        zone: fact.zone,
        x: fact.x,
        y: fact.y,
        target_x: rim.x,
        target_y: rim.y,
        flight_seconds: flight,
      };
      if (fact.assisterId) payload['assister_id'] = fact.assisterId;
      s = emit(s, 'SHOT_RELEASE', [fact.shooterId, ...(fact.assisterId ? [fact.assisterId] : [])], payload);
      clearBallIntent = true;
      return finish();
    }

    case 'ShotArrivedAtRim': {
      // Spatial resolve: nearest defender distance + zone from fact.
      // Contest and foul attribution are measured from the RELEASE point
      // (b.fromX/fromY), not the rim: a corner-3 shooter's defender stands
      // 25ft from the rim, so rim-distance made every perimeter shot "open"
      // and pinned shooting fouls on help defenders under the basket.
      const offense = s.possession.team ?? 'home';
      const def = otherTeam(offense);
      const releaseX = b.fromX ?? fact.x;
      const releaseY = b.fromY ?? fact.y;
      const nearestDist = findNearestDefenderDist(s.poses, offense, releaseX, releaseY);
      const shooterAbilities = s.abilities[fact.shooterId] ?? null;
      const nearestDefenderId = findNearestDefenderId(s.poses, offense, releaseX, releaseY);
      const defenderData = nearestDefenderId ? (s.playerData[nearestDefenderId] ?? null) : null;
      const shot = resolveShot(rng, RESOLVE, {
        shotValue: fact.shotValue,
        zone: fact.zone,
        shotType: (fact.shotType ?? b.shotType ?? 'other') as 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other',
        defenderDistFt: nearestDist,
        playerModifier: shotAbilityModifier(shooterAbilities, fact.shotValue, fact.zone),
        staminaModifier: staminaFactorOf(fact.shooterId),
        defenderWsCm: defenderData ? defenderData.physical.WS : undefined,
        defenderVjCm: defenderData ? defenderData.physical.VJ : undefined,
        defenderWtKg: defenderData ? defenderData.physical.WT : undefined,
      });
      const fts = (fact.shotValue === 3 ? 3 : 2) as 2 | 3;
      // A shooting foul whistled at release (IntentShotReleased) is
      // re-derived here: the last un-consumed shooting FOUL on this
      // shooter (no SHOT_RESULT for them since the whistle).
      const pendingFoul = pendingShotFoulFromEvents(s, fact.shooterId);
      const shotPayload = (blocked: boolean): Record<string, unknown> => {
        const payload: Record<string, unknown> = {
          shooter_id: fact.shooterId,
          shot_value: fact.shotValue,
          made: shot.made,
          zone: fact.zone,
          x: fact.x,
          y: fact.y,
          blocked,
        };
        if (fact.assisterId) payload['assister_id'] = fact.assisterId;
        return payload;
      };
      if (shot.blocked) {
        // Blocked shot → loose ball; no made basket. A foul whistled at
        // release still stands (2/3 FTs), otherwise it is a clean block.
        if (pendingFoul) {
          s = emit(s, 'SHOT_RESULT', [fact.shooterId, ...(fact.assisterId ? [fact.assisterId] : [])], shotPayload(true));
          pendingFt = {
            attempts: fts,
            shooterId: fact.shooterId,
            team: offense,
            andOne: false,
          };
          s = { ...s, phase: 'DEAD_FOUL' };
          episode = endEpisode('SHOOTING_FOUL', 'AFTER_INBOUND');
          clearBallIntent = true;
          return finish();
        }
        s = emit(s, 'SHOT_RESULT', [fact.shooterId, ...(fact.assisterId ? [fact.assisterId] : [])], shotPayload(true));
        b = ballLoose({ x: fact.x, y: fact.y });
        s = withPossession(s, def);
        s = withShotClock(s, shotClockAfter('turnover'));
        episode = endEpisode('TURNOVER', 'LIVE_BALL_TURNOVER_RECOVER');
        clearBallIntent = true;
        forceDecision = true;
        return finish();
      }

      if (pendingFoul) {
        // Whistle already sounded at release; settle the FT award now:
        // made → and-one (1 FT), miss → 2/3 FTs.
        s = emit(s, 'SHOT_RESULT', [fact.shooterId, ...(fact.assisterId ? [fact.assisterId] : [])], shotPayload(false));
        if (shot.made) {
          s = emit(s, 'MADE_BASKET_DEAD', [], { scoring_team: offense });
        }
        pendingFt = {
          attempts: shot.made ? 1 : fts,
          shooterId: fact.shooterId,
          team: offense,
          andOne: shot.made,
        };
        s = { ...s, phase: 'DEAD_FOUL' };
        episode = endEpisode(shot.made ? 'AND_ONE' : 'SHOOTING_FOUL', shot.made ? 'AFTER_MAKE' : 'AFTER_INBOUND');
        clearBallIntent = true;
        return finish();
      }

      s = emit(s, 'SHOT_RESULT', [fact.shooterId, ...(fact.assisterId ? [fact.assisterId] : [])], shotPayload(false));
      if (shot.made) {
        s = emit(s, 'MADE_BASKET_DEAD', [], { scoring_team: offense });
        s = { ...s, phase: 'DEAD_MAKE' };
        episode = endEpisode('MAKE', 'AFTER_MAKE');
        clearBallIntent = true;
        return finish();
      }

      // The rebound resolve selects the live contest's eligible recovery team;
      // the pose step still selects the actual player within that team.
      // P1.5 positional model: paint presence + box-out state + shot type.
      const rimForCount = rimNorm(offenseAttacks(offense, s.baskets) === 'right' ? 'right' : 'left');
      let offensePaintCount = 0;
      let defensePaintCount = 0;
      let offenseBoxOuts = 0;
      for (const pose of Object.values(s.poses)) {
        const dRim = Math.hypot((pose.x - rimForCount.x) * 94, (pose.y - rimForCount.y) * 50);
        if (dRim > 12) continue;
        if (pose.team === offense) {
          offensePaintCount += 1;
          if (pose.action === 'box_out') offenseBoxOuts += 1;
        } else {
          defensePaintCount += 1;
        }
      }
      const reboundOffense = resolveRebound(rng, RESOLVE, {
        shotType: (b.shotType ?? 'other') as 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other',
        offensePaintCount,
        defensePaintCount,
        offenseBoxOuts,
      }).offensive;
      // NBA shot-clock rule: a missed shot that touches the rim resets the
      // clock to 14 when it shows less than 14. Without this, a legal
      // release with ≤2s left bounced at the rim and the loose-ball window
      // (arrival → recovery) expired into a violation.
      s = withShotClock(s, s.clocks.shot < 14 ? 14 : s.clocks.shot);
      b = ballLoose({ x: fact.x, y: fact.y }, offense, reboundOffense ? offense : def);
      clearBallIntent = true;
      return finish();
    }

    case 'IntentDriveBegun': {
      s = emit(s, 'DRIVE', [fact.ballHandlerId], {
        ballHandlerId: fact.ballHandlerId,
        target_zone: 'paint',
      });
      // A drive may use a previously established physical screen. SCREEN_SET
      // and SCREEN_USE are emitted only by their completion facts.
      return finish();
    }

    case 'PlayerArrivedInPaint': {
      // AttackRim sub-state machine: must terminate
      const handler = fact.ballHandlerId;
      const pose = s.poses[handler];
      if (!pose || b.status !== 'held') return emptyResult(s, b);
      const offense = fact.offense;
      const nearestDefenderDist = findNearestDefenderDist(s.poses, offense, pose.x, pose.y);
      const pressurePenalty = Math.max(0, Math.min(0.35, (6 - nearestDefenderDist) / 6 * 0.35));
      const handlerAbilities = s.abilities[handler] ?? null;
      const defId = findNearestDefenderId(s.poses, offense, pose.x, pose.y);
      const defAbilities = defId ? (s.abilities[defId] ?? null) : null;
      // Better ball security finishes through pressure; elite on-ball
      // defenders stop more drives.
      const handlerMod = 1 + ((handlerAbilities?.handleSecurity ?? 0.5) - 0.5) * 0.5;
      const defenderMod = 1 - ((defAbilities?.onBallDefense ?? 0.5) - 0.5) * 0.4;
      // P1.8: the defender's FOUL tendency modulates the drive-foul rate —
      // a 0.9 FOUL enforcer fouls drives more than a 0.2 disciplined
      // stopper. The resolveDrive foul draw is already rolled; scale the
      // pressure penalty by tendency so the whistle rate tracks identity.
      const defData = defId ? (s.playerData[defId] ?? null) : null;
      const foulTendency = defData ? defData.tendency.FOUL / 99 : 0.5;
      const tendencyPressure = Math.max(0, Math.min(0.35, pressurePenalty * (0.6 + foulTendency)));
      let d = resolveDrive(rng, RESOLVE, {
        pressurePenalty: tendencyPressure,
        handlerModifier: handlerMod * staminaFactorOf(handler),
        defenderModifier: defenderMod,
        handlerHandle: handlerAbilities?.handleSecurity,
      });
      // Contact geometry: a shooting foul requires the defender within
      // reach. A chasedown from 18ft is a clean finish, not a whistle —
      // the drive resolves on its own success flag instead.
      if (d.fouled && nearestDefenderDist > CONTACT_FOUL_RADIUS_FT) {
        d = { success: d.success, fouled: false, lostBall: d.lostBall };
      }
      const def = otherTeam(offense);
      const onBallDef = defId ?? s.lineups[def][0] ?? handler;

      if (d.fouled) {
        if (d.success) {
          // And-one layup path: release + make + foul
          const right = offenseAttacks(offense, s.baskets) === 'right';
          const rim = rimNorm(right ? 'right' : 'left');
          b = startShot({
            from: pose,
            rim,
            shotValue: 2,
            team: offense,
            flightDuration: 0.45,
            zone: 'rim',
            assisterId: recentAssister(s, handler),
            realNow: s.realClock,
          });
          const assister = recentAssister(s, handler);
          // Whistle first — the foul is decided by the drive resolve, before
          // the release. Previous order (release → result → make → foul)
          // narrated every and-one as "basket, then foul".
          s = emit(s, 'FOUL', [onBallDef, handler], {
            offender_id: onBallDef,
            offender_team: def,
            victim_id: handler,
            foul_type: 'shooting',
            shooting: true,
            free_throws_awarded: 1,
          });
          s = emit(s, 'SHOT_RELEASE', [handler, ...(assister ? [assister] : [])], {
            shooter_id: handler,
            shot_value: 2,
            zone: 'rim',
            ...(assister ? { assister_id: assister } : {}),
          });
          // Immediate finish for short layup flight — resolve now (single path)
          const made = true; // fouled successful drive → and-one make
          s = emit(s, 'SHOT_RESULT', [handler, ...(assister ? [assister] : [])], {
            shooter_id: handler,
            shot_value: 2,
            made,
            zone: 'rim',
            ...(assister ? { assister_id: assister } : {}),
          });
          s = emit(s, 'MADE_BASKET_DEAD', [], { scoring_team: offense });
          pendingFt = { attempts: 1, shooterId: handler, team: offense, andOne: true };
          s = { ...s, phase: 'DEAD_FOUL' };
          b = ballLoose({ x: pose.x, y: pose.y });
          episode = endEpisode('AND_ONE', 'AFTER_MAKE');
          clearBallIntent = true;
          return finish();
        }
        // Shooting foul, miss path → 2 FT
        s = emit(s, 'FOUL', [onBallDef, handler], {
          offender_id: onBallDef,
          offender_team: def,
          victim_id: handler,
          foul_type: 'shooting',
          shooting: true,
          free_throws_awarded: 2,
        });
        pendingFt = { attempts: 2, shooterId: handler, team: offense, andOne: false };
        s = { ...s, phase: 'DEAD_FOUL' };
        episode = endEpisode('SHOOTING_FOUL', 'AFTER_INBOUND');
        clearBallIntent = true;
        return finish();
      }

      if (!d.success) {
        // A failed drive is mostly a contested finish (the shot resolves
        // normally at the rim), not a turnover. Real NBA: drive outcomes
        // are ~50% make / ~40% miss / ~7% turnover. P1.7: the lost-ball
        // probability is now inside resolveDrive, scaled by handleSecurity
        // vs the defender's pressure — the old fixed 30% is gone.
        if (d.lostBall) {
          s = emit(s, 'TURNOVER', [handler], {
            player_id: handler,
            team: offense,
            turnover_type: 'drive',
          });
          b = ballLoose({ x: pose.x, y: pose.y }, null, def);
          s = withPossession(s, def);
          s = withShotClock(s, shotClockAfter('turnover'));
          episode = endEpisode('TURNOVER', 'LIVE_BALL_TURNOVER_RECOVER');
          clearBallIntent = true;
          forceDecision = true;
          return finish();
        }
        // Bail-out read before the contested finish: a failed drive with a
        // wide-open teammate (>=8ft) is a reset swing in the NBA, not a forced
        // floater. Without this, 28% of drives (the failed share) all became
        // 8-14ft contested floaters (measured floater share 34% vs NBA ~12%).
        const bailMate = (() => {
          let best: string | null = null;
          let bestGap = 8;
          for (const mate of Object.values(s.poses)) {
            if (mate.team !== offense || mate.jersey === handler) continue;
            let gap = Infinity;
            for (const d2 of Object.values(s.poses)) {
              if (d2.team === offense) continue;
              const dd = Math.hypot((d2.x - mate.x) * 94, (d2.y - mate.y) * 50);
              if (dd < gap) gap = dd;
            }
            if (gap > bestGap) {
              bestGap = gap;
              best = mate.jersey;
            }
          }
          return best ? { jersey: best, gap: bestGap } : null;
        })();
        if (bailMate && rng.next() < 0.72) {
          const matePose = s.poses[bailMate.jersey]!;
          b = startPass({ from: pose, to: matePose, team: offense, passerId: handler, passType: 'drive_kick' });
          s = emit(s, 'PASS', [handler, bailMate.jersey], {
            passer_id: handler,
            receiver_id: bailMate.jersey,
            note: 'flight_start',
            pass_type: 'bail_out',
          });
          s = {
            ...s,
            poses: {
              ...s.poses,
              [bailMate.jersey]: setTarget(matePose, matePose.x, matePose.y, 'pass_receive'),
            },
          };
          clearBallIntent = true;
          forceDecision = true;
          return finish();
        }
        // Contested finish: the drive failed but the shot still goes up.
        // P3.3 note: the release zone is the HOLDER'S position at the
        // stall, not the rim — a 18ft drive failure is a contested
        // pull-up/mid shot, not a rim attempt. Only a paint arrival is
        // a real rim finish.
        const right = offenseAttacks(offense, s.baskets) === 'right';
        const rim = rimNorm(right ? 'right' : 'left');
        const dFt = Math.hypot((rim.x - pose.x) * 94, (rim.y - pose.y) * 50);
        const flight = Math.min(
          MOBILITY.ball.shot_flight_max,
          Math.max(MOBILITY.ball.shot_flight_min, dFt / (MOBILITY.ball.shot_flight_speed * 94)),
        );
        const stallZone = zoneFromPoint(pose.x, pose.y, offense, s.baskets);
        const stallValue = shotValueAt(pose.x, pose.y, offense, s.baskets);
        b = startShot({
          from: pose,
          rim,
          shotValue: stallValue,
          team: offense,
          flightDuration: flight,
          zone: stallZone,
          shotType: 'drive_finish',
          assisterId: recentAssister(s, handler),
          realNow: s.realClock,
        });
        s = emit(s, 'SHOT_RELEASE', [handler, ...(recentAssister(s, handler) ? [recentAssister(s, handler)!] : [])], {
          shooter_id: handler,
          shot_value: stallValue,
          zone: stallZone,
          x: pose.x,
          y: pose.y,
          target_x: rim.x,
          target_y: rim.y,
          flight_seconds: flight,
          ...(recentAssister(s, handler) ? { assister_id: recentAssister(s, handler) } : {}),
        });
        clearBallIntent = true;
        return finish();
      }

      // Successful drive without foul → DRIVE-KICK sub-decision:
      // if a teammate is genuinely open on the kick-out, dish it (the
      // collapsed defense leaves the perimeter); otherwise finish at the
      // rim. Real NBA: drive penetration creates the kick-out; forcing
      // every successful drive into a rim attempt inflates rimShare and
      // starves the corner three.
      const openMate = s.poses[handler] ? (() => {
        const rim0 = rimNorm(offenseAttacks(offense, s.baskets));
        let best: { jersey: string; dist: number } | null = null;
        for (const mate of Object.values(s.poses)) {
          if (mate.team !== offense || mate.jersey === handler) continue;
          // A kick-out target must be a REAL shooting threat: beyond the
          // arc (the kick-out three) or at the rim area (a dump). A mate
          // standing mid-range with his defender sagging (the weak-side
          // rim anchor pulls defenders 7+ft off their men by design) is
          // not a kick target — kicking to him produced infinite
          // drive_kick chains (6+ one-touch passes, shot-clock death).
          const mateRim = Math.hypot((mate.x - rim0.x) * 94, (mate.y - rim0.y) * 50);
          if (mateRim < 21 && mateRim > 8) continue;
          let closestDef = Infinity;
          for (const d of Object.values(s.poses)) {
            if (d.team === offense) continue;
            const dd = Math.hypot((d.x - mate.x) * 94, (d.y - mate.y) * 50);
            if (dd < closestDef) closestDef = dd;
          }
          if (closestDef >= 6 && (best === null || closestDef > best.dist)) {
            best = { jersey: mate.jersey, dist: closestDef };
          }
        }
        return best;
      })() : null;
      // Finish bias: a driver who has beaten the first line (paint arrival)
      // finishes more often than he kicks — NBA paint-arrival finishes run
      // ~55-65% even against late help. Kick only when the wall is real
      // (2+ defenders within 6ft of the driver) AND a genuine target exists.
      let paintWall = 0;
      for (const d of Object.values(s.poses)) {
        if (d.team === offense) continue;
        if (Math.hypot((d.x - pose.x) * 94, (d.y - pose.y) * 50) <= 6) paintWall += 1;
      }
      // Kick when the wall is real (2+ bodies) OR the finish would be heavily
      // contested by a rim protector (nearest defender <2.5ft) — forcing that
      // finish produced 42% of walled drives into bad rim attempts (rim share
      // 68% vs NBA 33%, measured).
      const finishContest = (() => {
        let best = Infinity;
        for (const d of Object.values(s.poses)) {
          if (d.team === offense) continue;
          const dd = Math.hypot((d.x - pose.x) * 94, (d.y - pose.y) * 50);
          if (dd < best) best = dd;
        }
        return best;
      })();
      const shouldKick = openMate !== null && (paintWall >= 2 || finishContest < 2.5);
      if (openMate && shouldKick) {
        const mate = s.poses[openMate.jersey]!;
        b = startPass({ from: pose, to: mate, team: offense, passerId: handler, passType: 'drive_kick' });
        s = emit(s, 'PASS', [handler, openMate.jersey], {
          passer_id: handler,
          receiver_id: openMate.jersey,
          note: 'flight_start',
          pass_type: 'drive_kick',
        });
        s = {
          ...s,
          poses: {
            ...s.poses,
            [openMate.jersey]: setTarget(mate, mate.x, mate.y, 'pass_receive'),
          },
        };
        clearBallIntent = true;
        forceDecision = true;
        return finish();
      }
      // No open kick-out → forced layup/short shot (termination).
      // Rim advance: a beaten drive finishes AT the rim (layups release
      // 2-3ft out), not from the 8ft paint-arrival trigger. Resolving from
      // the stall position released every drive as an 8-14ft floater
      // (measured: 0% of drives ever produced a <5ft attempt).
      const right = offenseAttacks(offense, s.baskets) === 'right';
      const rim = rimNorm(right ? 'right' : 'left');
      const driveVecLen = Math.hypot((rim.x - pose.x) * 94, (rim.y - pose.y) * 50) || 1;
      const releaseX = pose.x + ((rim.x - pose.x) * 94 / driveVecLen) * 3 / 94;
      const releaseY = pose.y + ((rim.y - pose.y) * 50 / driveVecLen) * 3 / 50;
      const finishPose = { ...pose, x: releaseX, y: releaseY };
      const priorPose = s.poses[handler];
      if (priorPose) {
        s = {
          ...s,
          poses: { ...s.poses, [handler]: setTarget(priorPose, releaseX, releaseY, priorPose.action, priorPose.movementRole) },
        };
      }
      const dFt = Math.hypot((rim.x - finishPose.x) * 94, (rim.y - finishPose.y) * 50);
      const flight = Math.min(
        MOBILITY.ball.shot_flight_max,
        Math.max(MOBILITY.ball.shot_flight_min, dFt / (MOBILITY.ball.shot_flight_speed * 94)),
      );
      const shotZone = zoneFromPoint(finishPose.x, finishPose.y, offense, s.baskets);
      const shotValue = shotValueAt(finishPose.x, finishPose.y, offense, s.baskets);
      // The BALL releases from the advanced finish point (a layup leaves the
      // hand 2-3ft from the rim), while the BODY animates there via the pose
      // target over the next 1-2 frames — both the shot truth AND the
      // animation are legal (measured: release-from-stall priced every
      // finish as a floater; snap-to-rim teleported).
      b = startShot({
        from: finishPose,
        rim,
        shotValue,
        team: offense,
        flightDuration: flight,
        zone: shotZone,
        assisterId: recentAssister(s, handler),
        realNow: s.realClock,
      });
      s = emit(s, 'SHOT_RELEASE', [handler, ...(recentAssister(s, handler) ? [recentAssister(s, handler)!] : [])], {
        shooter_id: handler,
        shot_value: shotValue,
        zone: shotZone,
        x: finishPose.x,
        y: finishPose.y,
        target_x: rim.x,
        target_y: rim.y,
        flight_seconds: flight,
        ...(recentAssister(s, handler) ? { assister_id: recentAssister(s, handler) } : {}),
      });
      clearBallIntent = true;
      return finish();
    }

    case 'IntentAdvance': {
      s = emit(s, 'ADVANCE_BACKCOURT', [fact.ballHandlerId], {
        ballHandlerId: fact.ballHandlerId,
      });
      // Do NOT clear sticky ball intent for advance — handler must keep
      // advancing between decision cycles. Without this, the handler
      // arrives at the first retarget target and freezes until the next
      // decision cycle (2.5+ seconds later).
      // CROSS_HALF is emitted by the position-fact guard in stepSimulationTick
      // (the holder crosses mid-court exactly once per possession, independent
      // of which tick the advance decision lands on). Do not emit it here —
      // dual emission sources race and confuse the audit layer.
      return finish();
    }

    case 'PlayerCrossedHalf': {
      s = emit(s, 'CROSS_HALF', [fact.ballHandlerId], {
        ballHandlerId: fact.ballHandlerId,
      });
      s = {
        ...s,
        ball: { ...s.ball, zone: 'frontcourt_center' },
      };
      return finish();
    }

    case 'ScreenSet': {
      s = emit(s, 'SCREEN_SET', [fact.screenerId, fact.ballHandlerId], {
        screener_id: fact.screenerId,
        ballHandlerId: fact.ballHandlerId,
        separation_bonus: fact.separationBonus,
        defender_id: fact.defenderId,
        anchor_x: fact.anchorX,
        anchor_y: fact.anchorY,
      });
      // Setting a screen creates an execution state; it is not yet screen use.
      forceDecision = true;
      return finish();
    }

    case 'ScreenUsed': {
      s = emit(s, 'SCREEN_USE', [fact.screenerId, fact.ballHandlerId], {
        screener_id: fact.screenerId,
        ballHandlerId: fact.ballHandlerId,
      });
      forceDecision = true;
      return finish();
    }

    case 'CosmeticScreen': {
      s = emit(s, 'SCREEN_SET', [fact.screenerId, fact.ballHandlerId], {
        screener_id: fact.screenerId,
        ballHandlerId: fact.ballHandlerId,
      });
      s = emit(s, 'SCREEN_USE', [fact.screenerId, fact.ballHandlerId], {
        screener_id: fact.screenerId,
        ballHandlerId: fact.ballHandlerId,
      });
      return finish();
    }

    case 'LooseRecovered': {
      const rec = s.poses[fact.recovererId];
      const reboundOffense = b.reboundOffenseTeam ?? null;
      const isRebound = reboundOffense !== null;
      const offensive = isRebound && fact.team === reboundOffense;
      const prevTeam = state.possession.team;

      if (isRebound) {
        s = emit(s, 'REBOUND', [fact.recovererId], {
          rebounder_id: fact.recovererId,
          team: fact.team,
          offensive,
        });
      }
      s = emit(s, 'LOOSE_BALL_RECOVER', [fact.recovererId], {
        recoverer_id: fact.recovererId,
        team: fact.team,
      });
      if (rec) b = ballHeld(rec, fact.team);
      s = withPossession(s, fact.team);

      if (isRebound) {
        // OREB: reset to 14 only when the clock has dropped below 14 —
        // a quick offensive rebound with 18s showing keeps 18 (NBA rule).
        s = withShotClock(s, offensive
          ? Math.max(shotClockAfter('offensive_rebound'), s.clocks.shot)
          : shotClockAfter('defensive_rebound'));
        episode = offensive
          ? { kind: 'continue_oreb' }
          : endEpisode('MISS_DREB', 'AFTER_DEFENSIVE_REBOUND');
        forceDecision = true;
        clearBallIntent = true;
      } else if (prevTeam !== fact.team) {
        s = withShotClock(s, shotClockAfter('turnover'));
        forceDecision = true;
        clearBallIntent = true;
      }
      return finish();
    }

    case 'ShotClockExpired': {
      // A foul whistled at release stops play: the shot continues (and-one
      // try) and the clock is irrelevant. Only an unfouled in-flight ball
      // can be killed by the horn.
      if (b.status === 'shot' && b.shooterId && pendingShotFoulFromEvents(s, b.shooterId)) {
        return emptyResult(s, b);
      }
      s = emit(s, 'SHOT_CLOCK_VIOLATION', [], { team: fact.team });
      s = { ...s, phase: 'DEAD_VIOLATION' };
      // A violation legally truncates any in-flight pass: the ball is dead
      // the instant the horn sounds. Leaving the pass-flight state alive
      // leaks a pending pass across the dead-ball boundary.
      b = ballDead({ x: b.x, y: b.y });
      episode = endEpisode('SHOT_CLOCK_VIOLATION', 'AFTER_INBOUND');
      clearBallIntent = true;
      return finish();
    }

    case 'GameClockExpired': {
      s = emit(s, 'CLOCK_EXPIRY_ADJUDICATION', [], {
        clock: 'game',
        residual_seconds: 0,
      });
      episode = endEpisode('PERIOD_END', 'PERIOD_END');
      clearBallIntent = true;
      return finish();
    }

    default:
      return emptyResult(s, b);
  }
}

/**
 * Convert a sticky/live ball intent into the initial completion fact(s)
 * that start physical processes (pass flight, shot flight, drive, …).
 * Does not resolve outcomes for flights — only starts them.
 */
function recentAssister(state: GameState, shooterId: string): string | null {
  for (let i = state.events.length - 1; i >= 0; i -= 1) {
    const event = state.events[i]!;
    if (event.type === 'PASS' && event.payload['note'] === 'flight_complete') {
      const receiver = event.payload['receiver_id'];
      const passer = event.payload['passer_id'];
      if (receiver === shooterId && typeof passer === 'string' && passer !== shooterId) return passer;
      if (receiver !== shooterId) return null;
    }
    if (event.type === 'POSSESSION_GAINED' || event.type === 'REBOUND' || event.type === 'STEAL' || event.type === 'TURNOVER') return null;
  }
  return null;
}
export function factsFromBallIntent(
  state: GameState,
  ballIntent: Intent,
  intents: readonly Intent[],
): CompletionFact[] {
  if (state.phase !== 'LIVE') return [];
  if (state.ballMotion.status !== 'held') return [];
  const holder = state.ballMotion.holderId;
  if (!holder || ballIntent.jersey !== holder) return [];
  const offense = state.possession.team;
  if (!offense || ballIntent.team !== offense) return [];

  switch (ballIntent.kind) {
    case 'pass': {
      const to =
        ballIntent.targetJersey ??
        state.lineups[offense].find((j) => j !== holder) ??
        holder;
      return [
        {
          kind: 'IntentPassStarted',
          passerId: holder,
          receiverId: to,
          team: offense,
        },
      ];
    }
    case 'handoff': {
      const to = ballIntent.targetJersey ?? state.lineups[offense].find((j) => j !== holder) ?? holder;
      return [{ kind: 'HandoffExchange', giverId: holder, receiverId: to, team: offense }];
    }
    case 'shoot': {
      const pose = state.poses[holder];
      const sx = pose?.x ?? 0.5;
      const sy = pose?.y ?? 0.5;
      const zone = zoneFromPoint(sx, sy, offense, state.baskets);
      const shotValue = shotValueAt(sx, sy, offense, state.baskets);
      // Shot-type classification: a shot from inside 4ft is a layup/dunk
      // (drive_finish), not a pull-up — NBA rim FG% of ~68% comes from
      // the drive_finish base rate (0.58) at the rim, not the pull_up
      // rate (0.40). Without this classification a roller catching at
      // 1ft and shooting immediately was priced as a pull-up → 34% FG
      // instead of the correct ~65-70%.
      const attackSide = offense === 'home' ? state.baskets.away : state.baskets.home;
      const rimPos = rimNorm(attackSide);
      const rimDist = Math.hypot((sx - rimPos.x) * 94, (sy - rimPos.y) * 50);
      const defaultShotType = rimDist <= 4
        ? 'drive_finish' as const
        : recentAssister(state, holder) ? 'catch_shoot' as const : 'pull_up' as const;
      return [{
        kind: 'IntentShotReleased',
        shooterId: holder,
        shotValue,
        zone,
        x: sx,
        y: sy,
        assisterId: recentAssister(state, holder),
        shotType: ballIntent.shotType ?? defaultShotType,
      }];
    }
    case 'drive': {
      const screener = intents.find((i) => i.kind === 'screen' && i.team === offense)?.jersey ?? null;
      return [{ kind: 'IntentDriveBegun', ballHandlerId: holder, screenerId: screener }];
    }
    case 'advance': {
      return [{ kind: 'IntentAdvance', ballHandlerId: holder }];
    }
    default:
      return [];
  }
}
