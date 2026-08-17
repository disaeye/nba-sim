/**
 * Dead-ball absolute alignment builders (tip, tip-receive, inbound, free throw).
 *
 * Extracted from alignment.ts without behavior change. These setups place
 * players at absolute court coordinates — they do NOT participate in the
 * relational live-retarget path (`retargetFromLiveWorld` / `applyLiveRetarget`).
 *
 * Coordinates match alignment.ts: full court, x∈[0,1] length, y∈[0,1] width,
 * home basket left at x=0, away basket right at x=1 at tip-off.
 *
 * Import direction: dead-setup → alignment (types/helpers only).
 * alignment.ts MUST NOT import from dead-setup (no cycle).
 */
import type { CourtZone, TeamId } from '../state/types.js';
import { rimNorm, tacticalSpot, type TacticalSlot } from './geometry.js';
import {
  type Alignment,
  type BasketSide,
  type Baskets,
  type CourtPlayer,
  offenseAttacks,
} from './alignment.js';

function clamp01(n: number): number {
  if (n < 0) return 0;
  if (n > 1) return 1;
  return n;
}

function halfcourtSpot(
  slot: TacticalSlot,
  attack: BasketSide,
): { x: number; y: number; zone: CourtZone } {
  const p = tacticalSpot(slot, attack);
  return { x: p.x, y: p.y, zone: p.zone };
}

function backcourtSpot(attack: BasketSide, y: number): { x: number; y: number; zone: CourtZone } {
  const own: BasketSide = attack === 'right' ? 'left' : 'right';
  const rim = rimNorm(own);
  return { x: clamp01(rim.x + (attack === 'right' ? 0.12 : -0.12)), y, zone: 'backcourt' };
}

/**
 * NBA tip-off: 2 jumpers in center circle (each on half farthest from own basket),
 * 8 non-jumpers alternating outside the restraining circle.
 */
export function buildJumpBallAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly homeJumper: string;
  readonly awayJumper: string;
}): Alignment {
  const { homeLineup, awayLineup, homeJumper, awayJumper } = args;
  // Home basket left → jumper stands on right half of circle (farther from own basket).
  // Away basket right → jumper on left half of circle.
  const players: CourtPlayer[] = [
    {
      jersey: homeJumper,
      team: 'home',
      x: 0.52,
      y: 0.5,
      zone: 'frontcourt_center',
      task: 'jump',
      hasBall: false,
    },
    {
      jersey: awayJumper,
      team: 'away',
      x: 0.48,
      y: 0.5,
      zone: 'frontcourt_center',
      task: 'jump',
      hasBall: false,
    },
  ];

  // The eight non-jumpers remain around the circle, but each team occupies
  // its own side. The outlet is always lineup[1], so this keeps the first
  // live handler in its backcourt instead of sending it into reverse motion.
  const radius = 0.14;
  const homeAngles = [135, 180, 225, 90] as const;
  const awayAngles = [45, 0, 315, 270] as const;
  const homeOthers = homeLineup.filter((j) => j !== homeJumper);
  const awayOthers = awayLineup.filter((j) => j !== awayJumper);
  homeOthers.forEach((jersey, i) => {
    const degrees = homeAngles[i] ?? 180;
    const ang = (degrees * Math.PI) / 180;
    players.push({
      jersey,
      team: 'home',
      x: clamp01(0.5 + radius * Math.cos(ang)),
      y: clamp01(0.5 + radius * Math.sin(ang)),
      zone: 'frontcourt_center',
      task: 'circle_spot',
      hasBall: false,
    });
  });
  awayOthers.forEach((jersey, i) => {
    const degrees = awayAngles[i] ?? 0;
    const ang = (degrees * Math.PI) / 180;
    players.push({
      jersey,
      team: 'away',
      x: clamp01(0.5 + radius * Math.cos(ang)),
      y: clamp01(0.5 + radius * Math.sin(ang)),
      zone: 'frontcourt_center',
      task: 'circle_spot',
      hasBall: false,
    });
  });

  return { context: 'jump_ball', offense: null, players };
}

/** After tip: winner controls in their backcourt; teammates spread to advance. */
/** After tip: winner receives near half-court and immediately attacks. */
export function buildTipReceiveAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly winner: TeamId;
  readonly controller: string;
  readonly baskets: Baskets;
}): Alignment {
  const { homeLineup, awayLineup, winner, controller, baskets } = args;
  const attack = offenseAttacks(winner, baskets);
  const defense: TeamId = winner === 'home' ? 'away' : 'home';
  const offLine = winner === 'home' ? homeLineup : awayLineup;
  const defLine = defense === 'home' ? homeLineup : awayLineup;
  const players: CourtPlayer[] = [];
  const forward = attack === 'right' ? 1 : -1;
  const center = 0.5;
  const controllerX = center - forward * 0.035;

  players.push({
    jersey: controller,
    team: winner,
    x: controllerX,
    y: 0.5,
    zone: 'backcourt',
    task: 'receive_tip',
    hasBall: true,
  });

  // Transition lanes begin at the circle, not at the own basket. Two players
  // run ahead; the remaining two trail the handler by only a few feet.
  const offOthers = offLine.filter((j) => j !== controller);
  const laneY = [0.25, 0.75, 0.38, 0.62] as const;
  offOthers.forEach((j, i) => {
    const ahead = i < 2 ? 0.045 : -0.015;
    players.push({
      jersey: j,
      team: winner,
      x: clamp01(center + forward * ahead),
      y: laneY[i] ?? 0.5,
      zone: 'backcourt',
      task: i < 2 ? 'advance' : 'space',
      hasBall: false,
    });
  });

  // Defenders start in a compact, half-court-ready shell rather than retreating
  // to the paint. They can match up while the offense crosses half-court.
  const defX = [0.08, 0.14, 0.20, 0.12, 0.18] as const;
  const defY = [0.5, 0.28, 0.72, 0.4, 0.6] as const;
  const defTasks: import('../court/alignment.js').PlayerTask[] = [
    'on_ball_defend', 'deny', 'deny', 'help', 'weak_side',
  ];
  defLine.forEach((j, i) => {
    players.push({
      jersey: j,
      team: defense,
      x: clamp01(center + forward * (defX[i] ?? 0.12)),
      y: defY[i] ?? 0.5,
      zone: 'frontcourt_center',
      task: defTasks[i] ?? 'help',
      hasBall: false,
    });
  });

  return { context: 'tip_receive', offense: winner, players };
}

export function buildInboundAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly inboundTeam: TeamId;
  readonly inbounder: string;
  readonly receiver: string;
  readonly baskets: Baskets;
  /** baseline = after a make (end line under the basket); sideline = violations/OOB. */
  readonly spot: 'baseline' | 'sideline';
}): Alignment {
  const { homeLineup, awayLineup, inboundTeam, inbounder, receiver, baskets, spot } = args;
  const attack = offenseAttacks(inboundTeam, baskets);
  const defense: TeamId = inboundTeam === 'home' ? 'away' : 'home';
  const offLine = inboundTeam === 'home' ? homeLineup : awayLineup;
  const defLine = defense === 'home' ? homeLineup : awayLineup;
  const sidelineX = attack === 'right' ? 0.08 : 0.92;
  const players: CourtPlayer[] = [];
  // The inbounder stands ON the end line (baseline inbound, after a make)
  // or at the near side line (violation/OOB inbound). The ball leaves from
  // there — the spectator sees the pass leave the line, not teleport in.
  // P5.x: the inbounder stands on THEIR OWN baseline (the end line under
  // the basket they defend), not the opponent's. attack=right means the
  // team is going TOWARD the right basket, so their own baseline is LEFT
  // (x≈0.045). The old `attack === 'right' ? 0.955` put the inbounder on
  // the wrong end — the ball appeared to teleport from the opponent's
  // baseline to midcourt.
  const inboundX = spot === 'baseline'
    ? attack === 'right' ? 0.045 : 0.955
    : sidelineX;
  const inboundY = spot === 'baseline' ? 0.5 : 0.02;
  players.push({
    jersey: inbounder,
    team: inboundTeam,
    x: inboundX,
    y: inboundY,
    zone: 'backcourt',
    task: 'inbound',
    hasBall: true,
  });
  // Receiver meets the ball 15-18ft upcourt of the inbounder (real NBA
  // baseline inbounds: the guard comes to the ball or breaks to the elbow
  // extension — a 36ft line-drive to midcourt is not an inbound pass).
  // The team then ADVANCES 50-60ft with the ball — this is what gives
  // backcourt pressure, pace identity, and the 8-second count room to
  // exist. The old midcourt receive (x≈0.43) flattened every possession
  // into a halfcourt start.
  const recvSpot = {
    x: clamp01(attack === 'right' ? 0.21 : 0.79),
    y: 0.42,
    zone: 'backcourt' as CourtZone,
  };
  players.push({
    jersey: receiver,
    team: inboundTeam,
    x: recvSpot.x,
    y: recvSpot.y,
    zone: 'backcourt',
    task: 'receive_tip',
    hasBall: false,
  });
  offLine
    .filter((j) => j !== inbounder && j !== receiver)
    .forEach((j, i) => {
      // Real NBA baseline inbound: the non-receivers SPRINT to their
      // frontcourt offensive positions while the ball is being inbounded —
      // corners, wings, and the trailing big fill the lanes. The old
      // backcourtSpot targets kept them standing at midcourt waiting for
      // the catch (measured: mates at x≈0.51 through the entire inbound
      // window, 0 frontcourt movement until possession gained).
      // The screener/big runs the rim lane; the wings fill wide lanes
      // 22-24ft from the rim; the trailer stays behind the ball.
      const rimX = attack === 'right' ? 0.944 : 0.056;
      const spots = [
        // Wing strong-side: wide lane at 3pt depth
        { x: clamp01(rimX + (attack === 'right' ? -0.25 : 0.25)), y: 0.28, zone: 'frontcourt_center' as CourtZone },
        // Wing weak-side: opposite wide lane
        { x: clamp01(rimX + (attack === 'right' ? -0.25 : 0.25)), y: 0.72, zone: 'frontcourt_center' as CourtZone },
        // Trailer: behind the ball at the top for the swing
        { x: clamp01(0.5 + (attack === 'right' ? -0.08 : 0.08)), y: 0.5, zone: 'frontcourt_center' as CourtZone },
      ];
      const spot = spots[i] ?? spots[2]!;
      players.push({
        jersey: j,
        team: inboundTeam,
        x: spot.x,
        y: spot.y,
        zone: spot.zone,
        task: 'space',
        hasBall: false,
      });
    });
  // Defense matches up against the inbound formation, NOT in the paint.
  // The old code stacked all 5 defenders deep in the paint — they were
  // 10-15ft from the receiver at the inbound catch, producing a p50
  // nearest-defender gap of 11ft at POSSESSION_GAINED. Real transition
  // defense puts one defender on the receiver (the primary threat) and
  // the rest matched up across the frontcourt.
  defLine.forEach((j, i) => {
    if (i === 0) {
      // Primary defender: guards the inbound receiver. Stand between
      // the receiver and the attacking basket — the standard denial
      // position for an inbound play.
      const rx = recvSpot.x;
      const ry = recvSpot.y;
      const rimX = attack === 'right' ? 1 : 0;
      const dx = (rimX - rx);
      const dy = (0.5 - ry);
      const dl = Math.hypot(dx, dy) || 1;
      players.push({
        jersey: j,
        team: defense,
        x: clamp01(rx + (dx / dl) * (6 / 94)),
        y: clamp01(ry + (dy / dl) * (6 / 50)),
        zone: 'frontcourt_center',
        task: 'on_ball_defend',
        hasBall: false,
      });
    } else {
      // Other defenders spread across midcourt to match up with the
      // trailing offensive players.
      const ys = [0.25, 0.5, 0.65, 0.8] as const;
      players.push({
        jersey: j,
        team: defense,
        x: clamp01(attack === 'right' ? 0.38 : 0.62),
        y: ys[i - 1] ?? 0.5,
        zone: 'frontcourt_center',
        task: i === 1 ? 'deny' : 'weak_side',
        hasBall: false,
      });
    }
  });
  return { context: 'inbound', offense: inboundTeam, players };
}

export function buildFtAlignment(args: {
  readonly homeLineup: readonly string[];
  readonly awayLineup: readonly string[];
  readonly shooterId: string;
  readonly shootingTeam: TeamId;
  readonly baskets: Baskets;
}): Alignment {
  const { homeLineup, awayLineup, shooterId, shootingTeam, baskets } = args;
  const attack = offenseAttacks(shootingTeam, baskets);
  const defense: TeamId = shootingTeam === 'home' ? 'away' : 'home';
  const offLine = shootingTeam === 'home' ? homeLineup : awayLineup;
  const defLine = defense === 'home' ? homeLineup : awayLineup;
  // FT line sits at lane-length (19ft) from the baseline — the same
  // authority as court-geometry's tacticalSpot. The old code used the
  // elbow spot (18-19ft diagonal), leaving every free throw 4ft short.
  const lineX = attack === 'right' ? 1 - 19 / 94 : 19 / 94;
  const rim = halfcourtSpot('rim', attack);
  const players: CourtPlayer[] = [
    {
      jersey: shooterId,
      team: shootingTeam,
      x: lineX,
      y: 0.5,
      zone: 'paint',
      task: 'ball_handler',
      hasBall: true,
    },
  ];
  const defLaneY = [0.28, 0.38, 0.5, 0.62, 0.72] as const;
  const offLaneY = [0.22, 0.34, 0.66, 0.78] as const;
  // NBA FT rebounding positions: the two lowest lane slots on each side
  // (first and second defensive, first and second offensive) are the
  // rebounders who box out. The shooter does not box out. The third lane
  // slot, the ends of the line, and the mid-court players stand and watch.
  // The old code labelled ALL defenders + first two offense as box_out
  // (7 players) — a mid-court player walking to a spot 40ft from the rim
  // with a box_out label is not boxing out anything.
  defLine.forEach((j, i) => {
    const y = defLaneY[i] ?? 0.5;
    // The center lane slot (y=0.5) sits directly in front of the shooter
    // at the line; it must sit on the rim side of the lane, not 2ft from
    // the shooter's spot. Off-center slots alternate to clear the shooter.
    const side = i === 2 ? (attack === 'right' ? 1 : -1) : (i % 2 === 0 ? -1 : 1);
    const isRebounder = i < 2; // only the two lowest defensive lane slots
    players.push({
      jersey: j,
      team: defense,
      x: clamp01(rim.x + (attack === 'right' ? -0.06 : 0.06) + side * 0.02),
      y,
      zone: 'paint',
      task: isRebounder ? 'box_out' : 'idle',
      hasBall: false,
    });
  });
  offLine
    .filter((j) => j !== shooterId)
    .forEach((j, i) => {
      const y = offLaneY[i] ?? 0.5 + (i - 1.5) * 0.08;
      const isRebounder = i < 2; // only the two lowest offensive lane slots
      players.push({
        jersey: j,
        team: shootingTeam,
        x: clamp01(rim.x + (attack === 'right' ? -0.12 - i * 0.03 : 0.12 + i * 0.03)),
        y,
        zone: 'paint',
        task: isRebounder ? 'box_out' : 'space',
        hasBall: false,
      });
    });
  return { context: 'dead', offense: shootingTeam, players };
}
