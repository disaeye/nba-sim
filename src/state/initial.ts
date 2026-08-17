/**
 * Initial-state factory for continuous dual-clock kernel (foundation 0.4.0).
 */
import type { GameInput, GameState } from './types.js';
import { posesFromLineups } from '../court/poses.js';
import { ballDead } from '../court/ball-motion.js';

export function createInitialState(input: GameInput): GameState {
  const homeLineup = [...input.home.starters];
  const awayLineup = [...input.away.starters];
  // Tip-off circle-ish spread before JUMP_BALL phase places exact spots.
  const homeSpots = homeLineup.map((_, i) => ({ x: 0.42, y: 0.15 + i * 0.17 }));
  const awaySpots = awayLineup.map((_, i) => ({ x: 0.58, y: 0.15 + i * 0.17 }));
  const poses = posesFromLineups(homeLineup, awayLineup, homeSpots, awaySpots);

  return {
    phase: 'PRE_GAME',
    clocks: { period: 1, game: 720.0, shot: 24.0 },
    realClock: 0,
    ball: { holderId: null, status: 'dead', zone: null },
    ballMotion: ballDead({ x: 0.5, y: 0.5 }),
    poses,
    possession: { team: null },
    score: { home: 0, away: 0 },
    lineups: { home: homeLineup, away: awayLineup },
    fouls: {
      team: { home: 0, away: 0 },
      players: {},
      bonus: { home: false, away: false },
    },
    timeouts: { remaining: { home: 7, away: 7 } },
    period: 1,
    events: [],
    seq: 0,
    baskets: { home: 'left', away: 'right' },
    abilities: {},
    playerData: {},
    coach: { home: undefined, away: undefined },
    chemistry: { home: [], away: [] },
    moraleExec: { home: 1, away: 1 },
    creatorOrder: { home: [], away: [] },
    alignment: null,
  };
}
