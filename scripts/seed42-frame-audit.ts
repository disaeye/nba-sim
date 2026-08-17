/**
 * Reproducible seed-42 frame audit (plan T0).
 * Uses kernel snapshots → render frames; writes evidence JSON.
 *
 *   npx tsx scripts/seed42-frame-audit.ts
 *   npx tsx scripts/seed42-frame-audit.ts --seed 42 --out .omo/evidence/seed42-frame-audit.json
 */
import { mkdirSync, writeFileSync, readFileSync, existsSync } from 'node:fs';
import { resolve, dirname } from 'node:path';
import { simulateGame } from '../src/simulate.js';
import type { GameInput, Player } from '../src/simulate.js';
import type { LineupPackage } from '../src/identity/types.js';
import { renderFromSnapshots } from '../src/render/from-snapshots.js';
import type { RenderFrame } from '../src/render/types.js';

interface ConfigShape {
  home_team: {
    id: string;
    roster: { id: string; jersey: string; teamId: string }[];
    lineup_packages: LineupPackage[];
  };
  away_team: ConfigShape['home_team'];
}

const DEF = new Set([
  'on_ball_defend',
  'deny',
  'help',
  'tag',
  'weak_side',
  'box_out',
]);

function distFt(
  a: { x: number; y: number },
  b: { x: number; y: number },
  len = 94,
  wid = 50,
): number {
  return Math.hypot((a.x - b.x) * len, (a.y - b.y) * wid);
}

function auditFrames(frames: readonly RenderFrame[]) {
  const playerCountHist: Record<string, number> = {};
  let multiBall = 0;
  let defendWithBall = 0;
  let liveFrames = 0;
  let stackedPairs = 0;
  let edgeHits = 0;
  const minPairLive: number[] = [];
  let teamClumped = 0;
  const eventHist: Record<string, number> = {};
  let align = 0;
  let scv = 0;
  let shotRelease = 0;

  for (const f of frames) {
    const n = f.players.length;
    playerCountHist[String(n)] = (playerCountHist[String(n)] ?? 0) + 1;
    const withBall = f.players.filter((p) => p.hasBall);
    if (withBall.length > 1) multiBall++;
    if (f.eventType) {
      eventHist[f.eventType] = (eventHist[f.eventType] ?? 0) + 1;
      if (f.eventType === 'ALIGN_HALFCOURT') align++;
      if (f.eventType === 'SHOT_CLOCK_VIOLATION') scv++;
      if (f.eventType === 'SHOT_RELEASE') shotRelease++;
    }
    for (const p of f.players) {
      if (p.x <= 0.001 || p.x >= 0.999) edgeHits++;
    }
    if (f.phase !== 'LIVE') continue;
    liveFrames++;
    for (const p of withBall) {
      if (DEF.has(p.action)) defendWithBall++;
    }
    let minD = Infinity;
    for (let a = 0; a < f.players.length; a++) {
      for (let b = a + 1; b < f.players.length; b++) {
        const d = distFt(f.players[a]!, f.players[b]!);
        if (d < minD) minD = d;
        if (d < 0.5) stackedPairs++;
      }
    }
    if (minD < Infinity) minPairLive.push(minD);
    for (const team of ['home', 'away'] as const) {
      const tp = f.players.filter((p) => p.team === team);
      if (tp.length < 5) continue;
      const cx = tp.reduce((s, p) => s + p.x, 0) / tp.length;
      const cy = tp.reduce((s, p) => s + p.y, 0) / tp.length;
      const rad = Math.max(...tp.map((p) => distFt(p, { x: cx, y: cy })));
      if (rad < 8) teamClumped++;
    }
  }

  minPairLive.sort((a, b) => a - b);
  const pct = (p: number) =>
    minPairLive.length === 0
      ? null
      : minPairLive[Math.floor((p / 100) * (minPairLive.length - 1))]!;

  const last = frames[frames.length - 1];
  return {
    nFrames: frames.length,
    liveFrames,
    playerCountHist,
    framesWith20Players: playerCountHist['20'] ?? 0,
    multiBall,
    defendWithBall,
    stackedPairs,
    edgeHits,
    teamClumped,
    alignHalfcourt: align,
    shotClockViolation: scv,
    shotRelease,
    eventHistTop: Object.entries(eventHist)
      .sort((a, b) => b[1] - a[1])
      .slice(0, 20),
    minPairDistLiveFt: minPairLive.length
      ? {
          p01: pct(1),
          p05: pct(5),
          p50: pct(50),
          p95: pct(95),
          fracBelow1ft:
            minPairLive.filter((d) => d < 1).length / minPairLive.length,
          fracBelow2ft:
            minPairLive.filter((d) => d < 2).length / minPairLive.length,
        }
      : null,
    finalScore: last?.score ?? null,
    finalPhase: last?.phase ?? null,
  };
}

function main(): void {
  const argv = process.argv;
  let seed = 42;
  let configPath = 'config/demo-game.json';
  let outPath = '.omo/evidence/seed42-frame-audit.json';
  for (let i = 2; i < argv.length; i++) {
    if (argv[i] === '--seed') seed = Number(argv[++i]);
    else if (argv[i] === '--config') configPath = String(argv[++i]);
    else if (argv[i] === '--out') outPath = String(argv[++i]);
  }

  const cfg = JSON.parse(
    readFileSync(resolve(process.cwd(), configPath), 'utf-8'),
  ) as ConfigShape;
  const input: GameInput = {
    home: {
      teamId: cfg.home_team.id,
      roster: cfg.home_team.roster as Player[],
      lineupPackages: cfg.home_team.lineup_packages,
    },
    away: {
      teamId: cfg.away_team.id,
      roster: cfg.away_team.roster as Player[],
      lineupPackages: cfg.away_team.lineup_packages,
    },
    seed,
  };

  const result = simulateGame(input);
  const snaps = result.snapshots ?? [];
  const frames = renderFromSnapshots(snaps, result.events, {
    stride: 2,
    language: 'cn',
  });
  const metrics = auditFrames(frames);

  const baselinePath = resolve(
    process.cwd(),
    '.omo/evidence/seed42-frame-audit-baseline.json',
  );
  let regressed = false;
  let baselineNote: string | null = null;
  if (existsSync(baselinePath)) {
    const bas = JSON.parse(readFileSync(baselinePath, 'utf-8')) as {
      multiBall?: number;
      framesWith20Players?: number;
    };
    if (
      (bas.multiBall !== undefined && metrics.multiBall > bas.multiBall) ||
      (bas.framesWith20Players !== undefined &&
        metrics.framesWith20Players > bas.framesWith20Players)
    ) {
      regressed = true;
      baselineNote = 'P0 metrics worsened vs baseline';
    }
  }

  const report = {
    meta: {
      seed,
      foundation_version: result.meta.foundation_version,
      generated_at: new Date().toISOString(),
      stride: 2,
      snapshotCount: snaps.length,
      frameCount: frames.length,
    },
    metrics,
    regressed,
    baselineNote,
  };

  const absOut = resolve(process.cwd(), outPath);
  mkdirSync(dirname(absOut), { recursive: true });
  writeFileSync(absOut, JSON.stringify(report, null, 2), 'utf-8');

  if (!existsSync(baselinePath) && seed === 42) {
    writeFileSync(
      baselinePath,
      JSON.stringify(
        {
          multiBall: metrics.multiBall,
          framesWith20Players: metrics.framesWith20Players,
          defendWithBall: metrics.defendWithBall,
          finalScore: metrics.finalScore,
          note: 'Pre-repair baseline captured by T0; update only intentionally',
        },
        null,
        2,
      ),
      'utf-8',
    );
  }

  process.stdout.write(
    JSON.stringify(
      {
        out: outPath,
        multiBall: metrics.multiBall,
        framesWith20Players: metrics.framesWith20Players,
        defendWithBall: metrics.defendWithBall,
        finalScore: metrics.finalScore,
        alignHalfcourt: metrics.alignHalfcourt,
        shotClockViolation: metrics.shotClockViolation,
        shotRelease: metrics.shotRelease,
        regressed,
      },
      null,
      2,
    ) + '\n',
  );
}

main();
