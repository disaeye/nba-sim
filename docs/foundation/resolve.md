# Resolve Contract

`foundation_version: 0.4.0`

This document freezes the **homogeneous** outcome resolver for v0.1.0: the model that turns a play step into a binary outcome (make/miss, complete/incomplete, succeed/fail), the set of resolve checks the engine performs, the **sanity bands** the pace regression will assert, and the **FT taxonomy** that limits free throw sequences to personal shooting fouls.

Implementation references: T18 (resolve + FT sequence), T21 (pace regression bands).

> **Authority.** This document plus `config/resolve.json` and `config/schemas/resolve.schema.json` are the sole authority for outcome resolution. Code that disagrees is a bug.

---

## Homogeneous Model

Decision: **all players have identical talent in v0.1.0** (Metis G2.3, G2.4). There is NO per-player talent field, NO per-player rating, NO `creator_rating`, NO `shooting_rating`. Two players swapped in the same `RoleSlot` produce statistically identical outcomes over the same RNG stream.

The probability of any outcome is:

```
P(outcome) = base_rate + situation_modifiers + noise
```

where:

| Term | Value in v0.1.0 |
|---|---|
| `base_rate` | The relevant entry in `config/resolve.json#base_rates` (e.g. `shot_make_2pt = 0.48`). |
| `situation_modifiers` | Pinned at neutral (0). Reserved for future tuning; the v0.1.0 contract treats every situation as nominal. |
| `noise` | A single `rng.next()` draw consumed by the resolve check. |

In code:

```
resolve(base_rate, rng):
  return rng.next() < base_rate   // true = success, false = failure
```

The `model` field in `config/resolve.json` pins this: `"homogeneous_v0_1"`. Any change to this string is a major `foundation_version` bump. A future `talented_v0_2` (or similar) model would introduce per-player ratings; that model is OUT of v0.1.0.

**Draw order** is fixed by `docs/foundation/rng.md` (T3): `duration → mode_select → play_select → action_resolve`. Each resolve check consumes exactly one `rng.next()`. Reordering draws is a major version bump (it would change every seed-stable output).

The homogeneous model is also a property gate (T22): swapping two jerseys in a `usageProfile` (so a different player fills the same `RoleSlot`) MUST NOT change outcomes, because the resolve function consumes only `base_rate + rng` — never the player ID.

---

## Resolve Checks

The engine performs one resolve check per play step that needs a binary outcome. Each check consumes exactly **one** `rng.next()` draw from the threaded single stream.

| Check | `base_rate` key | Triggered by play action | Outcome |
|---|---|---|---|
| **Pass** | `pass_success` | `PASS`, `PASS_TO_SPACER` | success → receiver gains possession (`PASS` event emitted); failure → `TURNOVER` |
| **Handoff** | `handoff_success` | `HANDOFF` | success → receiver takes the ball (`HANDOFF` event, on-ball defender retargets); failure → `TURNOVER` |
| **Drive** | `drive_success` | `ATTACK_RIM`, `DRIVE_OR_SHOOT`, `ISO_DRIVE` (the play-internal drive states) | success → ball reaches the rim → shot or continuation; failure → `STEAL`/`TURNOVER` attempt |
| **Shot (2pt)** | `shot_make_2pt` | `SHOT_RELEASE` with `shot_value = 2` | success → `SHOT_RESULT(made=true)`; failure → `SHOT_RESULT(made=false)` → `REBOUND` follow-up |
| **Shot (3pt)** | `shot_make_3pt` | `SHOT_RELEASE` with `shot_value = 3` | success → `SHOT_RESULT(made=true)`; failure → `SHOT_RESULT(made=false)` → `REBOUND` follow-up |
| **Rebound** | `offensive_rebound_rate` | After `SHOT_RESULT(made=false)` | success → `REBOUND(offensive=true)` (OREB continues possession per Metis G1.8); failure → `REBOUND(offensive=false)` (DREB flips possession) |
| **Free throw** | `ft_make` | Each `FT_ATTEMPT` in an `FT_SEQUENCE` | success → `FT_RESULT(made=true)`; failure → `FT_RESULT(made=false)` |

The shot-value decision (2 vs 3) is made by the play structure and the court zone at `SHOT_RELEASE`, not by the resolve function. The resolve function only consumes the appropriate `base_rate` and one `rng.next()`.

**Foul-on-drive** is a side outcome modeled by `foul_on_drive_rate`. When a drive resolves, the engine MAY consult an additional `rng.next()` to determine whether a personal shooting foul is whistled; this is the entry point to the FT taxonomy below. The exact draw site and order are pinned by the RNG draw-order appendix (T3).

**Steal attempts** are gated by `steal_attempt_success` and only consulted when the on-ball defender elects to attempt a steal (the elective policy is OUT of v0.1.0; in v0.1.0 the steal attempt policy is uniform and documented in T18).

---

## Sanity Bands

The pace/possession regression harness (T21) runs `simulateGame` over seeds `1..256` and asserts that aggregate outputs land inside these bands. The bands are also documented here as the foundation contract for "the engine produces modern NBA rhythm":

| Band | min | max | Applies to |
|---|---|---|---|
| Aggregate FG% | `0.25` | `0.65` | Field-goal percentage across all shots in all 256 games. |
| Aggregate 3PT% | `0.20` | `0.55` | 3-point field-goal percentage across all 3PA. |
| Per-team mean score | `60` | `160` | Mean final score per team across the 256 games. |
| Aggregate OREB% | `0.10` | `0.40` | Offensive rebound percentage (OREB / (OREB + opponent DREB)). |

Additionally, the **pace bands** documented in `docs/foundation/clocks-duration.md` (T6) and the plan's decision M8 apply:

- Per-team mean pace ∈ `[97, 103]` possessions per 48 minutes.
- Mean live possession length ∈ `[13.5, 15.5]` seconds.
- Transition share ∈ `[12, 18]`% of possessions.

If any band is violated, T21 exits non-zero and the engineer tunes **only** `config/duration.json` and/or `config/resolve.json` — never FSM hacks, never foundation-version bumps to silence failures.

The bands are encoded in `config/resolve.json#sanity_bands` and structurally validated by `config/schemas/resolve.schema.json` (each `*_min`/`*_max` pair is typed `number` with `minimum: 0` and `maximum: 1` for the percentage fields). The `*_max >= *_min` invariant is enforced by the foundation linter (T9) and the `jq '.sanity_bands'` audit; it is not expressed via `$data` because the v0.1.0 ajv-cli verification command does not pass `--data`.

---

## FT Taxonomy

Decision **Metis G3.3**: Phase-1 free throws are **personal shooting fouls only**. The full taxonomy:

| FT type | When awarded | Number of attempts |
|---|---|---|
| `2_shot` | Personal shooting foul on a 2-point field-goal attempt. | 2 |
| `3_shot` | Personal shooting foul on a 3-point field-goal attempt. | 3 |
| `and_one_1_shot` | Personal shooting foul on a successful field-goal attempt (the AND-ONE rule, see `docs/foundation/invariants.md` T4). | 1 |
| `bonus_2_shot` | Non-shooting personal foul by a defense that is in the bonus (see Bonus Rule below). | 2 |

### Bonus Rule

```
bonus_rule.team_fouls_per_period_threshold = 5
```

The 5th team foul in a period triggers the bonus. From that point until the end of the period, every non-shooting personal foul by that team awards `bonus_2_shot` (2 FTs) to the offended team. Shooting fouls are unaffected by the bonus — they always award their corresponding shot-equivalent FTs (`2_shot`, `3_shot`, or `and_one_1_shot`).

Team foul counters reset at the start of each period (regulation quarter or overtime period). Overtime has its own bonus counter independent of regulation.

### Excluded FT types

The following FT types are **OUT of Phase-1** and are listed as `forbidden` in `config/resolve.json#ft_taxonomy_v0_1`:

| Forbidden type | Reason |
|---|---|
| `technical` | Technical fouls. Out of scope; would require umpire/coach/behavior modeling. |
| `flagrant_1` | Flagrant foul type 1. Out of scope; would require ejection modeling. |
| `flagrant_2` | Flagrant foul type 2. Out of scope; would require ejection modeling. |
| `clear_path` | Clear-path fouls. Out of scope; rare and complex. |

Adding any of these is a major `foundation_version` bump. The schema enforces the closed 4-entry `allowed` and 4-entry `forbidden` arrays via `minItems=maxItems=4` plus per-item `enum`; the foundation linter (T9) additionally checks the exact set.

### AND-ONE interaction

The AND-ONE rule (T4 invariants) is the interaction between `SHOT_RELEASE` + `FOUL` at the same `t_game`:

- If `SHOT_RESULT(made=true)` AND `FOUL(shooting=true)` co-occur → `and_one_1_shot` (1 FT).
- If `SHOT_RESULT(made=false)` AND `FOUL(shooting=true)` co-occur → `2_shot` or `3_shot` (depending on the `shot_value` at `SHOT_RELEASE`), with no make credit.

The same `ft_make` base_rate governs each individual FT in either case.

---

## Cross-references

- **T2** — `FT_START`, `FT_ATTEMPT`, `FT_RESULT`, `FT_SEQUENCE_END`, `FOUL`, `SHOT_RELEASE`, `SHOT_RESULT`, `REBOUND` events.
- **T3** — `docs/foundation/rng.md` pins the per-check `rng.next()` draw order.
- **T4** — `docs/foundation/invariants.md` documents the AND-ONE rule (priority ranks 1–6).
- **T6** — `config/duration.json` defines `ft_attempt_logic` duration (engine timing between FT_ATTEMPTs).
- **T7** — `docs/foundation/identity-roles.md` defensive RoleSlots (the foul offender and victim are pulled from the live on-court binding).
- **T9** — `scripts/check-foundation.mjs` linter cross-references resolve keys and FT taxonomy entries.
- **T18** — Resolve + FT sequence implementation.
- **T21** — Pace regression harness consumes `sanity_bands` and the M8 pace bands.
- **T22** — Homogeneous property gate: jersey-swap invariance test.
