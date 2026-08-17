# Clocks, Duration Model, Quantization, and Mode Biases

`foundation_version: 0.4.0`

This document is the **sole authority** for how time advances in the NBA simulator kernel, how long each gameplay micro-segment takes, and how possession start-reasons bias the engine toward half-court or transition basketball. The closed machine-readable contract lives at [`config/duration.json`](../../config/duration.json) and is validated by [`config/schemas/duration.schema.json`](../../config/schemas/duration.schema.json). If prose here ever disagrees with the JSON config or schema, the JSON wins.

This document covers **only** kernel time. It deliberately does **not** specify any presentation clock, broadcast clock, or UI timing — the kernel has no concept of wall-clock and is fully driven by logical game time (see [`rng.md`](rng.md) for the determinism contract, todo 3).

## Clock Authority

The kernel maintains exactly five named clocks. All values are pinned in `config/duration.json#clocks` and are integers in seconds.

| Clock | Value | Notes |
|---|---|---|
| `period_seconds` | 720 | A regulation quarter is 12:00 = 720 s. Q1–Q4 each use this length. |
| `overtime_seconds` | 300 | Each overtime period is 5:00 = 300 s. Unlimited OT periods (decision M6). |
| `shot_clock_full` | 24 | Full shot clock reset value, used by the v0.1.0 reset rules below. |
| `shot_clock_reset` | 14 | Partial shot clock reset value, used by the v0.1.0 reset rules below. |
| `backcourt_seconds` | 8 | Time limit to advance the ball past the division line. v0.1.0 models this as a duration constraint, not a separate violation path; the engine samples `advance_backcourt` and accepts the result. |

Authoritative rules:

- Regulation game length is 4 × 720 s = 2880 s of live play, plus stoppages. The kernel never counts wall-clock time.
- A period ends when `period_seconds` remaining hits 0; the `PERIOD_END` event (rank 5 in the same-timestamp priority table) is emitted, the period counter advances, and the FSM routes to the next phase per the period router in [`fsm.md`](fsm.md) (todo 5).
- If Q4 or any OT period ends with the score tied, an overtime period of `overtime_seconds` is appended; otherwise the game ends via `GAME_END`.
- The shot clock is fully independent of the period clock: it counts down from `shot_clock_full` (24) or `shot_clock_reset` (14) depending on the trigger and resets per the rules below.
- A shot clock that reaches 0 while the offense still holds the ball triggers `SHOT_CLOCK_VIOLATION` (rank 4) and turns possession over.

## Shot Clock Reset Rules

v0.1.0 freezes a deliberately small subset of NBA shot-clock reset triggers. Triggers not listed here are not modeled in this foundation version; if a play (T8) or resolve path (T8) reaches one of them, that is a bug.

| Trigger | Reset to | Notes |
|---|---|---|
| New possession via **defensive rebound** | `shot_clock_full` (24) | The defense secures the miss and advances; full reset. |
| New possession via **steal** (`STEAL` event) | `shot_clock_full` (24) | Live-ball takeaway; full reset. |
| New possession via **non-steal turnover** (`TURNOVER` event, e.g. OOB by offense) | `shot_clock_full` (24) | Any dead- or live-ball turnover that hands the ball to the defense restarts the full clock. |
| New possession via **inbound** after a stoppage | `shot_clock_full` (24) | Baseline/sideline inbound starts the full 24. |
| New possession via **jump ball** (`JUMP_BALL_TAP` → `POSSESSION_GAINED`) | `shot_clock_full` (24) | Tip-off or held-ball resolution. |
| **Offensive rebound** after a missed shot that hit the rim | `shot_clock_reset` (14) | Possession does **not** change; same offensive team retains the ball, but the shot clock is reset to 14 (not 24). Modeled by `start_reason_bias.after_offensive_rebound.shot_clock_reset_to`. |
| **OOB by the defense** in the frontcourt (e.g. defense deflects the ball out) | `shot_clock_reset` (14) | The offense retains possession and inbounds in the frontcourt; partial reset only. If the clock was already below 14, the offense keeps the lower value (the partial reset does not extend the clock upward beyond the residual). |

Out of scope for v0.1.0 (deliberately deferred): kicked-ball resets, fouls during rebounding action, and the "0:00 residual → reset to 24" edge case are not modeled.

## Resolution & Quantization

All game-time values are expressed in seconds at a fixed resolution of **0.1 s** (`resolution_seconds` in the config). A raw sampled duration `delta_raw` is snapped to this resolution by the formula (decision M7):

```
delta = Math.round(delta_raw * 10) / 10
```

Concretely:

- The `quantize_rule` field in `config/duration.json` stores the canonical expression as a string: `"Math.round(delta_raw * 10) / 10"`.
- The kernel reads this string at startup and applies the equivalent `Math.round(x * 10) / 10` operation to every sampled `Δ` before committing it to the timeline.
- `t_game` on every emitted `Event` is therefore always a multiple of 0.1 s, which is what allows the same-timestamp priority rules in [`events.md`](events.md) to apply deterministically.
- The quantization pins `Math.round` semantics: 1.24 → 1.2, 1.25 → 1.3 (JS half-up rounding). The TDD test for the duration sampler (todo 13) locks these expected values to actual ES `Math.round` behavior.

Changing the resolution or the rounding mode is a **major** foundation bump — it changes every `t_game` and therefore invalidates the golden seed fixture (todo 20).

## Clamp Rule

Every sampled `Δ` is clamped to the live clocks before it is committed. The clamped value is:

```
delta_clamped = min(delta_quantized, shot_clock_remaining, game_clock_remaining)
```

Behavior:

- If `delta_clamped == delta_quantized`: normal advance. The relevant clock or clocks decrement by `delta_clamped`, no special event is emitted.
- If the clamp hit `shot_clock_remaining` (`delta_clamped == shot_clock_remaining` and the offense still holds the ball and has not released a legal attempt): emit `SHOT_CLOCK_VIOLATION` (rank 4) at the new `t_game`, possession turns over.
- If the clamp hit `game_clock_remaining` (`delta_clamped == game_clock_remaining`): emit `PERIOD_END` (rank 5) at the new `t_game`. The period router in [`fsm.md`](fsm.md) decides whether a new period, overtime, or `POST_GAME` follows.
- If both clocks would be hit at the same instant, the period clock wins: `PERIOD_END` is emitted and the shot clock is considered expired by the period end (no separate `SHOT_CLOCK_VIOLATION`).

The clamp is applied **after** quantization but **before** event emission, so the timeline only ever records post-clamp `t_game` values.

## Duration Distributions

> **Runtime authority note (foundation 0.5.0 / architecture.md §5).**
> Live play advances clocks by **tick integration only** (`gameClock -= dt` each LIVE tick).
> The `durations[]` table below is **design-intent reference** (expected human-scale means for calibration),
> **not** a bulk clock-deduction authority. `commit` / Decision must not `sampleDuration` to advance clocks.
> The `clocks` constants and shot-clock reset rules in this document remain runtime-authoritative.


The kernel samples the duration of every micro-segment from a closed 14-entry catalog pinned in `config/duration.json#durations`. The schema pins the catalog at exactly 14 entries (`minItems: 14, maxItems: 14`); the foundation linter (todo 9) additionally enforces that the set of `id` values matches the table below.

Each entry is `{ id, distribution, min, max }`. v0.1.0 uses only `uniform` — `trunc_normal` is reserved in the schema for future tuning but currently rejected by the linter. All values are seconds. The mean of a uniform is `(min + max) / 2`.

| # | id | distribution | min | max | mean | Used by |
|---|---|---|---|---|---|---|
| 1 | `jump_tap` | uniform | 1.0 | 1.5 | 1.25 | `JUMP_BALL_TAP` → `POSSESSION_GAINED`. |
| 2 | `inbound_setup` | uniform | 1.5 | 3.5 | 2.5 | Dead-ball inbound setup time before release. |
| 3 | `inbound_touch` | uniform | 0.6 | 1.5 | 1.05 | `INBOUND_TOUCH`: time from release to first legal touch. |
| 4 | `advance_backcourt` | uniform | 3.5 | 6.5 | 5.0 | `ADVANCE_BACKCOURT` after gaining possession in the backcourt (DREB / made-basket inbound). |
| 5 | `advance_transition` | uniform | 2.0 | 4.5 | 3.25 | Live-ball transition advance after steal or live-ball turnover. |
| 6 | `align_halfcourt` | uniform | 1.0 | 2.5 | 1.75 | `ALIGN_HALFCOURT`: setup into the half-court set after crossing the time line. |
| 7 | `pass` | uniform | 0.6 | 1.4 | 1.0 | `PASS` release-to-catch. |
| 8 | `handoff` | uniform | 0.6 | 1.2 | 0.9 | `HANDOFF` exchange. |
| 9 | `screen` | uniform | 1.0 | 2.5 | 1.75 | `SCREEN_SET` → `SCREEN_USE` setup window. |
| 10 | `attack_window` | uniform | 1.5 | 4.0 | 2.75 | Live-ball decision window between actions (drive, pass, shoot). |
| 11 | `shot_flight` | uniform | 0.5 | 1.0 | 0.75 | `SHOT_RELEASE` → `SHOT_RESULT`. |
| 12 | `rebound_window` | uniform | 0.6 | 1.4 | 1.0 | `REBOUND` resolution window after a miss. |
| 13 | `loose_ball` | uniform | 0.5 | 1.2 | 0.85 | `LOOSE_BALL_RECOVER` scramble time. |
| 14 | `ft_attempt_logic` | uniform | 0.5 | 1.0 | 0.75 | `FT_ATTEMPT` → `FT_RESULT` per free throw. |

Notes:

- All 14 IDs are cross-referenced by plays (T8) and validated by the foundation linter (T9). A play step that references a `duration_id` not in this table fails the linter.
- `trunc_normal` is present in the schema's `distribution` enum only so a future minor-version bump can introduce mean/std/clip fields without a schema shape change. Until then the linter rejects any entry that actually uses it.
- Free throw setup (`FT_START`) and sequence end (`FT_SEQUENCE_END`) delays are out of scope for v0.1.0 — only the per-attempt flight time is modeled.

## Start-Reason Timing Biases

Each new possession begins with a start reason. `modeSelect(start_reason, rng)` owns the logical `TRANSITION` versus `HALFCOURT` route; this table owns only the sampled timing/alignment bias. The two concerns stay separate so a timing adjustment cannot silently change mode semantics.

| start_reason | mode bias | align_extra_seconds | target mean possession | rationale |
|---|---|---|---|---|
| `after_make` | 98% `HALFCOURT`, 2% `TRANSITION` | +1.5 | ~15 s | Made-basket inbound normally faces a set defense; a small quick-advance probability remains available. |
| `after_defensive_rebound` | 85% `HALFCOURT`, 15% `TRANSITION` | 0.0 | ~15 s | DREB usually flows into an outlet/set, with a calibrated quick-break branch. |
| `after_steal` | `TRANSITION` | 0.0 | ~8 s | Live-ball takeaway: advance and early attack. |
| `after_turnover` | 30% `TRANSITION`, otherwise `HALFCOURT` | 0.0 | ~14.8 s overall | Live-ball turnover recovery is transition-biased but not guaranteed. |
| `after_offensive_rebound` | continues prior mode | 0.0 | continues | Same possession continues; shot clock resets to 14 and roles remain bound. |
| `after_inbound` | `HALFCOURT` | 0.0 | ~15 s | General dead-ball inbound after timeout/violation. |
| `after_jump_ball` | `HALFCOURT` | 0.0 | ~15 s | Game-start or held-ball tip; full advance then set. |

The `mode` column is descriptive calibration context. The runtime selector and its RNG draw order are authoritative for the actual mode; `align_extra_seconds` remains the timing-only bias.

`after_offensive_rebound` is the only entry that carries `shot_clock_reset_to: 14`; the schema enforces this via a distinct `biasEntryWithShotClockReset` definition.

## Made-FG Stoppage Rules

The game clock stops after a made field goal in the following windows:

| Window | Stops the clock? | Notes |
|---|---|---|
| Final 1:00 of Q1, Q2, Q3 | yes | `made_fg_stoppage.last_minute_periods_q1_q2_q3: true`. |
| Final 2:00 of Q4 | yes | `made_fg_stoppage.last_two_minutes_q4_ot: true`. |
| Final 2:00 of any OT period | yes | Same flag. |
| Any other time during regulation or OT | no | Clock continues to run; the offense must inbound and advance normally. |

Behavior:

- On `SHOT_RESULT` with `made: true`, the kernel evaluates the current period and remaining game clock. If the made basket falls inside a stoppage window, the engine emits `MADE_BASKET_DEAD` (rank 5), the game clock does **not** advance on the inbound, and the defense gets a dead-ball inbound.
- Outside these windows, a made basket does not stop the clock; the defense proceeds directly to inbound advancing the ball (the offense is still entitled to advance the ball up to `backcourt_seconds`).
- The stoppage windows are frozen at the NBA subset for v0.1.0. Additional stoppage triggers (kicked ball under 2:00, foul-resulting dead ball inside the window) are owned by the FSM and resolve contracts (T5, T8), not this config.

## Version Discipline

Bumps follow the policy in [`README.md`](README.md). Specific to this contract:

- **Patch**: prose or `description`/`$comment` clarifications. No value changes.
- **Minor**: additive and backward compatible — e.g. introducing a new `distribution: "trunc_normal"` entry with new optional fields, or widening the durations array past 14 with new IDs that existing plays do not yet reference.
- **Major**: changing any pinned value (`period_seconds`, `resolution_seconds`, `quantize_rule`, the made-FG windows), removing a duration ID, or renaming any field. Requires regenerating the golden seed (todo 20) and re-running the pace gate (todo 21).

Any change to `config/duration.json` without a matching `foundation_version` bump in `config/foundation.json` is a contract violation and will fail the foundation linter (todo 9).
