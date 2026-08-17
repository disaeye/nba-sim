# RNG & Draw-Order Contract

`foundation_version: 0.4.0` · `algorithm: mulberry32` · `stream_model: single` · `substreams: false`

This document is the **sole authority** for randomness in the NBA simulator kernel. The machine-readable contract lives at [`config/rng.json`](../../config/rng.json) and is validated by [`config/schemas/rng.schema.json`](../../config/schemas/rng.schema.json). If prose here ever disagrees with the JSON or schema, the JSON wins.

## RNG Algorithm

The kernel uses **mulberry32** (decision M3 of the work plan). mulberry32 is:

- **Deterministic** — for a fixed 32-bit seed, the produced sequence is fully reproducible across implementations, machines, processes, and runs.
- **Seedable** — internal state is initialized from a single `uint32` seed (`config/rng.json#seed_type`).
- **Single-state** — internal state is one 32-bit word, advanced in place by each `next()` call.
- Returns a `number` in `[0, 1)` (i.e., `0 <= x < 1`) per `rng.next()` call. The upper bound is exclusive.

The TypeScript signature is `interface Rng { next(): number }`; the implementation lands in todo 12. Every `next()` call advances the single stream by exactly one step — there is no look-ahead, no skip, and no rewind primitive in the v0.1 contract.

## Single-Stream Model

There is **exactly one** RNG instance per simulation. It is constructed at the entry of `simulateGame({ ..., seed })` and **threaded explicitly** into every stochastic call site:

- `step(state, rng)` receives `rng` as its second parameter and forwards it to each sub-call (the duration sampler, `mode_select`, `play_select`, and each `resolve` check). No sub-call ever reads from a closure, a module-global, or a static singleton.
- The `rng` is **never** stored on `GameState` basketball fields. `rng` is a step / function **parameter only**; persisting it inside `GameState` (e.g., `state.rng`, `state.possession.rng`, `state.play.rng`, `state.clocks.rng`) is **FORBIDDEN** by this contract. Any such field would also be rejected by the `GameState` schema and flagged by the foundation linter (todo 9), because `rng` is not serializable and would break replay determinism.
- There are **no substreams**. The v0.1 catalog forbids `rng.duration(...)`, `rng.resolve(...)`, `rng.play_select(...)`, `rng.pick(...)`, or any other named/derived sub-stream API. Every draw is a single `rng.next()` consumed directly by the helper that needs it (see Helper Functions).
- Private per-module seeds are forbidden: no `createRng(...)` call is legal anywhere except at `simulateGame` entry. The same seed + same input + same code path produces the identical draw sequence (see Determinism Guarantee).

The motivation is twofold:

1. **Thread-through makes every draw site auditable in code review.** The `rng` parameter is on the signature of every stochastic function, so any silent seed capture shows up as an unused parameter or a stray global read.
2. **A single stream makes draw-order changes mechanically detectable.** Reordering any two draws shifts every downstream `next()` value; the golden-seed fixture (todo 20) will fail loudly against such a change, which is exactly what we want when locking the contract.

## Draw-Order Contract

Inside any `step(state, rng)` that needs new randomness, draws occur in this **locked order**, and this order is itself part of the foundation contract:

1. **`duration`** — sample Δ (in seconds, resolution 0.1s, see [`docs/foundation/clocks-duration.md`](clocks-duration.md)) for the next action or play segment. Always the first draw of the step, because everything downstream (mode choice, play choice, resolve outcomes) happens *within* that segment.
2. **`mode_select`** — choose `TRANSITION` vs `HALFCOURT` for the upcoming possession, **unless** `start_reason` already pins the mode. `STEAL` and `LIVE_BALL_TURNOVER_RECOVER` force `TRANSITION`; `DEF_REB` defaults to `HALFCOURT` (see [`docs/foundation/possession-plays.md`](possession-plays.md)). When `start_reason` is conclusive, this draw is **skipped** (see "Skipped draws do not consume" below).
3. **`play_select`** — choose which play from the chosen mode's playbook (todo 8), via a single `weighted(...)` draw over the mode's plays.
4. **`resolve`** — outcome checks (pass success, shot make, steal success, foul trigger, etc.) executed **in play-step execution order**, one `rng.next()` per check, exactly as declared by the chosen play's `steps[]` array.

**Skipped draws do not consume.** If `mode_select` is determined by `start_reason`, the kernel does **not** call `rng.next()` for it; the stream is advanced only by draws that actually happen. This preserves the determinism invariant: two paths that produce the same `start_reason` produce the same downstream draws regardless of which other branches were *considered*.

**Why the order is locked.** Step outcomes depend on which `rng.next()` lands on which check. Swapping `mode_select` and `play_select`, or moving a `resolve` check before `play_select`, would produce a different event sequence for the same seed. Reordering this list is therefore a breaking change requiring a `foundation_version` **major** bump (see Determinism Guarantee).

### Draw Sites (v0.1)

| # | Draw site | Consumes | Owner doc |
|---|---|---|---|
| 1 | `duration` | 1 × `next()` | [`clocks-duration.md`](clocks-duration.md) (todo 6) |
| 2 | `mode_select` | 1 × `next()` only when `start_reason` is ambiguous; otherwise skipped | [`possession-plays.md`](possession-plays.md) (todo 8) |
| 3 | `play_select` | 1 × `next()` via `weighted(...)` | [`possession-plays.md`](possession-plays.md) (todo 8) |
| 4 | `resolve` | 1 × `next()` per declared play-step check, in `steps[]` order | [`resolve.md`](resolve.md) (todo 8) |

## Forbidden Sources

The following entropy sources are **banned** from every kernel path under `src/**`:

- **`Math.random()`** — non-deterministic; never use. (Mentioned here explicitly as the canonical forbidden call.)
- **`Date.now()` / `new Date().getTime()`** — wall-clock; banned inside gameplay code. Only allowed in CLI/audit/logging code that lives outside `src/kernel/**` and does not influence `Event` payloads or `GameState`.
- **`performance.now()`** — wall-clock; banned for the same reason.
- **`crypto.randomFillSync(...)` / `crypto.randomBytes(...)` / `crypto.getRandomValues(...)`** — non-deterministic entropy; banned even though it would be "more random".

The **only** entropy source for kernel outcomes is `rng.next()`, where `rng` is the mulberry32 instance created at `simulateGame({ seed })` entry. The foundation linter (todo 9) and the determinism audit (todo 20) enforce this via static `rg`-based scans of `src/` for these symbols.

This rule is the foundation of reproducibility: if any of the above symbols appears inside `src/`, the build is non-compliant and `npm run foundation:lint` exits nonzero.

## Helper Functions

All helpers live in `src/rng/` (todo 12) and consume exactly **one** `rng.next()` call per invocation — including `weighted`. They never close over a private RNG; `rng` is always an explicit parameter.

| Helper | Signature | Returns | Consumes | Notes |
|---|---|---|---|---|
| `unit`   | `unit(rng): number` | float in `[0, 1)` | 1 × `next()` | thin alias for `rng.next()`; present so call sites read as "draw a unit variate" |
| `int`    | `int(rng, min, max): number` | inclusive integer in `[min, max]` | 1 × `next()` | `min` and `max` are integers with `min <= max`; uses `Math.floor(unit(rng) * (max - min + 1)) + min` |
| `pick`   | `pick<T>(rng, arr: T[]): T` | a random element of `arr` | 1 × `next()` | `arr` must be non-empty; equivalent to `arr[int(rng, 0, arr.length - 1)]` and consumes exactly one `next()` |
| `weighted` | `weighted<T>(rng, items: T[], weights: number[]): T` | one element of `items` selected with probability `weights[i] / sum(weights)` | 1 × `next()` | `items.length === weights.length`; weights are non-negative and not all zero; uses a single cumulative-table lookup against one `unit(rng)` draw |

**Why every helper consumes exactly one `next()`.** A helper that internally called `next()` more than once would silently shift the stream relative to the Draw-Order Contract; that would make downstream draws depend on helper internals and break the byte-stable determinism guarantee. The "1 `next()` per helper" rule keeps draw accounting trivially auditable: across any code path, the count of `next()` calls equals the count of helper invocations plus direct `unit(rng)` calls — no surprises hidden inside a helper.

## Determinism Guarantee

For any fixed `(seed, input, code)` triple, the simulator produces the **same event sequence byte-for-byte**:

- same `seed` ⇒ same mulberry32 stream ⇒ same `next()` sequence;
- same `input` (lineup packages, mode weights, duration config, etc.) ⇒ same branches taken;
- same `code` ⇒ same draw-order, same helper implementations, same consumption counts.

The golden-seed fixture (`fixtures/golden-seed-42.json`, todo 20) materializes this guarantee: two runs of `simulateGame({ seed: 42, ... })` in the same process must produce event sequences that deep-equal the fixture, including all `t_game` values, payloads, scores, and the final box score. Any divergence is a regression and fails CI.

**Breaking-change rule.** The following changes are **breaking** and require a `foundation_version` **major** bump (per the README semver policy):

- Changing `algorithm` (e.g., `mulberry32` → `xorshift32`).
- Changing `seed_type` (e.g., `uint32` → `uint64`).
- Flipping `substreams` from `false` to `true`, or introducing any named/derived stream.
- **Reordering the Draw-Order Contract list above** — including swapping two `resolve` checks inside the same play, since that changes which `next()` lands on which check.
- Changing the consumption count of any helper function (e.g., making `weighted` consume two `next()` calls).

**Non-breaking** changes (additive, minor or patch):

- Adding a new play that consumes `rng.next()` at draw site #4 in the documented order.
- Adding a new `duration_id` (drawn at site #1).
- Tightening a helper's input validation without changing its draw count.

## Cross-References

- Decision M3 in [`.omo/plans/nba-sim-foundation.md`](../../.omo/plans/nba-sim-foundation.md) and the work plan TL;DR.
- Event catalog [`docs/foundation/events.md`](events.md) — every stochastic outcome is recorded as a cataloged event.
- Clocks & duration [`docs/foundation/clocks-duration.md`](clocks-duration.md) (todo 6) — owns the `duration` draw site.
- Possession & plays [`docs/foundation/possession-plays.md`](possession-plays.md) (todo 8) — owns `mode_select` and `play_select`.
- Resolve [`docs/foundation/resolve.md`](resolve.md) (todo 8) — owns the `resolve` draw site.
