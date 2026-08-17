/**
 * film-room-audit.ts — Loop N "录像带分析" 工具
 *
 * 与旧审计完全不同的切入角度:不做逐帧布尔规则,而是把一场比赛切成
 * possession(回合)级别的"叙事单元",对每个回合用连续量(速度谱、
 * 空间利用率、盯人残余、动作熵)刻画它的"比赛感",再输出最反常的
 * 回合供人工/模型审片。核心问题:
 *   1. 每个回合看起来像"一次进攻"吗?(时长、移动总量、阵型展开)
 *   2. 比赛过程是否机械?(回合特征分布是否退化、战术熵)
 *   3. 站位可解释吗?(盯人残余向量、油漆区拥挤度、空间利用率)
 *   4. 有实时博弈吗?(防守收缩-回扩周期、进攻响应延迟)
 *
 * Usage: npx tsx scripts/film-room-audit.ts [--seed 42] [--possessions 12]
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

interface Frame {
  t: number; t_game: number; shotClock: number; period: number; phase: string;
  score: { home: number; away: number };
  players: Array<{ jersey: string; team: string; x: number; y: number; zone: string; hasBall: boolean; action: string; stm: number; stmMax: number }>;
}
const FT = 94, WD = 50;
const d = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot((a.x - b.x) * FT, (a.y - b.y) * WD);
const RIM_L = { x: 0.0559, y: 0.5 }, RIM_R = { x: 0.9441, y: 0.5 };

// ---------- CLI ----------
const args = process.argv.slice(2);
const getOpt = (name: string, def: string) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? (args[i + 1] ?? def) : def;
};
const ticksPath = getOpt('ticks', 'spectator/game.ticks.ndjson');
const topN = parseInt(getOpt('top', '10'), 10);

const raw = readFileSync(resolve(process.cwd(), ticksPath), 'utf8').trim().split('\n');
const frames: Frame[] = raw.map((l) => JSON.parse(l));
console.log(`loaded ${frames.length} frames, t=${frames[0]?.t ?? 0}..${frames[frames.length - 1]?.t ?? 0}`);

// ---------- possession segmentation ----------
// A possession = LIVE segment between non-LIVE phases (or ball-out-of-hands terminal).
interface Seg { start: number; end: number; frames: Frame[]; }
const segs: Seg[] = [];
let cur: Seg | null = null;
for (let i = 0; i < frames.length; i++) {
  const frame = frames[i];
  if (!frame) continue;
  const liveFrame = frame.phase === 'LIVE';
  if (liveFrame && !cur) cur = { start: i, end: i, frames: [] };
  if (cur) cur.frames.push(frame);
  if (!liveFrame && cur) { cur.end = i - 1; segs.push(cur); cur = null; }
}
if (cur) { cur.end = frames.length - 1; segs.push(cur); }
const live = segs.filter((s) => s.frames.length >= 15);
console.log(`possessions(LIVE segs ≥1.5s): ${live.length}`);

// ---------- possession-level continuous descriptors ----------
function mirrorToAttack(f: Frame, attTeam: string, p: Frame['players'][number]) {
  // normalize so attacking team always goes toward x=1 (right rim)
  const mirrored = attTeam === 'home' ? false : true; // home attacks right? derive from ball trail instead
  return { x: mirrored ? 1 - p.x : p.x, y: mirrored ? 1 - p.y : p.y };
}

function analyzePossession(s: Seg) {
  // attacking team = team of the player who holds ball most
  const holds = new Map<string, number>();
  for (const f of s.frames) for (const p of f.players) if (p.hasBall) holds.set(`${p.team}|${p.jersey}`, (holds.get(`${p.team}|${p.jersey}`) ?? 0) + 1);
  let att = 'home'; let best = -1;
  for (const [k, v] of holds) { const team = k.split('|')[0] ?? 'home'; if (v > best) { best = v; att = team; } }
  // attack direction: which rim the ball converges to
  const xs = s.frames.map((f) => f.players.find((p) => p.hasBall)?.x).filter((v) => v !== undefined) as number[];
  const meanX = xs.reduce((a, b) => a + b, 0) / Math.max(1, xs.length);
  const attackRight = meanX > 0.5;
  const norm = (p: Frame['players'][number]) => attackRight
    ? { x: p.x, y: p.y }
    : { x: 1 - p.x, y: 1 - p.y };
  const rim = { x: attackRight ? RIM_R.x : RIM_L.x, y: 0.5 };

  const dur = (s.end - s.start) * 0.1;
  // ball movement
  const ballPos = (f: Frame) => f.players.find((p) => p.hasBall);
  let ballDist = 0; let prev: Frame['players'][number] | undefined;
  let passCount = 0; let holderSwitch = 0; let prevHolder: string | undefined;
  const holderSet = new Set<string>();
  for (const f of s.frames) {
    const h = ballPos(f);
    if (h) {
      holderSet.add(`${h.team}|${h.jersey}`);
      if (prevHolder !== undefined && `${h.team}|${h.jersey}` !== prevHolder) holderSwitch++;
      prevHolder = `${h.team}|${h.jersey}`;
      if (prev) ballDist += d(h, prev);
      prev = h;
    }
  }
  // per-player path length + speed
  const pathLen = new Map<string, number>();
  const maxSpd = new Map<string, number>();
  let prevP = new Map<string, { x: number; y: number }>();
  for (const f of s.frames) {
    for (const p of f.players) {
      const key = `${p.team}|${p.jersey}`;
      const q = prevP.get(key);
      const step = q ? d(q, p) : 0;
      pathLen.set(key, (pathLen.get(key) ?? 0) + step);
      maxSpd.set(key, Math.max(maxSpd.get(key) ?? 0, step / 0.1));
      prevP.set(key, { x: p.x, y: p.y });
    }
  }
  // spacing: convex-hull-lite = mean pairwise distance among attackers; paint crowding
  const lastF = s.frames[s.frames.length - 3 >= 0 ? s.frames.length - 3 : 0]!;
  const attPlayers = lastF.players.filter((p) => p.team === att);
  const defPlayers = lastF.players.filter((p) => p.team !== att);
  let pairSum = 0, pairN = 0;
  for (let i = 0; i < attPlayers.length; i++) for (let j = i + 1; j < attPlayers.length; j++) { pairSum += d(attPlayers[i]!, attPlayers[j]!); pairN++; }
  const spacing = pairN ? pairSum / pairN : 0;
  const paintCount = attPlayers.filter((p) => Math.hypot((p.x - rim.x) * FT, (p.y - rim.y) * WD) < 8).length;
  // man-marking residual: for each defender, distance to nearest attacker at end
  const markRes = defPlayers.map((dp) => {
    const n = norm(dp);
    let best = 1e9;
    for (const ap of attPlayers) { const na = norm(ap); best = Math.min(best, d(n, na)); }
    return best;
  });
  // action entropy over attackers (distinct action labels)
  const actCount = new Map<string, number>();
  for (const f of s.frames) for (const p of f.players) if (p.team === att) actCount.set(p.action, (actCount.get(p.action) ?? 0) + 1);
  let H = 0; const tot = [...actCount.values()].reduce((a, b) => a + b, 0);
  for (const v of actCount.values()) { const q = v / tot; H -= q * Math.log2(q); }
  // defender compression-expansion cycles (help rhythm)
  // distance of nearest defender to holder over time
  const holderDefDist: number[] = [];
  for (const f of s.frames) {
    const h = ballPos(f);
    if (!h) continue;
    let bd = 1e9;
    for (const p of f.players) if (p.team !== h.team) bd = Math.min(bd, d(h, p));
    if (bd < 1e8) holderDefDist.push(bd);
  }
  // count local minima below 5ft followed by rise >4ft => a "contain-then-release" cycle
  let cycles = 0;
  for (let i = 2; i < holderDefDist.length - 2; i++) {
    const current = holderDefDist[i];
    const previous = holderDefDist[i - 1];
    const next = holderDefDist[i + 1];
    const future = holderDefDist[i + 2];
    if (current !== undefined && previous !== undefined && next !== undefined && future !== undefined
      && current < 4.5 && current <= previous && current <= next && future - current > 3) cycles++;
  }
  return {
    start: s.start, end: s.end, dur, att, attackRight,
    holders: holderSet.size, holderSwitch, passCount,
    ballDist, spacing, paintCount, markRes,
    H, cycles, holderDefDist,
    pathLen, maxSpd, attPlayers: attPlayers.map((p) => p.jersey),
  };
}

const results = live.map((s) => analyzePossession(s));

// ---------- diagnostics ----------
// D1: duration distribution vs NBA (real median ~14s of 24 used; many quick ones too)
const durs = results.map((r) => r.dur).sort((a, b) => a - b);
const q = (arr: number[], p: number) => arr[Math.floor(p * (arr.length - 1))] ?? 0;
console.log('\n== possession duration (s): p10=%.1f p50=%.1f p90=%.1f', q(durs, 0.1), q(durs, 0.5), q(durs, 0.9));

// D2: mechanical-ness — correlation between successive possession durations & tactic mixing
const sameDur = results.slice(1).map((r, i) => {
  const previous = results[i];
  return previous ? Math.abs(r.dur - previous.dur) : 0;
});
console.log('== |Δduration| successive: p50=%.2f (mechanical if tiny)', q(sameDur.sort((a, b) => a - b), 0.5));

// D3: spacing quality final frame
const sp = results.map((r) => spacingOf(r)).sort((a, b) => a - b);
console.log('== final-frame attack spacing p50=%.1fft (NBA settled halfcourt ≈ 15-19ft)', q(sp, 0.5));

// D4: man-marking residual: median defender-to-nearest-attacker
const allMark = results.flatMap((r) => r.markRes).sort((a, b) => a - b);
console.log('== defender-to-nearest-attacker median=%.1fft p90=%.1fft (settled man ≈ 4-8ft)', q(allMark, 0.5), q(allMark, 0.9));

// D5: speed sanity
const spd = results.flatMap((r) => [...r.maxSpd.values()]).sort((a, b) => a - b);
console.log('== per-player max speed p50=%.1f p99=%.1f ft/s (NBA sprint ≈ 24-26; >32 suspicious)', q(spd, 0.5), q(spd, 0.99));

// D6: help-defense rhythm
const cyc = results.map((r) => r.cycles);
console.log('== contain-release cycles / possession: mean=%.2f', cyc.reduce((a, b) => a + b, 0) / Math.max(1, cyc.length));

// D7: ball travel vs dribble realism
console.log('== holders/possession: mean=%.2f  holderSwitch mean=%.2f', results.reduce((a, r) => a + r.holders, 0) / results.length, results.reduce((a, r) => a + r.holderSwitch, 0) / results.length);

// ---------- flag worst possessions ----------
function spacingOf(r: ReturnType<typeof analyzePossession>) { return r.spacing; }
interface Flag { idx: number; why: string; det: string; }
const flags: Flag[] = [];
results.forEach((r, idx) => {
  if (r.dur < 3 && r.holders === 1) flags.push({ idx, why: 'instant isolation', det: `dur=${r.dur.toFixed(1)}s holders=1` });
  if (r.markRes.length && Math.max(...r.markRes) > 26) flags.push({ idx, why: 'broken marking', det: `maxMarkRes=${Math.max(...r.markRes).toFixed(0)}ft` });
  if (r.spacing < 9 && r.dur > 6) flags.push({ idx, why: 'clogged spacing', det: `spacing=${r.spacing.toFixed(1)}ft dur=${r.dur.toFixed(1)}s` });
  if (r.paintCount >= 3) flags.push({ idx, why: 'paint jam', det: `paintCount=${r.paintCount}` });
  if ([...r.maxSpd.values()].some((v) => v > 32)) flags.push({ idx, why: 'speed spike', det: `maxSpd=${Math.max(...r.maxSpd.values()).toFixed(1)}ft/s` });
  if (r.H < 0.5 && r.dur > 8) flags.push({ idx, why: 'low action entropy', det: `H=${r.H.toFixed(2)} dur=${r.dur.toFixed(1)}s` });
});
console.log(`\n== flagged ${flags.length} possessions:`);
flags.slice(0, topN).forEach((f) => console.log(`  [${f.idx}] ${f.why}: ${f.det}`));

// ---------- deep dump of selected possession ----------
const dumpIdx = parseInt(getOpt('dump', '-1'), 10);
if (dumpIdx >= 0 && results[dumpIdx] && live[dumpIdx]) {
  const r = results[dumpIdx]!;
  const s = live[dumpIdx]!;
  console.log(`\n===== DUMP possession ${dumpIdx} (frames ${s.start}..${s.end}, ${r.dur.toFixed(1)}s, att=${r.att}) =====`);
  for (let k = 0; k < s.frames.length; k += 5) {
    const f = s.frames[k];
    if (!f) continue;
    const h = f.players.find((p) => p.hasBall);
    console.log(`t=${f.t.toFixed(1)} sc=${f.shotClock.toFixed(1)} holder=${h ? h.team + h.jersey + '@' + norm2(h) : '—'} action=${h?.action ?? '—'}`);
    for (const p of f.players) {
      const dd = h && p.team !== h.team ? ` dHolder=${d(p, h).toFixed(1)}` : '';
      console.log(`   ${p.team === r.att ? 'A' : 'D'}${p.jersey} (${p.x.toFixed(2)},${p.y.toFixed(2)}) ${p.action}${p.hasBall ? ' *BALL*' : ''}${dd}`);
    }
  }
}
function norm2(p: { x: number; y: number }) { return `(${p.x.toFixed(2)},${p.y.toFixed(2)})`; }
