import { describe, it, expect } from 'vitest';
import { cachedGame } from './simulate/_helpers.js';

describe('repro order', () => {
  it('finds bad drive-shot and long screen after alignment call order', () => {
    const seq = [42, 42, 42, 42, 1, 7, 7, 1, 1, 7, 42, 42, 1, 2, 3, 7, 41, 7, 41, 42, 99, 1, 2, 3, 7, 41, 42, 7, 42, 99, 7, 41, 42, 99, 7, 41, 42, 99, 7, 42, 99, 7, 41, 42, 99, 7, 41, 42, 99, 7, 41, 42];
    for (const seed of seq) cachedGame(seed);
    for (const seed of [1, 7, 42]) {
      const result = cachedGame(seed);
      const snapshots = new Map((result.snapshots ?? []).map((snapshot) => [snapshot.lastEventSeq, snapshot]));
      const shots = result.events.filter((event) => event.type === 'SHOT_RELEASE');
      for (const drive of result.events.filter((event) => event.type === 'DRIVE')) {
        const shot = shots.find((event) => event.seq > drive.seq
          && event.t_real - drive.t_real < 3
          && !result.events.some((between) => between.seq > drive.seq && between.seq < event.seq
            && (between.type === 'PASS' || between.type === 'TURNOVER')));
        if (!shot) continue;
        const frame = snapshots.get(shot.seq);
        const shooter = frame?.players.find((player) => player.jersey === String(shot.payload['shooter_id']));
        if (!frame || !shooter) continue;
        const rimX = shooter.team === 'home'
          ? (frame.period <= 2 ? 0.9441489361702128 : 0.05585106382978723)
          : (frame.period <= 2 ? 0.05585106382978723 : 0.9441489361702128);
        const distance = Math.hypot((Number(shot.payload['x']) - rimX) * 94, (Number(shot.payload['y']) - 0.5) * 50);
        const shootingFoul = result.events.some((event) => event.type === 'FOUL'
          && event.seq < shot.seq
          && Math.abs(event.t_real - shot.t_real) < 0.11
          && String(event.payload['victim_id']) === String(shot.payload['shooter_id'])
          && event.payload['shooting'] === true);
        if (!shootingFoul && distance > 20) {
          console.log('BAD_DRIVE_SHOT', seed, JSON.stringify({ drive, shot, frame, shooter, distance }));
          break;
        }
      }
      const screens = [];
      const snaps = result.snapshots ?? [];
      for (const set of result.events.filter((event) => event.type === 'SCREEN_SET')) {
        const screener = String(set.payload['screener_id']);
        const handler = String(set.payload['ballHandlerId']);
        const startIndex = snaps.findIndex((snapshot) =>
          Math.abs(snapshot.t_real - set.t_real) < 0.051
          && snapshot.tactical?.handler === handler
          && snapshot.tactical?.stage === 'SCREEN_APPROACH'
          && snapshot.tactical.assignments.some((assignment) => assignment.role === 'screener' && assignment.jersey === screener),
        );
        if (startIndex < 0) continue;
        let start = snaps[startIndex]!;
        for (let i = startIndex - 1; i >= 0; i -= 1) {
          const frame = snaps[i]!;
          const sameScreen = frame.tactical?.handler === handler
            && frame.tactical.stage === 'SCREEN_APPROACH'
            && frame.tactical.assignments.some((assignment) => assignment.role === 'screener' && assignment.jersey === screener);
          if (!sameScreen) break;
          start = frame;
        }
        const d = set.t_real - start.t_real;
        if (d >= 8) screens.push({ d, set, start: { t: start.t_real, shotClock: start.shotClock, tactical: start.tactical, players: start.players } });
      }
      if (screens.length) console.log('LONG_SCREEN', seed, JSON.stringify(screens[0]));
    }
    expect(true).toBe(true);
  });
});
