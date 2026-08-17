/**
 * Frame-by-frame process study for the continuous simulation kernel.
 *
 * Usage:
 *   npx tsx scripts/film-study.mts <seed> <startRealSeconds> <durationSeconds> [stepSeconds]
 *
 * The output deliberately stays at player/frame level: every sampled frame
 * includes all ten players, ball state, tactical stage, and nearby semantic
 * events. It is a diagnostic viewer, not a statistical aggregator.
 */
import { readFileSync } from 'node:fs';
import { simulateGame } from '../src/simulate.js';
import type { WorldSnapshot } from '../src/world/snapshot.js';
import type { TimelineEvent } from '../src/sim-utils.js';

const FT = 94;
const WD = 50;
const cfg = JSON.parse(readFileSync('config/generated-roster.json', 'utf8'));
const seed = Number.parseInt(process.argv[2] ?? '42', 10);
const start = Number.parseFloat(process.argv[3] ?? '0');
const duration = Number.parseFloat(process.argv[4] ?? '30');
const step = Number.parseFloat(process.argv[5] ?? '0.5');

const result = simulateGame({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed,
});
const snapshots = (result.snapshots ?? []) as readonly WorldSnapshot[];
const events = result.events as readonly TimelineEvent[];

function d(a: { x: number; y: number }, b: { x: number; y: number }): number {
  return Math.hypot((a.x - b.x) * FT, (a.y - b.y) * WD);
}
function rimX(period: number, team: string): number {
  const homeRight = period <= 2;
  return team === 'home' ? (homeRight ? 0.9441 : 0.0559) : (homeRight ? 0.0559 : 0.9441);
}
function nearEvent(t: number): string {
  return events
    .filter((event) => Math.abs(event.t_real - t) < 0.051)
    .map((event) => `${event.type}${event.payload && Object.keys(event.payload).length ? `(${Object.entries(event.payload).filter(([k]) => ['shooter_id', 'ballHandlerId', 'screener_id', 'receiver_id', 'targetJersey', 'rebounder_id'].includes(k)).map(([k, v]) => `${k}=${String(v)}`).join(',')})` : ''}`)
    .join(' ');
}
function nearestDefender(player: WorldSnapshot['players'][number], snapshot: WorldSnapshot): { jersey: string; ft: number } | null {
  let best: { jersey: string; ft: number } | null = null;
  for (const defender of snapshot.players) {
    if (defender.team === player.team) continue;
    const ft = d(player, defender);
    if (best === null || ft < best.ft) best = { jersey: defender.jersey, ft };
  }
  return best;
}
function sample(t: number): WorldSnapshot | undefined {
  return snapshots.find((snapshot) => Math.abs(snapshot.t_real - t) < 0.051);
}

if (!Number.isFinite(seed) || !Number.isFinite(start) || !Number.isFinite(duration) || !Number.isFinite(step) || step <= 0) {
  throw new Error('Usage: film-study.mts <seed> <startRealSeconds> <durationSeconds> [stepSeconds]');
}

console.log(`\nFRAME STUDY seed=${seed} real=${start.toFixed(1)}-${(start + duration).toFixed(1)} step=${step.toFixed(1)}s\n`);
for (let offset = 0; offset <= duration + 1e-6; offset += step) {
  const target = start + offset;
  const snapshot = sample(target);
  if (!snapshot) continue;
  const tactical = snapshot.tactical;
  const offense = tactical?.offense ?? null;
  const holder = snapshot.players.find((player) => player.hasBall);
  const attack = holder && offense ? { x: rimX(snapshot.period, offense), y: 0.5 } : null;
  const event = nearEvent(snapshot.t_real);
  console.log(`t=${snapshot.t_real.toFixed(1)} Q${snapshot.period} ${snapshot.phase} shot=${snapshot.shotClock.toFixed(1)} score=${snapshot.score.home}-${snapshot.score.away} ball=${snapshot.ball.status}${snapshot.ball.holderId ? `:${snapshot.ball.holderId}` : ''} tac=${tactical?.kind ?? '-'}:${tactical?.stage ?? '-'}${tactical?.screenDefense ? `/${tactical.screenDefense.mode}` : ''}${event ? ` events=[${event}]` : ''}`);
  if (holder && attack) console.log(`  HANDLER ${holder.jersey} team=${holder.team} rim=${d(holder, attack).toFixed(1)}ft xy=(${holder.x.toFixed(3)},${holder.y.toFixed(3)}) action=${holder.action}`);
  else console.log(`  BALL xy=(${snapshot.ball.x.toFixed(3)},${snapshot.ball.y.toFixed(3)})`);
  for (const player of snapshot.players) {
    const role = tactical?.assignments.find((assignment) => assignment.jersey === player.jersey);
    const near = nearestDefender(player, snapshot);
    const rim = attack && player.team === offense ? d(player, attack).toFixed(1) : '-';
    const target = role?.targetJersey ?? '-';
    console.log(`  ${player.team.charAt(0).toUpperCase()}${player.jersey.padStart(2, '0')} xy=(${player.x.toFixed(3)},${player.y.toFixed(3)}) rim=${rim}ft act=${player.action.padEnd(14)} role=${role?.role ?? '-'} tgt=${target} near=${near ? `${near.jersey}/${near.ft.toFixed(1)}ft` : '-'}`);
  }
}
console.log(`\nFINAL score=${result.box_score.home.reduce((sum, player) => sum + player.points, 0)}-${result.box_score.away.reduce((sum, player) => sum + player.points, 0)} events=${events.length} snapshots=${snapshots.length}\n`);
