# FSM Phases and Transition Table

`foundation_version: 0.4.0` · `phase_count: 15` · `transition_count: 28`

This document is the **sole authority** for the NBA simulator's finite-state machine: the closed set of phases the game may occupy, the legal `(from, trigger) → to` transitions between them, the period router that resolves end-of-period transitions, the jump-ball resolution rule, the team-timeout allotment, and the substitution legality table. The machine-readable transition table lives at [`config/fsm.json`](../../config/fsm.json); its JSON Schema is at [`config/schemas/fsm.schema.json`](../../config/schemas/fsm.schema.json). **If prose here ever disagrees with `config/fsm.json`, the JSON is authoritative.** Discrepancies introduced by future edits must be recorded in the "Cross-Reference Notes" subsection at the end of this document.

The FSM is referenced from:
- [`docs/foundation/events.md`](events.md) — events whose `mutates` includes `phase` (per the catalog) are emitted by FSM transitions.
- [`docs/foundation/invariants.md`](invariants.md) — I2 total event order; the rank-4 `TIMEOUT_START` legality pin; the AND-ONE rule that pairs `SHOT_RELEASE` (rank 3) with `FOUL` (rank 4) and routes through `DEAD_FOUL → FT_SEQUENCE`.
- [`docs/foundation/clocks-duration.md`](clocks-duration.md) — period/shot clock expiries feed the `PERIOD_CLOCK_EXPIRY` and `SHOT_CLOCK_EXPIRY` triggers.
- [`docs/foundation/possession-plays.md`](possession-plays.md) — every possession begins from `LIVE` (post-inbound) and terminates into one of the `DEAD_*` phases.
- [`docs/foundation/resolve.md`](resolve.md) — the FT sequence phase machine and its terminal trigger `FT_DONE_SHOOTING_FOUL`.

The phases and transitions below are **frozen for v0.1.0**. Any change — wording, scope, addition, removal — requires a `foundation_version` bump per the semver policy in [`docs/foundation/README.md`](README.md).

## Phases

The closed set of 15 FSM phases for v0.1.0. Each phase is a `GameState.phase` value; the only legal values are exactly these 15 strings.

| Phase | Description |
|---|---|
| `PRE_GAME` | Initial state. The game has been initialized but tip-off has not occurred. Entered exactly once; left via `GAME_START`. |
| `JUMP_BALL` | A jump ball is in progress (opening tip or a held-ball tip). Ball is not yet at any player's disposal; no shot clock is running. Resolved by `JUMP_BALL_TAP` → `LIVE`. |
| `LIVE` | The ball is live and a possession is in progress. Both the shot clock and the period clock are ticking. The vast majority of gameplay events (`PASS`, `HANDOFF`, `DRIVE`, `SHOT_RELEASE`, etc.) are emitted from this phase. |
| `DEAD_OOB` | Ball went out of bounds; possession awarded to the opponents. Left via `INBOUND_SETUP` → `LIVE`. |
| `DEAD_FOUL` | A foul has been called and the ball is dead. Left via either `FT_AWARDED` → `FT_SEQUENCE` (shooting or non-shooting-in-bonus) or `INBOUND_SETUP` → `LIVE` (non-shooting, not in bonus). |
| `DEAD_VIOLATION` | A rule violation (e.g. shot-clock violation, lane violation, kicked ball) has stopped play. Possession awarded to the opponents. Left via `INBOUND_SETUP` → `LIVE`. |
| `DEAD_MAKE` | A field goal was made; play stops while the stoppage rule is evaluated (last-1:00 of Q1-Q3, last-2:00 of Q4/OT — see [`docs/foundation/clocks-duration.md`](clocks-duration.md)). Left via `INBOUND_SETUP` → `LIVE`. |
| `DEAD_HELD` | Two opposing players are jointly holding the ball; play is dead pending a jump ball. Left via `JUMP_BALL_SETUP` → `JUMP_BALL`. |
| `DEAD_PERIOD_END` | The period clock reached zero. Left via one of the `PERIOD_ROUTER_*` triggers (see Period Router). |
| `FT_SEQUENCE` | A free-throw sequence is in progress. The shooter is at the line; `FT_ATTEMPT`/`FT_RESULT` pairs are emitted. Left via `FT_DONE_SHOOTING_FOUL` → `DEAD_FOUL` (which then re-evaluates possession). |
| `TIMEOUT` | A team-called timeout is in progress. The phase is reachable via the `TIMEOUT_START` event (rank 4) from any `DEAD_*` phase; play resumes via `TIMEOUT_END` to the prior dead-ball state. (See Timeout Rules — `TIMEOUT` entry/exit transitions are not enumerated in the v0.1.0 transition table; the FSM driver handles them inline via the catalog's `TIMEOUT_START`/`TIMEOUT_END` events whose `mutates` includes `phase`.) |
| `PERIOD_BREAK` | Between-periods break (Q1→Q2, Q3→Q4). Left via `BREAK_END` → `LIVE` (next period starts with a live-ball inbound). |
| `HALFTIME` | Halftime break (after Q2). Left via `BREAK_END` → `LIVE` (Q3 begins with a live-ball inbound). |
| `OVERTIME_SETUP` | An overtime period is about to begin (game was tied at end of Q4 or end of a prior OT). Left via `BREAK_END` → `JUMP_BALL` (OT begins with a tip, not an inbound). |
| `POST_GAME` | Terminal state. The game has ended (Q4 or OT ended with the score not tied). The engine halts; no further transitions fire. |

## Transition Table

The transition table is the structural enforcement of the FSM. Each row is a 5-tuple:

```
(from, trigger) -> to, emitting emit_events[]
```

- **`from`** — the source phase (must be one of the 15 phases above).
- **`trigger`** — the FSM-internal input that fires the transition. Triggers are **not** `EventType`s themselves; they are the kernel-side predicates that, when true, cause the transition to fire and the listed `emit_events` to be appended to the timeline.
- **`to`** — the destination phase (must be one of the 15 phases above).
- **`emit_events`** — the ordered list of `EventType`s to emit when this transition fires. Every name MUST exist in [`config/event-catalog.json`](../../config/event-catalog.json) (cross-checked by the foundation linter in todo 9). Emission order within the same `t_game` follows the catalog's `same_ts_priority_rank` (rank 1 first, rank 6 last) per I2 in [`docs/foundation/invariants.md`](invariants.md).
- **`notes`** — short human-readable explanation.

> **Authoritative source.** [`config/fsm.json`](../../config/fsm.json) is the single source of truth. The table below is rendered from the same content; if the two ever diverge, the JSON wins. The schema at [`config/schemas/fsm.schema.json`](../../config/schemas/fsm.schema.json) enforces: 15 phases (min/max), unique phase names, every `from`/`to` ∈ phases, every transition has the 5 required fields, no extra fields, timeout allotment integer consts, and the legal/illegal substitution phase lists.

### Full transition list (28 rows)

| # | from | trigger | to | emit_events |
|---|---|---|---|---|
| 1 | `PRE_GAME` | `GAME_START` | `JUMP_BALL` | `GAME_START` |
| 2 | `JUMP_BALL` | `JUMP_BALL_TAP` | `LIVE` | `JUMP_BALL_TAP`, `POSSESSION_GAINED` |
| 3 | `LIVE` | `MADE_BASKET` | `DEAD_MAKE` | `SHOT_RESULT`, `MADE_BASKET_DEAD` |
| 4 | `LIVE` | `OOB` | `DEAD_OOB` | `OOB` |
| 5 | `LIVE` | `FOUL_SHOOTING` | `DEAD_FOUL` | `FOUL` |
| 6 | `LIVE` | `FOUL_NON_SHOOTING_BONUS` | `DEAD_FOUL` | `FOUL` |
| 7 | `LIVE` | `FOUL_NON_SHOOTING_NO_BONUS` | `DEAD_FOUL` | `FOUL` |
| 8 | `LIVE` | `VIOLATION` | `DEAD_VIOLATION` | `VIOLATION` |
| 9 | `LIVE` | `SHOT_CLOCK_EXPIRY` | `DEAD_VIOLATION` | `CLOCK_EXPIRY_ADJUDICATION`, `SHOT_CLOCK_VIOLATION` |
| 10 | `LIVE` | `HELD_BALL` | `DEAD_HELD` | `HELD_BALL` |
| 11 | `LIVE` | `PERIOD_CLOCK_EXPIRY` | `DEAD_PERIOD_END` | `CLOCK_EXPIRY_ADJUDICATION`, `PERIOD_END` |
| 12 | `DEAD_MAKE` | `INBOUND_SETUP` | `LIVE` | `INBOUND_START`, `INBOUND_TOUCH`, `POSSESSION_GAINED` |
| 13 | `DEAD_OOB` | `INBOUND_SETUP` | `LIVE` | `INBOUND_START`, `INBOUND_TOUCH`, `POSSESSION_GAINED` |
| 14 | `DEAD_VIOLATION` | `INBOUND_SETUP` | `LIVE` | `INBOUND_START`, `INBOUND_TOUCH`, `POSSESSION_GAINED` |
| 15 | `DEAD_FOUL` | `FT_AWARDED` | `FT_SEQUENCE` | `FT_START` |
| 16 | `DEAD_FOUL` | `INBOUND_SETUP` | `LIVE` | `INBOUND_START`, `INBOUND_TOUCH`, `POSSESSION_GAINED` |
| 17 | `FT_SEQUENCE` | `FT_DONE_SHOOTING_FOUL` | `DEAD_FOUL` | `FT_SEQUENCE_END` |
| 18 | `DEAD_HELD` | `JUMP_BALL_SETUP` | `JUMP_BALL` | — |
| 19 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_Q1` | `PERIOD_BREAK` | `PERIOD_START` |
| 20 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_Q2` | `HALFTIME` | — |
| 21 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_Q3` | `PERIOD_BREAK` | — |
| 22 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_Q4_TIED` | `OVERTIME_SETUP` | — |
| 23 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_Q4_NOT_TIED` | `POST_GAME` | `GAME_END` |
| 24 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_OT_TIED` | `OVERTIME_SETUP` | — |
| 25 | `DEAD_PERIOD_END` | `PERIOD_ROUTER_OT_NOT_TIED` | `POST_GAME` | `GAME_END` |
| 26 | `PERIOD_BREAK` | `BREAK_END` | `LIVE` | `PERIOD_START` |
| 27 | `HALFTIME` | `BREAK_END` | `LIVE` | `PERIOD_START` |
| 28 | `OVERTIME_SETUP` | `BREAK_END` | `JUMP_BALL` | `PERIOD_START` |

> Row 19 (`PERIOD_ROUTER_Q1 → PERIOD_BREAK`) emits `PERIOD_START` at the moment of transition; rows 21 (`PERIOD_ROUTER_Q3`) and 26 (`PERIOD_BREAK → LIVE`) split the lifecycle differently — Q3's transition emits nothing on entry into the break and `PERIOD_START` fires when the break ends via row 26. This asymmetry is preserved verbatim from the v0.1.0 freeze; the FSM driver must honor the per-row `emit_events` list rather than inferring from the trigger name.

### Coverage invariants

- **Every `DEAD_*` phase has at least one exit.** Rows 12–14, 16 cover `DEAD_MAKE`, `DEAD_OOB`, `DEAD_VIOLATION`, `DEAD_FOUL`. Row 15 covers `DEAD_FOUL` → `FT_SEQUENCE`. Row 18 covers `DEAD_HELD`. Rows 19–25 cover `DEAD_PERIOD_END`. The dead-ball family is therefore fully reachable to `LIVE` or to a downstream state (`FT_SEQUENCE`, `JUMP_BALL`, break/halftime/OT, or terminal).
- **`TIMEOUT` is intentionally unreached by the v0.1.0 transition table.** The phase is reachable via the catalog's `TIMEOUT_START` event (rank 4), whose `mutates` includes `phase`. Phase-1 supports only team-called timeouts at dead balls (see Timeout Rules); the entry/exit transitions are handled inline by the FSM driver reacting to `TIMEOUT_START`/`TIMEOUT_END` events, not enumerated as discrete rows. This is documented here so the omission is not mistaken for an oversight.
- **`POST_GAME` is terminal.** No row has `from: POST_GAME`. The engine halts when this phase is entered.

## Period Router

The period router is the function `period_router(period_just_ended, score_diff)` that selects which `PERIOD_ROUTER_*` trigger fires when `DEAD_PERIOD_END` is reached. It is the structural answer to "what happens after the period clock hits zero."

| `period_just_ended` | `score_diff` (home − away) | Trigger fired | Next phase | Notes |
|---|---|---|---|---|
| Q1 | (any) | `PERIOD_ROUTER_Q1` | `PERIOD_BREAK` | Q2 break begins |
| Q2 | (any) | `PERIOD_ROUTER_Q2` | `HALFTIME` | Halftime begins |
| Q3 | (any) | `PERIOD_ROUTER_Q3` | `PERIOD_BREAK` | Q4 break begins |
| Q4 | `0` (tied) | `PERIOD_ROUTER_Q4_TIED` | `OVERTIME_SETUP` | OT setup begins |
| Q4 | `≠ 0` (not tied) | `PERIOD_ROUTER_Q4_NOT_TIED` | `POST_GAME` | Game ends; `GAME_END` emitted |
| OT (any index) | `0` (tied) | `PERIOD_ROUTER_OT_TIED` | `OVERTIME_SETUP` | Another OT setup begins (per decision M6: unlimited OT periods, each 5:00) |
| OT (any index) | `≠ 0` (not tied) | `PERIOD_ROUTER_OT_NOT_TIED` | `POST_GAME` | Game ends; `GAME_END` emitted |

The router is **deterministic**: given the period index and the score difference, exactly one trigger fires. There is no RNG involvement at this layer; the only RNG draw related to period boundaries is the jump-ball tip that follows `OVERTIME_SETUP → JUMP_BALL` (see Jump Ball).

> `score_diff` is computed from `GameState.score` at the moment `DEAD_PERIOD_END` is entered (i.e., after all rank-1..5 events at the same `t_game` as the `PERIOD_END` have been folded). The router MUST observe the post-fold score, not a pre-fold snapshot — otherwise a last-second made basket that ties the game could be missed.

## Jump Ball

A jump ball is the mechanism for starting play when no team has clear possession: at game start (opening tip) and after a held ball (`DEAD_HELD → JUMP_BALL`). The jump ball is resolved by the `JUMP_BALL_TAP` transition (`JUMP_BALL → LIVE`), which emits `JUMP_BALL_TAP` and `POSSESSION_GAINED` for the winning team.

### Resolution rule

- **Win probability is 0.5 for each team** (homogeneous-abilities Phase-1 pin). The kernel draws a single `unit(rng)` value; if `< 0.5`, the home team gains possession, otherwise the away team does. This draw occurs at the position specified by the draw-order contract in [`docs/foundation/rng.md`](rng.md) and is the only RNG draw associated with the jump ball.
- **The tapping team is the winning team.** A `JUMP_BALL_TAP` event is emitted with `tapping_team = winning_team`, `home_jumper_id` and `away_jumper_id` set to the designated jumpers (the kernel's `LineupPackage` provides a center-equivalent or tallest-remaining-roster selection — but per the I4 invariant, the kernel must NOT branch on a position label).
- **Possession is established.** A `POSSESSION_GAINED` event follows at the same `t_game` (rank 2, after the rank-5 `JUMP_BALL_TAP`). The shot clock resets to 24 via the catalog's `POSSESSION_GAINED` mutates.

### Phase-1 pin: no alternate-possession arrow

Per the work plan's T5 spec (`HELD_BALL → JUMP_BALL always`), Phase 1 always resolves a held ball with a jump ball (a fresh tip). The NBA's alternate-possession arrow rule is **not modeled** in v0.1.0; every held ball produces a new jump ball with a fresh 0.5/0.5 draw. A future minor-version bump may introduce the arrow if desired.

## Timeout Rules

Phase 1 supports **only team-called timeouts at dead balls**. The kernel accepts timeout requests via the `TIMEOUT_START` event (rank 4 in the catalog); the event's `mutates` includes `phase`, which transitions the FSM into the `TIMEOUT` phase.

### Allotments (per team)

| Pool | Count | Source |
|---|---|---|
| Regulation total (Q1–Q4) | **7** | `config/fsm.json#timeout_allotments.regulation_total` |
| Max in Q4 | **4** | `config/fsm.json#timeout_allotments.max_in_q4` (a team may not enter Q4 with more than 4 unused regulation timeouts) |
| Overtime (per OT period) | **2** | `config/fsm.json#timeout_allotments.overtime_total` |

These are the NBA modern mandatory allotments for a single team.

### Entry / exit

- **Entry:** `TIMEOUT_START` (rank 4) is honored only when the requesting team is the **offensive team** (has possession) or when the **ball is dead** (any `DEAD_*` phase). A defensive timeout request emitted during `LIVE` play is **ignored** per the AND-ONE §Phase-1 Pin in [`docs/foundation/invariants.md`](invariants.md). When honored, `TIMEOUT_START` mutates `phase` to `TIMEOUT` and decrements the requesting team's `timeouts.remaining`.
- **Exit:** `TIMEOUT_END` (rank 4) returns the FSM to the prior dead-ball state (the `from`-phase that fired the `TIMEOUT_START`). The kernel records the pre-timeout phase on entry and restores it on exit.

### Out of scope for v0.1.0

Per the work plan's "Must NOT have" list:

- **No mandatory / TV timeouts.** The Phase-1 game has no scripted stoppages for media breaks. Mandatory timeouts are deferred to a future minor version.
- **No coach challenge / replay center.** Not modeled.
- **No full timeout vs. 20-second timeout distinction.** Phase 1 treats every timeout as a single undifferentiated unit; only the allotment counters matter.

### Transition-table note

As called out in the Phases section, the `TIMEOUT` phase is reachable via the catalog's `TIMEOUT_START` event but is **not enumerated** as explicit transition rows in the v0.1.0 `config/fsm.json#transitions` array. The FSM driver recognizes `TIMEOUT_START`/`TIMEOUT_END` events directly (per their catalog `mutates: ["phase"]` declarations) and manages the `TIMEOUT` phase inline. A future minor-version bump may add explicit transition rows if the timeout lifecycle becomes more complex (e.g. mandatory timeouts).

## Substitution Legality

A `SUB` event (rank 4 in the catalog) is the only mechanism for swapping one player out and another in on the on-court lineup. The kernel accepts `SUB` events only during **legal phases**; emitting a `SUB` during an **illegal phase** is a kernel error and the FSM driver throws `IllegalPhaseError` (see todo 15 acceptance criteria: `SUB in LIVE throws/ignored per spec`).

### Legal phases (10)

`DEAD_OOB`, `DEAD_FOUL`, `DEAD_VIOLATION`, `DEAD_MAKE`, `DEAD_HELD`, `DEAD_PERIOD_END`, `TIMEOUT`, `PERIOD_BREAK`, `HALFTIME`, `OVERTIME_SETUP`.

This is the union of "any dead ball" with the break/halftime/OT phases where play is also stopped.

### Illegal phases (5)

`LIVE`, `JUMP_BALL`, `FT_SEQUENCE`, `PRE_GAME`, `POST_GAME`.

- `LIVE` and `JUMP_BALL` — ball is in active play; substitution would disrupt possession.
- `FT_SEQUENCE` — the special case; see below.
- `PRE_GAME` / `POST_GAME` — the game has not started or has ended; substitution is meaningless (the opening tip and the closing handshake are not live-ball situations for roster purposes).

### FT_SEQUENCE sub rule (CRITICAL)

`FT_SEQUENCE` is in the illegal list because mid-FT substitutions would let a team swap a poor free-throw shooter out mid-sequence, which is not legal under NBA rule and is not modeled in Phase 1. The two legal sub windows around an FT sequence are:

1. **Before the first FT attempt** — between the `FT_START` event (which enters `FT_SEQUENCE`) and the first `FT_ATTEMPT`. A `SUB` here is legal because the FSM is technically in `FT_SEQUENCE` but no attempt has been released yet. **Phase-1 simplification:** to avoid edge-case complexity, the v0.1.0 kernel treats the entire `FT_SEQUENCE` as illegal for subs and requires substitutions to occur before the `FT_AWARDED` transition fires (i.e., while still in `DEAD_FOUL`) or after the `FT_DONE_SHOOTING_FOUL` transition completes (i.e., after returning to `DEAD_FOUL`).
2. **After the sequence ends** — i.e., after `FT_SEQUENCE_END` has been emitted and the FSM has returned to `DEAD_FOUL`. Subs are freely legal here.

The simplification above is documented as a deliberate Phase-1 pin: subs are allowed only when the phase is in the legal list **excluding `FT_SEQUENCE`**. The catalog's `SUB` event remains the only substitution mechanism; the timing restriction is enforced by the FSM driver.

### Enforcement

- **Schema level:** `config/schemas/fsm.schema.json` enforces that every entry in `legal_phases` and `illegal_phases` is one of the 15 phases; the cross-property invariants (disjoint, union = all phases) are deferred to the foundation linter (todo 9) per the established ajv-cli strict-mode precedent.
- **Kernel level:** the FSM driver (todo 15) throws `IllegalPhaseError` on a `SUB` event whose current phase is not in `legal_phases`. The error is a typed kernel error, not a silent ignore — the work plan T15 acceptance criteria explicitly pins this behavior.
- **Test level:** todo 15's TDD suite includes a "no live sub allowed" test that emits a `SUB` during `LIVE` and asserts the throw.

## Out of Scope for v0.1.0

Per the work plan's "Must NOT have" list, the following are **explicitly excluded** from the Phase-1 FSM and must not appear in `config/fsm.json` or this document as live transitions:

- **Mandatory / TV timeout schedules.** Phase 1 has only team-called timeouts; the media-break lifecycle (single media timeouts, double media timeouts, etc.) is deferred.
- **Injury phases beyond a stub note.** A player injury does not produce a dedicated FSM phase; the injury is recorded via `STATE_NOTE` (cosmetic, rank 6) and the rotation system handles the substitution at the next legal window. No `INJURY_*` phase exists in the closed set.
- **Technical / flagrant / clear-path foul transitions.** The foul tree in Phase 1 is the personal-foul subset: shooting fouls (with AND-ONE per the invariants doc) and non-shooting fouls (bonus or non-bonus). No `DEAD_TECHNICAL`, `DEAD_FLAGRANT`, or `DEAD_CLEAR_PATH` phase exists.
- **Ejection / foul-out auto-handling.** A player accruing 6 personal fouls is a roster issue, not an FSM state change. The kernel records the foul via `FOUL` (which mutates `fouls.player`); the rotation system handles the consequence at the next legal substitution window.

These are reserved for a future minor or major foundation version bump.

## Cross-Reference Notes

This section records the result of cross-checking the FSM contract against [`config/event-catalog.json`](../../config/event-catalog.json) and the priority table in [`docs/foundation/invariants.md`](invariants.md) at freeze time. Future edits to any of these files must update this section.

### `emit_events` cross-check

Every event name appearing in any transition's `emit_events` array was cross-checked against the catalog. Command:

```bash
jq -r '.transitions[].emit_events[]' config/fsm.json | sort -u > /tmp/fsm-events.txt
jq -r '.events[].type' config/event-catalog.json | sort -u > /tmp/catalog-events.txt
comm -23 /tmp/fsm-events.txt /tmp/catalog-events.txt
```

The `comm -23` output (events used by the FSM but missing from the catalog) is **empty** at v0.1.0 freeze. The 18 distinct event types used by the FSM are:

```
CLOCK_EXPIRY_ADJUDICATION  FT_SEQUENCE_END     OOB                SHOT_CLOCK_VIOLATION
FOUL                       GAME_END            PERIOD_END         SHOT_RESULT
FT_START                   GAME_START          PERIOD_START       VIOLATION
HELD_BALL                  INBOUND_START       POSSESSION_GAINED
FT_SEQUENCE_END            INBOUND_TOUCH       MADE_BASKET_DEAD
```

(`JUMP_BALL_TAP` also appears, included in the count above.) Each is present in the catalog. **No discrepancy.**

### `from`/`to` enum cross-check

Every `from` and `to` value was cross-checked against the `phases` array. The schema enforces this directly via the `$defs/phaseName` enum reference; the v0.1.0 freeze passed schema validation on the first attempt (no out-of-enum phase).

### `legal_phases` / `illegal_phases` cross-check

- `legal_phases` (10 entries): `DEAD_OOB`, `DEAD_FOUL`, `DEAD_VIOLATION`, `DEAD_MAKE`, `DEAD_HELD`, `DEAD_PERIOD_END`, `TIMEOUT`, `PERIOD_BREAK`, `HALFTIME`, `OVERTIME_SETUP`.
- `illegal_phases` (5 entries): `LIVE`, `JUMP_BALL`, `FT_SEQUENCE`, `PRE_GAME`, `POST_GAME`.
- Union = all 15 phases ✓. Intersection = ∅ ✓.

The disjointness and coverage invariants are not expressible in JSON Schema draft-07 under ajv-cli strict mode; they are enforced by the foundation linter (todo 9) per the precedent set in `event-catalog.schema.json` and `duration.schema.json`.

### Priority-rank cross-check

The events emitted by FSM transitions are distributed across the priority ranks as follows (cross-referenced against the table in [`docs/foundation/invariants.md`](invariants.md)):

| Rank | FSM-emitted events |
|---|---|
| 1 | `GAME_START`, `PERIOD_START`, `GAME_END`, `CLOCK_EXPIRY_ADJUDICATION` |
| 2 | `POSSESSION_GAINED`, `INBOUND_START`, `INBOUND_TOUCH` |
| 3 | `SHOT_RESULT` |
| 4 | `FOUL`, `VIOLATION`, `SHOT_CLOCK_VIOLATION`, `FT_START`, `FT_SEQUENCE_END` |
| 5 | `JUMP_BALL_TAP`, `MADE_BASKET_DEAD`, `OOB`, `HELD_BALL`, `PERIOD_END` |

Rank 6 (cosmetic) events (`ADVANCE_BACKCOURT`, `ALIGN_HALFCOURT`, etc.) are **not** emitted by FSM transitions; they are emitted by the possession engine during `LIVE` play.

**No discrepancies at v0.1.0 freeze.**
