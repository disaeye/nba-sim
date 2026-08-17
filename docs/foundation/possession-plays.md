# Possession Modes, Plays v0, Bind, and Lifecycle

`foundation_version: 0.4.0`

This document freezes the offensive possession model: the set of valid **modes**, the closed catalog of v0 **plays** (`config/plays.json`), the algorithm the engine uses to **bind** players to play slots, and the full **possession lifecycle** including the critical OREB continuation rule.

Implementation references: identity bind + on-ball retarget, possession episode/play select, DecisionKernel (`src/decision`), continuous motion (`src/court/relations.ts`, dual-clock simulate).

> **Authority.** This document plus `config/plays.json`, `config/play-slots.json`, and their schemas are the sole authority for offensive possession **mode/play/bind** behavior. Live **movement targets** are governed by the relational live/dead split below. Code that disagrees is a bug.

### Live vs dead placement (foundation 0.4.0)

| Path | Placement model | Module |
|---|---|---|
| **Dead ball** (jump, tip receive, inbound, FT lane) | Rule-mandated **absolute** (or rule-relative) setups | `src/court/dead-setup.ts` |
| **Live ball** (possession start play call, decision windows) | **Relation-based retarget** from current poses + ball + attack rim + role binding | `src/court/relations.ts`, `config/play-slots.json` |

- Live path MUST NOT rebuild a full-team absolute halfcourt/transition stamp diagram as position truth.
- Live path MUST NOT use `waitUntilArrived` solely to “run a formation poster.”
- Plays remain **advisory** for DecisionKernel scoring (`WorldContext.playId`); there is no mechanical play-step walker as the live offense engine.
- `play-slots.json` maps play id → offense role → geometric `SlotRelation` kind. Unknown play ids use `default_halfcourt` only (no substring inference).

---

## Modes

Decision **M4**: only two offensive modes exist in v0.1.0.

| Mode | Description |
|---|---|
| `TRANSITION` | Live-ball advantage possession initiated by a steal, live-ball turnover recovery, or other after-turnover trigger. The ball moves up the floor quickly and the first action is a rim attack. |
| `HALFCOURT` | Set offense after the defense has set. The offense advances across half and runs structured actions under DecisionKernel with relation-based spacing (not a formation stamp-and-walk). |

**Excluded from v0.1.0.** `EARLY_OFFENSE` is intentionally absent. Early-offense actions (drags, early drag screens, quick hitters) are represented as half-court plays (`early_drag`) rather than as a separate mode. Adding a third mode is a major `foundation_version` bump.

The full mode set is published in `config/plays.json#modes` and pinned by `config/schemas/plays.schema.json` to exactly `["TRANSITION", "HALFCOURT"]` in that canonical order.

---

## Dead vs Live Possession Setup

Possession setups fall into two distinct paths:

**Dead-ball (absolute setups).** Tip-off, inbound, and free-throw alignments use absolute court coordinates from `src/court/dead-setup.ts`. Each builder (`buildJumpBallAlignment`, `buildTipReceiveAlignment`, `buildInboundAlignment`, `buildFtAlignment`) places all 10 players at fixed metric `(x, y)` positions. The engine then ticks players toward those targets via `waitUntilArrivedOrTimeout` with a timeout budget.

**Live possession (relational retarget).** Once a live possession begins, player positioning uses the relational retarget path: `LiveWorld` (current ball holder + basket side + lineups) is built, then `retargetFromLiveWorld` calls `resolveSlotTarget` for each `RelationKind` (e.g. ball handler, on-ball defender, help, tag) to compute targets relative to the live ball state. This is how the on-ball defender tracks the current `ball.holderId` per the retarget contract in `docs/foundation/identity-roles.md`.

**Play steps are advisory to DecisionKernel.** The play catalog defines step sequences (advance, screen, drive, etc.), but the live offense engine is `DecisionKernel` (`src/decision/`). Plays provide a `playId` bias; DecisionKernel consumes it as a soft constraint, not a hard formation script.

**No `waitUntilArrivedOrTimeout` for live formations.** The arrival-wait mechanism is used only for dead-ball absolute setups. Live possession does not pause for players to reach positions; the relational retarget updates targets every tick and players move toward them continuously.

---

## Mode Select

The mode selector consumes the live possession start reason and the single-stream RNG. `STEAL` is pinned to `TRANSITION` without a draw. Ambiguous live starts use calibrated probabilities; all dead-ball and unknown starts default to `HALFCOURT`.

```text
mode_select(start_reason, rng):
  if start_reason == STEAL:
    return TRANSITION
  if start_reason == AFTER_DEFENSIVE_REBOUND:
    return TRANSITION when rng.next() < 0.15 else HALFCOURT
  if start_reason == AFTER_MAKE:
    return TRANSITION when rng.next() < 0.02 else HALFCOURT
  if start_reason in {LIVE_BALL_TURNOVER_RECOVER, AFTER_TURNOVER}:
    return TRANSITION when rng.next() < 0.30 else HALFCOURT
  return HALFCOURT
```

| `start_reason` | Mode | RNG / notes |
|---|---|---|
| `STEAL` | `TRANSITION` | Pinned; no draw. |
| `LIVE_BALL_TURNOVER_RECOVER` | 30% `TRANSITION`, otherwise `HALFCOURT` | One draw; live-ball recovery is transition-biased. |
| `AFTER_TURNOVER` | 30% `TRANSITION`, otherwise `HALFCOURT` | One draw; transition-biased. |
| `AFTER_MAKE` | 2% `TRANSITION`, otherwise `HALFCOURT` | One draw; made-basket inbound normally faces a set defense. |
| `AFTER_DEFENSIVE_REBOUND` | 15% `TRANSITION`, otherwise `HALFCOURT` | One draw; quick outlet is possible without adding a third mode. |
| `AFTER_INBOUND` | `HALFCOURT` | Safe default; no draw. |
| `AFTER_JUMP_BALL` | `HALFCOURT` | Safe default; no draw. |
| `AFTER_OREB_CONTINUE` | (continues prior mode) | Mode is not re-selected; prior binding persists. |

The mode selector owns the logical route. `config/duration.json#start_reason_bias` remains the timing/alignment bias table; its `align_extra_seconds` and shot-clock fields do not override the probabilistic mode decision.

---

## Play Catalog v0

Closed catalog of 5 hardcoded plays (decision **Metis G3.5**: **NOT a play DSL**, **NOT a user-editable play editor**). Each play lists its mode, the 5 offensive RoleSlots it expects, and an ordered `steps` list. The full structure is in `config/plays.json`; the schema in `config/schemas/plays.schema.json` enforces the closed shape.

| # | `id` | Mode | Description |
|---|---|---|---|
| 1 | `transition_push` | `TRANSITION` | Push the ball up the floor and attack the rim before the defense sets. Two advance segments (`ADVANCE_BACKCOURT` → `CROSS_HALF`) collapse into a single `attack_window` rim attack. No halfcourt alignment step. |
| 2 | `handoff_wing` | `HALFCOURT` | Advance, align, then a guard-to-wing handoff (`HANDOFF`) that puts the receiver in a drive-or-shoot window on the wing. |
| 3 | `pnr_high` | `HALFCOURT` | Advance, align, set a high ball screen (`SCREEN_SET` by the screener, then `SCREEN_USE` by the ball handler), then drive-or-shoot. The screen step may fail into an isolation drive (`ISO_DRIVE`) as a fallback branch. |
| 4 | `iso_clear` | `HALFCOURT` | Advance, align, then clear one side and let the primary creator go to work in an isolation (`ISO_DRIVE`) without a screen. |
| 5 | `early_drag` | `HALFCOURT` | Advance, then immediately use a high screen (`SCREEN_USE` without an explicit `SCREEN_SET` step — drag screen implied) for a drive-or-shoot. Despite the name, this is a `HALFCOURT` play, NOT a third mode. |

Every play uses the same 5-slot binding:

```
["primary_creator", "secondary_creator", "screener", "spacer_strong", "spacer_weak"]
```

The schema enforces that each play's `slots` is a 5-element permutation of these names.

### Step shape

Each step in a play's `steps` array has exactly five keys:

| Key | Type | Description |
|---|---|---|
| `action` | `^[A-Z][A-Z0-9_]*$` | Action label for this step. Some labels coincide with `EventType` names from `config/event-catalog.json` (`ADVANCE_BACKCOURT`, `CROSS_HALF`, `ALIGN_HALFCOURT`, `HANDOFF`, `SCREEN_SET`, `SCREEN_USE`, `SHOT_RELEASE`, `PASS`, `TURNOVER`). Others (`ATTACK_RIM`, `DRIVE_OR_SHOOT`, `ISO_DRIVE`, `PASS_TO_SPACER`) are play-internal control states that the engine emits as a `DRIVE` event when resolving. Cross-reference is enforced by `scripts/check-foundation.mjs` (T9 linter). |
| `actor_slot` | enum (5 slot names) | The RoleSlot whose bound player performs this action. |
| `duration_id` | `^[a-z][a-z0-9_]*$` | Duration identifier resolved against `config/duration.json` (T6). Examples: `advance_transition`, `advance_backcourt`, `align_halfcourt`, `handoff`, `screen`, `attack_window`, `shot_flight`. |
| `on_success` | `^[A-Z][A-Z0-9_]*$` | Next action label when the resolve check for this step passes. The sentinel `END` terminates the play. |
| `on_fail` | `^[A-Z][A-Z0-9_]*$` | Next action label when the resolve check fails. Sentinels include `END`, `TURNOVER`, `PASS`, `PASS_TO_SPACER`, or a fallback action like `ISO_DRIVE`. |

The engine walks `steps[0]` → steps[k] following `on_success`/`on_fail` labels until it reaches a sentinel (`END`, `TURNOVER`, or an event-emitting terminal like `PASS` that flows out of the play).

---

## Bind Algorithm

`bindRoles(lineupPackage, play) → RoleBinding` is implemented in T16. The algorithm:

1. **Validate slot count.** The play's `slots.length` MUST equal 5. The schema enforces this at config load; the engine re-asserts at bind time. A mismatch is a fatal engine error (configuration drift between code and config).

2. **Fill `primary_creator`.** Take the first jersey in `lineupPackage.usageProfile.creator` (priority-ordered list). The `creator` list's first element is always the primary; an empty list is a configuration error.

3. **Fill `secondary_creator`.** Take the next jersey in `lineupPackage.usageProfile.creator` that is NOT already bound to `primary_creator`. If only one creator jersey is listed, the engine falls back to the next-available on-court player; this fallback is documented and tested in T16.

4. **Fill `screener`.** Take the first jersey in `lineupPackage.usageProfile.screener` that is NOT already bound to `primary_creator` or `secondary_creator`.

5. **Fill `spacer_strong` and `spacer_weak`.** From the remaining on-court players, prefer jerseys listed in `lineupPackage.usageProfile.spacer` in listed order; the strong slot takes the first available spacer and the weak slot the next. If the spacer list is exhausted, fall back to any remaining on-court player.

6. **Validate all 5 slots bound.** Exactly 5 distinct on-court players are now assigned to the 5 RoleSlots. The engine asserts that the bound set equals the on-court 5.

7. **Return the RoleBinding.** The binding is a frozen map `{ primary_creator, secondary_creator, screener, spacer_strong, spacer_weak }` of player IDs. It is held by the possession state and is reused for every step in the play.

The `usageProfile` priority order is what gives the "same jersey, different role on the bench unit" story (T7, T10): the same jersey can be a primary creator in the starters package and a spacer in the bench unit, depending on its position in each package's `creator` list.

The defense binds in parallel: `on_ball` targets `ball.holderId`, and on each `PASS`/`HANDOFF` complete the engine **retargets** `on_ball` to the new holder. `STEAL`/`STRIP`/`CONTEST` resolves target the current holder at resolve time, never a stale value (Metis G1.8). See `docs/foundation/identity-roles.md` (T7) for the defensive RoleSlots (`on_ball`, `deny`, `help`, `tag`, `weak_side`).

---

## Possession Lifecycle

A possession starts with a `start_reason` and ends with an `end_reason`. Between those points the engine walks the selected play's steps, emitting events from `config/event-catalog.json` and threading the RNG per `docs/foundation/rng.md` (T3).

### Start reasons

| Reason | Trigger |
|---|---|
| `JUMP_BALL_TAP` | Opening or held-ball tip recovered. |
| `AFTER_MAKE` | Inbound after the opponent made a field goal. |
| `AFTER_INBOUND` | Any dead-ball inbound where possession is awarded. |
| `AFTER_DREB` | Defensive rebound secured. |
| `AFTER_STEAL` | Steal event flipped possession. |
| `AFTER_LIVE_BALL_TURNOVER_RECOVER` | Loose-ball recovery after a live-ball turnover. |
| `AFTER_OREB_CONTINUE` | **Special:** OREB continues the current possession; see below. |

### End conditions

A possession ends when any of the following is reached:

| End condition | Resulting events / next phase |
|---|---|
| **Field goal made** (`MAKE`) | `SHOT_RESULT(made=true)`, `MADE_BASKET_DEAD`; possession flips to opponent with `AFTER_MAKE`. |
| **Field goal missed + defensive rebound** (`MISS_DREB`) | `SHOT_RESULT(made=false)`, `REBOUND(offensive=false)`; possession flips to opponent with `AFTER_DREB`. Shot clock reset to 24. |
| **Field goal missed + offensive rebound** (`MISS_OREB_CONTINUE`) | `SHOT_RESULT(made=false)`, `REBOUND(offensive=true)`. **The possession does NOT end** — see [OREB Continuation Rule](#oreb-continuation-rule-critical). |
| **Turnover** (`TURNOVER`) | `TURNOVER` (optionally co-emitted with `STEAL`); possession flips with `AFTER_TURNOVER` / `AFTER_LIVE_BALL_TURNOVER_RECOVER` / `AFTER_STEAL`. |
| **Shooting foul** (`SHOOTING_FOUL`) | `FOUL(shooting=true)`; play terminates and the engine enters `FT_SEQUENCE` with the appropriate taxonomy entry (see `docs/foundation/resolve.md` and FT Taxonomy below). Possession rules after FT follow NBA and-1 / shooting-foul rules. |
| **Non-shooting foul dead** (`NON_SHOOTING_FOUL_DEAD`) | `FOUL(shooting=false)`; if the fouling team is in the bonus, `FT_SEQUENCE` with `bonus_2_shot`; otherwise the ball is inbounded (possession preserved or flipped per the foul rule). |
| **Violation** (`VIOLATION`) | `VIOLATION` event; possession awarded per the violation type (typically flips). |
| **Shot-clock violation** (`SHOT_CLOCK_VIOLATION`) | `SHOT_CLOCK_VIOLATION`; possession flips. |
| **Period end** (`PERIOD_END`) | `PERIOD_END`; possession is abandoned mid-step. Any in-flight possession is truncated; the FSM (T5) routes to `PERIOD_BREAK`, `HALFTIME`, `OVERTIME_SETUP`, or `POST_GAME`. |

---

## OREB Continuation Rule (CRITICAL)

> **Decision Metis G1.8.** An offensive rebound does NOT start a new possession and does NOT rebind roles. The same `RoleBinding` persists.

When `SHOT_RESULT(made=false)` is followed by `REBOUND(offensive=true)`:

1. **No new possession.** The `possession_id` does not increment. The possession in progress continues.
2. **No rebind.** The `RoleBinding` from the start of the possession is preserved verbatim. The `primary_creator`, `secondary_creator`, `screener`, `spacer_strong`, and `spacer_weak` slots still hold the same player IDs.
3. **No mode re-select.** `mode_select` is NOT called. The prior mode (`TRANSITION` or `HALFCOURT`) is retained.
4. **Shot clock resets to 14.** Per `config/duration.json#clocks.shot_clock_reset` (T6), the shot clock is reset to 14 (the partial-reset value), NOT 24. The `OREB CONTINUES possession` rule and the 14-second reset together preserve offensive continuity while still imposing time pressure.
5. **Play re-selects.** The play runner picks a fresh play from the SAME mode's play set (the offense may run a different halfcourt set after the rebound). The new play's steps start from the bound RoleBinding without re-binding.
6. **Event audit.** The engine emits `REBOUND(offensive=true)` followed by the new play's first action event. No `POSSESSION_GAINED` is emitted — that event is reserved for actual possession changes.

This rule is the single most important invariant in the possession model. Code that rebinds roles on OREB, increments `possession_id` on OREB, or resets the shot clock to 24 on OREB is a critical bug.

The `possession_log` entry in `config/schemas/game-result.schema.json` represents OREB continuation via `start_reason = AFTER_OREB_CONTINUE` for audit-friendly per-possession analysis. The engine MAY log a single `possessionEntry` spanning the OREB or two linked entries; either representation is schema-valid. The OREB-continuation contract above applies to engine state regardless of audit-log representation.

---

## Cross-references

- **T2** — `config/event-catalog.json` defines every `EventType` referenced by play actions.
- **T3** — `docs/foundation/rng.md` pins the draw order: `duration → mode_select → play_select → action_resolve`.
- **T6** — `config/duration.json` defines every `duration_id` referenced by play steps; `after_offensive_rebound.shot_clock_reset_to = 14`.
- **T7** — `docs/foundation/identity-roles.md` defines the 5 offensive RoleSlots and the defensive retarget rule.
- **T9** — `scripts/check-foundation.mjs` cross-references play actions against the event catalog and play `duration_id` against `duration.json`.
- **T16** — `bindRoles` implementation.
- **T17** — `mode_select`, `play_select`, and the play runner implementation.
- **T18** — Resolve checks (pass, handoff, drive, shot, rebound, FT) implementation.
