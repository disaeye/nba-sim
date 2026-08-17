/**
 * game-psychology-audit.ts
 *
 * 第二轮第一性原理审计 — 比赛过程的博弈性与叙事连贯性。
 * 不从统计聚合出发,而是逐帧验证"这场比赛像不像一场真的NBA比赛":
 *
 *   A) 节奏博弈 (TEMPO) — 领先/落后/胶着时,进攻时间使用是否不同?
 *      领先方是否压时间,落后方是否抢攻?(NBA: 末节领先>8分,进攻时间显著拉长)
 *   B) 关键时刻 (CLUTCH) — 比赛最后2分钟分差≤5时,是否有真实的博弈?
 *      犯规战术、3分追分、防守强度提升
 *   C) 防守阵型整体性 (SCHEME) — 换防后阵型是否整体移动?
 *      联防时5人是否同步滑步?人盯人时是否各自跟随?
 *   D) 换防持续性 (SWITCH_CONT) — 换防发生后,防守人是否保持换防
 *      直到下一个死球?(NBA: 换防后不立即换回)
 *   E) 出手决策过程 (PROCESS) — 每次出手前是否有合理的决策过程?
 *      (接球→观察→出手) vs (接球就投/持球硬投)
 *   F) 推进路线 (TRANSITION_PATH) — 快攻推进是否走中路而非边线?
 *      持球人是否优先中路分球?
 *   G) 攻防转换节奏 (REBOUND_OUT) — 防守篮板后是否立即推进?
 *      篮板→推进→前场的衔接是否连贯?
 *   H) 战术重复度 (TACTIC_REPEAT) — 同一战术是否连续重复3次以上?
 *      (NBA教练会变招;连续5次同一战术是机械)
 *   I) 球员倾向稳定性 (IDENTITY) — 同一球员的出手选择是否稳定?
 *      (射手不突然变突破手)
 *   J) 死球后防守布置 (DEADBALL_D) — 死球后防守是否重新落位?
 *      (发球前防守人是否回到对位人身边)
 *
 * Usage: npx tsx scripts/game-psychology-audit.ts [--seeds 42,5,7]
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

const FT = 94, WD = 50;
const distFt = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot((a.x - b.x) * FT, (a.y - b.y) * WD);

interface Finding {
  code: string;
  severity: 'error' | 'warning' | 'info';
  t: number;
  period: number;
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
  const events = r.events;
  const findings: Finding[] = [];
  const periodSeconds = (p: number) => (p <= 4 ? 720 : 300);

  // ───────────────────────────────────────────────────────────────────
  // A) 节奏博弈 — 领先/落后时的进攻时间使用
  // 计算每次出手时的game clock和score margin,按margin分组
  // ───────────────────────────────────────────────────────────────────
  {
    // 节奏博弈样本 = 每次POSSESSION_GAINED(发球/抢断后的新球权)后的
    // 首次出手。OREB续攻不产生POSSESSION_GAINED,天然排除;
    // 续攻段的late shot也不会污染领先方的平均出手时机。
    const gains = events.filter((e) => e.type === 'POSSESSION_GAINED');
    const shotClocks: Record<string, number[]> = { lead: [], trail: [], close: [] };
    for (let i = 0; i < gains.length; i++) {
      const g = gains[i]!;
      const next = gains[i + 1] ?? null;
      // 该球权的首次出手
      const firstShot = events.find((e) => e.seq > g.seq
        && e.type === 'SHOT_RELEASE'
        && (next === null || e.seq < next.seq));
      if (!firstShot) continue;
      const gameClock = firstShot.clocks.game;
      const period = firstShot.period;
      if (period < 4 || gameClock >= 300) continue; // 末节后半段
      const shooter = String(firstShot.payload['shooter_id'] ?? '');
      const shooterTeam = (cfg.home_team.roster.some((p) => p.jersey === shooter)) ? 'home' : 'away';
      const diff = shooterTeam === 'home'
        ? firstShot.score.home - firstShot.score.away
        : firstShot.score.away - firstShot.score.home;
      if (diff >= 8) shotClocks.lead!.push(firstShot.clocks.shot);
      else if (diff <= -8) shotClocks.trail!.push(firstShot.clocks.shot);
      else shotClocks.close!.push(firstShot.clocks.shot);
    }
    const avg = (a: number[]) => a.length ? a.reduce((s, v) => s + v, 0) / a.length : NaN;
    const leadAvg = avg(shotClocks.lead!);
    const trailAvg = avg(shotClocks.trail!);
    const closeAvg = avg(shotClocks.close!);
    // NBA: 领先方末节出手时shot clock平均约15-17s(压时间),落后方约11-13s
    // 注意: POSSESSION_GAINED后的首投可能是OREB段的late shot(sc=2s),
    // 单独看平均数会被拖低。真正的判定: 领先方是否存在明显的压时间
    // 出手(≥16s)且落后方没有——压时间博弈的真实信号。
    const leadLate = shotClocks.lead!.filter((s) => s >= 16).length;
    const trailLate = shotClocks.trail!.filter((s) => s >= 16).length;
    const leadSamples = shotClocks.lead!.length;
    const trailSamples = shotClocks.trail!.length;
    if (leadSamples >= 3 && trailSamples >= 3 && leadLate === 0 && trailLate > 0) {
      findings.push({
        code: 'NO_TEMPO_GAME', severity: 'warning',
        t: 0, period: 4,
        description: `末节节奏博弈缺失:领先方${leadSamples}次出手无一次压时间(≥16s),落后方反而有${trailLate}次(NBA领先方末节显著压时间)`,
        metric: leadLate,
        sample: { leadAvg, trailAvg, closeAvg, leadN: leadSamples, trailN: trailSamples, leadLate, trailLate },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // B) 关键时刻博弈 — 最后2分钟分差≤5
  // ───────────────────────────────────────────────────────────────────
  {
    const clutchReleases = events.filter((e) => {
      if (e.type !== 'SHOT_RELEASE') return false;
      const period = e.period;
      const qLen = periodSeconds(period);
      if (e.clocks.game > 120) return false;
      const diff = Math.abs(e.score.home - e.score.away);
      return diff <= 5;
    });
    // 落后方关键时刻是否投更多3分?
    const clutch3 = clutchReleases.filter((e) => e.payload['shot_value'] === 3);
    const clutch3Rate = clutchReleases.length ? clutch3.length / clutchReleases.length : NaN;
    // 全场3分率对比
    const all3 = events.filter((e) => e.type === 'SHOT_RELEASE' && e.payload['shot_value'] === 3);
    const all3Rate = all3.length / Math.max(1, events.filter((e) => e.type === 'SHOT_RELEASE').length);
    if (clutchReleases.length >= 10 && clutch3Rate < all3Rate * 0.8) {
      findings.push({
        code: 'NO_CLUTCH_3PT', severity: 'warning',
        t: 0, period: 4,
        description: `关键时刻(≤2min分差≤5)3分率${(clutch3Rate * 100).toFixed(0)}%低于全场${(all3Rate * 100).toFixed(0)}%,落后方没有用3分追分的博弈`,
        metric: clutch3Rate,
        sample: { clutch3Rate, all3Rate, clutchN: clutchReleases.length, clutch3N: clutch3.length },
      });
    }
    // 关键时刻是否出现犯规战术(落后方最后30s犯规)
    const last30 = events.filter((e) => e.period >= 4 && e.clocks.game <= 30);
    const fouls = last30.filter((e) => e.type === 'FOUL');
    const trailingLate = last30.filter((e) => {
      const team = String(e.payload['team'] ?? '');
      const diff = team === 'home' ? e.score.away - e.score.home : e.score.home - e.score.away;
      return diff > 0;
    });
    if (trailingLate.length > 0 && fouls.length === 0) {
      findings.push({
        code: 'NO_FOUL_GAME', severity: 'info',
        t: 0, period: 4,
        description: `最后30s落后方${trailingLate.length}次控球但0次犯规战术(NBA落后≥3分时会故意犯规)`,
        metric: fouls.length,
        sample: { trailingControls: trailingLate.length, fouls: fouls.length },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // C) 防守阵型整体性 — 联防时5人是否占据各自的slot(阵型完整性)
  // 2-3联防的几何: 上2人(翼)距篮筐约20ft分居两侧,下3人(低位)
  // 距篮筐4-12ft覆盖油漆区。检测: 联防球员是否settle在正确区域。
  // 原"同向移动"检测方法错误——联防站定时全员静止,球移动时
  // 各自滑向不同slot,方向天然不同;阵型的真实性在"是否站对位置"。
  // ───────────────────────────────────────────────────────────────────
  {
    let zoneFrames = 0;
    let slotViolations = 0;
    for (const s of snaps) {
      if (s.phase !== 'LIVE' || s.tactical?.screenDefense?.zone !== 'ZONE_2_3') continue;
      const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
      const attackSide = s.tactical.offense;
      const rim = attackSide === 'home' ? { x: 0.0559, y: 0.5 } : { x: 0.9441, y: 0.5 };
      // 防守人settled后应距其slot合理: 全部5人距篮筐的平均距离
      // 2-3联防: 平均距篮筐应8-16ft(2人20ft+3人4-10ft)
      const defs = s.players.filter((p) => p.team === defTeam);
      if (defs.length !== 5) continue;
      zoneFrames++;
      const rimDists = defs.map((d) => Math.min(distFt(d, { x: 0.0559, y: 0.5 }), distFt(d, { x: 0.9441, y: 0.5 })));
      const avgRim = rimDists.reduce((a, b) => a + b, 0) / 5;
      // 联防要求: 至少2人在20ft外(翼),至少2人在12ft内(低位)
      const wings = rimDists.filter((d) => d > 17).length;
      const lows = rimDists.filter((d) => d < 12).length;
      const midDist = rimDists.filter((d) => d >= 12 && d <= 17).length;
      if (wings < 2 && lows < 2 && midDist >= 2) {
        // 3+人在12-17ft中带 — 阵型塌缩成一条线,没有层次
        slotViolations++;
      }
      // NOTE: 低位球员互相<8ft不是塌缩——2-3联防的低位3人
      // 本来就密集在油漆区(这是联防的本质,不是缺陷)。
      // 只有"2上3下"的层次消失才算阵型塌缩。
    }
    const violationRate = zoneFrames ? slotViolations / zoneFrames : NaN;
    if (zoneFrames > 50 && violationRate > 0.3) {
      findings.push({
        code: 'ZONE_COLLAPSED', severity: 'warning',
        t: 0, period: 1,
        description: `联防(2-3)阵型塌缩占${(violationRate * 100).toFixed(0)}%的帧(${slotViolations}/${zoneFrames}),缺乏"2上3下"的层次`,
        metric: violationRate,
        sample: { zoneFrames, slotViolations },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // D) 换防持续性 — 换防后是否保持(不立即换回)
  // 检测: 一次换防(SWITCH)发生后,防守人-对位人关系是否保持≥3s
  // ───────────────────────────────────────────────────────────────────
  {
    // 通过matchup变化检测: 防守人targetJersey突变即为换防
    let switchCount = 0;
    let earlyRevert = 0;
    const lastAssign = new Map<string, string>();
    const switchStart = new Map<string, number>();
    for (const s of snaps) {
      if (s.phase !== 'LIVE' || !s.tactical?.assignments) continue;
      const defTeam = s.tactical.offense === 'home' ? 'away' : 'home';
      for (const a of s.tactical.assignments) {
        const pose = s.players.find((p) => p.jersey === a.jersey);
        if (!pose || pose.team !== defTeam) continue;
        const key = a.jersey;
        const target = a.targetJersey ?? '';
        const prevTarget = lastAssign.get(key);
        if (prevTarget && prevTarget !== target && target !== '') {
          // 换防发生
          if (switchStart.has(key)) {
            const dur = s.t_real - switchStart.get(key)!;
            if (dur < 3) earlyRevert++;
          }
          switchStart.set(key, s.t_real);
          switchCount++;
        }
        lastAssign.set(key, target);
      }
    }
    if (switchCount > 10) {
      const revertRate = earlyRevert / switchCount;
      if (revertRate > 0.5) {
        findings.push({
          code: 'SWITCH_NOT_STICKY', severity: 'warning',
          t: 0, period: 1,
          description: `${switchCount}次换防中${earlyRevert}次(<3s内换回,${(revertRate * 100).toFixed(0)}%),换防不持续(NBA换防后保持到死球)`,
          metric: revertRate,
          sample: { switchCount, earlyRevert },
        });
      }
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // E) 出手决策过程 — 接球后是否至少有观察时间(≥0.3s)
  // 接球→出手间隔过短(<0.2s)说明"接球就投"机械
  // ───────────────────────────────────────────────────────────────────
  {
    const passes = events.filter((e) => e.type === 'PASS' && e.payload['note'] === 'flight_complete');
    const releases = events.filter((e) => e.type === 'SHOT_RELEASE');
    const gaps: number[] = [];
    for (const rel of releases) {
      const shooter = String(rel.payload['shooter_id']);
      const prevPass = [...passes].reverse().find((p) =>
        p.seq < rel.seq && String(p.payload['receiver_id']) === shooter && rel.t_real - p.t_real < 10);
      if (prevPass) gaps.push(rel.t_real - prevPass.t_real);
    }
    const instant = gaps.filter((g) => g < 0.2).length;
    const rate = gaps.length ? instant / gaps.length : NaN;
    if (gaps.length > 20 && rate > 0.15) {
      findings.push({
        code: 'INSTANT_SHOTS', severity: 'info',
        t: 0, period: 1,
        description: `${(rate * 100).toFixed(0)}%的出手在接球后<0.2s内完成(${instant}/${gaps.length}),缺乏观察-决策过程`,
        metric: rate,
        sample: { instant, total: gaps.length },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // F) 推进路线 — 快攻推进是否走中路
  // TRANSITION_PUSH时,持球人x是否保持在球场中央带(0.35-0.65)
  // 方向判断不依赖baskets(snapshot无此字段):用持球人的实际移动方向
  // ───────────────────────────────────────────────────────────────────
  {
    let transitionFrames = 0;
    let middleFrames = 0;
    for (let i = 1; i < snaps.length; i++) {
      const s = snaps[i]!;
      const prev = snaps[i - 1]!;
      if (s.phase !== 'LIVE' || s.tactical?.kind !== 'TRANSITION_PUSH') continue;
      const holder = s.players.find((p) => p.hasBall);
      const prevHolder = prev.players.find((p) => p.jersey === holder?.jersey);
      if (!holder || !prevHolder) continue;
      // 持球人正在向某侧推进(速度>2ft/s)
      const dx = holder.x - prevHolder.x;
      const dy = holder.y - prevHolder.y;
      const speed = Math.hypot(dx * FT, dy * WD) / Math.max(0.05, s.t_real - prev.t_real);
      if (speed < 2) continue;
      const pushing = Math.abs(dx) > Math.abs(dy);
      if (!pushing) continue; // 纵向移动不算推进
      transitionFrames++;
      if (holder.x > 0.35 && holder.x < 0.65) middleFrames++;
    }
    const middleRate = transitionFrames ? middleFrames / transitionFrames : NaN;
    if (transitionFrames > 100 && middleRate < 0.5) {
      findings.push({
        code: 'NO_MIDDLE_PUSH', severity: 'warning',
        t: 0, period: 1,
        description: `快攻推进持球人走中路仅${(middleRate * 100).toFixed(0)}%时间(${middleFrames}/${transitionFrames},NBA推进优先中路分球)`,
        metric: middleRate,
        sample: { transitionFrames, middleFrames },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // G) 篮板→推进衔接 — 防守篮板后是否立即推进(3s内过半场)
  // ───────────────────────────────────────────────────────────────────
  {
    const rebounds = events.filter((e) => e.type === 'REBOUND');
    let outCount = 0;
    let pushCount = 0;
    for (const rb of rebounds) {
      const team = String(rb.payload['team'] ?? '');
      if (team === '') continue;
      // 找之后3s内该队的ADVANCE_BACKCOURT或CROSS_HALF
      const next = events.find((e) => e.seq > rb.seq && e.t_real <= rb.t_real + 4
        && (e.type === 'ADVANCE_BACKCOURT' || e.type === 'CROSS_HALF')
        && e.actors[0] !== undefined);
      outCount++;
      if (next) pushCount++;
    }
    const pushRate = outCount ? pushCount / outCount : NaN;
    if (outCount > 20 && pushRate < 0.4) {
      findings.push({
        code: 'SLOW_REBOUND_OUT', severity: 'warning',
        t: 0, period: 1,
        description: `防守篮板后4s内推进仅${(pushRate * 100).toFixed(0)}%(${pushCount}/${outCount}),篮板→推进衔接慢`,
        metric: pushRate,
        sample: { outCount, pushCount },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // H) 战术重复度 — 连续5次以上同一战术
  // 只统计"死球后的新球权"——OREB续攻保持同一战术是合理的
  // (打同一个战术点),跨球权连续才是教练机械不变招。
  // ───────────────────────────────────────────────────────────────────
  {
    // 每个死球→LIVE后的第一个tactical.kind序列
    const possKinds: string[] = [];
    for (let i = 1; i < snaps.length; i++) {
      const prev = snaps[i - 1]!;
      const s = snaps[i]!;
      const isDeadPhase = prev.phase.startsWith('DEAD') || prev.phase === 'FT_SEQUENCE' || prev.phase === 'TIMEOUT' || prev.phase === 'PERIOD_BREAK';
      if (isDeadPhase && s.phase === 'LIVE' && s.tactical?.kind) {
        possKinds.push(s.tactical.kind);
      }
    }
    // 连续同一战术的run
    let maxRun = 0;
    let runKind: string | null = null;
    let runLen = 0;
    for (const k of possKinds) {
      if (k === runKind) runLen++;
      else { runKind = k; runLen = 1; }
      if (runLen > maxRun) maxRun = runLen;
    }
    if (maxRun >= 5) {
      findings.push({
        code: 'TACTIC_STUCK', severity: 'warning',
        t: 0, period: 1,
        description: `连续${maxRun}个新球权使用同一战术(${runKind}),教练机械不变招(NBA通常3次后变招)`,
        metric: maxRun,
        sample: { kind: runKind, possessions: maxRun, totalPossessions: possKinds.length },
      });
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // I) 球员倾向稳定性 — 同一球员的出手距离分布是否稳定
  // 3分射手不应突然大量投中距离
  // ───────────────────────────────────────────────────────────────────
  {
    const shooterShots = new Map<string, number[]>();
    for (const rel of events.filter((e) => e.type === 'SHOT_RELEASE')) {
      const shooter = String(rel.payload['shooter_id']);
      const snap = snaps.find((s) => s.lastEventSeq === rel.seq);
      const x = Number(rel.payload['x']), y = Number(rel.payload['y']);
      const d = Math.min(distFt({ x, y }, { x: 0.0559, y: 0.5 }), distFt({ x, y }, { x: 0.9441, y: 0.5 }));
      if (!shooterShots.has(shooter)) shooterShots.set(shooter, []);
      shooterShots.get(shooter)!.push(d);
    }
    for (const [jersey, dists] of shooterShots) {
      if (dists.length < 8) continue;
      const threes = dists.filter((d) => d >= 23).length;
      const mids = dists.filter((d) => d >= 12 && d < 23).length;
      const rims = dists.filter((d) => d < 12).length;
      const total = dists.length;
      const threeRate = threes / total;
      const midRate = mids / total;
      const rimRate = rims / total;
      // 球员射程一致性: 如果3分率>50%,中距离应<40%
      if (threeRate > 0.5 && midRate > 0.45) {
        findings.push({
          code: 'SHOOTER_SCHIZO', severity: 'info',
          t: 0, period: 1,
          description: `球员${jersey}3分率${(threeRate * 100).toFixed(0)}%但中距离也占${(midRate * 100).toFixed(0)}%,射程不稳定`,
          metric: midRate,
          sample: { jersey, threes, mids, rims, total },
        });
      }
    }
  }

  // ───────────────────────────────────────────────────────────────────
  // J) 死球后防守布置 — 死球结束后第一帧,防守人距对位人是否<10ft
  // ───────────────────────────────────────────────────────────────────
  {
    let deadEndFrames = 0;
    let farFromMan = 0;
    for (let i = 1; i < snaps.length; i++) {
      const s = snaps[i]!;
      const prev = snaps[i - 1]!;
      const deadPhases = ['DEAD_MAKE', 'DEAD_OOB', 'DEAD_VIOLATION', 'DEAD_HELD', 'FT_SEQUENCE'];
      if (!deadPhases.includes(prev.phase) || s.phase !== 'LIVE') continue;
      if (s.ball.status !== 'held') continue;
      deadEndFrames++;
      const offTeam = s.tactical?.offense;
      if (!offTeam) continue;
      const defTeam = offTeam === 'home' ? 'away' : 'home';
      // 每个防守人距最近进攻人
      for (const d of s.players.filter((p) => p.team === defTeam)) {
        let minD = Infinity;
        for (const o of s.players.filter((p) => p.team === offTeam)) {
          minD = Math.min(minD, distFt(d, o));
        }
        if (minD > 10) farFromMan++;
      }
    }
    const totalDefenders = deadEndFrames * 5;
    const farRate = totalDefenders ? farFromMan / totalDefenders : NaN;
    if (deadEndFrames > 20 && farRate > 0.2) {
      findings.push({
        code: 'DEADBALL_LOOSE_D', severity: 'warning',
        t: 0, period: 1,
        description: `死球后防守人${(farRate * 100).toFixed(0)}%的时间距对位人>10ft(${deadEndFrames}次死球),防守未重新布置`,
        metric: farRate,
        sample: { deadEndFrames, farFromMan },
      });
    }
  }

  // 汇总
  const summary: Record<string, { count: number; errors: number; warnings: number; infos: number }> = {};
  for (const f of findings) {
    if (!summary[f.code]) summary[f.code] = { count: 0, errors: 0, warnings: 0, infos: 0 };
    summary[f.code]!.count++;
    if (f.severity === 'error') summary[f.code]!.errors++;
    else if (f.severity === 'warning') summary[f.code]!.warnings++;
    else summary[f.code]!.infos++;
  }
  return {
    seed,
    finalScore: {
      home: r.box_score.home.reduce((s, p) => s + (p.points ?? 0), 0),
      away: r.box_score.away.reduce((s, p) => s + (p.points ?? 0), 0),
    },
    snapshots: snaps.length,
    findings,
    summary,
  };
}

function main() {
  const argv = process.argv.slice(2);
  const seedIdx = argv.indexOf('--seeds');
  const seeds = seedIdx >= 0
    ? argv[seedIdx + 1]!.split(',').map(Number).filter(Number.isInteger)
    : [42, 5, 7];
  const outIdx = argv.indexOf('--out');
  const outPath = outIdx >= 0 ? argv[outIdx + 1]! : '.omo/evidence/game-psychology.json';
  const configIdx = argv.indexOf('--config');
  const configValue = configIdx !== -1 ? argv[configIdx + 1] : undefined;
  const configPath = configValue !== undefined ? configValue : 'config/demo-game.json';
  const cfg = JSON.parse(readFileSync(resolve(process.cwd(), configPath), 'utf8')) as unknown as ConfigShape;
  const out: Record<string, unknown> = {};
  for (const s of seeds) {
    const result = analyze(s, cfg);
    out[String(s)] = result;
    console.log(`\n=== seed ${s} === score=${result.finalScore.home}-${result.finalScore.away} findings=${result.findings.length}`);
    for (const [code, stats] of Object.entries(result.summary).sort((a, b) => b[1].count - a[1].count)) {
      console.log(`  ${code}: ${stats.count} (${'!'.repeat(stats.errors)}${'~'.repeat(stats.warnings)}${'-'.repeat(stats.infos)})`);
    }
  }
  const absOut = resolve(process.cwd(), outPath);
  mkdirSync(dirname(absOut), { recursive: true });
  writeFileSync(absOut, JSON.stringify(out, null, 1), 'utf-8');
  console.log(`\ngame-psychology-audit: wrote ${absOut} (${seeds.join(',')})`);
}

main();