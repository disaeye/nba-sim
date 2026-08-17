/**
 * baseline-audit.ts — Proactive NBA metrics comparison
 *
 * Takes a simulated game result + tick stream and compares every measurable
 * dimension against config/nba-baselines.json. Outputs a prioritized list
 * of deviations — the ENGINEER decides which are bugs vs style choices.
 *
 * Usage: npx tsx scripts/baseline-audit.ts [--ticks path] [--seed N]
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

interface Baseline {
  band: [number, number];
  median: number;
  note?: string;
}

interface Baselines {
  [category: string]: { [metric: string]: Baseline };
}

interface Tick {
  t: number; t_game: number; shotClock: number; period: number; phase: string;
  score: { home: number; away: number };
  players: Array<{ jersey: string; team: string; x: number; y: number; action: string; hasBall: boolean; stm: number; stmMax: number }>;
  ball: { x: number; y: number; status: string; holderId: string | null };
  tactical?: { kind?: string; stage?: string; offense?: string; assignments?: Array<{ jersey: string; role: string; action: string }> } | null;
  eventType?: string | null;
  eventPayload?: Record<string, unknown> | null;
  tickEvents?: Array<{ type: string; payload?: Record<string, unknown> }> | null;
}

const args = process.argv.slice(2);
const getOpt = (name: string, def: string) => {
  const i = args.indexOf(`--${name}`);
  return i >= 0 ? (args[i + 1] ?? def) : def;
};
const ticksPath = getOpt('ticks', 'spectator/game.ticks.ndjson');
const baselinesPath = 'config/nba-baselines.json';

const baselines = JSON.parse(readFileSync(resolve(process.cwd(), baselinesPath), 'utf8')) as unknown as Baselines;
const raw = readFileSync(resolve(process.cwd(), ticksPath), 'utf8').trim().split('\n');
const ticks: Tick[] = raw.map((l) => JSON.parse(l));
console.log(`loaded ${ticks.length} ticks`);

const FT = 94, WD = 50;
const dist = (a: { x: number; y: number }, b: { x: number; y: number }) =>
  Math.hypot((a.x - b.x) * FT, (a.y - b.y) * WD);

// ─── measure everything ────────────────────────────────────────────────────

const measures: Record<string, { category: string; metric: string; value: number; baseline: Baseline }> = {};

function record(category: string, metric: string, value: number): void {
  const b = baselines[category]?.[metric];
  if (b) measures[`${category}.${metric}`] = { category, metric, value, baseline: b };
}

const lastTick = ticks[ticks.length - 1]!;
const totalPts = lastTick.score.home + lastTick.score.away;

// Scoring
record('scoring', 'total_ppg', totalPts);
record('scoring', 'team_ppg', totalPts / 2);

// Shot profile from events
const shots: Array<{ v: number; rd: number; zone: string; contest: number; t: number; team: string }> = [];
for (const f of ticks) {
  if (f.eventType !== 'SHOT_RELEASE') continue;
  const pl = f.eventPayload ?? {};
  const ps = new Map(f.players.map((p) => [p.jersey, p]));
  const sh = ps.get(String(pl['shooter_id'] ?? ''));
  if (!sh) continue;
  // Shot location truth: use the PAYLOAD x/y (the actual release point).
  // The player's body position can trail by 3-4ft during the rim-advance
  // animation — measuring from the body misclassifies rim finishes as
  // floaters (measured: 6.7ft body vs 3.7ft payload on the same shot).
  const px = (pl['x'] as number | undefined) ?? sh.x;
  const py = (pl['y'] as number | undefined) ?? sh.y;
  const tx = (pl['target_x'] as number | undefined) ?? (px > 0.5 ? 0.944 : 0.056);
  const ty = (pl['target_y'] as number | undefined) ?? 0.5;
  const rd = Math.hypot((px - tx) * FT, (py - ty) * WD);
  const contest = Math.min(...f.players.filter((p) => p.team !== sh.team).map((p) => dist({ x: px, y: py }, p)));
  shots.push({ v: (pl['shot_value'] as number) ?? 2, rd, zone: String(pl['zone'] ?? ''), contest, t: f.t, team: sh.team });
}
const fga = shots.length || 1;
record('shot_profile', 'rim_share', shots.filter((s) => s.rd < 5).length / fga);
record('shot_profile', 'short_mid_share', shots.filter((s) => s.rd >= 5 && s.rd < 14).length / fga);
record('shot_profile', 'long_mid_share', shots.filter((s) => s.rd >= 14 && s.rd < 22).length / fga);
record('shot_profile', 'three_share', shots.filter((s) => s.v === 3).length / fga);
record('shot_profile', 'corner_3_share_of_3pa', shots.filter((s) => s.v === 3 && s.zone.startsWith('corner')).length / Math.max(1, shots.filter((s) => s.v === 3).length));

// Contest distribution
record('contest_distribution', 'open_6ft_plus', shots.filter((s) => s.contest >= 6).length / fga);
record('contest_distribution', 'tight_2_4ft', shots.filter((s) => s.contest >= 2 && s.contest < 4).length / fga);
record('contest_distribution', 'very_tight_0_2ft', shots.filter((s) => s.contest < 2).length / fga);
record('contest_distribution', 'wide_open_10ft_plus', shots.filter((s) => s.contest >= 10).length / fga);

// Ball movement
let passes = 0;
const evs = new Map<number, { type: string; payload: Record<string, unknown>; t: number; frame: (typeof ticks)[number] }>();
for (const f of ticks) {
  for (const ev of f.tickEvents ?? []) {
    if (ev.type === 'PASS' && (ev.payload?.['note'] as string) === 'flight_start') passes++;
    if (ev.type) evs.set(f.t * 10 + evs.size, { type: ev.type, payload: ev.payload ?? {}, t: f.t, frame: f });
  }
}
// possessions
let possessions = 0;
let prevTeam = '';
for ( const [, e] of evs) {
  if (e.type === 'POSSESSION_GAINED' || e.type === 'LOOSE_BALL_RECOVER') {
    const tm = String(e.payload['team'] ?? '');
    if (tm && tm !== prevTeam) { possessions++; prevTeam = tm; }
  }
}
record('ball_movement', 'passes_per_possession', passes / Math.max(1, possessions));

// Pace
record('pace_and_timing', 'avg_shot_clock_at_release',
  shots.reduce((s, x) => s + x.t, 0) / fga > 0 ? shots.reduce((s, x) => s + (24 - (x.t % 24)), 0) / fga : 0);

// Rebounds
let orebs = 0; let drebs = 0;
let lastShotTeam = '';
for (const [, e] of evs) {
  if (e.type === 'SHOT_RELEASE') {
    const sh = e.frame.players.find((p) => String(p.jersey) === String(e.payload['shooter_id'] ?? ''));
    if (sh) lastShotTeam = sh.team;
  }
  if (e.type === 'LOOSE_BALL_RECOVER') {
    const tm = String(e.payload['team'] ?? '');
    if (tm === lastShotTeam) orebs++; else drebs++;
  }
}
record('rebounds', 'oreb_rate', orebs / Math.max(1, orebs + drebs));

// Movement: average offensive player speed
const speeds: number[] = [];
let prev: Record<string, { x: number; y: number }> = {};
for (const f of ticks) {
  if (f.phase !== 'LIVE') { prev = {}; continue; }
  const h = f.players.find((p) => p.hasBall);
  if (!h) continue;
  for (const p of f.players) {
    if (p.team !== h.team) continue;
    const q = prev[p.jersey];
    if (q) {
      const sp = dist(q, p) / 0.1;
      if (sp > 0.5 && sp < 35) speeds.push(sp);
    }
    prev[p.jersey] = { x: p.x, y: p.y };
  }
}
if (speeds.length > 0) {
  const sorted = speeds.sort((a, b) => a - b);
  record('movement_and_spatial', 'avg_player_speed_ft_s', sorted[Math.floor(sorted.length / 2)] ?? 0);
  record('movement_and_spatial', 'top_speed_ft_s', sorted[Math.floor(sorted.length * 0.99)] ?? sorted[sorted.length - 1]!);
}

// Spacing: settled halfcourt NN distance
const nnDists: number[] = [];
for (let i = 0; i < ticks.length; i += 6) {
  const f = ticks[i]!;
  if (f.phase !== 'LIVE') continue;
  const tk = f.tactical;
  if (!tk || tk.kind === 'TRANSITION_PUSH') continue;
  const h = f.players.find((p) => p.hasBall);
  if (!h) continue;
  const offense = f.players.filter((p) => p.team === h.team);
  for (let a = 0; a < offense.length; a++) {
    let min = Infinity;
    for (let b = 0; b < offense.length; b++) {
      if (a === b) continue;
      min = Math.min(min, dist(offense[a]!, offense[b]!));
    }
    if (min < Infinity) nnDists.push(min);
  }
}
if (nnDists.length > 0) {
  nnDists.sort((a, b) => a - b);
  record('movement_and_spatial', 'offense_nn_spacing_median', nnDists[Math.floor(nnDists.length / 2)] ?? 0);
}

// Defense: rim-side rate
let rimSide = 0; let defTotal = 0;
for (let i = 0; i < ticks.length; i += 6) {
  const f = ticks[i]!;
  if (f.phase !== 'LIVE') continue;
  const h = f.players.find((p) => p.hasBall);
  if (!h) continue;
  const rim = { x: h.x > 0.5 ? 0.944 : 0.056, y: 0.5 };
  for (const d of f.players.filter((p) => p.team !== h.team)) {
    const man = f.players.filter((p) => p.team === h.team).reduce((best, p) => dist(d, p) < dist(d, best) ? p : best);
    const vManRim = { x: rim.x - man.x, y: rim.y - man.y };
    const vManDef = { x: d.x - man.x, y: d.y - man.y };
    defTotal++;
    if (vManRim.x * vManDef.x + vManRim.y * vManDef.y > 0) rimSide++;
  }
}
if (defTotal > 0) record('defense', 'defender_rim_side_rate', rimSide / defTotal);

// Tactics
const tacticCounts: Record<string, number> = {};
for (const f of ticks) {
  const k = f.tactical?.kind;
  if (k) tacticCounts[k] = (tacticCounts[k] ?? 0) + 1;
}
const totalTactical = Object.values(tacticCounts).reduce((a, b) => a + b, 0) || 1;
record('tactics_and_style', 'pnr_possessions_per_game', ((tacticCounts['PNR_ROLL'] ?? 0) + (tacticCounts['PNR_POP'] ?? 0)) / totalTactical * possessions);
record('tactics_and_style', 'isolations_per_game', (tacticCounts['ISO'] ?? 0) / totalTactical * possessions);
record('tactics_and_style', 'post_ups_per_game', (tacticCounts['POST_UP'] ?? 0) / totalTactical * possessions);
record('tactics_and_style', 'off_ball_screens_per_game', (tacticCounts['OFF_BALL_SCREEN'] ?? 0) / totalTactical * possessions);
record('tactics_and_style', 'handoffs_per_game', (tacticCounts['HANDOFF'] ?? 0) / totalTactical * possessions);

// Game flow
let leadChanges = 0; let prevLeader = '';
for (const f of ticks) {
  if (f.phase !== 'LIVE') continue;
  const leader = f.score.home > f.score.away ? 'home' : f.score.away > f.score.home ? 'away' : '';
  if (leader && leader !== prevLeader) { leadChanges++; prevLeader = leader; }
}
record('game_flow', 'lead_changes_per_game', leadChanges);

// ─── report ────────────────────────────────────────────────────────────────

console.log('\n═══ NBA BASELINE AUDIT ═══');
console.log('Format: metric = value | band [lo, hi] | status\n');

const deviations: Array<{ key: string; severity: string; text: string }> = [];
for (const [key, m] of Object.entries(measures)) {
  const { value, baseline } = m;
  const [lo, hi] = baseline.band;
  let status = '✓';
  let sev = '';
  if (value < lo * 0.7 || value > hi * 1.3) { status = '‼️'; sev = 'SEVERE'; }
  else if (value < lo * 0.85 || value > hi * 1.15) { status = '⚠️'; sev = 'WARN'; }
  else if (value < lo || value > hi) { status = '±'; sev = 'EDGE'; }

  const text = `${status} ${key} = ${value.toFixed(3)} | [${lo}, ${hi}] ${baseline.note ? `(${baseline.note})` : ''}`;
  console.log(text);
  if (sev) deviations.push({ key, severity: sev, text });
}

console.log(`\n═══ SUMMARY ═══`);
console.log(`measured: ${Object.keys(measures).length} metrics`);
console.log(`in band: ${Object.keys(measures).length - deviations.length}`);
console.log(`edge (±): ${deviations.filter((d) => d.severity === 'EDGE').length}`);
console.log(`warning (⚠️): ${deviations.filter((d) => d.severity === 'WARN').length}`);
console.log(`severe (‼️): ${deviations.filter((d) => d.severity === 'SEVERE').length}`);

if (deviations.filter((d) => d.severity !== 'EDGE').length > 0) {
  console.log(`\n══─ ACTION ITEMS (sorted by severity) ─══`);
  for (const d of deviations.filter((x) => x.severity !== 'EDGE').sort((a, b) => a.severity < b.severity ? 1 : -1)) {
    console.log(`  [${d.severity}] ${d.key}`);
  }
}
