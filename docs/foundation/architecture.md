# Single-Axis Causal Chain

`foundation_version: 0.5.0` · `implemented` · `supersedes: dual-authority commit+tick+simulate`

This document is the **sole authority** for the kernel's layered architecture: the five immutable authorities that form a single causal chain from World through Decision through Completion through Adjudication through Episode, the per-layer import bans, and the migration phases that bridge from the current dual-authority implementation. **If code contradicts this document, the code is a bug.**

> **Status.** **Implemented at foundation 0.5.0.** Decision emits intents only; Completion produces facts; Adjudicate is the sole resolve/event/possession/shot-clock gateway; Episode end reasons come from Adjudicate via tick context. Residual calibration (SCV rate, FG%, pace bands) is **out of architecture scope** — tune mobility/resolve base rates after the single axis is stable.

## 0. Problem Statement

The engine currently has 2–3 authorities for each fact domain:

| Fact | Authority A | Authority B | Casualty |
|---|---|---|---|
| Pass completion | `commitDecisions` emits `PASS` immediately | `stepBall` re-emits at `pass_complete` | duplicate events; `passer_id='?'` |
| Shot result | `commit.shoot` emits `SHOT_RESULT` | `stepBall.shot_arrived` re-resolves | and-one path drops `SHOT_RESULT`; dual resolve |
| Rebound | commit resolves | ball-arrival re-resolves | OREB probability diluted |
| Shot clock reset | foundation says STEAL→24 | `applySteal` never resets | contract breach |
| Time advance | commit samples Δ·1.35+0.6 | tick integrates 0.1s | two competing time physics |
| Possession end | `CommitResult.possessionEnded` | simulate scans `newEvents` types | SCV invisible to possession_log |
| Position truth | Intent `targetX/Y` | `WorldSnapshot` poses | semantic/geometric decoupling |

This is not parameter debt — it is **structural ambiguity about who owns what**. No amount of `baseScore` tuning or `duration` refinement will consistently produce realistic basketball while the kernel has two time models, two resolve sites, and two lifecycle indicators.

## 1. Single-Axis Model

```
                     ┌─────────────────────────────────────────────────┐
                     │             Main tick loop @ 0.1 s               │
                     │   realClock += dt ; if LIVE: game/shot −= dt    │
                     ├─────────────────────────────────────────────────┤
                     │                                                 │
    ┌───────┐        │  ┌──────────┐   ┌──────────┐   ┌──────────┐   │
    │  RNG  │───────▶│  │ Decision │──▶│   Step   │──▶│Completion│   │
    └───────┘        │  │          │   │  World   │   │          │   │
     single stream   │  │ Intent[] │   │ poses+   │   │ predicates│   │
                     │  │ no RNG   │   │ ball     │   │ on world  │   │
                     │  └──────────┘   └──────────┘   └────┬─────┘   │
                     │                                     │          │
                     │                          ┌──────────▼──────┐   │
                     │                          │   Adjudicate    │   │
                     │                          │ ONE resolve     │   │
                     │                          │ ONE clock delta │   │
                     │                          │ ONE possession  │   │
                     │                          │ emit Event[]    │   │
                     │                          └────────┬────────┘   │
                     │                                   │            │
                     │                    ┌──────────────▼─────────┐  │
                     │                    │     Episode state      │  │
                     │                    │  OPEN|OREB|ENDED       │  │
                     │                    └────────────────────────┘  │
                     │                                                 │
                     │  ← WorldSnapshot (position truth, every tick)   │
                     └─────────────────────────────────────────────────┘
```

The five authorities form a **unidirectional** chain. No authority may read or invoke anything downstream. The arrow is `→` = "produces output consumed by."

### Authority 1 — World (physical ground truth)

**Owns:** `poses[10]`, `ball`, `realClock`, `gameClock`, `shotClock`, `phase`, `score`, `fouls`, `lineups`, `possession.team`, `baskets`, `binding`.

**Contract:**
- Position truth lives in `poses.x/y` and `ball.x/y`, **never in Event payloads**. Events may carry snapshot `zone` or `x/y` for logging but must not be treated as the source of position.
- Clocks advance by **tick integration only**: `gameClock = max(0, gameClock - dt)` each tick when `phase === LIVE`. No action may "sample a Δ then deduct it in bulk." The clock is an integral, not a ledger.
- `phase` is the single source of legal action sets; the FSM transition table (`config/fsm.json`) is the sole transition authority.

**Bans:**
- MUST NOT import `resolve/`, `adjudicate/`, `decision/`, `possession/`, `box-score/`
- MUST NOT call `rng.next()` except during completion-predicate evaluation delegated to Adjudicate

### Authority 2 — Decision (intent only)

**Owns:** `Intention[]` — per-player targets and action kinds.

**Contract:**
- `generateOptions(world, agent) → ActionOption[]` — option set derived from world state + agent policy + geometric constraints
- `scoreOption(option, world, agent, rng) → number` — scores one option; may consume 1× `rng.next()` for per-agent noise
- `decideAll(world, agents, rng) → DecisionBatch` — one Intent per on-court player
- Intent carries `{ kind, targetX, targetY, action, slot, satisfied }` — geometry authority for movement
- `playId` and `mode` are **bias signals** into scoring, not mechanical step-walkers

*(Since foundation 0.9.0 the concrete entry point is `runDecisionStep` (src/decision/step.ts) → `decideHandlerAction` (src/decision/expected-value.ts): options = shoot / drive / pass-to-each-mate / hold, each PRICED with the resolve rate model, argmax wins. The option/scoring split above is the conceptual contract; the EV core collapses scoring into one deterministic pricing pass.)*

**Bans (enforced by import lint):**
- MUST NOT import `adjudicate/`, `applyEvent`, `state/handlers`, `sim-utils`
- MUST NOT call `sampleDuration`, `pushEvent`, `resolveShot`, `resolvePass`
- MUST NOT consume RNG for outcomes — only for option scoring noise (1× per agent per decide)
- MUST NOT emit Events
- MUST NOT modify `state.clocks` or `state.possession` or `state.phase`

*(Since foundation 0.9.0 the EV decision core MAY import `resolve/` — it PRICES options with the same rate model resolve uses for outcomes. The dependency is one-way: resolve never imports decision.)*

**Rationale.** Decision is "what I want to do" — if decide already knows whether the shot went in, it's not a decision, it's a prophecy. Separating intent from outcome is what makes the system single-authority: the same intent can be intercepted by defense, blocked at the rim, or fouled, and those outcomes emerge from the World + Completion layer, not from the decider's imagination.

### Authority 3 — Completion (world predicates)

**Owns:** Pure functions that evaluate whether a physical process has finished given the current world.

Predicate set (closed; adding one is a minor foundation bump):

| Predicate | Trigger | Completion condition |
|---|---|---|
| `BallArrivedAtReceiver` | ball.status = `pass` | ball within ε of receiver pose; no defender in intercept corridor |
| `PassIntercepted` | ball.status = `pass` | defender within `intercept_radius` of ball trajectory segment |
| `ShotArrivedAtRim` | ball.status = `shot` | flight elapsed ≥ flightDuration |
| `HolderStripped` | ball.status = `held` + defender within `strip_radius` for τ ticks | strip window satisfied |
| `PlayerArrivedInPaint` | ball handler action = `drive` | handler pose within paint ε of attack rim |
| `PlayerCrossedHalf` | handler y crossed half-line toward attack side | zone transition |
| `ShotClockExpired` | shotClock ≤ 0 AND ball.status ≠ `shot` | clock hit zero without release |
| `GameClockExpired` | gameClock ≤ 0 | period ended |

**Contract:**
- Each predicate is a pure function `(world) → boolean`
- Predicates return facts, not outcomes — they report "what happened," not "what does it mean"
- The tick loop evaluates predicates after `stepWorld(dt)`; when a predicate fires, it produces a `Fact` enum value

**Bans:**
- MUST NOT call `rng.next()`
- MUST NOT emit Events
- MUST NOT resolve outcomes
- MUST NOT modify `state`

### Authority 4 — Adjudicate (single outcome resolution)

**Owns:** The ONLY site where:
1. RNG is consumed for outcome (one draw per fact)
2. Score changes
3. Possession changes
4. Shot-clock resets
5. Foul classification
6. Events are emitted

**Entry point:** `adjudicate(fact, world, rng) → { world', events[], episodeTransition? }`

**Resolve table (one RNG draw per row, in order):**

| Fact | Resolve draws | Rules |
|---|---|---|
| `BallArrivedAtReceiver` | 1× `pass_success` or `handoff_success` | success → holder = receiver, emit PASS/HANDOFF; fail → TURNOVER |
| `PassIntercepted` | 1× `steal_attempt_success` | success → STEAL + possession flip + **shotClock = 24**; fail → held (deflected, still offense ball) |
| `ShotArrivedAtRim` | 1× `shot_make_2pt` or `shot_make_3pt` | make → score + MADE_BASKET_DEAD + possession flip; miss → 1× `offensive_rebound_rate` → REBOUND |
| `HolderStripped` | 1× `steal_attempt_success` | success → STEAL + possession flip + **shotClock = 24** |
| `PlayerArrivedInPaint` | — no resolve (arrival is geometric) | Drive completion → sub-decision: shoot-at-rim / dish / draw-foul. **Must terminate** — cannot return to open-ended live play |
| `PlayerCrossedHalf` | — | zone = frontcourt |
| `ShotClockExpired` | — | SHOT_CLOCK_VIOLATION + **possession flips** + **shotClock = 24 for new offense** |
| `GameClockExpired` | — | PERIOD_END → period router |

Additional draws (situational, always after primary fact resolve):

| Situation | Draw | Notes |
|---|---|---|
| `foul_on_drive_rate` check | 1× rng | Only on drive-to-paint when a defender is within contact ε |
| `foul_on_shot_rate` check | 1× rng | Only on shot-release when defender contests |
| `shooting_foul` classification | already resolved; this is rule logic | FT count derived from shot_value + make/miss per AND-ONE contract in invariants.md |

**Shot-clock reset policy (single table in Adjudicate, NOT scattered across handlers):**

| Trigger | Reset to | Foundation ref |
|---|---|---|
| Possession gained via DREB | 24 | `clocks-duration.md` §Shot Clock Reset Rules |
| Possession gained via STEAL | 24 | ibid |
| Possession gained via TURNOVER | 24 | ibid |
| Possession gained via inbound after stoppage | 24 | ibid |
| Possession gained via jump ball | 24 | ibid |
| Offensive rebound (rim hit) | 14 | ibid |
| OOB by defense in frontcourt | 14 | ibid |
| SCV → new possession | 24 | Derived: new possession always = full reset |

**Bans:**
- Adjudicate is banned from Decision's internals (option scores, agent policies)
- Adjudicate is the ONLY module allowed to call `resolveShot`, `resolvePass`, `resolveRebound`, `resolveSteal`, `resolveFt`
- Adjudicate is the ONLY module allowed to call `emitSemantic` / `applyEvent`

### Authority 5 — Episode (possession lifecycle)

**Owns:** The finite state machine of a single possession.

```
States:  OPEN  →  CONTINUING_OREB  →  ENDED
          │              │                │
          └──────────────┘                │
         same binding; no new id          │
                                          ▼
                                    possession_log entry written
```

**State machine (closed transitions):**

| From | Trigger (arrives from Adjudicate) | To | Action |
|---|---|---|---|
| `OPEN` | `MAKE` | `ENDED` | Log possession; next startReason = AFTER_MAKE |
| `OPEN` | `MISS_DREB` | `ENDED` | Log possession; next startReason = AFTER_DREB |
| `OPEN` | `MISS_OREB_CONTINUE` | `CONTINUING_OREB` | NO new id; binding persists; shotClock already = 14; re-select play advisory |
| `OPEN` | `TURNOVER` | `ENDED` | Log possession; next startReason = LIVE_BALL_TURNOVER_RECOVER |
| `OPEN` | `SHOT_CLOCK_VIOLATION` | `ENDED` | Log possession; next startReason = AFTER_INBOUND |
| `OPEN` | `SHOOTING_FOUL` | `ENDED` | Log possession; FT sequence; after FT → appropriate startReason |
| `OPEN` | `NON_SHOOTING_FOUL` | `ENDED` (if bonus) or `ENDED` (if no bonus → inbound same team) | Per bonus rule in resolve.md |
| `OPEN` | `PERIOD_END` | `ENDED` | Truncated; no next possession (period router takes over) |
| `CONTINUING_OREB` | any terminal above | `ENDED` | Log continued possession; normal end-reason rules |

**Contract:**
- `possession_id` increments ONLY on `OPEN → ENDED` (not on OREB continue)
- `endReason` is set by Adjudicate, not by simulate-loop scanning event types
- The simulate loop reads `episode.state` — never infers lifecycle from event inspection

**Bans:**
- Episode MUST NOT know about decision options, resolve probabilities, or clock internals
- Episode MUST NOT emit events directly

## 2. The Single Causal Path (per tick)

```
for each tick:
  1. World: advance realClock; if LIVE, advance gameClock and shotClock

  2. Decision: if cadence window open AND ball.holderId can decide AND phase=LIVE:
       decideAll(world, agents, rng) → Intention[]
       apply intents to pose targets (setTarget only, no clock)

  3. World: stepPoses(dt, mobility) + stepBall(dt)
       → updated poses, ball position, ball status

  4. Completion: evaluate predicates(world) → Fact?
       if no fact: continue to next tick

  5. Adjudicate: if fact present:
       resolve outcome (ONE rng draw per fact)
       apply score, possession, shotClock, phase changes
       emit Event[] (projections only — world already updated)
       set episodeTransition for Episode layer

  6. Episode: apply transition if any; if ENDED, write possession_log entry

  7. Snapshot: WorldSnapshot(world) — position truth export
```

**Key: the causal chain is one pass.** There is no "commit pre-computes events, then tick re-interprets them." Events are emitted at step 5 and never fed back into World.

## 3. Module Import Bans (Enforceable)

These are structural contracts, not suggestions. The foundation linter (check-foundation.mjs) SHOULD enforce them by scanning `src/` import graphs once the migration target module tree exists.

```
Module      MAY import from              MUST NOT import from
─────────────────────────────────────────────────────────────────
world/      rng/, config/                decision/, resolve/, adjudicate/, box-score/
decision/   world/, rng/, config/, resolve/   adjudicate/, state/handlers, sim-utils
completion/ world/                       rng/, resolve/, adjudicate/, decision/
adjudicate/ world/, resolve/, config/,   decision/ (except types), completion/ internals
            state/handlers, state/apply
episode/    (types only)                 world/, decision/, resolve/, adjudicate/
simulate/   all of the above (orchestrator)  — (full access allowed; NEVER contains business logic branches)
box-score/  events, config/              decision/, resolve/, adjudicate/
```

`simulate/` has full access as orchestrator but is **forbidden from:**
- `if (eventType === 'TURNOVER') { endPossession() }` — reads episode state instead
- Any direct resolve call — routes through Adjudicate
- Any direct clock mutation outside tick integration

## 4. Intent Purity Contract

```
Intent {
  kind       // what I want to do
  targetX/Y  // where I want to go
  slot       // tactical relation
  satisfied  // already in position

  // FORBIDDEN on Intent:
  //   outcome (made/missed/success/fail)
  //   duration (seconds consumed)
  //   event (pre-built Event object)
  //   score delta
  //   possession delta
}
```

`DecisionBatch` carries ONLY `{ intents: Intent[], ballIntent: Intent }`. The `CommitResult` type (with its `events[]`, `possessionEnded`, `endReason`, `retargetPayload`) is **deleted** as a Decision-layer concept. Those fields are Adjudicate-layer outputs.

## 5. Duration Authority Migration

Current state: `config/duration.json` defines per-action duration distributions; `commit.ts` samples them and deducts game clock in bulk. This is the **second time physics** that conflicts with tick integration.

Target state:

- `config/duration.json` is **downgraded** from "runtime authority" to "reference targets." Its `clocks` section (period, shot, OT lengths) remains authoritative. Its `durations[]` table becomes **design-intent documentation** — the expected mean of each action type, used only for pace-band calibration, NOT for runtime clock deduction.
- Clock advance is purely `gameClock -= dt` integration in the tick loop.
- The completion predicate fires when the geometric condition is met; real time elapsed is whatever the tick loop accumulated, not a pre-sampled value.
- `made_fg_stoppage` rules remain authoritative (they determine clock behavior, not clock deduction).

**Why:** In real basketball, a pass takes however long the ball takes to travel at the speed it was thrown, given the distance. It does not take "a uniform-random value between 0.65 and 1.4 seconds extracted from a config table." The config table was a useful Phase-1 shortcut; the single-axis architecture kills the shortcut because it conflicts with physical continuity.

## 6. DRIVE Termination Contract

`DRIVE` as a standalone cosmetic marker that can be followed by `PASS`, `HANDOFF`, or further `DRIVE` indefinitely — without ever reaching a shot, foul, or turnover — has no basketball equivalent.

The `PlayerArrivedInPaint` predicate is the completion trigger for a drive intent. On completion, the handler **enters a sub-decision** constrained to one of:
- `shoot_at_rim` → `ShotArrivedAtRim` → Adjudicate
- `dish_to_cutter` → `BallArrivedAtReceiver` → Adjudicate
- `draw_contact` → foul check → Adjudicate

The sub-decision MAY consume 1× RNG for option selection but MUST NOT loop back to open-ended halfcourt play. A drive intent that does not reach paint within reasonable ticks (clamped by shot clock) triggers a forced termination — either a contested shot (resolve) or a turnover (held too long).

**This is a structural prohibition, not a probability tweak.** The option set beyond `PlayerArrivedInPaint` is closed; there is no "drive then hold then pass then drive..." path in the state space.

## 7. Event Catalog Amendments

No event types are added or removed. Three existing events change status:

| Event | Current status | Target status |
|---|---|---|
| `DRIVE` | Cosmetic (rank 6, mutates `ball.holderId` only) | **Semantic** — signals that a drive intent completed geometrically. Kept at rank 6. The `mutates` array gains no new paths (position truth remains in WorldSnapshot). |
| `ALIGN_HALFCOURT` | Cosmetic (rank 6, empty mutates) | **Unchanged** — remains cosmetic. Future: consider removal once relational retarget is the only alignment mechanism. |
| `ADVANCE_BACKCOURT` / `CROSS_HALF` | Cosmetic (rank 6) | **Unchanged** — remain semantic markers for broadcast. The `ball.zone` update is handled by World, not by these events' `mutates`. |

The `mutates` array for `POSSESSION_GAINED` is **confirmed** to include `clocks.shot` (already present in the catalog at 0.4.0). Other possession-change events (`STEAL`, `TURNOVER`, `LOOSE_BALL_RECOVER`, `REBOUND`) MUST have their shot-clock reset applied by Adjudicate, not by individual handler `mutates` — the reset policy is a single table in Adjudicate, not distributed across handlers. The catalog `mutates` for these events are **not** widened; the shot-clock change is applied by Adjudicate as a consequence of the possession flip, not as a per-event `mutates` entry.

## 8. Migration Phases (non-breaking until Phase 4)

Each phase has an **architectural acceptance criterion** — a structural property that can be verified by static analysis or targeted integration tests, NOT by score/pace regression.

### Phase 0 — Contract Lock (this document)
**Deliverable:** this document is reviewed, approved, merged as `docs/foundation/architecture.md`.
**Bump:** `foundation_version` → `0.5.0` (minor: additive document, no breaking code change).
**Acceptance:** foundation linter `xref` check passes the new doc reference.

### Phase 1 — Sever Decision from Outcome
**Code changes:**
1. Extract `commitDecisions` into: `commitIntents(batch) → { events: [], retarget: {} }` (no resolve, no duration, no `possessionEnded`)
2. Move resolve calls from `commit.ts` into a new `adjudicate.ts` stub called from `sim-tick`
3. `sim-tick.applyLiveDecision` calls commitIntents, then adjudicate separately
4. Delete `sampleDuration` calls from decision path; keep tick integration

**Acceptance:**
- `grep -r "resolveShot\|resolvePass\|resolveRebound\|resolveSteal\|resolveDrive\|resolveFt\|sampleDuration" src/decision/` returns zero matches
- Golden seed MUST break (major bump queued for Phase 4)
- All existing tests MUST pass after test expectation update (events shift in count but not in semantic categories)

### Phase 2 — Completion Predicates
**Code changes:**
1. Implement predicate functions in `src/completion/` (pure, no RNG, no events)
2. Wire predicates into tick loop between `stepWorld` and `adjudicate`
3. Move `stepBall` events (`pass_complete`, `shot_arrived`, `strip`) from direct-event-emission to Fact production
4. Ball-motion completion events are re-emitted by Adjudicate, not by `stepBall`

**Acceptance:**
- Every `SHOT_RELEASE` event has exactly 0 or 1 corresponding `SHOT_RESULT` (no and-one orphan)
- `STEAL` event is always preceded by a ball-status change to a stealing defender, and followed by `shotClock === 24` in the next `WorldSnapshot` (contract clause)
- `OREB` resolves exactly once per miss (not twice)

### Phase 3 — DRIVE Sub-State Machine & Intent Stickiness
**Code changes:**
1. Ball-handler intent persists across ticks until completion or redecide event
2. `AttackRim` sub-decision gears: drive → arrive-paint → {shoot, dish, contact}
3. Redecide triggers: possession change, pass caught, clock-band crossing, dead ball
4. Remove fixed-cadence decision as the sole entry point

**Acceptance:**
- No event sequence containing `DRIVE` followed by ≥3 non-terminal actions without a `SHOT_RELEASE`, `FOUL`, `TURNOVER`, or `SHOT_CLOCK_VIOLATION`
- Redecide fires on possession change within 1 tick

### Phase 4 — Duration Authority Sunset
**Code changes:**
1. Remove `sampleDuration` and all bulk-clock-deduction paths
2. `duration.json#durations` downgraded to reference-only; linter updated
3. Pace bands re-evaluated against tick-integral time
4. Regenerate golden fixtures; full pace:check re-run

**Acceptance:**
- `grep -r "sampleDuration\|duration.*sample" src/` returns zero matches outside `src/duration/` (which becomes a pure reference-data loader)
- Golden seed regenerated and stable
- Pace 1..256 within bands (bands may need widening if tick-integral time differs significantly — that's calibration, not architecture)

### Phase 5 — Import Ban Enforcement
**Code changes:**
1. Add import-graph checks to `scripts/check-foundation.mjs` per the module ban table (§3)

**Acceptance:** `npm run foundation:lint` fails on any banned import.

## 9. What Does NOT Change

These are preserved across migration — the single-axis architecture is additive, not a rewrite:

| Preserved | Why |
|---|---|
| Event catalog (38 types) | Events remain the log projection; no types added or removed |
| FSM transition table | Phases and legal transitions unchanged |
| RNG single-stream contract | `rng.next()` threading unchanged; draw order revised per §1 |
| Identity/role model | `usageProfile`, `RoleBinding`, bind algorithm untouched |
| Relational retarget (live path) | `relations.ts`, `play-slots.json`, `Intent.targetX/Y` preserved |
| Dead-ball absolute setups | `dead-setup.ts` unchanged; absolute placement for rule-mandated setups only |
| Homogeneous model | No per-player ratings; outcomes depend on role only |
| Box score 18-field schema | Output shape preserved; assist counting added (already in schema field #13) |
| Spectator/render pipeline | Consumes `WorldSnapshot[]` as today; architecture change is upstream |

## 10. Cross-References

- **events.md** — Event catalog (38 types); §7 amends event semantics without changing the closed set.
- **rng.md** — Single-stream contract; draw-order revisions in Adjudicate (§1.4) are additive (extra draws for new completion facts), not reordered.
- **invariants.md** — I1–I6 preserved; I3 (clamp) becomes "predicate fires when clock reaches zero via tick integration."
- **fsm.md** — 15 phases, 28 transitions unchanged. SCV now properly routes `DEAD_VIOLATION → INBOUND_SETUP` with correct possession flip.
- **clocks-duration.md** — Clock constants preserved; `duration.json#durations` downgraded per §5.
- **identity-roles.md** — Role binding, retarget contract, on-ball tracking preserved.
- **possession-plays.md** — Mode/play select preserved; play step-walker downgraded to advisory bias only (§1.2).
- **resolve.md** — Base rates preserved; resolve functions called ONLY from Adjudicate (§1.4).
- **timeline-io.md** — GameResult schema unchanged; `possession_log` entries now include `SHOT_CLOCK_VIOLATION` end-reason.
- **dual-clock-motion.md** (`.omo/plans/`) — This document is the **contract-level realization** of that plan; the plan remains the design narrative.
