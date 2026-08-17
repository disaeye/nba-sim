# NBA Sim

Event-driven NBA game simulator with frozen, machine-checkable foundation contracts. The engine plays a full NBA game (tip-off → live possessions → dead balls → free throws → period breaks → final score) under deterministic rules, with modern-NBA pacing verified across 256 seeds.

: `foundation_version: 0.13.0` · `ruleset: nba-subset-v0`

## Quick Start

```bash
npm run foundation:lint # validate all foundation configs
npm run test:fast # fast regression suite; skips batch/statistical tests
npm run test:slow # fast regressions plus opt-in pace/performance/homogeneity tests
npm run sim -- --seed 42 --config config/demo-game.json  # run one full game
npm run pace:check                                       # 256-seed pace-regression gate
# Spectator shell (观赛壳)
npm run broadcast -- --seed 42 --filter mid              # text live broadcast (CN)
npm run export-ndjson -- --seed 42 --out spectator/game  # slim .json + stream .ticks.ndjson(.gz) (推荐)
npm run spectator:serve                                  # http://0.0.0.0:4173 (all interfaces)

# Shell wrappers (same as above, friendlier CLI)
./scripts/sim.sh 42
./scripts/broadcast.sh --seed 42 --filter high
./scripts/export-game.sh 42
./scripts/watch.sh --seed 42          # export + serve 2D player
./scripts/check-all.sh                # lint + tsc + test + pace
./scripts/check-all.sh --skip-pace    # faster gate

```
## Simulation-vs-reality gap audit

The engine exposes a downstream audit that compares semantic events with continuous world snapshots. It checks causal lifecycle, physical spacing, holder consistency, continuous movement, and defensive response. Every error includes the observed evidence and a likely source-layer root cause; the audit never repairs or hides a violation.

npm run reality:check                         # deterministic seeds 1..32
npm run reality:check -- --seeds 1..256 --json # full sweep
```

The loop is: **simulate → observe events and snapshots → enforce basketball constraints → report the exact seed/sequence/evidence/root-cause hypothesis → reproduce with a focused regression → fix the source layer → rerun the batch**. A clean audit means no error-level gap was observed; warnings remain visible and must be reviewed rather than treated as proof of realism.

Audit implementation: `src/audit/reality-gap.ts`; batch entry point: `scripts/reality-gap-check.ts`; regression coverage: `tests/simulate/reality-gap.test.ts`.

Requirements: Node.js ≥ 20 (see `engines` in `package.json`).

## Architecture

The kernel is a pure event-driven step loop. Each module is a layered concern, and the foundation docs pin every contract one layer below the code. Data flows top-to-bottom:

```
              ┌─────────────────────────────────────────────┐
              │            Foundation (sole authority)        │
              │   docs/foundation/*.md  +  config/*.json      │
              │   + config/schemas/*.schema.json (ajv)        │
              └─────────────────────────────────────────────┘
                                    │ binds (machine-validated)
                                    ▼
   ┌──────────┐   ┌─────────┐   ┌─────────┐   ┌────────┐   ┌──────────┐   ┌─────────────┐
   │   rng    │──▶│ clocks  │──▶│  state  │──▶│ rules  │──▶│ identity │──▶│ possession  │
   │mulberry32│   │ duration│   │ (delta  │   │ (FSM   │   │ (roles / │   │ (mode + play│
   │  single  │   │ sampler │   │  events)│   │  driver│   │  matchup)│   │   runner)   │
   │  stream  │   │ quantize│   │immutable│   │ period │   │  retarget│   │ OREB contin.│
   └──────────┘   └─────────┘   └─────────┘   └────────┘   └──────────┘   └──────┬──────┘
                                                                                  │
                                                                                  ▼
                                                                          ┌─────────────┐
                                                                          │   resolve   │   shot / pass / handoff
                                                                          │ + ft-seq    │   / drive / rebound / ft
                                                                          └──────┬──────┘
                                                                                 │
                                                                                 ▼
                                                                         ┌──────────────┐
                                                                         │   simulate   │   phase loop until POST_GAME
                                                                         │  GameResult  │   events + box + possession_log
                                                                         └──────┬───────┘
                                                                                │
                                                                                ▼
                                                                  CLI / pace:check / tests
```

| Module         | Path                | Responsibility                                                       |
| -------------- | ------------------- | -------------------------------------------------------------------- |
| `rng`          | `src/rng/`          | mulberry32 single-stream RNG + helpers (`unit`, `int`, `pick`, `weighted`) |
| `clocks`       | `src/clocks/`       | Period / shot / game clock types and clamp/tick helpers              |
| `duration`     | `src/duration/`     | Distribution sampler (uniform, trunc_normal) with 0.1s quantization  |
| `state`        | `src/state/`        | Immutable `GameState`, `applyEvent` fold for delta-encoded events    |
| `rules`        | `src/rules/`        | FSM transition table driver + period router + timeout/sub legality   |
| `identity`     | `src/identity/`     | `LineupPackage` → `RoleBinding`, live on-ball retarget, matchup map  |
| `possession`   | `src/possession/`   | Mode select + advisory play pick; OREB continuation helpers |
| `decision`     | `src/decision/`     | **Unified DecisionKernel** — all 10 players share decide() + dynamic context |
| `resolve`      | `src/resolve/`      | Outcome checks (shot/pass/drive/rebound/ft) + FT sequence state machine |
| `simulate`     | `src/simulate.ts`   | Phase-loop: LIVE steps call DecisionKernel (play is advisory only) |
| `sim-utils`    | `src/sim-utils.ts`  | Types, period/sub helpers, GameResult builder, event stamping        |
| `box-score`    | `src/box-score.ts`  | Per-player 18-field box score aggregated from event timeline         |
| `cli`          | `src/cli.ts`        | CLI entry: reads config, runs simulateGame, prints score             |
| `spectator`    | `src/spectator/`    | Render layer: events→keyframes→**0.1s match stream** (no kernel authority) |
| `web shell`    | `spectator/`        | Plays `stream.ticks` @ fixed dt; event log from keyframes             |

**Unified DecisionKernel:** every on-court player runs the same `generateOptions → score → pick` path. Role bias + clocks + score + mode + advisory play id form a **dynamic context** that changes every decision window. Ball vs off-ball only changes the option set, not the code path.

**Draw order per LIVE window** (foundation M3 spirit): mode_select (possession start) → play_select (advisory) → per-agent option noise → duration → resolve.

**Step purity (M2):** `applyEvent(state, event)` returns a NEW state. The input state identity is never mutated; the simulate loop threads `let state = applyEvent(state, e)` through every event.

## Foundation Contract

The documents under [`docs/foundation/`](docs/foundation/) together with the JSON files under [`config/`](config/) are the **sole authority** for the engine's behavior. Every runtime event, FSM transition, score change, clock advance, and possession outcome must be derivable from this contract. Any code that contradicts these docs and config is a bug — not the docs and config.

The contract is machine-validated at three layers:

1. **Schema layer** — every config file validates against its `config/schemas/*.schema.json` via ajv.
2. **Cross-reference layer** — `scripts/check-foundation.mjs` enforces the invariants JSON Schema draft-07 cannot express (catalog membership, duration_id references, phase partitions, version consistency). 21 checks total.
3. **Behavioral layer** — the kernel's tests pin every contract clause the foundation states in prose.

See [`docs/foundation/README.md`](docs/foundation/README.md) for the per-domain index (events, RNG, invariants + AND-ONE, FSM, clocks/duration, identity/roles, possession/plays, resolve, timeline I/O) and the semver bump policy (patch = doc fix; minor = additive; major = breaking).

## Scripts

| Script                    | Purpose                                                                |
| ------------------------- | ---------------------------------------------------------------------- |
| `npm test` / `npm run test:fast` | Run the fast Vitest regression suite. Slow pace, performance, and homogeneity suites are opt-in. |
| `npm run test:slow` | Run all tests with `RUN_SLOW_TESTS=1`, including multi-seed statistics and benchmarks. |
| `npm run test:watch` | Run Vitest in watch mode. |
| `npm run foundation:lint` | Run `scripts/check-foundation.mjs` — schema + cross-ref checks. |
| `npm run pace:check` | Run `scripts/pace-check.ts` — 256-seed pace-regression gate. |
| `npm run reality:check` | Run `scripts/reality-gap-check.ts` — causal/statistical reality scan (default seeds `1..32`). |
| `npm run sim` | Run `src/cli.ts` — single-game simulation. Args: `--seed N --config PATH`. |
| `npx tsx scripts/frame-deep-audit.ts` | Frame-level realism audit (0.1s position truth): speeds/teleports, body overlaps, ball coherence, event-position correspondence, FT-line position, foul ordering, SCV contexts. Args: `--seeds 42,5,7 --out .omo/evidence/frame-deep-audit.json`. |

Examples:

```bash
npm run sim -- --seed 7 --config config/demo-game.json
npm run pace:check -- --seeds 1..16
```

## Testing

`npm run test:fast` is the default regression suite. It runs unit, integration, determinism, rendering, and causal/spatial audit tests; the opt-in statistical and benchmark files are skipped. `npm run test:slow` sets `RUN_SLOW_TESTS=1` and adds the pace subset, performance benchmarks, and homogeneous-usage permutation suite.

| Rung | Files | Purpose |
| --- | --- | --- |
| **Fast regression** | `tests/{court,clocks,duration,...}/`, `tests/simulate/`, `tests/determinism/`, `tests/render/` | Deterministic behavior, lifecycle, geometry, event and snapshot contracts |
| **Slow statistical** | `tests/pace/`, `tests/properties/` | Multi-seed pace, performance, homogeneity and permutation checks |

**TDD discipline.** Every behavior lands as a failing test first (Given/When/Then), then the minimum code to pass. Reverting a commit must break at least one test.

**Golden fixtures.** `fixtures/golden-seed-{42,7}.json` pin the exact `event_type_sequence`, `final_score`, `event_count`, and `periods` for two representative seeds. Any unintended kernel change fails the golden test; intentional changes require regenerating the committed fixture and bumping `foundation_version` if the contract moved.

**I5 forbidden-source audit.** `tests/determinism/audit.test.ts` scans `src/` for `Math.random`, `Date.now`, `performance.now` — the kernel must be fully deterministic. The property tests under `tests/properties/` legitimately use `performance.now()` to measure wall time; the audit only scans `src/`.

**Pace regression.** `npm run pace:check` runs the engine over the configured seed range (default `1..256`), aggregates per-team mean pace, mean possession length, transition share, FG%, mean score, and OREB%, and exits 1 if any band is violated. Use `npm run pace:check -- --seeds 1..16` for the fast smoke run.

## Current Calibration

Measured by `npx tsx scripts/pace-check.ts --seeds 1..16` (32-game run). Saved to `.omo/evidence/pace-report.json`. The 32-game `npm run reality:check` gate passes all eight statistical bands with zero causal findings.

| Metric                          | Measured | Band (honest)  | Status |
| ------------------------------- | -------- | -------------- | ------ |
| Per-team mean pace              | 102.83   | [97, 103]      | PASS   |
| Mean possession length (s)      | 14.00    | [13.5, 15.5]   | PASS   |
| Transition share                | 13.37%   | [12%, 18%]     | PASS   |
| FG%                             | 42.28%   | [25%, 65%]     | PASS   |
| Mean score per team             | 129.48   | [80, 160]      | PASS   |
| OREB%                           | 26.37%   | [18%, 32%]     | PASS   |
| Shot-clock violation rate       | 0.00%    | [0%, 12%]      | PASS   |
| Turnover rate                   | 10.77%   | [8%, 22%]      | PASS   |

Reference (2023-24 NBA): pace ≈ 99, possession ≈ 14.5s, transition share ≈ 15-17%, FG% ≈ 46%, ~111 pts/team, OREB% ≈ 26%, shot-clock violations < 0.5% of possessions, TOV rate ≈ 12.5%.

**Foundation 0.12.0: hardcore realism round (硬核模拟化).** The full 38-item plan landed: distribution-level measurement first, then resolve, decision, tactic, physical, orchestration and chemistry/morale layers.

- **P0 measurement (度量层)**: `src/audit/distributions.ts` folds every game into NBA-reference distribution bands (shot profile rim/mid/3, assists/FG, FTr, steals, blocks, OREB%, turnover types) wired into `scanReality`; `scripts/calibrate.ts` (deviation-ranked harness), `scripts/play-type-mix.ts` (tactic mix vs NBA reference) and `scripts/profile-validation.ts` (tendency-profile ↔ behavior correlation) form the calibration loop. Baseline findings: 75% threes, 11% rim, -0.51 T3 correlation — quantified the decision-layer drift.
- **P1 resolve (结算层)**: `resolveShot` rebuilt — continuous contest logistic (replaces 3-tier), shot-type taxonomy (`shot_type_rates`: catch_shoot/pull_up/post/drive_finish, wired through Intent → startShot → resolve), block model (WS/VJ/type bias), positional rebounding (paint presence + box-out + shot type), ability/distance-aware passing, three-path drives (beat/foul/lost-ball), FOUL-tendency fouls, FT-ability free throws, STL+GAMBLE steals.
- **P2 data wiring (数据接线)**: raw `playerData` now flows through GameState into resolve/decision — tendency priors in the EV (T3/TMID/TDRIVE/TPOST/PASS1ST/TAKEOVER/PUSH), awareness-gated perception noise (OFFR misreads), coverage reads, score×time situation layer, two-step pass lookahead with chain tax.
- **P3 offense/defense systems (攻防体系)**: coverage catalog DROP/SWITCH/BLITZ/HEDGE/ICE (coach/ability driven), help→X-out rotation chains, PNR coverage reads, 2-3 zone scheme, off-ball read&react (deny→backdoor, sag→stay live), pump-fake option, fit-filtered uniform tactic draw (mix spread across all 8 families).
- **P4 physical (物理层)**: SPD-derived runtime speed overrides, defender weight contest, DUR-scaled stamina burn.
- **P5 orchestration (编排层)**: fouled-out (6) + foul-trouble (4) auto-subs with ball-ownership transfer, momentum-run auto-timeouts, late-game intentional fouls, coach identity (zone/blitz/pace biases).
- **P6 chemistry/morale (化学士气)**: 12 chemistry channels fold into option EVs, morale exec multiplier rides the stamina channel, possession-share-ordered creator priority.
- **Timing structure**: halfcourt possessions time-gated (SETUP ≥10s + 3s EXECUTE cooldown + hard early-shot gate) — possession length restored to ~13-15s with release clocks concentrated in the 8-16s band.
- **Golden fixtures regenerated**; full suite 631 tests green; foundation check 24/24.

**Foundation 0.11.0: verification round (验证驱动修复).** Three real bugs surfaced by running the full verification battery (pace/reality/frame audits + the playerdata-generated roster as a second config):

- **Windup freeze (出手卡死)**: a physical ball action started inside the 0.4s catch window could outlive it, and since action progression (INITIATED → PROGRESSING → release) runs only inside the decision path, the 2.8s anti-hot-potato dwell then blocked the decision loop and froze the release forever. With low-ability rosters this produced 27% shot-clock violations (ball never left the holder's hands); the demo only escaped because its catches had clock to spare. Fix: the decision gate unlocks when the holder's INITIATED windup has elapsed. `scripts/pace-check.ts` / `scripts/reality-gap-check.ts` / `scripts/frame-deep-audit.ts` now accept `--config` so any roster can be verified.
- **Snapshot event loss**: the tick snapshot's `lastEventSeq` never captured events emitted by the main decision pass (only the team-change re-decision did), so the frame audit could locate only 5 of ~200 shot releases. One capture line fixed; shot-release/position checks now run on every attempt (205/205, 0ft error).
- **Jersey collision in the generated roster**: `scripts/generate-roster.ts` had lost its away-team jersey offset, so both teams used 1-10 — the kernel keys poses/abilities by jersey, the collision corrupted pose teams, and every possession whose holder shared a jersey stalled into an SCV (this was the entire 27%-SCV signal, not the bridge calibration). Fixed: home 1-10 / away 11-20.
- **Timing re-calibration**: the freeze fix released ~2.8s per catch-window action, shortening possessions 14.10 → 13.41s (pace 107, out of band). Compensation via real-action levers: shot windup 0.40 → 0.70s, `ball.pass_speed` 0.55 → 0.52, shot flight 0.5-1.0 → 0.8-1.3s (duration.json `pass` entry is dead — pass flight derives from `mobility.ball.pass_speed`). Both the demo (102.5 pace, 14.05s possession) and the playerdata-generated roster (97.3 / 14.80) pass the 32-seed pace + reality gates; the frame audit on seed 42 shows 0 teleports, 0 held-ball mismatches, 0 release-position errors, on-ball gap P50 2.6ft. Golden fixtures regenerated; foundation_version 0.10.0 → 0.11.0.

**Foundation 0.10.0: live possession rhythm (持球人不再站桩).** Two spatial-layer fixes removed the frozen-holder tableau (seed 42 Q1: holders stood still 5-10s per possession while teammates circled within 10ft):

- **Handler probe drift (anti-statue)**: a holder in the organize phase now keeps the ball live — a deterministic 2.5s-bucketed drift around his strike pocket (same bucket scheme as the spacer drift), so the set reads as a real dribble-probe instead of a parked ball. Decision timing is untouched (the EV core still prices shot/drive/pass/hold from the resolve rates).
- **Ball-relative spacing (anti-crowd)**: spacer pockets were rim-relative (22ft) and the handler's strike pocket sits at the same radius, so strong-corner/slot spots landed 5-12ft from the ball and the whole set collapsed into a ring around the holder. Each spacer now gets both side variants and takes the one farther from the ball — spacing is enforced by the ball, not the basket. The old "deny → relocate" escape branches are subsumed by this rule.
- Net effect: holder static streaks ≥4s per Q1 dropped from 7 (max 10.6s) to 2 (max 4.2s) on seed 42; the floor spreads to real wing/corner distances. Score per team rose 119.7 → 130.8 (spacing creates more open looks) — inside the declared band; pace/possession/FG%/TOV all remain in band (verified 32-game reality gate, 16-seed pace sweep). Golden fixtures regenerated; foundation_version 0.9.0 → 0.10.0.
- **Court painting fix (球场线)** — the spectator court now matches the real NBA layout: the corner three-point straight segments were drawn 5.7ft short of the arc (`cornerDist × 0.15` guess stopped at 8.5ft; the 23.75ft arc starts at 14.2ft), leaving an unpainted gap at every corner. `buildCourtDrawSpec` now computes the exact arc join (sqrt(23.75²−22²) = 8.95ft from the rim) and sweeps the arc between the two join angles, so corner lines and arc meet seamlessly. The previously dead `key_hash_marks` config is now real: NBA lane hash marks (3/10/14/19 ft from the baseline, 2ft into the lane) are generated and rendered. `spectator/court-draw-spec.json` is regenerated via `scripts/regen-court-spec.ts`; the demo `spectator/game.json` was re-exported with the 0.10.0 stream.

**Foundation 0.9.0: the expected-value decision core (期望价值决策内核).** The hand-tuned threshold tree is gone. Every possession-level choice — shoot / drive / pass / hold — is now priced with the same rate model resolve uses, and the argmax wins:

- **`decideHandlerAction` (src/decision/expected-value.ts)**: expectedShotPoints / expectedDrivePoints / expectedPassPoints price each option from the resolve base rates, zone modifiers, catch-shoot ability, and the live contest (a closeout defender 3-8ft away is IN FLIGHT and prices as ≤2.5ft). No phase gates, no threshold ladders — an open corner is a shot, a guarded handler organizes, a wide-open teammate earns the pass, and a ≤4s clock forces the look. The only structural constants are *organization value* (a pending screen is worth waiting for; once the screen executes its value is consumed) and *time decay* (organization value fades from 16s of shot clock).
- **Catch-and-shoot window**: a fresh catch prices the look at open for ~0.4s (the closeout is still in flight) — swing passes become real shots instead of the start of an infinite pass chain. This is what makes possessions move the ball and still end on time.
- **Matchup-fit tactics**: halfcourt systems are drawn by capability fit (handler creation vs the defender's on-ball ability, screener gap, rim openness) + live matchup advantages, converted to weights — PNR, ISO, POST, OFF_BALL_SCREEN and HANDOFF all occur, and the mix shifts with the lineup on the floor. `scripts/tactic-mix.ts` measures the mix from the frame-level snapshot stream (the same `tactical.kind` the spectator renders): in a seed-42 game the starter quarters run PNR_ROLL 47% / DRIVE_KICK 30% / PNR_POP 15%, while the rotation quarters shift to PNR_ROLL 58% / DRIVE_KICK 13% + ISO 6% + OFF_BALL_SCREEN 6% + POST_UP 5% — the distribution moves with the matchups, no fixed weight table could produce that.
- **Coverage from rates**: deny/sag now derive from the *expected open shot* (base 3PT × zone × catch-shoot) rather than a hand-set catchShoot threshold; sag gaps settle over the possession.
- **Calibration**: the drive EV was retuned to the resolve finish model (finish at 1.5ft contest), pass success is priced AND resolved at 0.88 (one model, no decision/outcome drift), and the decision throttle gives a committed tactic ~4s of execution time instead of a 0.1s re-roll. Possession length landed at 14.5s with pace 99.6 — the shape emerges from the EV core, not from tuned share tables.
- **Court-geometry unification**: the EV layer used to classify zones with a ring approximation (rim-distance buckets) that could never produce `slot`/`frontcourt_center` and mislabeled elbows; the resolve layer used the authoritative `zoneFromPoint`. The decision core now calls the SAME geometry (`zoneFromPoint`/`shotValueAt` — paint rectangle, corner strips, elbow spots, 23.75ft arc / 22ft corners), so a decision's zone modifier is the one the outcome resolves with. Two structural consequences: ISO now initiates at the ELBOW (18ft mid-range) instead of the 22ft strike zone, and drives price their time-to-rim (~1.5s), so a late-clock drive collapses toward the rushed finish and the mid-range pull-up becomes the correct buzzer choice. The remaining gap is honest: mid-range shots are still ~0% of attempts because the coverage layer never concedes them — a defense that closes out threes AND protects the rim without ever sagging into the mid-range gives the offense no reason to shoot there. That concession is a deliverable of the defense-decision milestone, not a tuned share.
- **Frozen-frames note**: the frame audit's `offenseFrozenShare` (≥4/5 offense players < 1.5ft/s) reads ~0.17 (0.9.0 raised it from 0.05 as the threat model stopped the deny→relocate churn). 0.10.0 made the remaining statuary legible (holder probe drift + ball-relative spacing), and 0.11.0 removed the catch-window windup freeze entirely — holder static streaks ≥4s in Q1 are now at most one short episode, and remaining frozen frames are formation-holds by off-ball spacers (like real halfcourt sets). Off-ball targets still re-derive every tick (drift buckets by game clock); `scripts/diag-freeze.ts` measures holder static streaks for regression checks.

**Foundation 0.8.0: the off-ball game (无球攻防博弈).** Individual talent and a live coverage layer replaced the homogeneous neutral engine:

- **Roster abilities**: every demo player carries a `LineupCapability` overlay (catchShoot, pullUp, rimFinishing, creation, handleSecurity, onBallDefense, helpDefense…). Resolve applies per-player modifiers (a 0.86 catch-shoot spacer shoots ~43% from three, a 0.48 shooter ~33%); drives respect ball security and the defender's on-ball ability; steals scale with the stealer's defense.
- **Coverage game (defense)**: off-ball defenders now decide per tick — `deny` the strong-side shooter (2.3-3.2ft, shading the pass), `sag` the weak-side/low-threat spacer (6.8ft+ help lane), `chase` declared cuts (2.8ft), `box_out` on misses. The decision reads the attacker's catchShoot and the defender's onBallDefense (weak defenders sag deeper). A swing pass flips a deny into a sag in real time — the frame audit measures ~3,600 coverage flips per game.
- **Offense response**: a denied spacer relocates to the opposite pocket (摆脱); a sagged spacer holds and is a live catch-and-shoot target; kick-outs and entries favor the best catch-shoot option; a big who catches away from the rim gives the ball straight back instead of running his own pick-and-roll. Offensive "frozen" share dropped 0.21 → 0.05 of live frames.
- **Calibration**: failed drives are now mostly contested finishes (30% lost-ball turnovers), OREB rate raised to 0.26, inbound receivers are the primary creators (a non-creator takes the ball out), and the period horn truncates in-flight passes. Score per team landed at 111.7 (NBA ~111).

**Foundation 0.7.0 behavioral changes** (regression-pinned by the frame-level audit `scripts/frame-deep-audit.ts` and the causal reality gate):

- **Shot-clock violations eliminated** (11% → 0%): the SETUP→EXECUTE system draw could pick PUSH_PACE, pinning the plan to TRANSITION_PUSH/ADVANCE forever — the handler "advanced" to the frontcourt cap and stood until the horn. Halfcourt draws now exclude PUSH_PACE; a ≤3s forced-shot safety net sits before the stage gate; the dwell anti-hot-potato timer is capped by the remaining shot clock; a missed shot that touches the rim resets the clock to 14s.
- **Free throws at the line**: FT_START was emitted inside the LIVE tick, jumping straight to FT_SEQUENCE and skipping the FT alignment — players shot from wherever they ended the previous play (up to 22ft away). The whistle now fires after the walk to the true FT line (19ft, not the elbow spot).
- **Fouls whistle at contact**: shooting fouls were emitted at rim arrival (1.4s late, with the shooter already relocating) and attributed to `lineup[0]` or the defender nearest the *rim*. The foul now sounds at release with the defender nearest the *release point*; order is FOUL → SHOT_RESULT → MADE_BASKET_DEAD.
- **Three-pointers fixed**: 2PT-calibrated zone modifiers (wing 0.65) were applied to threes, crushing 3PT% to ~21%. A dedicated 3PT zone table (corner +8%) restores ~36%; contest distance is measured from the release point, not the rim.
- **Shot flight by distance**: flight time was a distance-blind 0.55-1.05s draw; a 3PT now takes ~1.4s (real ballistics, 20.7 ft/s horizontal).
- **No more body interpenetration / speed spikes**: the contact solver now reads current positions per pair (Gauss-Seidel) and damps velocity on contact — previously a 20ft/s cutter against a stationary defender produced 30-43ft/s frames and bodies visibly merged (~1000 frames/game < 1ft).
- **Dead-ball walk speeds**: dead-ball repositioning (FT walk, inbound setups) ran at live jog speeds (8-15ft/s). A `dead_ball_speed_scale: 0.45` config makes players walk.
- **Arrival snap capped**: the anti-oscillation snap-to-target could teleport up to 2.8ft in one 0.1s tick (30-31ft/s arrival frames); it now approaches at max speed.
- **Staggered rotation**: the halftime all-ten swap (every player exactly 24min) is replaced by a default 10-man rotation — bench unit closes Q1/Q3 and opens Q2, starters return for the Q2/Q4 stretch and close the game (starters ≈ 27min, bench ≈ 21min). Override via `input.scriptedSubs`.
- **Buzzer-beater resolution**: a legal shot released before the buzzer resolves before the period boundary (basket counts / rebound ends the period).
- **FT possession transfer**: after a made last free throw the other team inbounds; a miss is a live ball at the rim — previously the FT shooter kept the ball and the same team's episode restarted.

**Single-game performance** (measured in `tests/properties/perf.test.ts`): p50 ≈ 2.2s, p95 ≈ 2.6s per game standalone on the reference machine (a 48-minute game simulates ~32k 0.1s ticks with full position snapshots; gate ceiling 3.2s, which the full parallel slow suite can exceed under CPU contention); 256-game pace:check sweep ≈ 1.4s wall (target < 5s).

**Phase-1 homogeneity.** Player abilities are homogeneous — outcomes are driven by role binding (`usageProfile`) and stochastic resolve rates, not by talent differentiation. The property tests in `tests/properties/homogeneity.test.ts` verify the engine responds to ROLE, not JERSEY ID: swapping starter jerseys 1↔5, 2↔4 with correspondingly swapped usage profiles produces bit-identical game outcomes (margin drift = 0%, total-points drift = 0%, event-count drift = 0%).

- **Real-pass inbounds (发底线球)**: after a make the ball no longer teleports into the receiver's hands. The dead-ball branch picks the inbounder CLOSEST to the end line (real basketball: the nearest player throws it in), walks them to the line, holds the ball in their hands for a beat (`INBOUND_START`, `spot_zone: baseline`), then fires a hard short pass (`PASS` flight) that the spectator sees leave the baseline and arrive at the receiver (`INBOUND_TOUCH` → `POSSESSION_GAINED` on the catch). The 24s clock stays frozen during the inbound and starts on the catch; the inbound flight is immune to steals (defenders may not reach over the line); a buzzer-truncated inbound cannot leak into the next period. 97-100% of baseline inbounds have the inbounder actually standing at the end line when the ball leaves.

## Player Data (球员数据体系)

`docs/playerdata-design.md` (v1.3a) is the sole authority for the layered player-data system, implemented in `src/playerdata/`:

- **Five layers**: 体测 Physical (8, public except DUR) → 能力 Ability (24) / 倾向 Tendency (9) / 判断 Awareness (4, hidden) → 属性 Attribute (7, display grades S..F, observed with error). Data flows one-way; player operations touch only the lower layers.
- **观测模型**: `σ = base × k_scout × √(200/n)` for attributes/tendencies/awareness (12/15/10), sample cap 2000, half-life 500 rounds; L5 presents with σ halved. `buildScoutReport` renders the five-block report (§1.7).
- **角色体系 (§2)**: 10 offense + 7 defense roles with 球权需求值, two-stage possession allocation (conflict compression by Fit, 92% normalization), 8 synergy + 7 clash chemistry pairs.
- **Fit (§5)**: `f×g×100` with per-role weight tables and smooth tendency gates; transmission coefficients (possession weight, execution efficiency, deviation penalty); observed Fit for the scout layer.
- **Season systems (§3/§4)**: training points → growth with POT ceilings and age efficiency; age decline curves; injury probability/severity; experience → Awareness; role immersion → Tendency offsets; mismatch friction.
- **生成器 (§7)**: 12 prototypes with league shares, physical generation, POT, tendency sampling with §1.4 validation, draft-pool ecosystem guarantees. Fully deterministic (`mulberry32`).
- **体力 (§8) / 士气 (§9)**: STM gauge + fatigue accumulation; MOR events + output bands — morale never mutates truths.
- **事件流对接 (§10)**: `consume.ts` implements the four player-side queries over the kernel timeline.

**Kernel fusion**: `Player.playerData` is an optional layered-data payload; the kernel derives the `LineupCapability` overlay from it via `bridgeCapabilities` (explicit `abilities` still win per dimension). Without `playerData` the engine behaves exactly as before — the golden fixtures are untouched.

```bash
npm run roster:gen                     # regenerate config/generated-roster.json (seed 7)
npm run sim -- --seed 42 --config config/generated-roster.json   # play a game with a generated roster
```

`tests/playerdata/` pins every formula; `tests/playerdata/integration.test.ts` runs real games on generated rosters (determinism + bridge-consumed proof).

## Spectator shell (观赛壳)

```
simulateGame → GameResult.events     (sparse, authoritative)
       ↓
projectGame  → keyframes             (event-aligned, ALIGNMENT positions)
       ↓
renderMatchStream(dt=0.1) → stream.ticks   (uniform match stream for frontend)
       ↓
spectator/app.js plays ticks @ wall time ≈ dt / speed
```

Kernel stays event-driven. **Render layer** interpolates 10-player positions between `ALIGNMENT` keyframes onto a **0.1s game-clock grid**. Frontend only consumes `stream.ticks`.

| Command | What it does |
| --- | --- |
| `npm run broadcast -- --seed 42` | Chinese play-by-play to stdout |
| `npm run export-ndjson -- --seed 42 --out spectator/game` | Package: events + keyframes + **0.1s stream** (slim JSON + NDJSON ticks, 峰值内存约为旧 export-game 一半) |
| `npm run export-game -- --seed 42 --out spectator/game.json` | 旧版单文件导出（内存峰值 ~900MB，仅兼容旧 shell） |
| `./scripts/watch.sh --seed 42` | Export + serve on `0.0.0.0:4173` |

Web controls: **Space** play/pause · **←/→** step tick · **1–5** speed · load JSON / Load demo path.

## Public API

```typescript
import {
  simulateGame, computeBoxScore, FOUNDATION_VERSION,
  projectGame, buildSpectatorPackage, formatBroadcastLine,
} from 'nba-sim';
import type { GameInput, GameResult, SpectatorFrame, SpectatorPackage } from 'nba-sim';
```

`simulateGame(input: GameInput): GameResult` — kernel entry point.  
`buildSpectatorPackage(result)` — frames + narrated lines for the web shell.
