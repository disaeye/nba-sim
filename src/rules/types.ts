/**
 * Rules FSM types — the runtime contract for `config/fsm.json`.
 *
 * The JSON file (validated by `config/schemas/fsm.schema.json`) is the sole
 * authority; if this interface ever disagrees with the schema, the schema
 * wins. See `docs/foundation/fsm.md` for the full prose contract.
 */
import type { EventType, Phase } from '../state/types.js';

/**
 * An FSM trigger name. Triggers are kernel-internal predicates, NOT
 * `EventType`s — they select which transition row fires when the kernel
 * decides "a made basket happened" (trigger `MADE_BASKET`) or "the period
 * clock hit zero" (trigger `PERIOD_CLOCK_EXPIRY`). The transition's
 * `emit_events` array names the actual events that hit the timeline.
 *
 * Typed as a `string` alias (not a closed literal union): the transition
 * table in `config/fsm.json` is the authority, and `transition()` returns
 * null for any trigger not found in the current `(phase, trigger)` lookup.
 */
export type Trigger = string;

/** One row of the transition table — see `config/fsm.json#transitions`. */
export interface TransitionRow {
  readonly from: Phase;
  readonly trigger: string;
  readonly to: Phase;
  readonly emit_events: readonly EventType[];
  readonly notes: string;
}

/**
 * The parsed `config/fsm.json`. Only the fields consumed by the driver
 * are typed here; the JSON also carries `foundation_version` which the
 * foundation linter (todo 9) cross-checks for consistency.
 */
export interface FsmConfig {
  readonly foundation_version: string;
  readonly phases: readonly Phase[];
  readonly transitions: readonly TransitionRow[];
  readonly timeout_allotments: {
    readonly regulation_total: number;
    readonly max_in_q4: number;
    readonly overtime_total: number;
  };
  readonly substitution_legality: {
    readonly legal_phases: readonly Phase[];
    readonly illegal_phases: readonly Phase[];
    readonly notes: string;
  };
}

/**
 * The result of a legal transition: the destination phase, the ordered
 * events to emit, and the row's human-readable notes. Returned by
 * `transition(state, trigger)` when the `(phase, trigger)` pair matches
 * a row in the table; `null` otherwise.
 */
export interface TransitionResult {
  readonly phase: Phase;
  readonly emit_events: readonly EventType[];
  readonly notes: string;
}

/**
 * The four possible outcomes of `periodRouter` when the period clock
 * hits zero. A subset of `Phase` — the router never returns a LIVE,
 * DEAD_*, or PRE_GAME/POST_GAME value directly (POST_GAME is included
 * because Q4/OT-not-tied routes there).
 */
export type PeriodRouterResult =
  | 'PERIOD_BREAK'
  | 'HALFTIME'
  | 'OVERTIME_SETUP'
  | 'POST_GAME';
