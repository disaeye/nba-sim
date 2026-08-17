/**
 * Deep frame-by-frame realism audit (position truth at 0.1s tick).
 *
 * Verifies the continuous layer the way a viewer sees it:
 *   - kinematics: speeds, teleports (>30ft/s), frozen offense, direction reversals
 *   - ball coherence: held-ball mismatch, pass/shot flight vs distance
 *   - event-position correspondence: shot release point, rebound, steal,
 *     foul contact, FT-line position, SCV context
 *   - defensive tracking: assignment distances over time
 *
 *   npx tsx scripts/frame-deep-audit.ts [--seeds 42,5,7] [--out .omo/evidence/frame-deep-audit.json]
 */
import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { Player } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';
import type { WorldSnapshot } from '../src/world/snapshot.js';
import type { TimelineEvent } from '../src/sim-utils.js';

interface ConfigShape {
  home_team: { id: string; roster: Player[]; lineup_packages: LineupPackage[] };
  away_team: ConfigShape['home_team'];
}

const FT = 94;
const WD = 50;
const input = (cfg: ConfigShape, seed: number) => ({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});
const distFt = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot((a.x - b.x) * FT, (a.y - b.y) * WD);
const isDead = (ph: string) => ph.startsWith('DEAD');

interface EC { n: number; maxD: number; over: number; overList: string[]; }
function pct(sorted: number[], p: number): number {
  if (sorted.length === 0) return 0;
  return sorted[Math.min(sorted.length - 1, Math.floor(p * sorted.length))]!;
}

function analyze(seed: number, cfg: ConfigShape) {
  const r = simulateGame(input(cfg, seed));
  const snaps = r.snapshots ?? [];
  const evBySeq = new Map<number, TimelineEvent>();
  for (const e of r.events) evBySeq.set(e.seq, e);
  const snapByT = new Map<number, WorldSnapshot>();
  for (const s of snaps) if (!snapByT.has(s.t_real)) snapByT.set(s.t_real, s);

  const perPlayer = new Map<string, { max: number; p99: number; p50: number; mean: number; n: number; speeds: number[]; teleports: number }>();
  const teleports: { t: number; j: string; ftps: number }[] = [];
  const liveOver25: { t: number; j: string; ftps: number; action: string }[] = [];
  const deadOver12: { t: number; j: string; ftps: number; phase: string }[] = [];
  const reversals: { t: number; j: string }[] = [];
  const overlaps: { t: number; a: string; aAct: string; b: string; bAct: string; ft: number }[] = [];
  let liveFrames = 0;
  let offenseFrozenFrames = 0;
  let heldMismatch = 0;
  const heldMismatchSamples: { t: number; ballX: number; ballY: number; holder: string; d: number }[] = [];
  const passDurs: number[] = [];
  const passSpeeds: number[] = [];
  const shotDurs: number[] = [];
  const shotDurs3: number[] = [];
  const shotSpeeds: number[] = [];
  let ballMaxTickPass = 0;
  let ballMaxTickShot = 0;
  const ecs: Record<string, EC> = {};
  const ec = (k: string, d: number, thr: number, note: string) => {
    const e = ecs[k] ??= { n: 0, maxD: 0, over: 0, overList: [] };
    e.n++; if (d > e.maxD) e.maxD = d;
    if (d > thr) { e.over++; if (e.overList.length < 6) e.overList.push(note); }
  };
  const scvCtx: { t: number; ballStatus: string; ballPos: string; sinceLastRelease: number }[] = [];
  const defTrack: Record<string, { n: number; mean: number; max: number }> = {};
  const handlerDist: number[] = [];
  // ── off-ball coverage game (deny/sag/chase) ────────────────────────────
  const coverage: Record<string, { n: number; sum: number; max: number }> = {};
  const coveragePrev = new Map<string, string>();
  let coverageFlips = 0;
  const shotZones: Record<string, number> = {};
  // order sanity: and-one fouls must precede their SHOT_RESULT/MADE events
  let foulAfterMake = 0;
  let foulAfterResult = 0;

  {
    let inFlight = false;
    let start: WorldSnapshot | null = null;
    for (const s of snaps) {
      if (s.ball.status === 'pass' && !inFlight) { inFlight = true; start = s; }
      else if (s.ball.status !== 'pass' && inFlight && start) {
        const dur = Math.max(0.05, s.t_real - start.t_real);
        const d = distFt({ x: start.ball.x, y: start.ball.y }, { x: s.ball.x, y: s.ball.y });
        passDurs.push(dur); passSpeeds.push(d / dur);
        inFlight = false; start = null;
      }
    }
  }
  for (let i = 1; i < snaps.length; i++) {
    const a = snaps[i - 1]!, b = snaps[i]!;
    const dt = Math.max(0.05, b.t_real - a.t_real);
    if (a.ball.status === 'pass' && b.ball.status === 'pass') {
      const v = distFt(a.ball, b.ball) / dt;
      if (v > ballMaxTickPass) ballMaxTickPass = v;
    }
    if (a.ball.status === 'shot' && b.ball.status === 'shot') {
      const v = distFt(a.ball, b.ball) / dt;
      if (v > ballMaxTickShot) ballMaxTickShot = v;
    }
  }

  for (let i = 0; i < snaps.length; i++) {
    const s = snaps[i]!;
    const prev = i > 0 ? snaps[i - 1]! : null;
    const dtSec = prev ? s.t_real - prev.t_real : 0.1;
    if (prev && (dtSec <= 0 || dtSec > 0.2)) continue;
    const players = s.players;
    if (players.length !== 10) continue;
    const live = s.phase === 'LIVE';
    if (s.phase === 'POST_GAME') continue;
    for (const p of players) {
      const k = perPlayer.get(p.jersey) ?? { max: 0, p99: 0, p50: 0, mean: 0, n: 0, speeds: [] as number[], teleports: 0 };
      let speed = 0;
      if (prev) {
        const q = prev.players.find((q) => q.jersey === p.jersey);
        if (q) {
          speed = distFt(q, p) / dtSec;
          if (speed > 30) { k.teleports++; teleports.push({ t: s.t_real, j: p.jersey, ftps: speed }); }
          if (speed > 25 && live) liveOver25.push({ t: s.t_real, j: p.jersey, ftps: speed, action: p.action });
          if (speed > 12 && isDead(s.phase)) deadOver12.push({ t: s.t_real, j: p.jersey, ftps: speed, phase: s.phase });
          if (i >= 2) {
            const q2 = snaps[i - 2]!.players.find((q) => q.jersey === p.jersey);
            if (q2) {
              const v1 = { x: p.x - q.x, y: p.y - q.y }, v2 = { x: q.x - q2.x, y: q.y - q2.y };
              const n1 = Math.hypot(v1.x, v1.y), n2 = Math.hypot(v2.x, v2.y);
              if (n1 > 0.02 && n2 > 0.02) {
                const cos = (v1.x * v2.x + v1.y * v2.y) / (n1 * n2);
                if (cos < -0.5 && (n1 / dtSec) * FT > 8 && (n2 / dtSec) * FT > 8) reversals.push({ t: s.t_real, j: p.jersey });
              }
            }
          }
        }
      }
      k.n++; k.mean += speed; k.speeds.push(speed);
      if (speed > k.max) k.max = speed;
      perPlayer.set(p.jersey, k);
    }
    if (live) {
      liveFrames++;
      if (s.tactical && s.tactical.offense) {
        const off = s.players.filter((p) => p.team === s.tactical!.offense);
        if (off.length === 5 && prev) {
          let still = 0;
          for (const p of off) {
            const q = prev.players.find((q) => q.jersey === p.jersey);
            if (q && distFt(q, p) / dtSec < 1.5) still++;
          }
          if (still >= 4) offenseFrozenFrames++;
        }
      }
    }
    for (let a = 0; a < players.length; a++) for (let b = a + 1; b < players.length; b++) {
      const d = distFt(players[a]!, players[b]!);
      if (d < 1) overlaps.push({ t: s.t_real, a: players[a]!.jersey, aAct: players[a]!.action, b: players[b]!.jersey, bAct: players[b]!.action, ft: d });
    }
    if (s.ball.status === 'held' && s.ball.holderId) {
      const h = players.find((p) => p.jersey === s.ball.holderId);
      if (h) {
        const d = distFt(h, s.ball);
        if (d > 2) { heldMismatch++; if (heldMismatchSamples.length < 8) heldMismatchSamples.push({ t: s.t_real, ballX: s.ball.x, ballY: s.ball.y, holder: s.ball.holderId, d }); }
      }
    }
    if (s.tactical?.assignments) {
      for (const a of s.tactical.assignments) {
        if (!a.targetJersey) continue;
        const d = players.find((p) => p.jersey === a.jersey);
        const t = players.find((p) => p.jersey === a.targetJersey);
        if (d && t) {
          const dd = distFt(d, t);
          const k = defTrack[`${a.role}@${a.action}`] ??= { n: 0, mean: 0, max: 0 };
          k.n++; k.mean += dd; if (dd > k.max) k.max = dd;
        }
      }
      if (s.tactical.handler && s.tactical.offense) {
        const h = players.find((p) => p.jersey === s.tactical!.handler);
        if (h) {
          let minD = Infinity;
          for (const d of players.filter((p) => p.team !== s.tactical!.offense)) {
            const dd = distFt(d, h); if (dd < minD) minD = dd;
          }
          if (minD < Infinity) handlerDist.push(minD);
        }
      }
      // Off-ball coverage game: bucket each spacer's nearest defender by the
      // defender's coverage action, and count real-time coverage flips
      // (deny → sag when the ball swings, → chase when the spacer runs).
      if (live && s.tactical.handler && s.tactical.offense) {
        const offenseTeam = s.tactical.offense;
        const defenders = players.filter((p) => p.team !== offenseTeam);
        for (const off of players.filter((p) => p.team === offenseTeam && p.jersey !== s.tactical!.handler)) {
          let best: (typeof players)[number] | null = null;
          let bestD = Infinity;
          for (const def of defenders) {
            const dd = distFt(def, off);
            if (dd < bestD) { bestD = dd; best = def; }
          }
          if (best) {
            const rec = coverage[best.action] ??= { n: 0, sum: 0, max: 0 };
            rec.n++; rec.sum += bestD; if (bestD > rec.max) rec.max = bestD;
            const prev = coveragePrev.get(best.jersey);
            if (prev !== undefined && prev !== best.action) coverageFlips++;
            coveragePrev.set(best.jersey, best.action);
          }
        }
      }
    }
    if (s.lastEventSeq !== null) {
      const ev = evBySeq.get(s.lastEventSeq);
      if (ev) {
        const pl: Record<string, unknown> = ev.payload as unknown as Record<string, unknown>;
        const str = (k: string): string | null => (typeof pl[k] === 'string' ? (pl[k] as string) : null);
        const num = (k: string): number | null => (typeof pl[k] === 'number' ? (pl[k] as number) : null);
        const findP = (id: string | null) => (id ? players.find((p) => p.jersey === id) : undefined);
        switch (ev.type) {
          case 'SHOT_RELEASE': {
            const sh = findP(str('shooter_id'));
            const px = num('x'), py = num('y');
            if (sh && px !== null && py !== null) ec('shot_release_pos', distFt(sh, { x: px, y: py }), 2.5, `t=${s.t_real}`);
            const zone = str('zone'); if (zone) shotZones[zone] = (shotZones[zone] ?? 0) + 1;
            const resEv = r.events.find((e2) => e2.seq > ev.seq && e2.type === 'SHOT_RESULT');
            if (resEv) {
              const resSnap = snapByT.get(resEv.t_real);
              if (resSnap) {
                const dur = Math.max(0.05, resSnap.t_real - s.t_real);
                const d = distFt({ x: px ?? s.ball.x, y: py ?? s.ball.y }, { x: resSnap.ball.x, y: resSnap.ball.y });
                shotDurs.push(dur); shotSpeeds.push(d / dur);
                if (num('shot_value') === 3) shotDurs3.push(dur);
              }
            }
            break;
          }
          case 'SHOT_CLOCK_VIOLATION': {
            const lastRel = r.events.filter((e2) => e2.seq < ev.seq && e2.type === 'SHOT_RELEASE').at(-1);
            scvCtx.push({
              t: s.t_real,
              ballStatus: s.ball.status,
              ballPos: `(${s.ball.x.toFixed(3)},${s.ball.y.toFixed(3)})`,
              sinceLastRelease: lastRel ? +(s.t_real - lastRel.t_real).toFixed(1) : -1,
            });
            break;
          }
        }
      }
    }
    const evAtT = r.events.filter((e) => e.t_real === s.t_real && !(s.lastEventSeq !== null && e.seq === s.lastEventSeq) && ['REBOUND', 'STEAL', 'FOUL', 'FT_ATTEMPT'].includes(e.type));
    for (const ev of evAtT) {
      const pl: Record<string, unknown> = ev.payload as unknown as Record<string, unknown>;
      const str = (k: string): string | null => (typeof pl[k] === 'string' ? (pl[k] as string) : null);
      const findP = (id: string | null) => (id ? players.find((p) => p.jersey === id) : undefined);
      switch (ev.type) {
        case 'REBOUND': {
          const rb = findP(str('rebounder_id'));
          if (rb) ec('rebound_rim_dist', distFt(rb, { x: rb.x < 0.5 ? 0 : 1, y: 0.5 }), 18, `t=${s.t_real}`);
          break;
        }
        case 'STEAL': {
          const st = findP(str('stealer_id')), vi = findP(str('victim_id'));
          if (st && vi) ec('steal_victim_dist', distFt(st, vi), 4, `t=${s.t_real}`);
          break;
        }
        case 'FOUL': {
          const of = findP(str('offender_id')), vi = findP(str('victim_id'));
          if (of && vi) ec('foul_contact_dist', distFt(of, vi), 5, `t=${s.t_real}`);
          break;
        }
        case 'FT_ATTEMPT': {
          const sh = findP(str('shooter_id'));
          if (sh) {
            // FT line is 19ft (lane length) from the baseline.
            const dLine = Math.min(distFt(sh, { x: 19 / 94, y: 0.5 }), distFt(sh, { x: 1 - 19 / 94, y: 0.5 }));
            ec('ft_line_pos', dLine, 3, `t=${s.t_real} x=${sh.x.toFixed(3)} y=${sh.y.toFixed(3)}`);
          }
          break;
        }
      }
    }
  }

  // And-one foul ordering: whistle must precede its SHOT_RESULT and
  // MADE_BASKET_DEAD (FOUL → SHOT_RESULT → MADE_BASKET_DEAD).
  for (let i = 0; i < r.events.length; i++) {
    const ev = r.events[i]!;
    if (ev.type !== 'FOUL') continue;
    const pl = ev.payload as Record<string, unknown>;
    if (pl['free_throws_awarded'] !== 1) continue;
    const rest = r.events.slice(i + 1);
    const nextResult = rest.find((e2) => e2.type === 'SHOT_RESULT')?.seq ?? Infinity;
    const nextMade = rest.find((e2) => e2.type === 'MADE_BASKET_DEAD')?.seq ?? Infinity;
    if (nextMade < ev.seq) foulAfterMake++;
    else if (nextResult < ev.seq) foulAfterResult++;
  }

  const kinOut: Record<string, { max: number; p99: number; p50: number; mean: number; teleports: number }> = {};
  for (const [j, k] of perPlayer) {
    k.speeds.sort((a, b) => a - b);
    kinOut[j] = { max: k.max, p99: pct(k.speeds, 0.99), p50: pct(k.speeds, 0.5), mean: k.mean / k.n, teleports: k.teleports };
  }
  const defOut: Record<string, { n: number; mean: number; max: number }> = {};
  for (const [k, v] of Object.entries(defTrack)) defOut[k] = { n: v.n, mean: +(v.mean / v.n).toFixed(1), max: +v.max.toFixed(1) };
  const hd = handlerDist.sort((a, b) => a - b);
  const s3 = [...shotDurs3].sort((a, b) => a - b);
  const sd = [...shotDurs].sort((a, b) => a - b);
  const ss = [...shotSpeeds].sort((a, b) => a - b);
  const pd = [...passDurs].sort((a, b) => a - b);
  const ps = [...passSpeeds].sort((a, b) => a - b);

  return {
    seed,
    finalScore: r.events[r.events.length - 1]?.score ?? null,
    snapshots: snaps.length,
    kinematics: {
      liveFrames,
      offenseFrozenShare: liveFrames ? +(offenseFrozenFrames / liveFrames).toFixed(4) : 0,
      teleports,
      liveOver25,
      deadOver12: deadOver12.length,
      deadOver12Sample: deadOver12.slice(0, 3),
      reversals: reversals.length,
      overlaps: overlaps.length,
      overlapMinFt: overlaps.length ? Math.min(...overlaps.map((o) => o.ft)) : null,
      overlapSamples: overlaps.slice(0, 5).map((o) => ({ t: o.t, a: `${o.a}[${o.aAct}]`, b: `${o.b}[${o.bAct}]`, ft: +o.ft.toFixed(2) })),
      perPlayer: kinOut,
    },
    ball: {
      heldMismatch,
      heldMismatchSamples,
      passFlight: { n: passDurs.length, durP50: +pct(pd, 0.5).toFixed(2), durP95: +pct(pd, 0.95).toFixed(2), speedP50: +pct(ps, 0.5).toFixed(1), speedP95: +pct(ps, 0.95).toFixed(1) },
      shotFlight: { n: shotDurs.length, durP50: +pct(sd, 0.5).toFixed(2), durP95: +pct(sd, 0.95).toFixed(2), speedP50: +pct(ss, 0.5).toFixed(1) },
      shotFlight3: { n: shotDurs3.length, durP50: +pct(s3, 0.5).toFixed(2), durP95: +pct(s3, 0.95).toFixed(2) },
      ballMaxTickPassFtps: +ballMaxTickPass.toFixed(1),
      ballMaxTickShotFtps: +ballMaxTickShot.toFixed(1),
    },
    events: ecs,
    order: { foulAfterMake, foulAfterResult },
    scvCtx: scvCtx.slice(0, 6),
    defense: {
      onBallP50: hd.length ? +pct(hd, 0.5).toFixed(1) : null,
      onBallP95: hd.length ? +pct(hd, 0.95).toFixed(1) : null,
      assignments: defOut,
    },
    coverage: Object.fromEntries(
      Object.entries(coverage).map(([k, v]) => [k, { n: v.n, meanGapFt: +(v.sum / v.n).toFixed(1), maxGapFt: +v.max.toFixed(1) }]),
    ),
    coverageFlips,
    topScorers: [...r.box_score.home, ...r.box_score.away]
      .map((p) => ({ jersey: p.jersey, points: p.points, tpm: p.tpm, tpa: p.tpa }))
      .sort((a, b) => b.points - a.points)
      .slice(0, 5),
    shotZones,
  };
}

function main(): void {
  const argv = process.argv.slice(2);
  const seedIdx = argv.indexOf('--seeds');
  const seeds = seedIdx >= 0
    ? argv[seedIdx + 1]!.split(',').map(Number).filter(Number.isInteger)
    : [42, 5, 7];
  const outIdx = argv.indexOf('--out');
  const outPath = outIdx >= 0 ? argv[outIdx + 1]! : '.omo/evidence/frame-deep-audit.json';
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const configPath = configValue !== undefined ? configValue : 'config/demo-game.json';
  const cfg = JSON.parse(readFileSync(resolve(process.cwd(), configPath), 'utf8')) as unknown as ConfigShape;
  const out: Record<string, unknown> = {};
  for (const s of seeds) out[String(s)] = analyze(s, cfg);
  const absOut = resolve(process.cwd(), outPath);
  mkdirSync(dirname(absOut), { recursive: true });
  writeFileSync(absOut, JSON.stringify(out, null, 1), 'utf-8');
  console.log(`frame-deep-audit: wrote ${absOut} (${seeds.join(',')})`);
}

main();
