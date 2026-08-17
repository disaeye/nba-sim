/**
 * Free-throw sequence emitter — see `docs/foundation/invariants.md`
 * §AND-ONE Rule and `docs/foundation/resolve.md` §FT Taxonomy.
 *
 * Emits the canonical event chain:
 *
 *   FT_START → (FT_ATTEMPT → FT_RESULT)+ → FT_SEQUENCE_END
 *
 * Each FT_RESULT consumes exactly one `rng.next()` via `resolveFt` (the
 * homogeneous `ft_make` rate). The FSM driver (T15) handles phase
 * transitions on these events; this function ONLY produces the event
 * sequence — it does not touch GameState, does not classify the foul,
 * does not decide who shoots. Those concerns belong to the foul
 * classifier in T19's possession engine.
 *
 * AND-ONE interaction (T4 invariants §AND-ONE):
 *   - andOne=true  → 1 FT (the field-goal make already counted via
 *     SHOT_RESULT; one extra FT is awarded).
 *   - andOne=false → 2 FTs (2pt shooting foul or bonus non-shooting)
 *     or 3 FTs (3pt shooting foul).
 *
 * If the caller passes `andOne=true` with `attempts > 1`, the and-one
 * rule overrides: `attempts` is clamped to 1. This matches the
 * foundation's "shot made + shooting foul" classification, which always
 * yields exactly one FT regardless of the shot value.
 *
 * Emitted events carry placeholder envelope fields (`t_game=0`,
 * `seq=local counter`, `clocks`/`score` zeroed). The step loop (T19) is
 * responsible for stamping real `t_game` / `seq` / `clocks` / `score`
 * when these events are appended to the live timeline. Per-catalog
 * `required_payload_fields` are populated for every event.
 */
import type { Rng } from '../rng/types.js';
import type { Event, EventType } from '../state/types.js';
import { resolveFt } from './checks.js';
import type { FtSequenceConfig, ResolveContext } from './types.js';

/**
 * Produce the FT event chain for one shooting sequence.
 *
 * @param ft   shooter + team + attempt count + and-one flag.
 * @param rng  the threaded single-stream RNG; advanced exactly `effectiveAttempts` times.
 * @param ctx  per-call resolve context (consumes only `baseRates.ft_make`).
 * @returns    the ordered event array; seq is locally monotonic (caller renumbers globally).
 */
export function runFtSequence(
  ft: FtSequenceConfig,
  rng: Rng,
  ctx: ResolveContext,
  staminaModifier = 1,
  ftAbility?: number,
  catchShoot?: number,
): Event[] {
  // AND-ONE forces 1 attempt regardless of the `attempts` arg.
  const effectiveAttempts: 1 | 2 | 3 = ft.andOne ? 1 : ft.attempts;

  const events: Event[] = [];

  // FT_START (catalog required payload: shooter_id, team, attempts).
  events.push(
    ftEvent('FT_START', ft.shooterId, {
      shooter_id: ft.shooterId,
      team: ft.team,
      attempts: effectiveAttempts,
      and_one: ft.andOne,
    }),
  );

  // One FT_ATTEMPT + FT_RESULT pair per attempt. resolveFt consumes
  // exactly one rng.next() per call, so the total draw count for this
  // sequence is exactly effectiveAttempts.
  for (let attemptNumber = 1; attemptNumber <= effectiveAttempts; attemptNumber++) {
    events.push(
      ftEvent('FT_ATTEMPT', ft.shooterId, {
        shooter_id: ft.shooterId,
        attempt_number: attemptNumber,
      }),
    );
    const { made } = resolveFt(rng, ctx, { staminaModifier, ftAbility, catchShoot });
    events.push(
      ftEvent('FT_RESULT', ft.shooterId, {
        shooter_id: ft.shooterId,
        attempt_number: attemptNumber,
        made,
      }),
    );
  }

  // FT_SEQUENCE_END (catalog: no required payload; next_phase optional).
  events.push(ftEvent('FT_SEQUENCE_END', null, {}));

  // Assign locally-monotonic seq values so the array is well-ordered.
  // T19's step loop will renumber with globally-unique seq values when
  // appending to the live timeline.
  return events.map((e, i) => ({ ...e, seq: i }));
}

/**
 * Construct one FT event with placeholder envelope fields. The payload
 * is caller-supplied (catalog `required_payload_fields` vary by type);
 * the envelope defaults match the conventions in `tests/state/_helpers.ts`
 * `makeEvent` so a fold over these events via `applyEvent` works
 * out-of-the-box against a fresh `createInitialState`.
 */
function ftEvent(
  type: EventType,
  actorId: string | null,
  payload: Record<string, unknown>,
): Event {
  return {
    type,
    t_game: 0,
    t_real: 0,
    seq: 0,
    actors: actorId === null ? [] : [actorId],
    payload,
    clocks: { game: 0, shot: 0 },
    score: { home: 0, away: 0 },
  };
}


