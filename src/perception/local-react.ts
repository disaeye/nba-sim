/**
 * Per-tick local perception micro-adjustments.
 *
 * This runs BETWEEN planSpatialTargets (which sets coarse targets every
 * decision tick) and stepAllPoses (which physically moves players). It
 * gives every player a small amount of real-time reactivity that makes
 * them look alive instead of like pieces sliding to fixed board positions.
 *
 * Each function returns adjusted poses — it never mutates input.
 */
import type { GameState, TeamId } from '../state/types.js';
import type { PoseMap, PoseState } from '../court/poses.js';
import { setTarget } from '../court/poses.js';
import { distFeet } from '../court/geometry.js';
import { perceiveLiveCourt } from './live-court.js';

function clamp01(n: number): number {
  return n < 0 ? 0 : n > 1 ? 1 : n;
}

/**
 * On-ball defender tracks the live holder position every tick, not just
 * when the decision layer replans. This produces continuous mirroring
 * instead of 2.8s stale targets.
 */
function adjustOnBallDefender(poses: PoseMap, state: GameState): PoseMap {
  if (state.ballMotion.status !== 'held' || !state.ballMotion.holderId) return poses;
  const offense = state.possession.team;
  if (!offense) return poses;
  const defense: TeamId = offense === 'home' ? 'away' : 'home';
  const holder = poses[state.ballMotion.holderId];
  if (!holder) return poses;

  // The attack direction: rim side the offense is going toward.
  const attackSide = offense === 'home' ? state.baskets.away : state.baskets.home;
  const dir = attackSide === 'right' ? 1 : -1;

  // Find the on-ball defender — the one whose action is on_ball_defend,
  // or the closest defender if no one is assigned.
  let defender: PoseState | null = null;
  let closest = Infinity;
  for (const p of Object.values(poses)) {
    if (p.team !== defense) continue;
    if (p.action === 'on_ball_defend') {
      defender = p;
      break;
    }
    const d = distFeet(p.x, p.y, holder.x, holder.y);
    if (d < closest) {
      closest = d;
      defender = p;
    }
  }
  if (!defender) return poses;

  // Position the defender between the holder and the rim, 4ft from holder.
  // This is a BLEND with the existing target — not a replacement.
  const rimX = attackSide === 'right' ? 1 : 0;
  const rimY = 0.5;
  const hx = holder.x;
  const hy = holder.y;
  // The gap anchor direction: from the DEFENDER's current position toward
  // the holder, extended to the gap distance. The old holder→rim ray
  // placed the anchor ON THE FAR SIDE of the holder — a retreating
  // defender sprinting to that point had to run THROUGH the holder, and
  // body contact parked them at 2.2ft (the rubber-band look: 115+
  // constant-gap tracks/game). Placing the anchor on the defender's own
  // side of the holder keeps the pursuit natural: they close to the gap
  // and stop, without path-crossing.
  const dx = hx - defender.x;
  const dy = hy - defender.y;
  const dl = Math.hypot(dx, dy) || 1;
  // Direction from defender → holder (the approach direction).
  const ux = dx / dl;
  const uy = dy / dl;
  // A committed driver HAS a step: when the holder's action is 'drive'
  // and they are attacking the rim, the on-ball defender is BEATEN — the
  // tracking anchor must not glue them back onto the ball line (the old
  // blend held a 3.5ft gap for the entire drive; drives stalled at 13-15ft
  // and none ever reached the rim). A beaten defender recovers from
  // BEHIND at full sprint: widen the gap anchor and soften the blend so
  // separation gained is separation kept until the defender physically
  // re-closes over ~1s.
  const driverBeaten = holder.action === 'drive';
  const wobble = Math.sin(state.realClock * 2.3 + holder.jersey.charCodeAt(0)) * 0.8;
  const dynamicGap = driverBeaten
    ? Math.max(Math.hypot((defender.x - hx) * 94, (defender.y - hy) * 50), 6.5)
    : Math.max(2.8, Math.min(5.2, 4 + wobble));
  const gapX = hx - ux * (dynamicGap / 94);
  const gapY = hy - uy * (dynamicGap / 50);

  // Adaptive blend: when the defender is FAR from the 4ft anchor (e.g.
  // sprinting back in transition, 10ft+ away), blend hard so they
  // actually reach the anchor; when they are CLOSE (already at 2-4ft),
  // blend softly so the gap breathes instead of freezing. The old flat
  // 25-40% blend turned the anchor into a moving target the defender
  // never caught (transition on-ball gaps froze at 2.1-2.3ft while the
  // plan said 4ft — the "magnet" rubber-band look).
  const currentGap = Math.hypot((defender.x - hx) * 94, (defender.y - hy) * 50);
  const blend = currentGap > 8 ? 0.6 : currentGap > 5 ? 0.4 : 0.2;
  const blendX = defender.targetX * (1 - blend) + gapX * blend;
  const blendY = defender.targetY * (1 - blend) + gapY * blend;

  return {
    ...poses,
    [defender.jersey]: setTarget(defender, clamp01(blendX), clamp01(blendY), 'on_ball_defend'),
  };
}

/**
 * Weak-side help defender watches the paint. If the ball handler is driving
 * into the paint (close to rim), the nearest off-ball defender shades toward
 * the rim to provide help. When the ball goes back outside, they recover.
 */
function adjustHelpDefenders(poses: PoseMap, state: GameState): PoseMap {
  const sense = perceiveLiveCourt(state);
  if (!sense) return poses;
  if (state.ballMotion.status !== 'held') return poses;
  // Transition is not a help-defense situation: the defense is RETREATING
  // (running back to match up), not rotating to protect the paint. The
  // audit found 200+ frames/game of defenders labeled tag/help while
  // sprinting back 25-40ft from the rim in TRANSITION_PUSH — a retreating
  // defender is a weak_side/recovery assignment, not a helper.
  if (state.possession?.team && state.possession.team === sense.offense
    && (state.phase === 'LIVE')) {
    // mode is not on state; detect transition by the holder's court
    // position: if the handler is still in the backcourt or the
    // possession just started, the defense is recovering. The tactical
    // stage is not authoritative here (local-react runs between plans),
    // so use the spatial signal: no offensive player settled in the
    // frontcourt set means the break is on.
    const handler = sense.offensePlayers.find((p) => p.jersey === sense.handler);
    if (handler) {
      const attackSide = sense.attack;
      const inFrontcourt = attackSide === 'right' ? handler.pose.x >= 0.5 : handler.pose.x <= 0.5;
      const offenseSettled = sense.offensePlayers.filter(
        (p) => attackSide === 'right' ? p.pose.x >= 0.5 : p.pose.x <= 0.5,
      ).length >= 3;
      if (!inFrontcourt || !offenseSettled) return poses;
    }
  }

  // Is ANY offensive player in the paint area (within 12ft of rim)?
  // Real help defense reacts to ALL paint threats, not just the ball
  // handler — a cutter rolling to the rim draws help even if the ball
  // is still on the perimeter. Without this, weak-side defenders stood
  // idle while two attackers rolled to the basket unguarded.
  const handlerInPaint = sense.offensePlayers.find((p) => p.jersey === sense.handler);
  if (!handlerInPaint) return poses;
  const handlerRimDist = handlerInPaint.distanceToRimFt;
  // Check if any non-handler attacker is deep in the paint
  const anyAttackerDeep = sense.offensePlayers.some(
    (p) => p.jersey !== sense.handler && p.distanceToRimFt < 12,
  );
  // handlerDriving: expanded from <14 to <18. In real NBA, weak-side
  // defenders start shading toward the rim the moment the handler
  // begins a drive from the perimeter (18-20ft). The old <14 threshold
  // meant no help reaction until the handler was already deep — by
  // then the ball was often passed out and adjustHelpDefenders
  // returned early (ball.status !== 'held'). Measured: Drive #1 had
  // all 4 weak-side defenders frozen at 'weak' while the handler
  // drove 25→12ft uncontested.
  // 威胁必须基于"是否在向篮下进攻",而非静态距离:handler在17-19ft
  // 的relocate(组织运球)不是篮下威胁,却触发了全员tag
  // (审计HELP_GHOST:ICE模式下4名守外围的防守人被标tag,距球28ft)。
  // 只有drive/cut动作或真正深入油漆区(<12ft)才触发协防。
  const handlerPose = state.poses[handlerInPaint.jersey];
  const handlerAction = handlerPose?.action ?? '';
  const isAttacking = handlerAction === 'drive' || handlerAction === 'cut'
    || handlerAction === 'ball_handler' && handlerRimDist < 14;
  const handlerDriving = handlerRimDist < 18 && (isAttacking || anyAttackerDeep);
  const paintThreat = handlerDriving || anyAttackerDeep;

  // P3.2 rotation chain: the help defender leaves their matchup to
  // protect the rim; the nearest weak-side defender X-outs to the
  // vacated attacker; the original help defender recovers when the
  // threat passes. Two roles in one pass:
  //   1. find the help defender (perimeter matchup, rim-side)
  //   2. X-out: the closest OTHER off-ball defender shifts toward the
  //      vacated attacker's spot (not to the rim — to the attacker)
  let next = poses;
  let helper: { jersey: string; pose: PoseState; attacker: string } | null = null;

  for (const defender of sense.defensePlayers) {
    const pose = poses[defender.jersey];
    if (!pose) continue;
    if (pose.action === 'on_ball_defend') continue;

    const myAttacker = sense.offensePlayers.reduce((best, att) => {
      const d = distFeet(pose.x, pose.y, att.pose.x, att.pose.y);
      return d < best.d ? { att, d } : best;
    }, { att: sense.offensePlayers[0]!, d: Infinity }).att;

    const attackerRimDist = myAttacker.distanceToRimFt;
    // A defender previously committed to help (or rotating as a tag) must
    // RECOVER the moment the paint threat passes. The old code only wrote
    // new poses for the helper and the X-out rotators — when the threat
    // disappeared, the stale help/tag label (and its rim-ward target)
    // persisted through the rest of the decision cooldown (measured:
    // 6k+ frames/game of help/tag labels stuck 3-4s after the ball left
    // the paint — the "help too long" audit failure). Recovery snaps the
    // label back to the neutral weak_side assignment; the next spatial
    // plan re-asserts the precise deny/sag target.
    if (!paintThreat && (pose.action === 'help' || pose.action === 'tag')) {
      next = {
        ...next,
        [defender.jersey]: setTarget(pose, pose.targetX, pose.targetY, 'weak_side', pose.movementRole),
      };
      continue;
    }
    if (paintThreat && attackerRimDist > 16) {
      // This defender is the helper: shade to the rim (help).
      // 距球<=10ft的防守人是在正常贴防(handoff/ISO的贴身人),
      // 不应被标为help/tag——直接跳过(不赋值help、不参与X-out)。
      const distToBall = distFeet(pose.x, pose.y, sense.ball.x, sense.ball.y);
      if (distToBall <= 10) continue;
      const urgency = handlerRimDist <= 5 ? 0.60 : handlerRimDist <= 10 ? 0.40 : handlerRimDist <= 14 ? 0.25 : 0.15;
      const helpX = sense.rim.x + (sense.rim.x < 0.5 ? 0.04 : -0.04);
      const helpY = 0.5 + (pose.y - 0.5) * 0.3;
      const blendX = pose.targetX * (1 - urgency) + helpX * urgency;
      const blendY = pose.targetY * (1 - urgency) + helpY * urgency;
      // The label must be TRUTHFUL to the resulting position: a weak
      // 15% shade from 35ft away produces a target still 25-30ft from
      // the rim — calling that `help` is a ghost. Only a target within
      // 14ft of the rim is real rim help; anything farther is a tag
      // (rotating toward the paint, not yet protecting it).
      const targetRimDist = Math.min(
        Math.hypot((blendX - (sense.rim.x < 0.5 ? 0 : 1)) * 94, (blendY - 0.5) * 50),
        Math.hypot((blendX - (sense.rim.x < 0.5 ? 1 : 0)) * 94, (blendY - 0.5) * 50),
      );
      const label = targetRimDist <= 14 ? 'help' : 'tag';
      next = {
        ...next,
        [defender.jersey]: setTarget(pose, clamp01(blendX), clamp01(blendY), label),
      };
      if (!helper) helper = { jersey: defender.jersey, pose, attacker: myAttacker.jersey };
    } else if (helper !== null) {
      // X-out: this weak-side defender shifts toward the HELPER'S
      // vacated attacker — the rotation covers the open man instead of
      // leaving a wide-open shooter on the weak side.
      const vacated = sense.offensePlayers.find((p) => p.jersey === helper!.attacker);
      if (vacated) {
        // P5.x: the X-out must stay on the WEAK side of the drive lane.
        // The old blend (60% own target + 40% vacated attacker) could
        // pull the rotator onto the ball handler's drive path when the
        // vacated attacker sat near the ball — three players collided
        // and the body-contact solver exploded them apart at 30+ft/s
        // (single-frame teleports). The rotator takes the vacated
        // attacker's SPOT but never closer to the handler than the
        // vacated attacker's defender distance allows; cap the blend at
        // 25% and keep ≥5ft from the ball.
        const handlerPose = sense.offensePlayers.find((p) => p.jersey === sense.handler)?.pose;
        const xOutX = pose.targetX * 0.75 + vacated.pose.x * 0.25;
        const xOutY = pose.targetY * 0.75 + vacated.pose.y * 0.25;
        if (handlerPose) {
          const dBall = Math.hypot((xOutX - sense.ball.x) * 94, (xOutY - sense.ball.y) * 50);
          if (dBall < 5) {
            // Pull back along the line from the ball: keep the rotator
            // off the drive lane without teleporting them.
            const bx = sense.ball.x;
            const by = sense.ball.y;
            const dx = xOutX - bx;
            const dy = xOutY - by;
            const len = Math.hypot(dx, dy) || 1;
            const scale = (5 / 94) / len;
            next = {
              ...next,
              [defender.jersey]: setTarget(pose, clamp01(bx + dx * scale), clamp01(by + dy * scale), 'deny'),
            };
          } else {
            next = {
              ...next,
              [defender.jersey]: setTarget(pose, clamp01(xOutX), clamp01(xOutY), 'deny'),
            };
          }
        } else {
          next = {
            ...next,
            [defender.jersey]: setTarget(pose, clamp01(xOutX), clamp01(xOutY), 'deny'),
          };
        }
      }
    }
  }
  return next;
}

/**
 * A help label is an assignment, but the viewer reads it as an active
 * defensive position. While a defender is still more than 18ft from the
 * defensive rim, `tag` is the truthful label: they are rotating toward the
 * paint, not yet providing rim help. This keeps the target/label pair
 * interpretable without cancelling the existing help target.
 */
function normalizeHelpLabels(poses: PoseMap, state: GameState): PoseMap {
  if (state.ballMotion.status !== 'held') return poses;
  const sense = perceiveLiveCourt(state);
  if (!sense) return poses;
  const next: Record<string, PoseState> = {};
  let changed = false;
  for (const pose of Object.values(poses)) {
    const isDefense = pose.team === sense.defense;
    const rimDist = distFeet(pose.x, pose.y, sense.rim.x, sense.rim.y);
    // 回防中的防守人(距防守篮筐>18ft且距球>20ft)不是协防者:
    // 他们在跑回对位人的路上。tag/help标签在回防中没有意义
    // (审计HELP_GHOST:TRANSITION_PUSH中防守人带着上一回合的
    // tag残留,距篮筐24ft距球26ft——转换开始时应重置为回防语义)。
    const recovering = isDefense && rimDist > 18
      && distFeet(pose.x, pose.y, sense.ball.x, sense.ball.y) > 20;
    if (recovering && (pose.action === 'help' || pose.action === 'tag')) {
      next[pose.jersey] = setTarget(pose, pose.targetX, pose.targetY, 'weak_side', pose.movementRole);
      changed = true;
    } else if (pose.action === 'help' && isDefense && rimDist > 18) {
      // Rotating toward the paint, not yet providing rim help.
      next[pose.jersey] = setTarget(pose, pose.targetX, pose.targetY, 'tag', pose.movementRole);
      changed = true;
    } else if (pose.action === 'tag' && isDefense && rimDist > 25) {
      // A tag 25ft+ from the defensive rim is not a tag — it is a
      // defender still recovering to their matchup. Labeling it tag
      // produced the "ghost help" frames (defenders labeled tag while
      // standing 30-40ft from any play). weak_side is the truthful
      // recovery label; the spatial planner re-asserts the real
      // assignment on the next decision tick.
      next[pose.jersey] = setTarget(pose, pose.targetX, pose.targetY, 'weak_side', pose.movementRole);
      changed = true;
    } else {
      next[pose.jersey] = pose;
    }
  }
  return changed ? next : poses;
}

/**
 * Keep flexible off-ball players from arriving in the same lane. A plan's
 * absolute targets can be six feet apart while two players are still
 * converging from a turnover or a pass rotation; the old target-only nudge
 * left 3–4ft teammate clumps for several seconds. We move only the flexible
 * spacer target, preserving a declared cut/pass-receive lane and never
 * disturbing a live screen pair.
 * `cut`/`relocate` were added after the first-principles audit: a PNR roll
 * target (rim-side, 3ft) and a corner spacer target (arc) share the same
 * approach corridor, so without them in the flexible set the roller plowed
 * through spacers every SCREEN_USE (measured 9k+ frames of <5ft off-ball
 * pairs per game vs NBA ~2-4% of live frames).
 */
const FLEXIBLE_SPACING_ACTIONS: ReadonlySet<string> = new Set(['space', 'cut', 'relocate']);
const SPACING_MIN_FT = 6;
// Targets are still integrated through the normal speed controller, so the
// nudge can close the entire spacing deficit in one planning tick without a
// teleport. A 1.5ft cap left two spacers converging for 3–5 seconds while
// every subsequent target remained inside the same lane.
const SPACING_NUDGE_FT = 3;

function applyOffenseSpacing(poses: PoseMap): PoseMap {
  const players = Object.values(poses);
  const next: Record<string, PoseState> = { ...poses };
  let changed = false;
  const isFlexible = (pose: PoseState): boolean => FLEXIBLE_SPACING_ACTIONS.has(pose.action);

  for (let a = 0; a < players.length; a += 1) {
    for (let b = a + 1; b < players.length; b += 1) {
      const first = players[a]!;
      const second = players[b]!;
      if (first.team !== second.team || first.hasBall || second.hasBall) continue;
      if (first.action === 'screen' || second.action === 'screen') continue;
      const dxFt = (second.x - first.x) * 94;
      const dyFt = (second.y - first.y) * 50;
      const distance = Math.hypot(dxFt, dyFt);
      if (distance >= SPACING_MIN_FT) continue;

      const firstFlexible = isFlexible(first);
      const secondFlexible = isFlexible(second);
      if (!firstFlexible && !secondFlexible) continue;
      const length = distance || 1;
      const ux = dxFt / length;
      const uy = dyFt / length;
      const nudge = Math.min(SPACING_NUDGE_FT, (SPACING_MIN_FT - distance) * 0.5);

      if (firstFlexible) {
        const current = next[first.jersey] ?? first;
        next[first.jersey] = setTarget(
          current,
          clamp01(current.targetX - (ux * nudge) / 94),
          clamp01(current.targetY - (uy * nudge) / 50),
          current.action,
          current.movementRole,
        );
        changed = true;
      }
      if (secondFlexible) {
        const current = next[second.jersey] ?? second;
        next[second.jersey] = setTarget(
          current,
          clamp01(current.targetX + (ux * nudge) / 94),
          clamp01(current.targetY + (uy * nudge) / 50),
          current.action,
          current.movementRole,
        );
        changed = true;
      }
    }
  }
  return changed ? next : poses;
}


/**
 * Apply all local perception adjustments for this tick.
 */
export function applyLocalPerception(poses: PoseMap, state: GameState, zoneActive = false): PoseMap {
  if (state.phase !== 'LIVE' || state.ballMotion.status === 'inbound' || state.ballMotion.status === 'dead') return poses;
  let result = poses;
  // 联防(2-3)时,全部5人的站位由zoneSlots几何决定——人盯人的
  // 微调(adjustOnBallDefender/adjustHelpDefenders/normalizeHelpLabels)
  // 会把zone球员拉向持球人,与slot冲突,导致翼位在20-28ft漂移、
  // 阵型无法成形(审计ZONE_COLLAPSED: 72-77%帧塌缩)。
  // zone的top guard追持球人由zoneSlots[0]自身保证(其target就是
  // handler位置),不需要额外的人盯人blend。
  if (!zoneActive) {
    result = adjustOnBallDefender(result, state);
    result = adjustHelpDefenders(result, state);
    result = normalizeHelpLabels(result, state);
  }
  result = applyOffenseSpacing(result);
  return result;
}


