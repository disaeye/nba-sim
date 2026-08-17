# NBA Sim Foundation

`foundation_version: 0.5.0` · `ruleset: nba-subset-v0`

## Foundation Authority

The documents under `docs/foundation/`, together with the JSON files under `config/`, constitute the **sole** authority for the engine's behavior. Every runtime event, FSM transition, score change, clock advance, and possession outcome must be derivable from this contract. Any code that contradicts these docs and config is a bug — not the docs and config. All later waves (T2 onward) must extend and obey this contract; the kernel in `src/` is a black-box realization of it.

## Semver Policy

The `foundation_version` stamp in `config/foundation.json` is the canonical semver version of this contract. Bumps are governed by the following semver policy, and **any new EventType, phase, or CourtZone requires a foundation_version bump**.

| Bump | Triggers | Examples |
|---|---|---|
| `patch` | Documentation fix only — wording, typos, clarifications, examples. No contract change. | README typo, references updated, formatting. |
| `minor` | Additive changes that are **backward compatible**. New event types, new phases, new zones, new optional config fields with defaults. Existing callers continue to work unchanged. | New `EventType`, new phase, new `CourtZone`, additive field to an existing event payload (only if old callers reject unknown fields is the bump `major` instead). |
| `major` | Breaking changes to the closed catalog, FSM transitions, score schema, time/clock schema, identity model, or any field consumed by the kernel. Old callers must change. | Renaming or removing a field, changing a transition, schema reshape, RNG draw-order change. |

Adding a new `EventType`, phase, or `CourtZone` without bumping `foundation_version` is a contract violation.

## Index

Foundation docs and configs (all created across T2–T10):

- [architecture.md](architecture.md) — single-axis architecture contract (target)
- [events.md](events.md) — event catalog (T2)
- [rng.md](rng.md) — RNG contract (T3)
- [invariants.md](invariants.md) — invariants + AND-ONE (T4)
- [fsm.md](fsm.md) — FSM phases and transitions (T5)
- [clocks-duration.md](clocks-duration.md) — clocks and duration model (T6)
- [identity-roles.md](identity-roles.md) — identities, roles, usage profiles, court zones (T7)
- [possession-plays.md](possession-plays.md) — possession and plays model (T8)
- [resolve.md](resolve.md) — resolve engine (T8)
- [timeline-io.md](timeline-io.md) — timeline I/O (T8)

Config artifacts:
- [fsm.json](../../config/fsm.json) — FSM transition table
- [event-catalog.json](../../config/event-catalog.json) — event catalog
- [duration.json](../../config/duration.json) — duration distribution definitions
- [plays.json](../../config/plays.json) — play definitions
- [resolve.json](../../config/resolve.json) — resolve engine parameters
- [court-zones.json](../../config/court-zones.json) — court zone definitions
- [rng.json](../../config/rng.json) — RNG parameters
- [demo-game.json](../../config/demo-game.json) — demo game setup (two teams)
- [generated-roster.json](../../config/generated-roster.json) — demo roster generated from the playerdata generator (§7), seed 7

Player-data layer (outside the kernel contract — the kernel consumes it through the capability bridge):
- [playerdata-design.md](../playerdata-design.md) — 球员数据/角色/成长/战术/生成/体力/士气, v1.3a, sole authority for `src/playerdata/`
