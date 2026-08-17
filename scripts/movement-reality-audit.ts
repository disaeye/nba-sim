/**
 * movement-reality-audit.ts
 *
 * 逐帧分析球员动作标签 (action) 与其实际跑位 (x, y, speed) 的一致性。
 * 不看统计聚合，只看每一帧每个球员"标签说的"和"实际做的"是否吻合。
 *
 * 输出: /tmp/movement-reality-<seed>.json
 */
import { readFileSync, writeFileSync } from 'node:fs';
import { simulateGame } from '../src/simulate.js';
import type { WorldSnapshot } from '../src/world/snapshot.js';

const FT = 94, WD = 50;
const RIM_L = { x: 0.0559, y: 0.5 };
const RIM_R = { x: 0.9441, y: 0.5 };
const dist = (ax: number, ay: number, bx: number, by: number) =>
  Math.hypot((ax - bx) * FT, (ay - by) * WD);

interface Case {
  t: number;
  period: number;
  phase: string;
  offenseTeam: string | null;
  rule: string;
  jersey: string;
  team: string;
  action: string;
  x: number; y: number;
  detail: string;
  metric: number;
}

function loadInput(seed: number) {
  const cfg = JSON.parse(readFileSync('config/generated-roster.json', 'utf8'));
  return {
    home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
    away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
    seed,
  };
}

function analyze(seed: number) {
  const result = simulateGame(loadInput(seed));
  const snaps = result.snapshots as readonly WorldSnapshot[];
  const cases: Case[] = [];

  for (let i = 1; i < snaps.length; i++) {
    const s = snaps[i]!;
    const prev = snaps[i - 1]!;
    const dt = (s.t_real - prev.t_real) || 0.1;
    const holder = s.players.find((p) => p.hasBall) ?? null;
    const offenseTeam = s.tactical?.offense ?? (holder?.team ?? null);
    const ballX = s.ball.x, ballY = s.ball.y;
    const isDead = s.phase.startsWith('DEAD') || s.phase === 'FT_SEQUENCE' || s.phase === 'TIMEOUT' || s.phase === 'PERIOD_BREAK';

    for (const p of s.players) {
      const prevP = prev.players.find((q) => q.jersey === p.jersey);
      const speed = prevP ? dist(prevP.x, prevP.y, p.x, p.y) / dt : 0;

      // Rule 1: on_ball_defend must be near ball — EXCEPT during transition
      // (holder advancing upcourt, defender sprinting back). The 36ft gap
      // during transition is correct recovery running, not a tracking
      // failure. Transition = holder in backcourt or tactical stage ADVANCE.
      if (p.action === 'on_ball_defend' && !isDead) {
        const d = dist(p.x, p.y, holder?.x ?? ballX, holder?.y ?? ballY);
        const stage = s.tactical?.stage ?? '';
        const isTransition = stage === 'ADVANCE' || stage === 'TRANSITION' || stage === 'PUSH'
          || (holder && (holder.x < 0.45 || holder.x > 0.55));
        const isRecovery = !s.tactical || s.ball.status !== 'held' || !holder;
        if (d > 8 && !isTransition && !isRecovery) {
          cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'OBD_FAR', jersey: p.jersey, team: p.team, action: p.action, x: p.x, y: p.y, detail: `贴身防守人距球 ${d.toFixed(1)}ft`, metric: d });
        }
      }

      // Rule 2: help defender must be near a rim
      if (p.action === 'help') {
        const nearRim = Math.min(dist(p.x, p.y, RIM_L.x, RIM_L.y), dist(p.x, p.y, RIM_R.x, RIM_R.y));
        if (nearRim > 18) {
          cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'HELP_FAR_FROM_RIM', jersey: p.jersey, team: p.team, action: p.action, x: p.x, y: p.y, detail: `协防人距最近篮筐 ${nearRim.toFixed(1)}ft`, metric: nearRim });
        }
      }

      // Rule 3: dead-ball phase sprinting
      if (isDead && speed > 8 && prevP) {
        cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'DEAD_SPRINT', jersey: p.jersey, team: p.team, action: p.action, x: p.x, y: p.y, detail: `死球阶段速度 ${speed.toFixed(1)}ft/s`, metric: speed });
      }

      // An advance has a declared 0.15s gather/windup. The first frame can
      // be nearly stationary while the handler loads the dribble; only flag
      // a stall once that physical action is progressing.
      const active = s.tactical?.activeAction;
      const inAdvanceWindup = active?.jersey === p.jersey
        && active.kind === 'advance'
        && active.stage === 'INITIATED'
        && active.elapsedSeconds < active.windupSeconds;
      if (p.hasBall && p.action === 'advance' && s.phase === 'LIVE' && speed < 1.5 && !inAdvanceWindup) {
        cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'ADVANCE_STALL', jersey: p.jersey, team: p.team, action: p.action, x: p.x, y: p.y, detail: `推进标签但速度仅 ${speed.toFixed(1)}ft/s`, metric: speed });
      }

      // Rule 5: box_out far from rim — only flag during LIVE play. During
      // dead-ball phases (FT setup, foul reset) players walk to spots and
      // the box_out label is a lineup task, not an active box-out action.
      if (p.action === 'box_out' && !isDead) {
        const nearRim = Math.min(dist(p.x, p.y, RIM_L.x, RIM_L.y), dist(p.x, p.y, RIM_R.x, RIM_R.y));
        if (nearRim > 16) {
          cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'BOXOUT_FAR_FROM_RIM', jersey: p.jersey, team: p.team, action: p.action, x: p.x, y: p.y, detail: `卡位但距篮筐 ${nearRim.toFixed(1)}ft`, metric: nearRim });
        }
      }
    }

    // Rule 6: two non-ball offense players within 4ft (clogging, excluding screen)
    if (s.phase === 'LIVE' && offenseTeam) {
      const off = s.players.filter((p) => p.team === offenseTeam);
      for (let a = 0; a < off.length; a++) {
        for (let b = a + 1; b < off.length; b++) {
          const first = off[a]!;
          const second = off[b]!;
          const d = dist(first.x, first.y, second.x, second.y);
          if (d < 4 && !first.hasBall && !second.hasBall) {
            const screening = first.action === 'screen' || second.action === 'screen';
            if (!screening) {
              cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'OFFENSE_CLOGGED', jersey: `${first.jersey}+${second.jersey}`, team: offenseTeam, action: `${first.action}/${second.action}`, x: (first.x + second.x) / 2, y: (first.y + second.y) / 2, detail: `两无球进攻人相距仅 ${d.toFixed(1)}ft`, metric: d });
            }
          }
        }
      }
    }

    // Rule 7: 3+ defenders >12ft from nearest offense man
    if (s.phase === 'LIVE' && offenseTeam) {
      const defTeam = offenseTeam === 'home' ? 'away' : 'home';
      const defenders = s.players.filter((p) => p.team === defTeam);
      const off = s.players.filter((p) => p.team === offenseTeam);
      const lost = defenders.filter((defender) => Math.min(...off.map((o) => dist(defender.x, defender.y, o.x, o.y))) > 12);
      if (lost.length >= 3) {
        const d = lost[0]!;
        const nearestOff = Math.min(...off.map((o) => dist(d.x, d.y, o.x, o.y)));
        cases.push({ t: s.t_real, period: s.period, phase: s.phase, offenseTeam, rule: 'DEFENSE_ABANDONED', jersey: d.jersey, team: d.team, action: d.action, x: d.x, y: d.y, detail: `${lost.length}/5 防守人距最近进攻人 >12ft (最近 ${nearestOff.toFixed(1)}ft)`, metric: nearestOff });
      }
    }
  }

  return { seed, totalFrames: snaps.length, totalCases: cases.length, cases };
}

const seeds = process.argv.slice(2).map(Number);
const runSeeds = seeds.length ? seeds : [5, 7, 42];
for (const seed of runSeeds) {
  const r = analyze(seed);
  writeFileSync(`/tmp/movement-reality-${seed}.json`, JSON.stringify(r, null, 2));
  const byRule = new Map<string, number>();
  for (const c of r.cases) byRule.set(c.rule, (byRule.get(c.rule) ?? 0) + 1);
  console.log(`\n=== seed ${seed} === frames=${r.totalFrames} cases=${r.totalCases}`);
  for (const [rule, n] of [...byRule.entries()].sort((a, b) => b[1] - a[1])) {
    console.log(`  ${rule}: ${n}`);
  }
}
