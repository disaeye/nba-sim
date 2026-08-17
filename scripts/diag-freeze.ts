/**
 * Diagnostic: seed 42 demo game, window around Q1 11:50 (t_game ≈ 710).
 * Prints holder/action/position per tick and the semantic events in the
 * window, then measures holder static streaks.
 */
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { simulateGame } from '../src/simulate.js';

const cfg = JSON.parse(readFileSync(resolve(process.cwd(), 'config/demo-game.json'), 'utf-8'));
const result = simulateGame({
  home: { teamId: cfg.home_team.id, roster: cfg.home_team.roster, lineupPackages: cfg.home_team.lineup_packages },
  away: { teamId: cfg.away_team.id, roster: cfg.away_team.roster, lineupPackages: cfg.away_team.lineup_packages },
  seed: 42,
});

const snaps = result.snapshots ?? [];

// ── event window ──
const events = result.events;
const windowStart = 718; // t_game
const windowEnd = 640;
console.log('== events in Q1 t_game 718..640 ==');
for (const e of events) {
  if (e.period === 1 && e.t_game <= windowStart && e.t_game >= windowEnd) {
    const actors = (e.actors ?? []).join(',');
    console.log(`t=${e.t_game.toFixed(1)} seq=${e.seq} ${e.type} [${actors}] ${JSON.stringify(e.payload).slice(0, 140)}`);
  }
}

// ── snapshot window: every 10 ticks around 11:50 ──
console.log('\n== snapshots t_game 715..700 (every 5th) ==');
let prevHolder: string | null = null;
let prevX = 0, prevY = 0;
const staticStreak: Array<{ from: number; to: number; seconds: number; jersey: string }> = [];
let streakStart: number | null = null;
let streakHolder: string | null = null;
let lastX = 0, lastY = 0;

for (let i = 0; i < snaps.length; i++) {
  const s = snaps[i]!;
  if (s.period !== 1) continue;
  if (s.t_game > 715 || s.t_game < 600) continue;
  const holder = s.ball.holderId;
  const holderPlayer = holder ? s.players.find((p) => p.jersey === holder) : null;

  // static-streak measurement
  if (holder && holderPlayer) {
    if (streakHolder === holder && streakStart !== null) {
      const dx = (holderPlayer.x - lastX) * 94;
      const dy = (holderPlayer.y - lastY) * 50;
      const dist = Math.hypot(dx, dy);
      if (dist < 0.3) {
        // still static; extend
      } else {
        const secs = streakStart - s.t_game;
        if (secs >= 4) staticStreak.push({ from: streakStart, to: s.t_game, seconds: secs, jersey: holder });
        streakStart = s.t_game;
        streakHolder = holder;
      }
    } else {
      streakStart = s.t_game;
      streakHolder = holder;
    }
    lastX = holderPlayer.x;
    lastY = holderPlayer.y;
  } else {
    if (streakHolder && streakStart !== null) {
      const secs = streakStart - (snaps[Math.max(0, i - 1)]?.t_game ?? streakStart);
      if (secs >= 4) staticStreak.push({ from: streakStart, to: snaps[Math.max(0, i - 1)]?.t_game ?? streakStart, seconds: secs, jersey: streakHolder });
    }
    streakStart = null;
    streakHolder = null;
  }

  if (s.t_game % 5 !== 0 || s.phase !== 'LIVE') continue;
  const line = s.players
    .map((p) => {
      const ball = p.jersey === holder ? '*' : ' ';
      return `${ball}${p.jersey.padStart(2)}:${p.action.slice(0, 14).padEnd(14)}(${(p.x * 94).toFixed(0)},${(p.y * 50).toFixed(0)})`;
    })
    .join(' ');
  console.log(`t=${s.t_game.toFixed(1)} phase=${s.phase} holder=${holder ?? '-'} ${line}`);
}

console.log('\n== holder static streaks ≥4s (Q1 t_game 715..600) ==');
for (const st of staticStreak) console.log(`${st.jersey} static ${st.seconds.toFixed(1)}s (${st.from} → ${st.to})`);
