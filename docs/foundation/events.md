# Event Catalog & Delta Encoding

`foundation_version: 0.4.0` · `catalog_size: 38`

This document is the **sole authority** for every event the NBA simulator kernel may emit, and for how those events reconstruct `GameState`. The closed machine-readable catalog lives at [`config/event-catalog.json`](../../config/event-catalog.json) and is validated by [`config/schemas/event-catalog.schema.json`](../../config/schemas/event-catalog.schema.json). If prose here ever disagrees with the JSON catalog or schema, the JSON wins.

## Delta Encoding

The simulator is **delta-based** (decision M1 of the work plan). `GameState` is never mutated in place by gameplay code; instead, every state-changing moment is recorded as an `Event` whose catalog entry declares, via its `mutates` array, the dot-paths of `GameState` it is allowed to touch. The authoritative list of allowed paths is therefore the union of every event's `mutates` array; an event that mutates a path not listed in its catalog entry is a contract violation.

Concretely:

- Each `Event` carries a `type` from this catalog, a `t_game` (game-clock timestamp, seconds, resolution 0.1s), a payload whose required keys are pinned by `required_payload_fields`, and (transitively) the `mutates` declaration inherited from the catalog.
- `state_at(t)` — the value of `GameState` at game time `t` — is reconstructed by folding the events emitted so far in `(t_game, priority)` order and applying each event's `mutates` to an initial state.
- A `step(state, rng)` call therefore never returns a hand-written new state; it returns the state obtained by appending zero or more events to the timeline and folding them. This is what makes the kernel deterministic and replayable from the event log alone.

Two consequences worth calling out:

1. **Every state change is auditable.** If a `score.away` change appears in `state_at(t)` but no `SHOT_RESULT` or `FT_RESULT` for the away team exists in the folded prefix, that is a kernel bug, not an event-log gap.
2. **Replay is byte-stable.** Re-folding the same event sequence against the same initial state yields an identical `GameState`, which is the basis for the golden-seed fixture (todo 20).

## Fold Algorithm

The fold is intentionally minimal — it knows nothing about basketball, only dot-paths. The canonical pseudocode (≤15 lines):

```text
function fold(events, initial_state):
    state := deep_clone(initial_state)
    ordered := events.sort_by(t_game asc, same_ts_priority_rank asc, seq asc)
    for ev in ordered:
        decl := CATALOG[ev.type]                  # catalog entry, mutates[] is a list of dot-paths
        if ev.t_game > current_fold_horizon: break # caller passes the prefix it wants
        for path in decl.mutates:
            value := resolve_value(ev, decl, path) # event-specific: payload field or computed new value
            set_dot_path(state, path, value)       # create intermediate objects as needed
    return state
state_at(t) := fold(events.filter(e => e.t_game <= t), INITIAL_STATE)
```

Notes on the algorithm:

- `seq` is the emission order (a monotonic counter assigned when the kernel appends an event); it breaks ties after `(t_game, rank)` so equal-rank events remain deterministic.
- `resolve_value` is the only basketball-aware step. It maps the event payload to the concrete new value for each path (e.g. `SHOT_RESULT` with `made=true` and shooting team `AWAY` writes `score.away += shot_value`).
- Events with an empty `mutates` array (`STATE_NOTE`, `ALIGN_HALFCOURT`, `SCREEN_SET`, `SCREEN_USE`, `SHOT_RELEASE`, `FT_ATTEMPT`) are **read-only markers**: they are part of the timeline for replay and analytics but contribute nothing to the folded `GameState`.

## Same-Timestamp Priority

When two events share a `t_game`, the fold applies them in ascending `same_ts_priority_rank` (1 = highest precedence, 6 = lowest). Rank assignments for v0.1.0 are pinned in the catalog and cross-referenced by `docs/foundation/invariants.md` (todo 4). The full rank distribution:

| Rank | Events |
|---|---|
| 1 | `CLOCK_EXPIRY_ADJUDICATION`, `GAME_START`, `GAME_END`, `PERIOD_START`, `HALFTIME` |
| 2 | `PASS`, `HANDOFF`, `LOOSE_BALL_RECOVER`, `POSSESSION_GAINED`, `INBOUND_START`, `INBOUND_TOUCH` |
| 3 | `SHOT_RELEASE`, `SHOT_RESULT`, `FT_ATTEMPT`, `FT_RESULT` |
| 4 | `FOUL`, `VIOLATION`, `SHOT_CLOCK_VIOLATION`, `SUB`, `TIMEOUT_START`, `TIMEOUT_END`, `FT_START`, `FT_SEQUENCE_END` |
| 5 | `REBOUND`, `HELD_BALL`, `JUMP_BALL_TAP`, `STEAL`, `TURNOVER`, `PERIOD_END`, `MADE_BASKET_DEAD`, `OOB` |
| 6 | `ADVANCE_BACKCOURT`, `CROSS_HALF`, `ALIGN_HALFCOURT`, `SCREEN_SET`, `SCREEN_USE`, `DRIVE`, `STATE_NOTE` |

## Closed Catalog

The v0.1.0 catalog is **closed** at exactly 38 events. Adding or removing any event requires a `foundation_version` bump (see Version Discipline below) and a parallel update to this list, the JSON catalog, the schema's `minItems`/`maxItems`, and the invariants priority table (todo 4). One-line purpose for each:

| # | Type | Purpose |
|---|---|---|
| 1 | `GAME_START` | Marks the start of the game; transitions `PRE_GAME` → `JUMP_BALL`. |
| 2 | `JUMP_BALL_TAP` | Tip-off or held-ball tap; one jumper directs the ball to a teammate. |
| 3 | `POSSESSION_GAINED` | Ball control established; resets shot clock to 24. |
| 4 | `INBOUND_START` | Begins a dead-ball inbound procedure at a given spot. |
| 5 | `INBOUND_TOUCH` | Inbound pass released and first touched by the receiver. |
| 6 | `ADVANCE_BACKCOURT` | Ball handler advancing within the backcourt (cosmetic). |
| 7 | `CROSS_HALF` | Ball handler crossing the time line into the frontcourt (cosmetic). |
| 8 | `ALIGN_HALFCOURT` | Semantic/cosmetic marker that the offense has reached its halfcourt set. Not pose authority — actual player positions come from `LiveWorld` relation resolution, not from this event. |
| 9 | `PASS` | Live-ball pass from passer to receiver. |
| 10 | `HANDOFF` | Ball exchanged via handoff between two teammates. |
| 11 | `SCREEN_SET` | Off-ball screener establishes a screen (cosmetic). |
| 12 | `SCREEN_USE` | Ball handler uses a teammate's screen (cosmetic). |
| 13 | `DRIVE` | Ball handler drives toward the basket (cosmetic). |
| 14 | `SHOT_RELEASE` | Shot taken; shooter, value, and zone locked; result pending. |
| 15 | `SHOT_RESULT` | Resolution of a shot; on make increments score, on miss triggers rebound. |
| 16 | `REBOUND` | Player gains possession after a missed shot. |
| 17 | `LOOSE_BALL_RECOVER` | Player recovers a loose ball and establishes possession. |
| 18 | `STEAL` | Defender legally takes the ball from an offensive player. |
| 19 | `TURNOVER` | Offensive team loses possession without a recorded steal. |
| 20 | `FOUL` | Personal foul; updates player/team foul counters, may award FTs. |
| 21 | `VIOLATION` | Non-foul rule infraction; possession awarded per rule. |
| 22 | `SHOT_CLOCK_VIOLATION` | Shot clock expires without a legal attempt; possession turns over. |
| 23 | `PERIOD_END` | A period expires; advances period counter and transitions phase. |
| 24 | `PERIOD_START` | A new period begins; transitions out of a break phase. |
| 25 | `HALFTIME` | Halftime begins after Q2. |
| 26 | `TIMEOUT_START` | Team-called timeout begins; decrements allotment. |
| 27 | `TIMEOUT_END` | Timeout concludes; phase returns to prior dead state. |
| 28 | `SUB` | Substitution at a legal dead-ball window. |
| 29 | `FT_START` | Free throw sequence begins for a shooter with N attempts. |
| 30 | `FT_ATTEMPT` | A single free throw attempt released. |
| 31 | `FT_RESULT` | Resolution of a free throw attempt; on make increments score. |
| 32 | `FT_SEQUENCE_END` | Free throw sequence completes; phase exits `FT_SEQUENCE`. |
| 33 | `MADE_BASKET_DEAD` | Dead ball after a made basket; evaluates stoppage rules. |
| 34 | `OOB` | Ball goes out of bounds; possession to opposing team. |
| 35 | `HELD_BALL` | Two opposing players jointly possess the ball; jump ball follows. |
| 36 | `CLOCK_EXPIRY_ADJUDICATION` | Authoritative decision when a clock reaches zero; may emit follow-up events. |
| 37 | `GAME_END` | Final period concludes without a tie; transitions to `POST_GAME`. |
| 38 | `STATE_NOTE` | Cosmetic annotation; never mutates `GameState`. |

## Version Discipline

The catalog is the spine of every downstream wave. Version discipline follows the policy in [`docs/foundation/README.md`](README.md) and is summarized here for the event surface specifically:

- **Patch** (`0.1.0` → `0.1.1`): prose or `notes` clarifications, doc fixes. No new event types, no priority changes, no `mutates` shape changes.
- **Minor** (`0.1.0` → `0.2.0`): additive and backward compatible. New `EventType`, new optional payload field on an existing event, or a new `mutates` path on an existing event. Existing event semantics must remain intact. The schema's `minItems: 38` / `maxItems: 38` constraints must be widened at the same time, and the new event must be added to this doc's Closed Catalog table.
- **Major** (`0.1.0` → `1.0.0`): breaking. Renaming or removing an event, changing a `same_ts_priority_rank`, removing a `mutates` path, or changing the meaning of an existing payload field. Requires regenerating the golden seed (todo 20) and re-running the pace gate (todo 21).

Any change to `config/event-catalog.json` without a matching `foundation_version` bump in `config/foundation.json` is a contract violation and will fail the foundation linter (todo 9).
