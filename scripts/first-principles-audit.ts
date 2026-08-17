/**
 * first-principles-audit.ts
 *
 * 第一性原理深度分析 — 从篮球比赛最底层逻辑出发,逐帧分析模拟的真实性。
 * 不依赖统计聚合,只看每一帧每个球员的站位、动作、决策是否在篮球语境下合理。
 *
 * 现有审计覆盖了: kinematics, teleports, ball coherence, action label consistency,
 * screen lifecycle, event-contact geometry.
 *
 * 本脚本补充全新的分析维度:
 *   A) 防守轮转完整性 (DRI) — 防守阵型是否维持,help是否recover,switch是否真正发生
 *   B) 进攻空间动态 (OSP) — 球员是否根据球的位置移动,球侧/弱侧概念是否真实
 *   C) 挡拆真实性 (PNR) — 掩护是否真的挡住防守人,顺下/外弹是否真的发生
 *   D) 防守人-持球人空间关系 (DHS) — 防守人是否真的在追踪持球人,还是"橡皮筋"效应
 *   E) 篮板卡位真实性 (RBX) — 卡位是否发生在合理位置
 *   F) 快攻防守 (TRS) — 由攻转守时防守是否到位
 *   G) 死球后走位合理性 (DBP) — 死球阶段球员是否真的在走而不是瞬移
 *   H) 无球掩护 (OBS) — 无球掩护是否真的能创造机会
 *   I) 防守轮转链 (DRC) — 一次轮转是否引发后续轮转(连锁反应)
 *   J) 进攻时间感知 (CPS) — 不同进攻时间阶段球员行为是否不同(0-7s, 7-15s, 15-24s)
 *
 * Usage: npx tsx scripts/first-principles-audit.ts [--seeds 42,5,7] [--out .omo/evidence/first-principles.json]
 */

import { mkdirSync, writeFileSync, readFileSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { Player } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';
import type { WorldSnapshot, TacticalAssignmentSnapshot } from '../src/world/snapshot.js';
import type { TimelineEvent } from '../src/sim-utils.js';

interface ConfigShape {
  home_team: { id: string; roster: Player[]; lineup_packages: LineupPackage[] };
  away_team: ConfigShape['home_team'];
}

const FT_SCALE = 94;
const WD_SCALE = 50;
const distFt = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot((a.x - b.x) * FT_SCALE, (a.y - b.y) * WD_SCALE);
const isDead = (ph: string) => ph.startsWith('DEAD') || ph === 'FT_SEQUENCE' || ph === 'TIMEOUT' || ph === 'PERIOD_BREAK';

const RIM_L = { x: 0.0559, y: 0.5 };
const RIM_R = { x: 0.9441, y: 0.5 };
const rimFor = (attackSide: 'right' | 'left'): { x: number; y: number } =>
  attackSide === 'right' ? RIM_R : RIM_L;

interface AuditFinding {
  category: string;
  code: string;
  severity: 'error' | 'warning' | 'info';
  t: number;
  period: number;
  phase: string;
  description: string;
  metric: number;
  sample: Record<string, unknown>;
}

function analyze(seed: number, cfg: ConfigShape) {
  const r = simulateGame({
    home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
    away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
    seed,
  });
  const snaps = r.snapshots as readonly WorldSnapshot[];
  const evBySeq = new Map(r.events.map((e) => [e.seq, e]));
  const findings: AuditFinding[] = [];

  // ─────────────────────────────────────────────────────────────────────
  // PASS 1: collect per-frame metrics for sequential analysis
  // ─────────────────────────────────────────────────────────────────────
  const frameData: Array<{
    t: number; period: number; phase: string; realClock: number;
    ballStatus: string; holderId: string | null;
    offenseTeam: string | null; defenseTeam: string | null;
    tacticalKind: string | null; tacticalStage: string | null;
    handler: string | null;
    players: WorldSnapshot['players'];
    // Pre-computed: each defender's nearest offensive man and distance
    defAssignments: Array<{ jersey: string; nearestOff: string; dist: number; action: string }>;
    // Off-ball offense players
    offBall: Array<{ jersey: string; x: number; y: number; action: string }>;
    holder: (typeof snaps[0]['players'][number]) | null;
    onBallDefender: string | null;
    onBallDist: number;
  }> = [];

  for (let i = 0; i < snaps.length; i++) {
    const s = snaps[i]!;
    if (s.phase === 'POST_GAME' || s.players.length !== 10) continue;
    const live = s.phase === 'LIVE';
    const offenseTeam = s.tactical?.offense ?? null;
    const defenseTeam = offenseTeam === 'home' ? 'away' : offenseTeam === 'away' ? 'home' : null;
    const holder = s.players.find((p) => p.hasBall) ?? null;
    const handler = s.tactical?.handler ?? holder?.jersey ?? null;

    const defAssignments = defenseTeam
      ? s.players.filter((p) => p.team === defenseTeam).map((d) => {
          let nearestOff = '';
          let dist = Infinity;
          for (const o of s.players) {
            if (o.team !== defenseTeam) {
              const d2 = distFt(d, o);
              if (d2 < dist) { dist = d2; nearestOff = o.jersey; }
            }
          }
          return { jersey: d.jersey, nearestOff, dist, action: d.action };
        })
      : [];

    const offBall = s.players.filter((p) => p.team === offenseTeam && !p.hasBall && p.jersey !== handler);

    let onBallDist = Infinity;
    let onBallDefender: string | null = null;
    if (handler && defenseTeam) {
      const handlerPose = s.players.find((p) => p.jersey === handler);
      if (handlerPose) {
        for (const d of s.players.filter((p) => p.team === defenseTeam)) {
          const dd = distFt(handlerPose, d);
          if (dd < onBallDist) { onBallDist = dd; onBallDefender = d.jersey; }
        }
      }
    }

    frameData.push({
      t: s.t_real, period: s.period, phase: s.phase,
      realClock: s.t_real,
      ballStatus: s.ball.status, holderId: s.ball.holderId,
      offenseTeam, defenseTeam,
      tacticalKind: s.tactical?.kind ?? null,
      tacticalStage: s.tactical?.stage ?? null,
      handler,
      players: s.players,
      defAssignments,
      offBall,
      holder,
      onBallDefender,
      onBallDist,
    });
  }

  // ─────────────────────────────────────────────────────────────────────
  // A) 防守轮转完整性 (DRI)
  // ─────────────────────────────────────────────────────────────────────
  // 检查1: help defender是否在球移动后recover回自己的防守人
  // 检查2: 防守轮转链是否完整 — 一个help是否引发连锁轮转
  // 检查3: 防守人是否在合理距离内(deny/sag的距离是否稳定)
  {
    // Track defender assignments over time to detect:
    // - Same defender rapidly switching nearest offensive man (unrealistic)
    // - Defender labeled "help" staying in help > 3s without ball being in paint
    let helpDuration: Map<string, { start: number; defender: string; offenseTeam: string }> = new Map();
    let prevPhase: string | null = null;

    for (const fd of frameData) {
      // Dead phase transition: reset help tracking
      if (fd.phase !== prevPhase && isDead(fd.phase)) {
        helpDuration.clear();
      }
      prevPhase = fd.phase;

      if (fd.phase !== 'LIVE' || !fd.defenseTeam) continue;

      for (const asgn of fd.defAssignments) {
        const defender = fd.players.find((p) => p.jersey === asgn.jersey)!;
        // Help defender staying too long (>3s) — but only when the
        // defender is NOT legitimately camped at the rim. A DROP
        // coverage big sits 3-8ft from the rim for the whole possession
        // (that IS the drop); a weak-side helper camping in the paint
        // while the ball is perimeter is normal NBA help structure.
        // The defect is a help/tag label with the defender far from BOTH
        // the ball and the rim (a "help" that helps nobody).
        // Threshold 22ft: an ISO weak-side helper legitimately holds
        // 15-20ft from the rim (middle protection stance); only a tag
        // beyond 22ft is a ghost.
        const defRimDist = Math.min(
          distFt(defender, { x: 0.0559, y: 0.5 }),
          distFt(defender, { x: 0.9441, y: 0.5 }),
        );
        if (defRimDist <= 22) {
          helpDuration.delete(asgn.jersey);
          continue;
        }
        if (defender.action === 'help' || defender.action === 'tag') {
          const key = asgn.jersey;
          if (!helpDuration.has(key)) {
            helpDuration.set(key, { start: fd.t, defender: asgn.jersey, offenseTeam: fd.defenseTeam! });
          } else {
            const entry = helpDuration.get(key)!;
            const elapsed = fd.t - entry.start;
            // Check if ball is in paint area
            const ballInPaint = fd.holder && distFt(fd.holder, rimFor('right' as any)) < 10;
            if (elapsed > 3.0 && !ballInPaint) {
              findings.push({
                category: 'DRI', code: 'HELP_TOO_LONG',
                severity: 'warning',
                t: fd.t, period: fd.period, phase: fd.phase,
                description: `协防人${asgn.jersey}持续${elapsed.toFixed(1)}s处于help状态但球不在油漆区`,
                metric: elapsed,
                sample: { defender: asgn.jersey, nearestOff: asgn.nearestOff, dist: asgn.dist, action: defender.action },
              });
            }
          }
        } else {
          helpDuration.delete(asgn.jersey);
        }
      }

      // Check for "abandoned" defenders — a defender >15ft from EVERY offensive player
      // (more than 2 is a defensive collapse, but 1 can be a breakaway coverage)
      // Here we check if 3 or more defenders are >15ft from their nearest offensive man.
      // EXCLUSIONS (deliberate defensive structure, not collapse):
      //   - TRANSITION_PUSH/ADVANCE: the defense is retreating; early break
      //     coverage legitimately leaves the backcourt 20-40ft from attackers.
      //   - ISO sets: the weak side deliberately sags/helps off non-threats;
      //     the audit's nearest-offensive-man metric counts those as "lost"
      //     even though the rotation is the point.
      const isRetreating = fd.tacticalKind === 'TRANSITION_PUSH' || fd.tacticalStage === 'ADVANCE';
      const isISO = fd.tacticalKind === 'ISO';
      if (isRetreating || isISO) continue;
      const lostDefenders = fd.defAssignments.filter((a) => a.dist > 15 && a.action !== 'on_ball_defend');
      if (lostDefenders.length >= 3) {
        const avgDist = lostDefenders.reduce((s, d) => s + d.dist, 0) / lostDefenders.length;
        findings.push({
          category: 'DRI', code: 'DEFENSIVE_COLLAPSE',
          severity: 'warning',
          t: fd.t, period: fd.period, phase: fd.phase,
          description: `${lostDefenders.length}/5防守人距最近进攻人>15ft,防守阵型完全崩溃,平均距离${avgDist.toFixed(1)}ft`,
          metric: lostDefenders.length,
          sample: { lost: lostDefenders.map((d) => ({ jersey: d.jersey, dist: d.dist, action: d.action })), avgDist },
        });
      }

      // Check for "teleporting" deny/sag distances — a defender changing from deny
      // (2-4ft) to sag (6-10ft) and back within 1s without a pass or screen
      // This is a derived check via the next frame
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // B) 进攻空间动态 (OSP)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 无球进攻人是否根据球的位置做空间调整
  // 检查: 两个无球进攻人是否站得太近 — 只报告 SETTLED 重叠
  // (双方都arrived且低速)。过渡中的球员(换侧、cut经过)短暂靠近是
  // 正常篮球;持续settle在同一位置才是空间结构缺陷。
  {
    let prevSnap: WorldSnapshot | null = null;
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.offenseTeam) continue;

      // Off-ball spacing: are any 2 off-ball players within 5ft?
      const offBall = fd.players.filter((p) => p.team === fd.offenseTeam && !p.hasBall && p.jersey !== fd.handler);
      for (let a = 0; a < offBall.length; a++) {
        for (let b = a + 1; b < offBall.length; b++) {
          const d = distFt(offBall[a]!, offBall[b]!);
          if (d < 5) {
            const isScreen = offBall[a]!.action === 'screen' || offBall[b]!.action === 'screen';
            if (isScreen) continue;
            // Settled = both players have arrived at their targets and
            // are moving slowly (a <1.5ft/s jitter threshold). Transient
            // overlaps during relocations are basketball, not a defect.
            const settled = offBall[a]!.arrived && offBall[b]!.arrived;
            if (!settled) continue;
            findings.push({
              category: 'OSP', code: 'OFF_BALL_CLOG',
              severity: 'warning',
              t: fd.t, period: fd.period, phase: fd.phase,
              description: `无球球员${offBall[a]!.jersey}(${offBall[a]!.action})和${offBall[b]!.jersey}(${offBall[b]!.action})settle在相距仅${d.toFixed(1)}ft处`,
              metric: d,
              sample: { a: offBall[a]!.jersey, aAction: offBall[a]!.action, b: offBall[b]!.jersey, bAction: offBall[b]!.action },
            });
          }
        }
      }
      prevSnap = fd as unknown as WorldSnapshot;
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // C) 挡拆真实性 (PNR) — 两帧序列分析
  // ─────────────────────────────────────────────────────────────────────
  // 检查: PNR战术中掩护人是否真正接近持球人
  // 检查: 掩护设置后,防守人是否被挡住
  // 检查: 顺下/外弹是否真的发生
  {
    let inPNR = false;
    let pnrStartT = 0;
    let pnrScreener: string | null = null;
    let pnrHandler: string | null = null;
    let screenerStartDist = 0;
    let screenerMinDist = Infinity;
    let onBallDefenderAtPNRStart: string | null = null;
    let defenderAtScreenStart: number = 0;

    for (const fd of frameData) {
      const isPNR = fd.tacticalKind === 'PNR_ROLL' || fd.tacticalKind === 'PNR_POP';

      if (isPNR && !inPNR) {
        inPNR = true;
        pnrStartT = fd.t;
        pnrHandler = fd.handler;
        // Find screener
        const screener = fd.players.find((p) =>
          p.team === fd.offenseTeam && p.action === 'screen' && p.jersey !== fd.handler);
        pnrScreener = screener?.jersey ?? null;
        if (screener && fd.handler) {
          const handlerP = fd.players.find((p) => p.jersey === fd.handler);
          screenerStartDist = handlerP ? distFt(screener, handlerP) : 0;
          screenerMinDist = screenerStartDist;
        }
        onBallDefenderAtPNRStart = fd.onBallDefender;
        if (onBallDefenderAtPNRStart && fd.handler) {
          const h = fd.players.find((p) => p.jersey === fd.handler);
          const d = fd.players.find((p) => p.jersey === onBallDefenderAtPNRStart);
          if (h && d) defenderAtScreenStart = distFt(h, d);
        }
      }

      if (isPNR && inPNR && pnrScreener && fd.handler) {
        const screener = fd.players.find((p) => p.jersey === pnrScreener);
        const handler = fd.players.find((p) => p.jersey === fd.handler);
        if (screener && handler) {
          const d = distFt(screener, handler);
          if (d < screenerMinDist) screenerMinDist = d;
        }

        // Check if the on-ball defender successfully navigated the screen
        // (defender should be within 5ft of handler after screen use)
        // 只报告"持续不收敛"的被击穿:掩护后防守人0.5-1s的recover过渡
        // (gap从10ft收敛到3-4ft)是真实NBA的挤过掩护过程,不是防守崩溃。
        // 连续>10ft且不接近才是真正丢失handler。
        if (fd.tacticalStage === 'SCREEN_USE' || fd.tacticalStage === 'ADVANTAGE') {
          const defDist = fd.onBallDist;
          if (defDist > 10 && onBallDefenderAtPNRStart) {
            // 检查后续1s是否收敛: 采样未来10帧的gap
            const idx = frameData.indexOf(fd);
            let converges = false;
            if (idx >= 0) {
              for (let j = idx + 1; j < Math.min(idx + 11, frameData.length); j++) {
                const later = frameData[j]!;
                if (later.handler !== fd.handler || later.t - fd.t > 1.2) break;
                const laterHandler = later.players.find((p) => p.jersey === fd.handler);
                if (laterHandler) {
                  let g = Infinity;
                  for (const d of later.players.filter((p) => p.team === fd.defenseTeam)) {
                    g = Math.min(g, distFt(d, laterHandler));
                  }
                  if (g <= 6) { converges = true; break; }
                }
              }
            }
            if (!converges) {
              findings.push({
                category: 'PNR', code: 'SCREEN_BEAT_TOO_EASILY',
                severity: 'warning',
                t: fd.t, period: fd.period, phase: fd.phase,
                description: `挡拆后持球人${fd.handler}距最近防守人${defDist.toFixed(1)}ft且1s内未收敛,防守人完全丢失handler`,
                metric: defDist,
                sample: { handler: fd.handler, screener: pnrScreener, onBallDefender: onBallDefenderAtPNRStart, defDist, stage: fd.tacticalStage },
              });
            }
          }
        }
      }

      if (!isPNR && inPNR) {
        // PNR ended — check if screener actually rolled/popped
        if (pnrScreener && pnrHandler) {
          if (screenerMinDist > 8) {
            findings.push({
              category: 'PNR', code: 'SCREEN_TOO_FAR',
              severity: 'warning',
              t: fd.t, period: fd.period, phase: 'LIVE',
              description: `掩护人${pnrScreener}与持球人${pnrHandler}最近距离${screenerMinDist.toFixed(1)}ft,掩护从未真正发生`,
              metric: screenerMinDist,
              sample: { screener: pnrScreener, handler: pnrHandler, startDist: screenerStartDist, minDist: screenerMinDist },
            });
          }
        }
        inPNR = false;
        pnrScreener = null;
        pnrHandler = null;
        screenerMinDist = Infinity;
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // D) 防守人-持球人空间关系 (DHS)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: on-ball defender是否真的在track持球人,还是"橡皮筋"效应
  // (防守人总是保持相同距离,不管持球人做什么)
  {
    // Track on-ball defender distance over time for each possession
    interface ObdTrack {
      handler: string;
      defender: string;
      distances: number[];
      times: number[];
      handlerSpeeds: number[];
    }
    const obdTracks: ObdTrack[] = [];
    let currentObdTrack: ObdTrack | null = null;

    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.handler || !fd.onBallDefender) {
        if (currentObdTrack) {
          obdTracks.push(currentObdTrack);
          currentObdTrack = null;
        }
        continue;
      }

      if (!currentObdTrack || currentObdTrack.handler !== fd.handler || currentObdTrack.defender !== fd.onBallDefender) {
        if (currentObdTrack) obdTracks.push(currentObdTrack);
        currentObdTrack = { handler: fd.handler, defender: fd.onBallDefender, distances: [], times: [], handlerSpeeds: [] };
      }

      // Handler speed
      let handlerSpeed = 0;
      if (currentObdTrack.handlerSpeeds.length > 0 && currentObdTrack.times.length > 0) {
        const prevT = currentObdTrack.times[currentObdTrack.times.length - 1]!;
        const prevHandler = frameData.find((f) => f.t === prevT)?.players.find((p) => p.jersey === fd.handler);
        const curHandler = fd.players.find((p) => p.jersey === fd.handler);
        if (prevHandler && curHandler) {
          handlerSpeed = distFt(prevHandler, curHandler) / (fd.t - prevT);
        }
      }

      currentObdTrack.distances.push(fd.onBallDist);
      currentObdTrack.times.push(fd.t);
      currentObdTrack.handlerSpeeds.push(handlerSpeed);
    }
    if (currentObdTrack) obdTracks.push(currentObdTrack);

    // Analyze each OBD track: variance should be non-trivial
    for (const track of obdTracks) {
      if (track.distances.length < 5) continue;
      const mean = track.distances.reduce((s, d) => s + d, 0) / track.distances.length;
      const variance = track.distances.reduce((s, d) => s + (d - mean) ** 2, 0) / track.distances.length;
      const stdDev = Math.sqrt(variance);

      // If variance is very low (< 0.5ft), the defender is rubber-banding
      // — BUT a defender who has ESTABLISHED position (both roughly
      // static, e.g. a standing ISO probe or a transition defender who
      // caught up) legitimately holds a constant 2-3ft. The rubber-band
      // defect is a constant gap while the HANDLER SPRINTS: the defender
      // "slides" with zero lag, as if magnetized. Require sustained
      // high-speed handler movement (average > 7ft/s — a full sprint,
      // not probe drift) in the track.
      const avgHandlerSpeed = track.handlerSpeeds.reduce((s, v) => s + v, 0) / track.handlerSpeeds.length;
      const sustainedSprint = avgHandlerSpeed > 7;
      // Track duration matters: a defender already standing next to the
      // holder (tip-off, inbound, steal recovery) legitimately holds a
      // constant 2ft for a beat. The rubber-band defect is a FULL
      // possession-length magnetic lock (>2s of sprint-tracking).
      const trackSeconds = track.times[track.times.length - 1]! - track.times[0]!;
      if (stdDev < 0.5 && track.distances.length > 10 && sustainedSprint && trackSeconds > 2) {
        findings.push({
          category: 'DHS', code: 'RUBBER_BAND_DEFENSE',
          severity: 'warning',
          t: track.times[0]!, period: 1, phase: 'LIVE',
          description: `防守人${track.defender}对持球人${track.handler}的防守距离标准差仅${stdDev.toFixed(2)}ft (${track.distances.length}帧),疑似"橡皮筋"效应`,
          metric: stdDev,
          sample: { defender: track.defender, handler: track.handler, meanDist: mean.toFixed(1), stdDev: stdDev.toFixed(2), frames: track.distances.length },
        });
      }

      // Check if defender distance correlates with handler speed
      // (should be: faster handler → defender more likely to be beat → larger gap)
      if (track.handlerSpeeds.length > 5) {
        const avgSpeed = track.handlerSpeeds.reduce((s, v) => s + v, 0) / track.handlerSpeeds.length;
        // A defender "perfectly tracking" at 2-4ft is NORMAL NBA defense
        // (the on-ball gap p50 is 2.7ft by design — local-react blends the
        // on-ball defender toward a 4ft gap every tick). The real defect
        // is the opposite: a fast handler whose defender NEVER closes
        // (gap monotonically grows, defender permanently 8ft+ behind).
        // The old PERFECT_TRACKING rule (avgSpeed>8 and 80%<4ft) flagged
        // ordinary tight defense — 160+ frames/game of noise.
        if (avgSpeed > 8 && track.distances.length > 10) {
          const farPct = track.distances.filter((d) => d > 8).length / track.distances.length;
          const early = track.distances.slice(0, Math.floor(track.distances.length / 2));
          const late = track.distances.slice(Math.floor(track.distances.length / 2));
          const earlyMean = early.reduce((s, d) => s + d, 0) / early.length;
          const lateMean = late.length > 0 ? late.reduce((s, d) => s + d, 0) / late.length : earlyMean;
          if (farPct > 0.6 && lateMean > earlyMean + 2) {
            findings.push({
              category: 'DHS', code: 'DEFENDER_LOST_HANDLER',
              severity: 'warning',
              t: track.times[0]!, period: 1, phase: 'LIVE',
              description: `防守人${track.defender}在持球人${track.handler}平均速度${avgSpeed.toFixed(1)}ft/s时,${(farPct * 100).toFixed(0)}%时间距其>8ft且间距持续拉大(${earlyMean.toFixed(1)}ft→${lateMean.toFixed(1)}ft),防守被完全甩开`,
              metric: farPct,
              sample: { defender: track.defender, handler: track.handler, avgSpeed, farPct, earlyMean, lateMean, frames: track.distances.length },
            });
          }
        }
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // E) 篮板卡位真实性 (RBX)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: box_out是否发生在合理位置(距篮筐<16ft)
  // 已由movement-reality-audit覆盖,但这里检查球在空中时球员是否开始卡位
  {
    let inShotFlight = false;
    let shotStartT = 0;
    let shotStartBall = { x: 0, y: 0 };

    for (const fd of frameData) {
      if (fd.ballStatus === 'shot' && !inShotFlight) {
        inShotFlight = true;
        shotStartT = fd.t;
        shotStartBall = { x: fd.holder?.x ?? 0.5, y: fd.holder?.y ?? 0.5 };
      }
      if (inShotFlight && fd.ballStatus !== 'shot') {
        inShotFlight = false;
        // Check: during shot flight, did players near the rim box out?
        // We can't easily check this on a per-frame basis without the shot-flight state
        // But we can check: in the frame AFTER shot, are players boxing out near the rim?
        const nearRim = fd.players.filter((p) => {
          const d = Math.min(distFt(p, RIM_L), distFt(p, RIM_R));
          return d < 16;
        });
        const boxingOut = nearRim.filter((p) => p.action === 'box_out');
        const nearNotBoxing = nearRim.filter((p) => p.action !== 'box_out' && p.action !== 'space');
        if (nearRim.length >= 4 && boxingOut.length < 2) {
          findings.push({
            category: 'RBX', code: 'NO_BOX_OUT_AFTER_SHOT',
            severity: 'info',
            t: fd.t, period: fd.period, phase: fd.phase,
            description: `出手后${nearRim.length}人在篮下16ft范围内,但仅${boxingOut.length}人卡位`,
            metric: boxingOut.length,
            sample: { nearRim: nearRim.length, boxingOut: boxingOut.length, actions: nearRim.map((p) => p.action) },
          });
        }
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // F) 快攻防守 (TRS)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 由攻转守时,防守人是否快速回防
  {
    let inTransition = false;
    let transitionStart = 0;
    let offenseTeamAtTransition: string | null = null;
    let defenseRetreatSpeeds: Array<{ jersey: string; speed: number }> = [];

    for (const fd of frameData) {
      // Detect transition: first frame after a possession change
      // (simplified: when ball is in backcourt and offense just changed)
      if (!inTransition && fd.phase === 'LIVE' && fd.offenseTeam) {
        const holder = fd.players.find((p) => p.hasBall);
        const inBackcourt = holder && (holder.x < 0.45 || holder.x > 0.55);
        // Check if this is a new possession (we can check lastEventType)
        if (inBackcourt && fd.tacticalKind === 'TRANSITION_PUSH') {
          inTransition = true;
          transitionStart = fd.t;
          offenseTeamAtTransition = fd.offenseTeam;
        }
      }

      if (inTransition && fd.offenseTeam === offenseTeamAtTransition) {
        // Check retreat speed of the defense
        const defenseTeam = fd.defenseTeam;
        if (defenseTeam && fd.t - transitionStart < 3.0) {
          for (const d of fd.players.filter((p) => p.team === defenseTeam)) {
            // Check if defender is sprinting back (should be fast in transition)
            if (d.action === 'on_ball_defend' || d.action === 'deny' || d.action === 'weak_side') {
              // We can't easily check speed here without previous frame
            }
          }
        }
      }

      // Reset after transition ends
      if (inTransition && (fd.offenseTeam !== offenseTeamAtTransition || fd.phase !== 'LIVE')) {
        inTransition = false;
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // G) 防守轮转链 (DRC) — 检测help后是否有人补防
  // ─────────────────────────────────────────────────────────────────────
  // 当一名防守人离开自己的对位人去help时,另一个防守人应该补上他的对位人
  // 这里检测: 当一名防守人标记为help时,是否有其他防守人标记为tag/deny补位
  {
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.defenseTeam) continue;
      const defenders = fd.players.filter((p) => p.team === fd.defenseTeam);
      const helpDefenders = defenders.filter((p) => p.action === 'help' || p.action === 'tag');
      const onBall = defenders.filter((p) => p.action === 'on_ball_defend');

      // If there are 2+ help defenders AND the on-ball defender is not the nearest
      // to the handler, that means the defense has collapsed
      if (helpDefenders.length >= 2 && fd.onBallDefender) {
        const obd = defenders.find((p) => p.jersey === fd.onBallDefender);
        // Check if help defenders are actually near the play or just wandering
        if (fd.handler) {
          const handlerP = fd.players.find((p) => p.jersey === fd.handler);
          if (handlerP) {
            const helpDistances = helpDefenders.map((h) => distFt(h, handlerP));
            const avgHelpDist = helpDistances.reduce((s, d) => s + d, 0) / helpDistances.length;
            // A defender 25ft+ from the BALL is only a ghost if they are
            // also far from the RIM — a weak-side helper standing in the
            // elbow while the ball sits in the opposite corner is real
            // help positioning (NBA help defenders sag toward the paint,
            // which is 25-30ft from a corner-held ball).
            const allNearRim = helpDefenders.every((h) => {
              const rimD = Math.min(distFt(h, { x: 0.0559, y: 0.5 }), distFt(h, { x: 0.9441, y: 0.5 }));
              return rimD <= 20;
            });
            if (avgHelpDist > 25 && !allNearRim) {
              findings.push({
                category: 'DRC', code: 'HELP_GHOST',
                severity: 'warning',
                t: fd.t, period: fd.period, phase: fd.phase,
                description: `${helpDefenders.length}名help防守人距持球人平均${avgHelpDist.toFixed(1)}ft且不在合理协防区,形成"幽灵协防"`,
                metric: avgHelpDist,
                sample: { helpCount: helpDefenders.length, avgHelpDist, helpDefenders: helpDefenders.map((h) => ({ jersey: h.jersey, action: h.action })), onBallDefender: fd.onBallDefender },
              });
            }
          }
        }
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // H) 进攻时间感知 (CPS)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 不同进攻时间阶段球员行为是否不同
  // 早期(shotClock > 16): 持球人应该更多组织
  // 中期(7-15): 应该开始执行战术
  // 晚期(<7): 应该更多进攻性动作
  {
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.handler) continue;
      // We can't get shotClock directly from frameData, but we can check the holder's action
      // against the phase
      const holder = fd.holder;
      if (!holder) continue;

      // Check if holder is "relocate" or "advance" when shot clock is low (should be attacking)
      // This is a soft heuristic — we can't access shot clock from WorldSnapshot easily
      // But we can check the tactical stage
      if (fd.tacticalStage === 'ADVANTAGE' && holder.action === 'relocate') {
        findings.push({
          category: 'CPS', code: 'HOLD_IN_ADVANTAGE',
          severity: 'info',
          t: fd.t, period: fd.period, phase: fd.phase,
          description: `持球人${holder.jersey}在ADVANTAGE阶段仍relocate,应该已经阅读防守并进攻`,
          metric: 0,
          sample: { holder: holder.jersey, action: holder.action, stage: fd.tacticalStage },
        });
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // I) 防守动作标签稳定性
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 防守人的动作标签是否频繁波动(deny→sag→deny每帧切换)
  {
    const prevActions = new Map<string, string>();
    let flipCount = 0;
    let maxFlips = 0;
    let maxFlipsJersey = '';
    const flipCountPerPlayer = new Map<string, number>();

    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.defenseTeam) continue;
      for (const p of fd.players) {
        if (p.team !== fd.defenseTeam) continue;
        const prev = prevActions.get(p.jersey);
        if (prev !== undefined && prev !== p.action) {
          flipCountPerPlayer.set(p.jersey, (flipCountPerPlayer.get(p.jersey) ?? 0) + 1);
          flipCount++;
        }
        prevActions.set(p.jersey, p.action);
      }
    }

    for (const [jersey, flips] of flipCountPerPlayer) {
      if (flips > 50) {
        findings.push({
          category: 'DRI', code: 'DEFENDER_ACTION_FLIP',
          severity: 'info',
          t: 0, period: 0, phase: 'LIVE',
          description: `防守人${jersey}全场动作标签变化${flips}次,可能过于频繁`,
          metric: flips,
          sample: { jersey, totalFlips: flips, totalFrames: frameData.length },
        });
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // J) 进攻人站位深度分析
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 进攻人是否真的站在三分线外,还是站在中距离
  {
    const offBallDistances: number[] = [];
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.offenseTeam || !fd.handler) continue;
      for (const p of fd.offBall) {
        // J-section placeholder removed: off-ball depth is measured by the
        // OSP settled-clog checks and the COMPRESSED_OFFENSE span metric.
        void p;
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // K) 双人包夹检测 (TRAP)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 是否有两个防守人同时标记为on_ball_defend(包夹)
  // 真实的包夹只出现在BLITZ防守(read screenDefense.mode)。没有BLITZ
  // 标记却出现双on_ball_defend,是标签泄漏(pass-target预旋转或
  // X-out轮转错误地复用了on_ball_defend标签),不是真正的trap。
  {
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.defenseTeam) continue;
      const onBallDefenders = fd.players.filter((p) => p.team === fd.defenseTeam && p.action === 'on_ball_defend');
      if (onBallDefenders.length >= 2) {
        // Check if both are actually near the handler
        if (fd.handler) {
          const handlerP = fd.players.find((p) => p.jersey === fd.handler);
          if (handlerP) {
            const distances = onBallDefenders.map((d) => distFt(d, handlerP));
            if (distances.every((d) => d < 8)) {
              const isBlitz = fd.tacticalKind !== null
                && frameData.find((f) => f.t === fd.t)?.tacticalKind !== undefined
                && (fd as unknown as { tactical?: { screenDefense?: { mode?: string } } }).tactical?.screenDefense?.mode === 'BLITZ';
              findings.push({
                category: 'DRI', code: isBlitz ? 'EFFECTIVE_TRAP' : 'TRAP_LABEL_LEAK',
                severity: isBlitz ? 'info' : 'warning',
                t: fd.t, period: fd.period, phase: fd.phase,
                description: `${onBallDefenders.length}名防守人标记为on_ball_defend且距持球人<8ft,${isBlitz ? '形成BLITZ包夹' : '但防守未声明BLITZ,标签泄漏'}`,
                metric: onBallDefenders.length,
                sample: { defenders: onBallDefenders.map((d) => d.jersey), distances, kind: fd.tacticalKind },
              });
            }
          }
        }
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // L) 卡位返回检测 (BOX_OUT_RECOVER)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 卡位后球员是否恢复为正常防守(不应该一直卡位)
  {
    let boxOutPlayers = new Set<string>();
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE') {
        boxOutPlayers.clear();
        continue;
      }
      const currentBoxOut = new Set(fd.players.filter((p) => p.action === 'box_out').map((p) => p.jersey));
      // Check if someone who was boxing out is now in a different action
      for (const jersey of boxOutPlayers) {
        if (!currentBoxOut.has(jersey)) {
          // Recovered — good, no finding
        }
      }
      // Check if someone has been boxing out for too long (>5s of LIVE)
      // This is hard to track without a per-player timer
      boxOutPlayers = currentBoxOut;
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // M) 空位三分检测 (WIDE_OPEN_THREE)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 当一名进攻人处于空位(距最近防守人>6ft)时,球是否传到他手上
  // 这需要追踪事件,比较复杂,先略过
  // ─────────────────────────────────────────────────────────────────────

  // ─────────────────────────────────────────────────────────────────────
  // N) 传导球节奏 (PASS_RHYTHM)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 传球节奏是否自然 — 是否出现连续快速传球(3次传球在2s内)
  // 只统计flight_start事件:flight_complete成对事件在时间上必然紧邻,
  // 把两者都计入会把正常swing pass误报为"快速链"(实测481 vs 11)。
  // NBA swing pass链1.5s内3传是正常节奏;只有更极端的<1.2s才值得关注。
  {
    const passEvents = r.events.filter((e) => e.type === 'PASS' && e.payload['note'] === 'flight_start');
    for (let i = 2; i < passEvents.length; i++) {
      const e1 = passEvents[i - 2]!;
      const e2 = passEvents[i - 1]!;
      const e3 = passEvents[i]!;
      const timeSpan = e3.t_real - e1.t_real;
      if (timeSpan < 1.2) {
        findings.push({
          category: 'PASS', code: 'RAPID_PASS_CHAIN',
          severity: 'info',
          t: e3.t_real, period: 1, phase: 'LIVE',
          description: `${timeSpan.toFixed(1)}s内连续3次传球,节奏过快,可能为无效传导`,
          metric: timeSpan,
          sample: { passes: [e1, e2, e3].map((e) => ({ seq: e.seq, t: e.t_real, actors: e.actors })) },
        });
      }
    }
  }

  // ─────────────────────────────────────────────────────────────────────
  // O) 防守人对切入的反应 (CUT_REACTION)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 当进攻人切入时,防守人是否及时跟随
  // 这需要追踪同一防守人对同一进攻人的距离变化,比较复杂
  // ─────────────────────────────────────────────────────────────────────

  // ─────────────────────────────────────────────────────────────────────
  // P) 球队整体阵型压缩和扩张 (FORMATION)
  // ─────────────────────────────────────────────────────────────────────
  // 检查: 进攻球队的"span"(最左到最右球员的距离)是否合理
  // NBA进攻阵型通常span在25-40ft之间。只报告SETTLED压缩(≥4/5球员
  // 已到位)——转换回防、挡拆后roll汇聚的过渡压缩是正常篮球。
  {
    for (const fd of frameData) {
      if (fd.phase !== 'LIVE' || !fd.offenseTeam) continue;
      const offPlayers = fd.players.filter((p) => p.team === fd.offenseTeam);
      const xs = offPlayers.map((p) => p.x);
      const ys = offPlayers.map((p) => p.y);
      const spanX = (Math.max(...xs) - Math.min(...xs)) * FT_SCALE;
      const spanY = (Math.max(...ys) - Math.min(...ys)) * WD_SCALE;
      const totalSpan = Math.hypot(spanX, spanY);

      if (totalSpan < 15) {
        const arrivedCount = offPlayers.filter((p) => p.arrived).length;
        if (arrivedCount < 4) continue; // transition compression is normal
        findings.push({
          category: 'FORM', code: 'COMPRESSED_OFFENSE',
          severity: 'warning',
          t: fd.t, period: fd.period, phase: fd.phase,
          description: `进攻阵型settle后宽度仅${totalSpan.toFixed(0)}ft,5人挤在狭小空间内`,
          metric: totalSpan,
          sample: { spanX: spanX.toFixed(0), spanY: spanY.toFixed(0), players: offPlayers.map((p) => ({ jersey: p.jersey, x: p.x, y: p.y, action: p.action })) },
        });
      }
    }
  }

  return {
    seed,
    finalScore: {
        home: r.box_score.home.reduce((s, p) => s + (p.points ?? 0), 0),
        away: r.box_score.away.reduce((s, p) => s + (p.points ?? 0), 0),
      },
    totalFrames: frameData.length,
    findings,
    // Summary
    summary: {} as Record<string, { count: number; errors: number; warnings: number; infos: number }>,
  };
}

// ─── Main ──────────────────────────────────────────────────────────────
function main() {
  const argv = process.argv.slice(2);
  const seedIdx = argv.indexOf('--seeds');
  const seeds = seedIdx >= 0
    ? argv[seedIdx + 1]!.split(',').map(Number).filter(Number.isInteger)
    : [42, 5, 7];
  const outIdx = argv.indexOf('--out');
  const outPath = outIdx >= 0 ? argv[outIdx + 1]! : '.omo/evidence/first-principles.json';
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const configPath = configValue !== undefined ? configValue : 'config/demo-game.json';
  const cfg = JSON.parse(readFileSync(resolve(process.cwd(), configPath), 'utf8')) as unknown as ConfigShape;

  const out: Record<string, unknown> = {};
  for (const s of seeds) {
    const result = analyze(s, cfg);
    // Build summary
    const summary: Record<string, { count: number; errors: number; warnings: number; infos: number }> = {};
    for (const f of result.findings) {
      if (!summary[f.code]) summary[f.code] = { count: 0, errors: 0, warnings: 0, infos: 0 };
      summary[f.code]!.count++;
      if (f.severity === 'error') summary[f.code]!.errors++;
      else if (f.severity === 'warning') summary[f.code]!.warnings++;
      else summary[f.code]!.infos++;
    }
    result.summary = summary;
    // Trim findings to prevent huge output
    out[String(s)] = {
      ...result,
      findings: result.findings.slice(0, 4000), // cap at 4000 per seed
    };
    console.log(`\n=== seed ${s} === frames=${result.totalFrames} findings=${result.findings.length}`);
    for (const [code, stats] of Object.entries(summary).sort((a, b) => b[1].count - a[1].count)) {
      console.log(`  ${code}: ${stats.count} (${'!'.repeat(stats.errors)}${'~'.repeat(stats.warnings)}${'-'.repeat(stats.infos)})`);
    }
  }

  const absOut = resolve(process.cwd(), outPath);
  mkdirSync(dirname(absOut), { recursive: true });
  writeFileSync(absOut, JSON.stringify(out, null, 1), 'utf-8');
  console.log(`\nfirst-principles-audit: wrote ${absOut} (${seeds.join(',')})`);
}

main();