/**
 * Pure render: WorldSnapshot[] (+ optional events for callouts) → RenderFrame[].
 * Single path — no event→keyframe interpolation dual track.
 */
import type { TimelineEvent } from '../sim-utils.js';
import type { WorldSnapshot } from '../world/snapshot.js';
import { formatBroadcastLine } from '../spectator/narrate.js';
import type { RenderFrame, RenderOptions, RenderPlayer, StreamTick } from './types.js';

function intensityFor(type: string | null): 'low' | 'mid' | 'high' | null {
  if (!type) return null;
  if (
    type === 'SHOT_RESULT' ||
    type === 'FOUL' ||
    type === 'STEAL' ||
    type === 'PERIOD_END' ||
    type === 'GAME_END'
  ) {
    return 'high';
  }
  if (type === 'PASS' || type === 'REBOUND' || type === 'FT_RESULT') return 'mid';
  return 'low';
}

/**
 * Map kernel snapshots to render frames (or stream ticks when `asTicks`).
 * Callouts attach when snapshot.lastEventSeq points at a semantic event.
 *
 * `asTicks: true` produces the spectator stream shape directly (adds
 * `gameClock` + `keyframeIndex`, drops the separate frames pass) so the
 * exporter never holds frames AND ticks in memory at once.
 */
export function renderFromSnapshots(
  snapshots: readonly WorldSnapshot[],
  events: readonly TimelineEvent[] = [],
  opts: RenderOptions = {},
): (RenderFrame | StreamTick)[] {
  return Array.from(renderTicks(snapshots, events, opts));
}

/**
 * Generator form of renderFromSnapshots: yields one frame/tick at a time so
 * streaming exporters can render → write → discard without ever materializing
 * the full 34k-frame array (peak export memory drops to a constant).
 */
export function* renderTicks(
  snapshots: readonly WorldSnapshot[],
  events: readonly TimelineEvent[] = [],
  opts: RenderOptions = {},
): Generator<RenderFrame | StreamTick> {
  const stride = Math.max(1, opts.stride ?? 1);
  const lang = opts.language ?? 'cn';
  const asTicks = opts.asTicks ?? false;
  const bySeq = new Map<number, TimelineEvent>();
  for (const e of events) bySeq.set(e.seq, e);
  const eventIndexBySeq = asTicks
    ? new Map(events.map((event, index) => [event.seq, index]))
    : null;
  for (let i = 0; i < snapshots.length; i += stride) {
    const s = snapshots[i]!;
    const players: RenderPlayer[] = s.players.map((p) => ({
      jersey: p.jersey,
      team: p.team,
      x: p.x,
      y: p.y,
      zone: p.zone,
      hasBall: p.hasBall,
      action: p.action,
      stm: p.stm,
      stmMax: p.stmMax,
    }));
    let callout: string | null = null;
    let eventType: string | null = s.lastEventType;
    let eventPayload: Readonly<Record<string, unknown>> | undefined;
    let keyframeIndex: number | null = null;
    if (s.lastEventSeq !== null) {
      const ev = bySeq.get(s.lastEventSeq);
      if (ev) {
        callout = formatBroadcastLine(ev, lang);
        eventType = ev.type;
        eventPayload = { ...ev.payload, __seq: ev.seq };
        keyframeIndex = eventIndexBySeq?.get(ev.seq) ?? null;
      }
    }
    const tickEvents = s.tickEventSeqs
      .map((seq) => bySeq.get(seq))
      .filter((ev): ev is TimelineEvent => ev !== undefined);
    const base = {
      t: s.t_real, t_game: s.t_game, shotClock: s.shotClock,
      period: s.period, phase: s.phase, score: s.score, players,
      ball: { x: s.ball.x, y: s.ball.y, status: s.ball.status, holderId: s.ball.holderId },
      tactical: s.tactical,
      eventType, eventSeq: s.lastEventSeq, eventPayload, callout,
      intensity: intensityFor(eventType),
      // Every semantic event emitted on this tick (same-tick pairs used to
      // collapse to the rank-last one — SHOT_RESULT on makes and REBOUND were
      // invisible to stream consumers).
      tickEvents: tickEvents.map((ev) => ({ type: ev.type, seq: ev.seq, payload: ev.payload })),
    };
    yield asTicks
      ? { ...base, gameClock: s.t_game, keyframeIndex }
      : base;
  }
}
