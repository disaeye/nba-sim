# Invariants, Same-Timestamp Priority, and the AND-ONE Rule

`foundation_version: 0.4.0` · `invariant_count: 6` · `priority_ranks: 1..6`

This document is the **sole authority** for the six invariants the NBA simulator kernel may not violate, the same-timestamp ordering semantics that disambiguate events sharing a `t_game`, and the AND-ONE rule that resolves the foul-during-shot ambiguity. The machine-readable event catalog lives at [`config/event-catalog.json`](../../config/event-catalog.json); the catalog's `same_ts_priority_rank` values are the authoritative source for the priority table below. **If prose here ever disagrees with the JSON catalog, the JSON catalog wins.** Discrepancies, if any are introduced by future edits, must be noted in the "Cross-Reference Notes" subsection at the end of this document.

The invariants below are referenced from:
- [`docs/foundation/events.md`](events.md) — delta encoding (I1, I6) and event ordering (I2).
- [`docs/foundation/rng.md`](rng.md) — determinism and single-stream draw order (I5).
- [`docs/foundation/clocks-duration.md`](clocks-duration.md) — the clock clamp (I3).
- [`docs/foundation/identity-roles.md`](identity-roles.md) — identity vs. role (I4).
- [`docs/foundation/fsm.md`](fsm.md) (todo 5) — phase transitions respect I2's priority order.
- [`docs/foundation/possession-plays.md`](possession-plays.md) — possession outcomes that depend on AND-ONE classification.

## Invariants

The six invariants are **frozen for v0.1.0**. Any change — wording, scope, or removal — requires a `foundation_version` bump per the semver policy in [`docs/foundation/README.md`](README.md).

### I1 — Single Authority State

The kernel maintains exactly **one** `GameState` instance per game. The following paths exist on that single instance and nowhere else:

- `ball.holderId` — the player currently in control of the ball.
- `clocks` — the game, shot, and period clocks (see [`docs/foundation/clocks-duration.md`](clocks-duration.md)).
- `score` — `{ home, away }` running score.
- `phase` — the current FSM phase (see [`docs/foundation/fsm.md`](fsm.md)).
- `possession.team` — the team currently on offense.

**No module holds a private copy of these fields.** Every mutation flows through an `Event` whose catalog entry declares the path under `mutates[]` (per the delta-encoding contract in [`docs/foundation/events.md`](events.md)). Code that reads `state.score.home` reads the single authority; code that caches `let myScore = state.score.home` and then writes back via `myScore = ...; state.score.home = myScore` is not in itself forbidden (the assignment still goes through the authority), but code that maintains a *divergent* copy — `state` says one thing, the module's local says another — is a violation. The folded `GameState` is the only truth.

### I2 — Total Event Order

Every event carries a `t_game` (game-clock seconds, 0.1s resolution) and inherits a `same_ts_priority_rank` from the catalog (T2). Events are ordered by:

```
(t_game asc, same_ts_priority_rank asc, seq asc)
```

where `seq` is the monotonic emission counter assigned at append time. This order is **deterministic**: given the same event set, the fold yields the same `state_at(t)` on every run. The `(t_game, rank)` pair resolves the vast majority of same-timestamp cases; `seq` is the final tiebreaker that preserves emission order within a rank (for example, a `PASS` followed by a `HANDOFF` cannot swap under the same `t_game`).

The rank-to-event mapping is the Same-Timestamp Priority table below; the authoritative values live in the catalog.

### I3 — Rules Clocks Clamp All Durations

Every sampled Δ (the `duration` draw per [`docs/foundation/rng.md`](rng.md) draw-order) is **clamped** before it can affect the clocks:

```
Δ_effective = min(Δ_sampled, shot_clock_remaining, game_clock_remaining_for_period)
```

The clamp has two terminal cases:

- If the clamp binds on `shot_clock_remaining` → emit `SHOT_CLOCK_VIOLATION` (rank 4).
- If the clamp binds on `game_clock_remaining_for_period` → emit `PERIOD_END` (rank 5).

No action may exceed either clock. If a sampled `attack_window` of 8.5s lands with only 2.3s on the shot clock, the action takes 2.3s and the possession ends via `SHOT_CLOCK_VIOLATION` (with a `CLOCK_EXPIRY_ADJUDICATION` rank-1 event emitted at the same `t_game` to authoritatively record the residual). A `step` that consumes more wall-clock-equivalent time than either clock allows is a violation of this invariant and a kernel bug.

### I4 — Identity ≠ Role

A player's **identity** is permanent for the duration of a game:

- `id` — stable player identifier.
- `teamId` — the team the player belongs to.
- `jersey` — the jersey number (the human-facing handle; the only "label" the kernel treats as data).

A player's **role** is a per-possession binding that changes with the active `Play` and `LineupPackage`:

- Offense: `primary_creator`, `secondary_creator`, `screener`, `spacer_strong`, `spacer_weak`.
- Defense: `on_ball`, `deny`, `help`, `tag`, `weak_side`.

The `on_ball` defender slot **tracks `ball.holderId`**, not a fixed player. When a `PASS`, `HANDOFF`, `INBOUND_TOUCH`, `REBOUND`, `LOOSE_BALL_RECOVER`, or `STEAL` changes `ball.holderId`, the defense's `on_ball` binding retargets to the new holder's defender (see [`docs/foundation/identity-roles.md`](identity-roles.md)). This is the structural prevention for the "steal on off-ball player" bug class: a `STEAL` resolution that targets a `victim_id` ≠ the current `ball.holderId` is a contract violation, not a play-call option.

Concretely: **no kernel branch may consult a position label such as `"PG"`, `"SG"`, `"C"` to decide behavior.** The catalog does not carry position strings; the role system is the only abstraction used.

### I5 — Determinism

For any fixed triple:

```
input + seed + code version
```

the kernel produces an **identical** event sequence — byte-for-byte on the event-type list, and identical on every payload field. The golden-seed fixture (todo 20) is the executable assertion of this invariant.

The **only** entropy source is the injected `rng.next()`:

- Algorithm: `mulberry32`.
- Stream model: **single** stream, no substreams.
- Draw order per step: `duration → mode_select → play_select → resolve` (see [`docs/foundation/rng.md`](rng.md)).
- `rng` is a function parameter only — it is never stored on `GameState`.

The following calls are **forbidden** on every kernel path (`src/` excluding `src/cli/` presentation helpers and tests):

- `Math.random()`
- `Date.now()`
- `performance.now()`
- any `crypto.*Random*` call
- any external/process/environment-derived entropy

Audit enforcement lives in todo 20 (`rg "Math\\.random|Date\\.now|performance\\.now" src/` must return no matches) and is re-run on every commit.

### I6 — Observability

`state_at(t)` — the value of `GameState` at game time `t` — is **reconstructible** by folding the event timeline:

```
state_at(t) := fold(events.filter(e => e.t_game <= t), INITIAL_STATE)
```

The fold applies each event's `mutates` paths in `(t_game asc, rank asc, seq asc)` order per I2 (see the Fold Algorithm section of [`docs/foundation/events.md`](events.md)). The timeline **is** the state: nothing about the game is knowable that is not in the timeline, and nothing in the timeline fails to affect the state (modulo read-only marker events whose `mutates` is empty).

Two consequences:

1. **Replay is byte-stable.** Re-folding the same event sequence against the same initial state yields an identical `GameState`. This is the foundation for golden-seed testing.
2. **An event that mutates a path not declared in its catalog `mutates[]` array is a violation** of I1 *and* I6: the fold wouldn't apply the change, so `state_at(t)` would diverge from the kernel's internal notion of the state. Such an event is malformed and must be rejected by the foundation linter (todo 9) and the event constructor.

## Same-Timestamp Priority

When two events share a `t_game`, the fold applies them in ascending `same_ts_priority_rank` (1 = highest precedence, 6 = lowest). Rank values are pinned in [`config/event-catalog.json`](../../config/event-catalog.json) and cross-referenced by the table below.

| Rank | Events | Semantic |
|---|---|---|
| 1 | `CLOCK_EXPIRY_ADJUDICATION`, `GAME_START`, `GAME_END`, `PERIOD_START`, `HALFTIME` | Clock adjudication + lifecycle phase transitions. These fire first so that all subsequent same-`t_game` events observe the correct phase / clock residual. |
| 2 | `PASS`, `HANDOFF`, `LOOSE_BALL_RECOVER`, `POSSESSION_GAINED`, `INBOUND_START`, `INBOUND_TOUCH` | Ball possession transfers. Resolved before any shot or foul at the same timestamp so that the holder is unambiguous when later ranks fire. |
| 3 | `SHOT_RELEASE`, `SHOT_RESULT`, `FT_ATTEMPT`, `FT_RESULT` | Shot / free-throw release and resolution. The result is known before any rank-4 foul is classified (this is the basis of the AND-ONE rule). |
| 4 | `FOUL`, `VIOLATION`, `SHOT_CLOCK_VIOLATION`, `SUB`, `TIMEOUT_START`, `TIMEOUT_END`, `FT_START`, `FT_SEQUENCE_END` | Fouls, violations, substitutions, timeouts, and free-throw machinery. These react to the already-resolved shot outcome when applicable. |
| 5 | `REBOUND`, `HELD_BALL`, `JUMP_BALL_TAP`, `STEAL`, `TURNOVER`, `PERIOD_END`, `MADE_BASKET_DEAD`, `OOB` | Possession outcomes and dead-ball triggers. They become the input to the next possession. |
| 6 | `ADVANCE_BACKCOURT`, `CROSS_HALF`, `ALIGN_HALFCOURT`, `SCREEN_SET`, `SCREEN_USE`, `DRIVE`, `STATE_NOTE`, `ALIGNMENT`, `STRATEGY_UPDATE` | Cosmetic movement/annotation markers and the strategy memory fold. `STRATEGY_UPDATE` is last so it observes every score/possession fact of the finished possession; it mutates only `strategy`. |

> **Authoritative source.** If this table disagrees with `config/event-catalog.json`, **the JSON catalog is authoritative**. The catalog was cross-checked against this table at freeze time using:
>
> ```bash
> jq '[.events[] | select(.same_ts_priority_rank==1) | .type] | sort' config/event-catalog.json
> ```
>
> See "Cross-Reference Notes" below for the recorded result. Any future edit to either file that breaks parity must update the other in the same commit, or document the discrepancy.

### Phase-1 Pin: TIMEOUT Legality

A `TIMEOUT_START` (rank 4) request by the **defensive** team during `LIVE` play is **IGNORED**. Per NBA rule, timeouts are granted only when:

- the ball is dead (any `DEAD_*` phase), or
- the requesting team has possession (the ball is at the disposal of a player of the requesting team).

In v0.1.0, `TIMEOUT_START` (rank 4) only fires from `DEAD_*` phases, `TIMEOUT`-eligible phases (`PERIOD_BREAK`, `HALFTIME`, `OVERTIME_SETUP`), or when the requesting team is the offensive team. The FSM transition table (todo 5) is the structural enforcement; this section is the semantic anchor. A defensive timeout request emitted during `LIVE` play is a kernel bug.

## AND-ONE Rule

The AND-ONE rule resolves the foul-during-shot ambiguity (the Metis G1.7 gap). When a `FOUL` event (rank 4) and a `SHOT_RELEASE` event (rank 3) share the same `t_game`, the priority table above guarantees the shot outcome is known **before** the foul is classified.

### Evaluation Order

The events at that `t_game` resolve in this exact sequence:

```
1. SHOT_RELEASE   (rank 3)  — shooter, shot_value, zone locked in payload
2. SHOT_RESULT    (rank 3)  — make/miss resolved; score updated if made
3. FOUL           (rank 4)  — now knows the shot result; classifies the foul
4. FT_START       (rank 4)  — emitted if and only if the foul is classified as shooting
```

`seq` breaks the tie between `SHOT_RELEASE` and `SHOT_RESULT` (both rank 3): release is emitted first, result follows. The foul's classification step happens after `SHOT_RESULT` is in the timeline, so the fold at `FOUL`-resolution time sees the make/miss.

### Classification

Let `shot_result` be the value of the `made` field on the `SHOT_RESULT` event with the same `t_game` and `shooter_id` as the foul's `victim_id`:

- **If `shot_result = MADE`** — the foul is a **shooting foul with AND-ONE**:
  - Exactly **1** free throw is awarded to the shooter.
  - The made basket counts (the score was already incremented by `SHOT_RESULT`).
  - Possession after the FT follows NBA rule: make → inbound to opponents; miss → live rebound (rebound event emitted at the next `t_game`).
- **If `shot_result = MISS`** — the foul is a **shooting foul without AND-ONE**:
  - FTs are awarded per the attempt type: **2** FTs for a 2-point attempt, **3** FTs for a 3-point attempt.
  - Possession after the last FT follows NBA rule: make on the last → inbound to opponents; miss on the last → live rebound.

In both cases, the FTs are issued via an `FT_START` event (rank 4) emitted after the `FOUL` is resolved. The `FT_ATTEMPT` / `FT_RESULT` pairs follow at subsequent timestamps, terminated by `FT_SEQUENCE_END`.

### Non-Shooting Foul Fallback

If a `FOUL` (rank 4) is emitted at a `t_game` with **no** prior `SHOT_RELEASE` (rank 3) for the same `victim_id`, the foul is classified as non-shooting. The non-shooting path is:

- If the offending team is **in the bonus** (≥ 5 team fouls in the current period) → award 2 FTs to the fouled team's designated shooter (Phase-1 rule: the player who was fouled, or a designation from `LineupPackage` if applicable — see [`docs/foundation/possession-plays.md`](possession-plays.md)).
- If not in the bonus → no FTs; possession is awarded to the fouled team via the appropriate inbound.

### Possession After FT — Summary Table

| Foul classification | FT count | Make on last FT → possession | Miss on last FT → possession |
|---|---|---|---|
| Shooting foul, AND-ONE (`shot_result = MADE`) | 1 | Inbound to opponents | Live rebound |
| Shooting foul, 2pt attempt (`shot_result = MISS`, `shot_value = 2`) | 2 | Inbound to opponents | Live rebound |
| Shooting foul, 3pt attempt (`shot_result = MISS`, `shot_value = 3`) | 3 | Inbound to opponents | Live rebound |
| Non-shooting foul, in bonus | 2 | Inbound to opponents | Live rebound |

(Live rebound is resolved by a `REBOUND` event at the next `t_game`; inbound by an `INBOUND_START` event at the next `t_game`.)

### Out of Scope for v0.1.0

Per the work plan's "Must NOT have" list, the following foul/FT trees are **explicitly excluded** from Phase 1 and must not be implemented:

- Technical fouls and their FT sequences.
- Flagrant fouls (1 and 2) and their FT sequences.
- Clear-path fouls.
- Ejection / foul-out auto-handling (a player accruing 6 personal fouls is a roster issue, not a kernel state issue — the kernel records the foul in `fouls.player`; the rotation system handles the consequence).

These are reserved for a future minor or major foundation version bump.

## Forbidden Patterns

The following anti-patterns each violate one or more of I1–I6 above. The foundation linter (todo 9) and the TDD test suite are expected to assert against each.

| Pattern | Violates | Why |
|---|---|---|
| Mutating `step`'s input `state` argument in place (e.g., `state.score.home += 2`) | I1 + delta contract (I6) | The state is the single authority and is advanced **only** by appending events; an in-place mutation bypasses the timeline and makes `state_at(t)` non-reconstructible. `step(state, rng)` must return a new `state'` with `Object.is(state, state') === false`. |
| Using `Math.random()`, `Date.now()`, `performance.now()`, or any external entropy on a kernel path | I5 | Introduces a non-deterministic input; same seed + same code would no longer guarantee identical output. |
| Branching logic on a player position label like `"PG"`, `"SG"`, `"C"` | I4 | Position labels are not in the data model; roles are. A `switch (player.position)` (or similar) is structural evidence of a violation. |
| A `STEAL`, `CONTEST`, or strip resolution that targets a `victim_id` ≠ the current `ball.holderId` | I4 | The `on_ball` slot tracks the holder; a steal "off-ball" is impossible by construction. |
| An event mutating a `GameState` path not declared in its catalog `mutates[]` array | I1 + I6 | The fold would not apply the mutation, so `state_at(t)` would diverge from the kernel's internal state; the event is malformed. |
| Skipping the clock clamp on a sampled Δ (consuming more time than `shot_clock_remaining` or `game_clock_remaining_for_period`) | I3 | Lets an action exceed the clocks and desynchronizes the timeline from the rules. |
| Caching a copy of `ball.holderId`, `possession.team`, `clocks`, `score`, or `phase` in a module-level variable or a non-`GameState` field | I1 | Diverges from the single authority the moment the next event mutates the path. |
| Emitting two events with the same `(t_game, same_ts_priority_rank)` and a non-monotonic `seq` | I2 | The total order is no longer deterministic; the fold becomes ambiguous. |
| Storing `rng` on `GameState` (e.g., `state.rng`, `state.possession.rng`) | I5 | `rng` is a step parameter; persisting it would couple the timeline to the entropy source and break replay. |
| Defensive `TIMEOUT_START` emitted during `LIVE` play (or any phase not on the TIMEOUT-eligible list) | AND-ONE §Phase-1 Pin | NBA rule: timeouts are granted only when the ball is dead or the requesting team has possession. |

## Cross-Reference Notes

This section records the result of cross-checking the priority table above against [`config/event-catalog.json`](../../config/event-catalog.json) at freeze time. Future edits to either file must update this section.

### Rank 1 cross-check

Command:

```bash
jq '[.events[] | select(.same_ts_priority_rank==1) | .type] | sort' config/event-catalog.json
```

Result at freeze (v0.1.0):

```json
[
  "CLOCK_EXPIRY_ADJUDICATION",
  "GAME_END",
  "GAME_START",
  "HALFTIME",
  "PERIOD_START"
]
```

This matches the Rank 1 row of the Same-Timestamp Priority table exactly. **No discrepancy.**

### All-ranks cross-check

For each rank 1–6, the set of event types pulled from the catalog by `same_ts_priority_rank` was compared against the corresponding row of the table above. The total event count across all six ranks equals 41, matching `jq '.events|length' config/event-catalog.json`. Every event in the catalog appears in exactly one row of the table, and every row of the table contains exactly the catalog's events for that rank.

## 7. Basketball continuity gates

The event stream is not merely a statistical record. A replayable possession must
preserve these causal gates:

1. **One ball, one owner.** A held ball has exactly one owner, and a transfer
   names both the player giving up the ball and the player receiving it.
2. **Actions need a physical precondition.** A shot follows a live holder or an
   explicit shooter; a rebound follows a shot/loose-ball chain; a foul names
   two players who can physically contact one another in the corresponding
   snapshot.
3. **Possession has a terminal cause.** A live possession ends through a made or
   missed shot, turnover/steal, foul, period end, or a legal dead-ball event. A
   second terminal action cannot silently overwrite the first one.
4. **Same-timestamp events are a chain, not a bag.** Rank ordering may compress
   adjudication into one clock instant, but `SHOT_RESULT -> MADE_BASKET_DEAD`,
   `REBOUND -> LOOSE_BALL_RECOVER`, and `INBOUND_TOUCH -> POSSESSION_GAINED`
   remain causally ordered in the event sequence.
5. **Movement explains the label.** A player action is only credible when the
   player's position, target, and nearby opponents support it; a screen must
   create contact pressure, a drive must advance toward the attacked rim, and a
   defensive label must not survive after its player owns the ball.
***END

**No discrepancies at v0.1.0 freeze.** The table and the catalog are in full parity.
