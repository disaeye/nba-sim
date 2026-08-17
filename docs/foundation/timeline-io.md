# Timeline & I/O (GameResult)

`foundation_version: 0.4.0`

This document freezes the **input/output shape** of `simulateGame(input) → GameResult` for v0.1.0: the top-level `GameResult` object, the per-event timeline shape, the **exact PlayerBox field set** (no more, no less), and the per-possession audit log. The schema in `config/schemas/game-result.schema.json` is the machine-checkable contract; this document is the human-readable authority.

Implementation references: T19 (`simulateGame` end-to-end + timeline + box), T20 (golden determinism fixture), T21 (pace regression consumes `possession_log`).

> **Authority.** This document plus `config/schemas/game-result.schema.json` are the sole authority for the simulator's output shape. Code that emits a `GameResult` not matching this contract is a bug.

---

## GameResult Schema

Top-level shape:

```
GameResult {
  meta:             GameMeta
  events:           TimelineEvent[]
  box_score:        { home: PlayerBox[], away: PlayerBox[] }
  possession_log:   PossessionEntry[]
}
```

The top-level object has exactly four keys, pinned by `additionalProperties: false` + `required`. A `foundation_version` is carried inside `meta` (not at the top level) so the entire run is self-describing.

### `GameMeta`

| Field | Type | Required | Description |
|---|---|---|---|
| `foundation_version` | string (semver) | yes | Copied from `config/foundation.json` at simulation time. Two `GameResult`s with different `foundation_version` are not directly comparable. |
| `seed` | integer ≥ 0 | yes | uint32 seed passed to mulberry32 (T12). Identical seed + identical input + identical `foundation_version` ⇒ byte-identical events and final score (golden fixture T20). |
| `home_team_id` | string | yes | Identifier of the home team (matches a team id in `config/demo-game.json` T10). |
| `away_team_id` | string | yes | Identifier of the away team. |
| `started_at` | string | no | Optional ISO-8601 timestamp marking when `simulateGame` was invoked. Used for human-readable provenance only; never consumed by replay. |

The engine MUST populate the four required fields. `started_at` is allowed but not required so that pure tests can construct minimal valid `GameResult`s without wall-clock data (T2 learning: keep the verification gate inline-constructible).

### `TimelineEvent`

| Field | Type | Required | Description |
|---|---|---|---|
| `type` | string (`^[A-Z][A-Z0-9_]*$`) | yes | `EventType` name; must be a member of `config/event-catalog.json#events[].type` (T2). Cross-referenced by the foundation linter (T9). |
| `t_game` | number ≥ 0 | yes | Game-clock timestamp (seconds) at which the event was emitted. Quantized to 0.1s per `docs/foundation/clocks-duration.md` (T6). |
| `period` | integer ≥ 1 | yes | Period number. 1–4 for regulation; ≥5 for overtimes (decision M6). |

Per-event payload fields are intentionally permissive in the schema (`additionalProperties: true`). The required-payload-fields contract for each `EventType` lives in `config/event-catalog.json` and is enforced by the foundation linter (T9). The schema here only pins the structural trio common to every event.

A completed game produces `events.length > 50` (acceptance criterion T19); empty `events` is permitted only for early-abort run shapes used in unit tests.

### `PossessionEntry`

| Field | Type | Description |
|---|---|---|
| `possession_id` | integer ≥ 0 | 0-indexed sequence number. |
| `period` | integer ≥ 1 | Period in which the possession began. |
| `start_t_game` | number ≥ 0 | Game clock at possession start. |
| `end_t_game` | number ≥ 0 | Game clock at possession termination. |
| `offensive_team_id` | string | Team that had the ball at possession start. |
| `mode` | enum `TRANSITION` \| `HALFCOURT` | Mode selected by `mode_select` (T17). |
| `start_reason` | string | `JUMP_BALL_TAP`, `AFTER_MAKE`, `AFTER_DREB`, `AFTER_STEAL`, `AFTER_LIVE_BALL_TURNOVER_RECOVER`, `AFTER_OREB_CONTINUE`, etc. (see `docs/foundation/possession-plays.md`). |
| `end_reason` | string | `MAKE`, `MISS_DREB`, `MISS_OREB_CONTINUE`, `TURNOVER`, `SHOOTING_FOUL`, `NON_SHOOTING_FOUL_DEAD`, `VIOLATION`, `SHOT_CLOCK_VIOLATION`, `PERIOD_END`, etc. |

The pace regression (T21) consumes `possession_log` to compute per-team mean possession length, pace (possessions per 48 min), and transition share.

**OREB continuation audit:** the engine MAY log a single `possessionEntry` spanning an offensive rebound or two linked entries (`end_reason = MISS_OREB_CONTINUE` followed by `start_reason = AFTER_OREB_CONTINUE`). Both representations are schema-valid. The OREB-continuation contract from `docs/foundation/possession-plays.md` (Metis G1.8 — no rebind, shot clock resets to 14) applies to engine state regardless of audit-log representation.

---

## Box Score Fields

Decision **Metis G3.6**: the box score is the **canonical traditional NBA box score**, exactly these fields and no others.

### `PlayerBox` — exact field set

Each entry in `box_score.home[]` and `box_score.away[]` has exactly these keys, no more, no less:

| # | Field | Type | Description |
|---|---|---|---|
| 1 | `playerId` | string | Stable player identifier; matches `id` from `config/identity.json` (T7). |
| 2 | `jersey` | string | Jersey number/string worn in this game. |
| 3 | `teamId` | string | Team identifier the player appeared for. |
| 4 | `points` | integer ≥ 0 | Total points scored. |
| 5 | `fgm` | integer ≥ 0 | Field goals made (includes 3PM). |
| 6 | `fga` | integer ≥ 0 | Field goals attempted (includes 3PA). |
| 7 | `tpm` | integer ≥ 0 | 3-point field goals made. |
| 8 | `tpa` | integer ≥ 0 | 3-point field goals attempted. |
| 9 | `ftm` | integer ≥ 0 | Free throws made. |
| 10 | `fta` | integer ≥ 0 | Free throws attempted. |
| 11 | `oreb` | integer ≥ 0 | Offensive rebounds. |
| 12 | `dreb` | integer ≥ 0 | Defensive rebounds. |
| 13 | `ast` | integer ≥ 0 | Assists. |
| 14 | `tov` | integer ≥ 0 | Turnovers committed. |
| 15 | `pf` | integer ≥ 0 | Personal fouls committed. |
| 16 | `stl` | integer ≥ 0 | Steals. |
| 17 | `blk` | integer ≥ 0 | Blocks. |
| 18 | `minutes` | number ≥ 0 | Time played (engine convention documented in T19; `number` to permit fractional minutes). |

**Exact-set enforcement.** The schema pins this set three ways simultaneously:

1. `required` lists all 18 field names — every key MUST be present.
2. `properties` declares all 18 field names — every key MUST have a valid type.
3. `additionalProperties: false` rejects any key not in the 18-field list.
4. `minProperties: 18` and `maxProperties: 18` round-trip enforce the count.

The canonical invariant `points == 2*fgm + 3*tpm + ftm` and `fga >= tpm` (and similar) are guaranteed by the engine at event-fold time, not enforced by this schema (the schema permits the engine to populate the fields independently; the linter in T9 may add runtime invariant checks).

---

## No Advanced Stats

**Decision Metis G3.6.** Advanced derived statistics are OUT of the foundation. The following (and any similar derived metrics) MUST NOT appear in `PlayerBox` or anywhere else in the `GameResult`:

| Stat | Reason excluded |
|---|---|
| `TS%` (True Shooting %) | Derived: `points / (2 * TSA)` where `TSA = FGA + 0.44 * FTA`. Consumer can compute from the canonical set. |
| `eFG%` (Effective FG %) | Derived: `(FGM + 0.5 * TPM) / FGA`. |
| `USG%` (Usage Rate) | Derived from FGA, FTA, TOV, and team-mate minutes. |
| `PER` (Player Efficiency Rating) | Composite derived rating; out of scope for the homogeneous phase. |
| `BPM`, `OBPM`, `DBPM` (Box Plus/Minus) | Derived from play-by-play and box; out of scope. |
| `VORP` | Derived from BPM and minutes. |
| `Win Shares` | Derived from offensive/defensive ratings. |
| `DD2`, `TD3` (double-doubles, triple-doubles) | Counting stat; trivially derivable from the canonical set. |
| `+/-` (plus/minus) | Requires per-possession on/off tracking; out of scope for v0.1.0. |

Any consumer that needs these stats MUST compute them post-hoc from the canonical 18-field `PlayerBox`. Adding an advanced-stat field to `PlayerBox` is a major `foundation_version` bump and a violation of the homogeneous model (T22 property gate).

`PlayerBox` excludes `(x, y)` coordinate fields — the box score carries no spatial data. Court zone is the only spatial signal on the box score and lives on individual events (e.g. `SHOT_RELEASE.zone`).

Coordinates do exist elsewhere in the foundation: simulation poses, `WorldSnapshot`, and the spectator layer all use metric `(x, y)` from the court geometry (`config/court-geometry.json`). The zone label on an event is derived from those coordinates via `zoneFromPoint(x, y)`, but the box score itself remains coordinate-free.

---

## Determinism & Golden Fixture

Two `simulateGame` calls with the same `seed`, same `lineups`, and same `foundation_version` MUST produce byte-identical:

1. `events[].type` sequence.
2. `events[].t_game` sequence (the timeline clock stamps).
3. Final score (`box_score.home[].points` summed, etc.).

The golden seed fixture (T20) is committed under `fixtures/golden-seed-42.json` and re-validated on every test run. A diff in the golden output is a contract break and requires either a deliberate tuning change (separate commit `fix(pace): ...` after T21) or a `foundation_version` bump.

Wall-clock timestamps (`started_at`), `Date.now()`, `Math.random()`, and `performance.now()` are FORBIDDEN on kernel paths (decision: single threaded RNG, M3). The audit may record `started_at` for human readability but the engine MUST NOT consume it.

---

## Cross-references

- **T2** — `config/event-catalog.json` defines every `EventType` that may appear in `events[]`.
- **T6** — `docs/foundation/clocks-duration.md` defines the `t_game` quantization rule.
- **T7** — `docs/foundation/identity-roles.md` defines `playerId`, `jersey`, `teamId` and the on-court 5.
- **T8** — This document (timeline + box + possession_log shape).
- **T9** — `scripts/check-foundation.mjs` linter cross-references `events[].type` against the catalog.
- **T10** — `config/demo-game.json` provides the canonical `home_team_id` / `away_team_id` and the 10-jersey rosters used in default runs.
- **T19** — `simulateGame` implementation; emits events, populates the box score by folding events, and writes the possession log.
- **T20** — Golden seed fixture (`fixtures/golden-seed-42.json`) and determinism test.
- **T21** — Pace regression consumes `possession_log` to assert the M8 bands.
- **T22** — Homogeneous property gate asserts that swapping jerseys in a `usageProfile` does not change aggregate outputs.
