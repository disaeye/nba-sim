function summarizeBroadcast(events: readonly GameResult['events'][number][]): { text: string; eventSeq: number; period: number; gameClock: number }[] {
  const out: { text: string; eventSeq: number; period: number; gameClock: number }[] = [];
  let previousClock = '';
  let previousType = '';
  for (const event of events) {
    const clock = `${event.period}:${Math.floor(event.clocks.game)}`;
    if (event.type === 'ADVANCE_BACKCOURT' && previousClock === clock) continue;
    if (event.type === 'PASS' && event.payload['note'] === 'flight_start') continue;
    if (event.type === 'PASS' && previousType === 'PASS' && previousClock === clock) continue;
    out.push({ text: formatBroadcastLine(event, 'cn'), eventSeq: event.seq, period: event.period, gameClock: event.clocks.game });
    previousClock = clock;
    previousType = event.type;
  }
  return out;
}
/**
 * Build export package for the web player from kernel snapshots + render layer.
 */
import type { GameResult } from '../sim-utils.js';
import { formatBroadcastLine } from './narrate.js';
import { renderFromSnapshots } from '../render/from-snapshots.js';
import type { RenderFrame } from '../render/types.js';
import { buildCourtDrawSpec, type CourtDrawSpec } from '../court/geometry.js';

export interface SpectatorPackage {
  readonly meta: GameResult['meta'] & {
    readonly home_name?: string;
    readonly away_name?: string;
    readonly exported_at: string;
    readonly stream_dt: number;
  };
  readonly box_score: GameResult['box_score'];
  readonly court: CourtDrawSpec;
  readonly frames: readonly RenderFrame[];
  readonly stream: {
    readonly dt: number;
    readonly tickCount: number;
    readonly duration: number;
    readonly ticks: readonly RenderFrame[];
  };
  readonly broadcast: readonly string[];
  /** Event seq for each displayed play-by-play line. */
  readonly broadcastEventSeqs: readonly number[];
  /** Condensed event-group commentary for the spectator timeline. */
  readonly broadcastSummary: readonly { readonly text: string; readonly eventSeq: number; readonly period: number; readonly gameClock: number }[];
  readonly event_count: number;
}

export function buildSpectatorPackage(
  result: GameResult,
  opts?: { homeName?: string; awayName?: string; dt?: number; stride?: number },
): SpectatorPackage {
  const dt = opts?.dt ?? 0.1;
  const snaps = result.snapshots ?? [];
  // Replay needs the exact event snapshot. Striding here drops SCREEN_SET,
  // PASS start, and SHOT_RELEASE frames that fall between render samples.
  const stride = opts?.stride ?? 1;
  // Single pass: ticks directly (asTicks), so frames are never materialized
  // as a separate array — halves peak memory on the 34k-frame export.
  const ticks = renderFromSnapshots(snaps, result.events, {
    stride,
    language: 'cn',
    asTicks: true,
  });
  const broadcast: string[] = [];
  const broadcastEventSeqs: number[] = [];
  for (const event of result.events) {
    if (event.type === 'PASS' && event.payload['note'] === 'flight_start') continue;
    broadcast.push(formatBroadcastLine(event, 'cn'));
    broadcastEventSeqs.push(event.seq);
  }
  const duration = ticks.length > 0 ? ticks[ticks.length - 1]!.t : 0;
  const broadcastSummary = summarizeBroadcast(result.events);
  return {
    meta: {
      ...result.meta,
      home_name: opts?.homeName,
      away_name: opts?.awayName,
      exported_at: new Date().toISOString(),
      stream_dt: dt * stride,
    },
    box_score: result.box_score,
    court: buildCourtDrawSpec(),
    frames: ticks,
    stream: {
      dt: dt * stride,
      tickCount: ticks.length,
      duration,
      ticks,
    },
    broadcast,
    broadcastEventSeqs,
    broadcastSummary,
    event_count: result.events.length,
  };
}
