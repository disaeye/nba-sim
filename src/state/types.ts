/**
 * GameState types — the spine of the kernel. Every later module (FSM,
 * possession, resolve, sim) operates on values of these types. See
 * `docs/foundation/events.md` for the delta-fold algorithm and
 * `docs/foundation/identity-roles.md` for the player/role model.
 *
 * The interfaces below are the runtime authority. `config/event-catalog.json`
 * is the JSON authority for the event surface; `tests/state/types.test.ts`
 * cross-checks the two so they cannot drift silently.
 */
import type { Clocks } from '../clocks/types.js';
import type { LineupCapability } from '../identity/types.js';

// ─── closed-set unions ──────────────────────────────────────────────────────

/**
 * The 15 FSM phases — see `docs/foundation/fsm.md` and `config/fsm.json`.
 * The catalog is closed at v0.1.0; adding a phase is a `foundation_version`
 * bump (major).
 */
export type Phase =
  | 'PRE_GAME'
  | 'JUMP_BALL'
  | 'LIVE'
  | 'DEAD_OOB'
  | 'DEAD_FOUL'
  | 'DEAD_VIOLATION'
  | 'DEAD_MAKE'
  | 'DEAD_HELD'
  | 'DEAD_PERIOD_END'
  | 'FT_SEQUENCE'
  | 'TIMEOUT'
  | 'PERIOD_BREAK'
  | 'HALFTIME'
  | 'OVERTIME_SETUP'
  | 'POST_GAME';

/**
 * The 14 logical court zones — see `docs/foundation/identity-roles.md` and
 * `config/court-zones.json`. Labels only; no `(x, y)` anywhere in v0.1.0.
 */
export type CourtZone =
  | 'backcourt'
  | 'frontcourt_center'
  | 'slot_L'
  | 'slot_R'
  | 'wing_L'
  | 'wing_R'
  | 'corner_L'
  | 'corner_R'
  | 'elbow_L'
  | 'elbow_R'
  | 'paint'
  | 'dunker_L'
  | 'dunker_R'
  | 'rim';

/**
 * The 38 event types — see `docs/foundation/events.md` and
 * `config/event-catalog.json`. The closed catalog is what the foundation
 * linter and the type-level parity test guard.
 */
export type EventType =
  | 'GAME_START'
  | 'JUMP_BALL_TAP'
  | 'POSSESSION_GAINED'
  | 'INBOUND_START'
  | 'INBOUND_TOUCH'
  | 'ADVANCE_BACKCOURT'
  | 'CROSS_HALF'
  | 'ALIGN_HALFCOURT'
  | 'PASS'
  | 'HANDOFF'
  | 'SCREEN_SET'
  | 'SCREEN_USE'
  | 'DRIVE'
  | 'SHOT_RELEASE'
  | 'SHOT_RESULT'
  | 'REBOUND'
  | 'LOOSE_BALL_RECOVER'
  | 'STEAL'
  | 'TURNOVER'
  | 'FOUL'
  | 'VIOLATION'
  | 'SHOT_CLOCK_VIOLATION'
  | 'PERIOD_END'
  | 'PERIOD_START'
  | 'HALFTIME'
  | 'TIMEOUT_START'
  | 'TIMEOUT_END'
  | 'SUB'
  | 'FT_START'
  | 'FT_ATTEMPT'
  | 'FT_RESULT'
  | 'FT_SEQUENCE_END'
  | 'MADE_BASKET_DEAD'
  | 'OOB'
  | 'HELD_BALL'
  | 'CLOCK_EXPIRY_ADJUDICATION'
  | 'GAME_END'
  | 'STATE_NOTE'
  /** Jump-ball circle alignment (2 jumpers + 8 outside). */
  | 'JUMP_CIRCLE_ALIGN'
  /** Full 10-player court alignment snapshot (authoritative positions). */
  | 'ALIGNMENT';

/**
 * The two team literals used everywhere in the kernel. Pinned to a
 * discriminated union (not `string`) so the compiler rejects typos at
 * every call site.
 */
export type TeamId = 'home' | 'away';

/**
 * Live ball controller state. `held` is the steady state during live
 * play; `loose` covers deflections; `inbound` covers the brief window
 * between INBOUND_START and INBOUND_TOUCH; `dead` is every other phase.
 */
export type BallStatus = 'held' | 'loose' | 'inbound' | 'dead' | 'pass' | 'shot';

// ─── structural interfaces ──────────────────────────────────────────────────

/** Player identity. Logic uses only `id`, `teamId`, `jersey` (per foundation). */
export interface Player {
  readonly id: string;
  readonly teamId: TeamId;
  readonly jersey: string;
}

/** Scoreboard snapshot. `home` and `away` are cumulative points. */
export interface Score {
  home: number;
  away: number;
}

/** Possession indicator. `team` is null before tip-off resolves. */
export interface Possession {
  team: TeamId | null;
}

/** The live ball: who controls it, its status, its zone. */
export interface Ball {
  holderId: string | null;
  status: BallStatus;
  zone: CourtZone | null;
}

/** Per-team foul accounting. `team` is current-period team foul count. */
export interface Fouls {
  team: { home: number; away: number };
  players: Record<string, number>;
  bonus: { home: boolean; away: boolean };
}

/** Team-called timeout allotment remaining. */
export interface Timeouts {
  remaining: { home: number; away: number };
}

/** On-court lineups: 5 identifying strings per team. */
export interface Lineups {
  home: string[];
  away: string[];
}

/** Snapshot of clocks/score carried inside an event for replay. */
export interface EventSnapshot {
  game: number;
  shot: number;
}

// ─── the core state + event ─────────────────────────────────────────────────

/**
 * The full game state at a point in time. Reconstructed ONLY by folding
 * events over `createInitialState(...)`; gameplay code MUST NOT mutate
 * any field directly.
 */
/**
 * One on-court player in the latest authoritative alignment.
 * Coordinates are full-court normalized (see src/court/alignment.ts).
 */
export interface AlignedPlayer {
  readonly jersey: string;
  readonly team: TeamId;
  readonly x: number;
  readonly y: number;
  readonly zone: CourtZone;
  readonly task: string;
  readonly hasBall: boolean;
}

/** Latest 10-player formation for kernel + spectator. */
export interface CourtAlignment {
  readonly context: string;
  readonly offense: TeamId | null;
  readonly players: readonly AlignedPlayer[];
}

/** Which physical basket each team defends (tip-off: home left, away right). */
export interface Baskets {
  readonly home: 'left' | 'right';
  readonly away: 'left' | 'right';
}

export interface GameState {
  phase: Phase;
  clocks: Clocks;
  /** Monotone real seconds from tip — always advances on each sim tick. */
  realClock: number;
  ball: Ball;
  /** Continuous ball motion (held/pass/shot/loose). Position truth with poses. */
  ballMotion: import('../court/ball-motion.js').BallMotionState;
  /** Continuous player poses — position truth (not events). */
  poses: import('../court/poses.js').PoseMap;
  possession: Possession;
  score: Score;
  lineups: Lineups;
  fouls: Fouls;
  timeouts: Timeouts;
  period: number;
  events: Event[];
  seq: number;
  baskets: Baskets;
  /**
   * Per-jersey talent overlay (roster abilities merged over package-derived
   * defaults). Read by the resolve/adjudicate layer for per-player outcome
   * rates; never mutated by events.
   */
  abilities: Readonly<Record<string, LineupCapability>>;
  /**
   * Raw layered player data per jersey (24-ability / 9-tendency /
   * 4-awareness / physical). Present only for rosters that carry
   * playerData; the resolve and decision layers read the fine-grained
   * quantities directly instead of only the bridged 14-dim overlay.
   */
  playerData: Readonly<Record<string, import('../playerdata/types.js').PlayerData>>;
  /** P5.5 coach identity per team (scheme/pace biases). */
  coach: Readonly<Record<TeamId, import('../sim-utils.js').CoachProfile | undefined>>;
  /** P6.1 active chemistry effects per team lineup (channel modifiers). */
  chemistry: Readonly<Record<TeamId, readonly import('../playerdata/types.js').ChemistryEffect[]>>;
  /** P6.2 per-team morale exec multiplier (§9.3 band). */
  moraleExec: Readonly<Record<TeamId, number>>;
  /** P6.3 possession-share-ordered creator priority per team. */
  creatorOrder: Readonly<Record<TeamId, readonly string[]>>;
  /** @deprecated position truth is poses + snapshots; kept null in 0.3.0 */
  alignment: CourtAlignment | null;
}

/**
 * Semantic marker on the timeline. Positions come from WorldSnapshot, not events.
 * t_real is wall/sim continuous time; t_game is live game-clock remaining.
 */
export interface Event {
  type: EventType;
  t_game: number;
  /** Continuous real time when the fact occurred (0.3.0+). */
  t_real: number;
  seq: number;
  actors: string[];
  payload: Record<string, unknown>;
  clocks: EventSnapshot;
  score: Score;
}

// ─── initial-state input ────────────────────────────────────────────────────

/** Input for one team's initial lineup. */
export interface TeamInput {
  readonly id: TeamId;
  /** Exactly 5 on-court player identifiers at tip-off. */
  readonly starters: readonly string[];
}

/** Input to `createInitialState`. */
export interface GameInput {
  readonly home: TeamInput;
  readonly away: TeamInput;
}

// ─── typed errors ───────────────────────────────────────────────────────────

/**
 * Discriminator for state-fold errors. Stable strings let callers branch
 * on `err.code` rather than parsing the message.
 */
export type StateErrorCode =
  | 'STATE_UNKNOWN_EVENT_TYPE'
  | 'STATE_SUB_TARGET_NOT_IN_LINEUP';

/**
 * Typed error raised by `applyEvent` when the kernel receives a
 * well-formed event that conflicts with the live state (e.g. a `SUB`
 * whose `player_out_id` is not on the court). Carries the offending
 * values so a caller can surface a structured report.
 */
export class StateError extends Error {
  readonly code: StateErrorCode;
  readonly eventType: EventType;

  constructor(code: StateErrorCode, eventType: EventType, message: string) {
    super(message);
    this.name = 'StateError';
    this.code = code;
    this.eventType = eventType;
  }
}
