import { describe, it, expect } from 'vitest';
import { narrateEvent, formatBroadcastLine } from '../../src/spectator/narrate.js';
import { buildSpectatorPackage } from '../../src/spectator/export.js';
import type { TimelineEvent } from '../../src/sim-utils.js';

function ev(partial: Partial<TimelineEvent> & Pick<TimelineEvent, 'type'>): TimelineEvent {
  return {
    type: partial.type,
    t_game: partial.t_game ?? 600,
    t_real: partial.t_real ?? 0,
    seq: partial.seq ?? 1,
    actors: partial.actors ?? [],
    payload: partial.payload ?? {},
    clocks: partial.clocks ?? { game: 600, shot: 14 },
    score: partial.score ?? { home: 10, away: 8 },
    period: partial.period ?? 1,
  };
}

describe('spectator narrate', () => {
  it('narrates a made basket with score', () => {
    const n = narrateEvent(
      ev({
        type: 'SHOT_RESULT',
        payload: { shooter_id: '1', shot_value: 2, made: true },
        score: { home: 12, away: 8 },
      }),
    );
    expect(n.intensity).toBe('high');
    expect(n.cn).toContain('进球');
    expect(n.en.toLowerCase()).toContain('bucket');
  });

  it('formats a broadcast line with clock and score', () => {
    const line = formatBroadcastLine(
      ev({
        type: 'DRIVE',
        payload: { ballHandlerId: '11' },
        clocks: { game: 125.4, shot: 10 },
        period: 2,
        score: { home: 20, away: 18 },
      }),
      'cn',
    );
    expect(line).toMatch(/Q2/);
    expect(line).toContain('20-18');
    expect(line).toContain('11');
  });

  it('falls back for unknown-looking types still in catalog', () => {
    const n = narrateEvent(ev({ type: 'STATE_NOTE', payload: { note: 'x' } }));
    expect(n.cn).toBe('STATE_NOTE');
  });

  it('summary entries retain event clock metadata for replay seeking', () => {
    const result = {
      meta: { foundation_version: 'test', seed: 1, home_team_id: 'home', away_team_id: 'away' },
      events: [ev({ type: 'FOUL', seq: 9, period: 1, clocks: { game: 659.2, shot: 12 }, payload: { offender_id: '14', victim_id: '2' } })],
      snapshots: [],
      box_score: { home: [], away: [] },
    } as never;
    const pkg = buildSpectatorPackage(result);
    expect(pkg.broadcastSummary[0]).toMatchObject({ eventSeq: 9, period: 1, gameClock: 659.2 });
  });

  it('keeps distinct event sequences when multiple events share one clock', () => {
    const result = {
      meta: { foundation_version: 'test', seed: 1, home_team_id: 'home', away_team_id: 'away' },
      events: [
        ev({ type: 'DRIVE', seq: 7, period: 1, clocks: { game: 717.8, shot: 20 }, payload: { ballHandlerId: '12' } }),
        ev({ type: 'STEAL', seq: 8, period: 1, clocks: { game: 717.8, shot: 20 }, payload: { stealer_id: '1', victim_id: '12' } }),
        ev({ type: 'TURNOVER', seq: 9, period: 1, clocks: { game: 717.8, shot: 20 }, payload: { player_id: '12' } }),
      ],
      snapshots: [],
      box_score: { home: [], away: [] },
    } as never;
    const pkg = buildSpectatorPackage(result);
    expect(pkg.broadcastSummary.map((entry) => entry.eventSeq)).toEqual([7, 8, 9]);
  });

  it('narrates pass out of bounds and includes OOB in summary', () => {
    const event = ev({
      type: 'OOB',
      seq: 12,
      clocks: { game: 660, shot: 10 },
      payload: { team_causing: 'home', player_id: '13', receiver_id: '15' },
    });
    expect(formatBroadcastLine(event, 'cn')).toContain('传球出界');
    const pkg = buildSpectatorPackage({
      meta: { foundation_version: 'test', seed: 1, home_team_id: 'home', away_team_id: 'away' },
      events: [event], snapshots: [], box_score: { home: [], away: [] },
    } as never);
    expect(pkg.broadcastSummary[0]?.eventSeq).toBe(12);
  });
});
