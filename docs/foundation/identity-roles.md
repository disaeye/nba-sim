# Identity, Roles & Court Zones

`foundation_version: 0.4.0` · `player_identity_model: jersey-based` · `zones: 14`

This document is the **sole authority** for who is on the court, how a lineup assigns them to roles for a single possession, and how those roles stay bound to the live ball. The closed machine-readable CourtZone enum lives at [`config/court-zones.json`](../../config/court-zones.json) and is validated by [`config/schemas/court-zones.schema.json`](../../config/schemas/court-zones.schema.json). The `LineupPackage` shape is validated by [`config/schemas/lineup-package.schema.json`](../../config/schemas/lineup-package.schema.json). If prose here ever disagrees with the JSON or schema, the JSON wins.

## Player Identity

A `Player` is identified by three fields and **only** three fields:

| Field | Type | Purpose |
|---|---|---|
| `id` | string | Stable game-scoped player identifier (e.g. `"home_p7"`). Used as the foreign key in events, the box score, and `RoleBinding`. |
| `teamId` | `"home" \| "away"` | The team the player belongs to for this game. Only those two literal strings are legal. |
| `jersey` | string | The jersey string shown to humans and used by `LineupPackage.usageProfile` and `RoleBinding` (e.g. `"23"`, `"7A"`). Matches `^[a-zA-Z0-9]+$`. |

**No position field drives logic.** The kernel MUST NOT branch on `PG | SG | SF | PF | C` strings or any equivalent positional label. Role assignment is driven entirely by `LineupPackage.usageProfile` (below) combined with the chosen play's slots. Adding a position field that the kernel reads is a contract violation.

A `label` field MAY exist on a `Player` for display only (e.g. `"J. Smith"`). The kernel MUST NOT read, branch on, or persist `label` into any event payload or box-score field. It is invisible to the simulation.

## LineupPackage

A `LineupPackage` describes the **five players on court** for one team at one moment, plus that unit's preferred role priority. A team carries multiple packages per game (decision M5) — typically a `starters` package and one or more bench units — and the **same jersey may be priority-ordered differently across packages**: a player who is `primary_creator` on the bench unit may be `secondary_creator` or a `spacer` on the starters. This is the property the owner locked during planning and the reason role priority lives on the package, not on the player.

Shape:

```json
{
  "id": "starters",
  "players": ["3", "7", "11", "23", "32"],
  "usageProfile": {
    "creator":   ["3", "11"],
    "screener":  ["32"],
    "spacer":    ["7", "23"]
  }
}
```

| Field | Type | Constraint |
|---|---|---|
| `id` | string | Package identifier (`"starters"`, `"bench_unit"`, …). |
| `players` | string[5] | Exactly five unique jersey strings. Order has no semantics. |
| `usageProfile.creator` | string[] (priority-ordered) | At least one entry. Index 0 binds `primary_creator`; index 1 (if present) binds `secondary_creator` when the play declares that slot. |
| `usageProfile.screener` | string[] (priority-ordered) | May be empty for packages with no designated screener. |
| `usageProfile.spacer` | string[] (priority-ordered) | First eligible binds `spacer_strong`; next binds `spacer_weak`. |

Every jersey appearing in `usageProfile.creator`, `.screener`, or `.spacer` MUST also appear in `players`. This closed reference inside the package is enforced by the foundation linter (todo 9); JSON Schema draft-07 cannot express cross-array subset constraints portably. A jersey MAY appear in more than one usage list (e.g. a creator who also doubles as a spacer is fine) — only the across-list disjunction is unconstrained.

## Role Slots

Each play (T8) declares a fixed set of offense slots and a fixed set of defense live slots. A `RoleBinding` (next section) fills those slots with jerseys for one possession.

### Offense Slots

| Slot | Filled From | Purpose |
|---|---|---|
| `primary_creator` | `usageProfile.creator[0]` | Initiates the action; default ball-handler at the start of the possession. |
| `secondary_creator` | `usageProfile.creator[1]` (optional) | Secondary initiation option; used only by plays that declare a second creator slot. |
| `screener` | `usageProfile.screener[0]` | Sets the on-ball screen in `pnr_high`, `handoff_wing`, etc. |
| `spacer_strong` | `usageProfile.spacer[0]` | Strong-side floor spacer (typically the stronger shooter / strong-side corner or wing threat). |
| `spacer_weak` | `usageProfile.spacer[1]` | Weak-side floor spacer. |

### Defense Live Slots

| Slot | Purpose |
|---|---|
| `on_ball` | Defender guarding the **current ball handler**. See Live Assignment & On-Ball Retarget below. |
| `deny` | Defender one pass away; denies the next-likely receiver. |
| `help` | Primary help defender at the basket or in the gap. |
| `tag` | Low help on roll cuts; tags the roll man or the dunker spot. |
| `weak_side` | Weak-side defender maintaining floor balance and rotation coverage. |

The five defense slots map one-to-one onto the opposing five offensive jerseys, but the mapping is **dynamic with respect to the ball** — only `on_ball` is special-cased by the live assignment rule below.

## RoleBinding

A `RoleBinding` maps each play's role slot → jersey for **one possession**. It is computed from the active `LineupPackage.usageProfile` combined with the play's declared slots, and it lives only as long as that possession. Concretely:

- The kernel calls `bindRoles(package, play)` once at possession start, producing an opaque `RoleBinding` object.
- That object is consumed by the play runner and the resolver for the lifetime of the possession.
- A new possession (after a make, defensive rebound, turnover, or stoppage) triggers a fresh bind from whatever package is then active.
- **Offensive rebounds do NOT trigger a rebind.** The same possession continues with the same `RoleBinding` (see `docs/foundation/possession-plays.md`, todo 8).
- Substitutions (`SUB` events at legal windows) swap the active `LineupPackage`; the next possession binds from the new package.

`RoleBinding` is a runtime construct, not a config file. Its concrete shape and the `bindRoles` algorithm are specified in `docs/foundation/possession-plays.md` (T8); the offense/defense slot names above are the vocabulary that algorithm operates on.

## Live Assignment & On-Ball Retarget

This section is **the contract that prevents the "steal on an off-ball player" bug** the owner raised during planning. It is non-negotiable in v0.1.0.

1. **`ball.holderId` is the authoritative ball controller at all times.** It is a top-level field on `GameState` and is updated by exactly the events whose catalog entry declares `ball.holderId` in `mutates` (see `config/event-catalog.json`: `JUMP_BALL_TAP`, `POSSESSION_GAINED`, `INBOUND_TOUCH`, `PASS`, `HANDOFF`, `REBOUND`, `LOOSE_BALL_RECOVER`, `STEAL`).

2. **The `on_ball` defense slot always tracks the current `ball.holderId`**, never the initial `primary_creator`. If the ball moves, the on-ball defender moves with it. The other four defense slots (`deny`, `help`, `tag`, `weak_side`) are computed once per possession; only `on_ball` is recomputed on every ball-moving event.

3. **`PASS` and `HANDOFF` are atomic events.** A single `PASS` or `HANDOFF` event simultaneously:
   - updates `ball.holderId` to the receiver, AND
   - retargets `on_ball` to the new holder's defender,

   in the **same** event resolution. There is no intermediate state in which the ball is "with the receiver" but `on_ball` is still guarding the passer. Any kernel code that produces such an intermediate state is a bug.

4. **`STEAL`, `STRIP`, and `CONTEST` targets resolve against `ball.holderId` at resolve time**, never a stale player ID captured earlier in the step. If the resolver is asked to compute a steal outcome for a target that does not equal the current `ball.holderId`, that is a kernel bug and the resolver MUST reject it.

5. **A steal target that ≠ current `holderId` is a kernel bug**, full stop. The kernel tests (todo 16) include a property test that no `STEAL` event ever names a `target_id` different from the `ball.holderId` value folded from the event prefix up to that moment; the foundation linter (todo 9) cross-checks the same invariant on saved timelines.

This rule is what makes the on-ball defender a *defender of the live ball* rather than a *defender of an offensive player*. Treating it as the latter is the failure mode this contract exists to prevent.

## Court Zones

`CourtZone` is a **closed enum of exactly 14 logical position labels** for v0.1.0. The enum itself carries no metric information. A zone tells the resolver and the play runner *which region of the half-court* an action happened in.

Continuous player poses and `WorldSnapshot` use metric `(x, y)` coordinates from the court geometry (`config/court-geometry.json`, `spectator/court-geometry.json`). Zones are derived from those coordinates via `zoneFromPoint(x, y)` at event emission time. The zone label on an event is a classification, not a coordinate. Live movement targets are produced by **slot relations** (`config/play-slots.json`, `src/court/relations.ts`) evaluated in the current ball/pose world — not by stamping absolute halfcourt diagrams.

The closed list, defined in [`config/court-zones.json`](../../config/court-zones.json):

| # | Zone | Group |
|---|---|---|
| 1 | `backcourt` | Before half-court (advance / transition). |
| 2 | `frontcourt_center` | Top of the key / near-arc center. |
| 3 | `slot_L` | Left slot. |
| 4 | `slot_R` | Right slot. |
| 5 | `wing_L` | Left wing (3PT). |
| 6 | `wing_R` | Right wing (3PT). |
| 7 | `corner_L` | Left corner (3PT). |
| 8 | `corner_R` | Right corner (3PT). |
| 9 | `elbow_L` | Left elbow of the key. |
| 10 | `elbow_R` | Right elbow of the key. |
| 11 | `paint` | The painted area / key. |
| 12 | `dunker_L` | Left dunker spot (baseline corner of the paint). |
| 13 | `dunker_R` | Right dunker spot. |
| 14 | `rim` | At the basket. |

Adding or removing any zone requires a `foundation_version` bump per the policy in [`docs/foundation/README.md`](README.md). The schema pins `minItems: 14, maxItems: 14` and `uniqueItems: true` so any add/remove or accidental duplicate is a schema failure first.

## Shot Value & Zone Consistency

`SHOT_RELEASE` carries `shot_value: 2 | 3` in its payload (see `config/event-catalog.json` — `shot_value` is a required field alongside `shooter_id` and `zone`). The shot's point value is **authoritative on the payload**, not inferred from the zone alone, because zones like `frontcourt_center` and `wing_*` are spatially ambiguous about arc distance and a future minor bump could re-zone them.

For consistency, the kernel MUST keep the zone and the shot value aligned:

- If `shot_value == 3`, then `zone` MUST be one of the 3PT zones: `corner_L`, `corner_R`, `wing_L`, `wing_R`, `frontcourt_center`.
- If `shot_value == 2`, then `zone` MUST NOT be one of the four corner/wing zones (`corner_L`, `corner_R`, `wing_L`, `wing_R`). `frontcourt_center` is allowed for a 2PT mid-range look; `paint`, `rim`, `dunker_L`, `dunker_R`, `elbow_L`, `elbow_R`, `slot_L`, `slot_R`, and `backcourt` are 2PT-only (a `SHOT_RELEASE` from `backcourt` is legal but rare and treated as a 3PT heave only if `shot_value == 3`, which then violates the rule above — so in practice a `backcourt` shot is `shot_value == 2`).

This consistency is **not** enforced by config schema — it is a property of emitted events, not of the static catalog. The runtime check lives in the foundation linter (todo 9) and in the kernel's event-emission tests (todo 14): every `SHOT_RELEASE` the kernel emits must satisfy the rule above, and a violating event fails the lint pass and the test.

## Version Discipline

The identity model, role slots, CourtZone enum, and `ball.holderId` retarget contract follow the same semver policy as the rest of the foundation (see [`docs/foundation/README.md`](README.md)):

- **Patch** (`0.1.0` → `0.1.1`): prose clarifications only.
- **Minor** (`0.1.0` → `0.2.0`): additive and backward compatible (e.g. a new `CourtZone`, a new optional `usageProfile` field). The `court-zones.schema.json` `minItems`/`maxItems` constraints and the `lineup-package.schema.json` `additionalProperties: false` boundary must be widened at the same time.
- **Major** (`0.1.0` → `1.0.0`): removing or renaming a role slot, a zone, or the `holderId` retarget rule; reshaping `LineupPackage`; or introducing a position-string logic field (explicitly forbidden in v0.1.0 and any future introduction must be a major bump).
