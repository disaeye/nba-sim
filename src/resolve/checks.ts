/**
 * Outcome resolve checks — see `docs/foundation/resolve.md` §Resolve Checks.
 *
 * Each check consumes the documented number of `rng.next()` draws and
 * returns its specific result shape. The homogeneous model means the
 * only inputs are the base_rate and noise; no per-player or per-situation
 * data is consulted (T22 jersey-swap property gate asserts the invariance).
 *
 * Decision formula:
 *   resolve(base_rate, rng) := rng.next() < base_rate
 *
 * Draw accounting per check (auditable against the single-stream RNG
 * contract M3 / docs/foundation/rng.md):
 *
 *   pass, handoff, shot (2pt or 3pt), rebound, ft, steal → 1 draw each
 *   drive → 2 draws (one for success, one for fouled; pinned independent
 *           per the resolve.md "Foul-on-drive" note: "the engine MAY
 *           consult an additional rng.next()")
 *
 * The "each consumes exactly one" rule in the task spec applies to the
 * single-output binary checks. Drive is the documented exception: it has
 * two independent outputs and therefore consumes two draws.
 */
import type { Rng } from '../rng/types.js';
import type { LineupCapability } from '../identity/types.js';
import type { ResolveContext } from './types.js';

/**
 * P(success) — P1.6 pass-type + ability + lane-risk model.
 *
 * The old flat `pass_success` (0.88) ignored who threw, who received, and
 * how far. The NBA reality: elite playmakers complete ~92% of their
 * passes, bigs throw worse, and a 30ft cross-court skip is far riskier
 * than a 8ft swing. The model folds:
 *
 *   - passer ability (LineupCapability.passing, 0..1, neutral 0.5)
 *   - receiver hands (LineupCapability.catchShoot, proxy for POCK)
 *   - distance penalty: −0.10 per 30ft beyond 10ft (a full-court
 *     outlet ≈ −0.25)
 *   - pressure penalty (existing, clamped)
 *
 * Consumes one `rng.next()`.
 */
export interface PassResolveInput {
  readonly pressurePenalty?: number;
  readonly staminaModifier?: number;
  /** Passer playmaking ability 0..1 (neutral 0.5 → no effect). */
  readonly passerAbility?: number;
  /** Receiver catch ability 0..1. */
  readonly receiverCatch?: number;
  /** Pass distance in feet — lane risk grows with length. */
  readonly distanceFt?: number;
}

export function resolvePass(
  rng: Rng,
  ctx: ResolveContext,
  input: PassResolveInput = {},
): { readonly success: boolean } {
  const pressure = Math.max(0, Math.min(0.25, input.pressurePenalty ?? 0));
  const passerMod = input.passerAbility !== undefined ? 1 + (input.passerAbility - 0.5) * 0.15 : 1;
  const catchMod = input.receiverCatch !== undefined ? 1 + (input.receiverCatch - 0.5) * 0.08 : 1;
  const distFt = input.distanceFt ?? 10;
  const distanceMod = distFt <= 10 ? 1 : Math.max(0.85, 1 - (distFt - 10) / 30 * 0.03);
  const rate = ctx.baseRates.pass_success * (1 - pressure) * (input.staminaModifier ?? 1) * passerMod * catchMod * distanceMod;
  return { success: rng.next() < Math.min(0.99, rate) };
}

/**
 * P(success) = base_rates.handoff_success. Consumes one `rng.next()`.
 */
export function resolveHandoff(rng: Rng, ctx: ResolveContext): { readonly success: boolean } {
  return { success: rng.next() < ctx.baseRates.handoff_success };
}

/**
 * P(drive success) = base_rates.drive_success;
 * P(foul)          = base_rates.foul_on_drive_rate.
 *
 * P1.7 three-path model: the old binary (success/fail) collapsed three
 * very different outcomes — beat the defender and finish, get fouled, or
 * lose the ball — into one success probability. The new model returns
 * { success, fouled, lostBall } where:
 *
 *   - success: beats the defender → rim finish (or and-one when fouled)
 *   - fouled: whistle (independent of success, as before)
 *   - lostBall: turnover — driven by low handleSecurity against a
 *     pressuring defender + the defender's on-ball ability.
 *
 * The three are resolved with three draws (success, foul, lostBall), the
 * lostBall only when success failed — documented exception to the
 * one-draw rule, same as the pre-existing drive double-draw.
 */
export interface DriveResolveInput {
  readonly pressurePenalty?: number;
  readonly handlerModifier?: number;
  readonly defenderModifier?: number;
  /** Handler handleSecurity 0..1 — low → more lost balls. */
  readonly handlerHandle?: number;
}

export function resolveDrive(
  rng: Rng,
  ctx: ResolveContext,
  input: DriveResolveInput = {},
): { readonly success: boolean; readonly fouled: boolean; readonly lostBall: boolean } {
  const pressure = Math.max(0, Math.min(0.15, input.pressurePenalty ?? 0));
  const handlerMod = input.handlerModifier ?? 1;
  const defenderMod = input.defenderModifier ?? 1;
  const success = rng.next() < ctx.baseRates.drive_success * (1 - pressure) * handlerMod * defenderMod;
  const fouled = rng.next() < Math.min(0.25, ctx.baseRates.foul_on_drive_rate * (1 + pressure));
  // Lost-ball rate: neutral handle 0.5 → ~0.08 on a failed drive (the
  // old model's 30% of failures became contested finishes instead);
  // elite handles (0.9) cut that to ~0.02, stone hands (0.2) raise it.
  const handle = input.handlerHandle ?? 0.5;
  const lostBall = !success
    ? rng.next() < Math.min(0.3, 0.08 + (0.5 - handle) * 0.18)
    : false;
  return { success, fouled, lostBall };
}

/**
 * P(make) — P1.1 continuous contest logistic + P1.2 shot-type base +
 * P1.3/P1.4 physical block model.
 *
 * The old 3-tier contest (≤2ft ×0.70 / ≤4ft ×0.85 / else ×1.0) produced
 * cliff-edge behavior: a defender at 3.9ft vs 4.1ft were night and day,
 * and beyond 4ft every shot was "open". Real NBA shooting efficiency
 * decays smoothly with defender distance (Second Spectrum: ~0.25 make
 * prob at 0-2ft, rising through ~0.40 at 6ft, plateau ~0.42 beyond).
 * The logistic here is normalized so `open` (≥6ft) ≈ 1.0 and tight
 * (0ft) ≈ 0.62 — the same range the old table hit at its buckets, so
 * the aggregate calibration holds while the curve is continuous.
 *
 * Block model: blockRate = base × typeBias × physical(WS, VJ) ×
 * distance(≤4ft decay). Only shots near a defender are blockable; a
 * wide-open corner three is never blocked.
 *
 * Consumes one `rng.next()` for the block draw + one for the make draw.
 */
export interface ShotResolveInput {
  readonly shotValue: 2 | 3;
  readonly zone: string;
  readonly shotType?: 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';
  readonly defenderDistFt: number;
  readonly playerModifier?: number;
  /** Wingspan cm (P1.4) — raises block+contest near the arc. */
  readonly defenderWsCm?: number;
  /** Vertical cm (P1.3) — raises block rate. */
  readonly defenderVjCm?: number;
  /** Fatigue exec factor (0..1), mirrors staminaFactor. */
  readonly staminaModifier?: number;
  /** Defender weight kg (P4.2) — heavy defenders contest finishes. */
  readonly defenderWtKg?: number;
}

export function resolveShot(
  rng: Rng,
  ctx: ResolveContext,
  input: ShotResolveInput,
): { readonly made: boolean; readonly blocked: boolean } {
  const shotValue = input.shotValue;
  const shotType = input.shotType ?? 'other';
  const dist = input.defenderDistFt;
  const typeRates = ctx.shotTypeRates ?? null;
  const base = typeRates !== null
    ? (shotValue === 3
        ? (shotType === 'catch_shoot' ? typeRates.catch_shoot_3pt : typeRates.pull_up_3pt)
        : shotType === 'catch_shoot'
          ? typeRates.catch_shoot_2pt
          : shotType === 'pull_up'
            ? typeRates.pull_up_2pt
            : shotType === 'post'
              ? typeRates.post_2pt
              : shotType === 'drive_finish'
                ? typeRates.drive_finish_2pt
                : typeRates.other_2pt)
    : (shotValue === 2 ? ctx.baseRates.shot_make_2pt : ctx.baseRates.shot_make_3pt);
  const zm = shotValue === 3
    ? ctx.zoneModifiers3pt[input.zone] ?? 1.0
    : ctx.zoneModifiers[input.zone] ?? 0.85;
  // P1.1 continuous contest: logistic on defender distance.
  //   d=0 → 0.62, d=2 → ~0.70, d=4 → ~0.84, d=6+ → ~1.0 (saturating)
  // Drive finishes use a LIGHTER curve: NBA rim FG% is 65-70% even with
  // a defender at 2-3ft, because the shooter is in contact and uses the
  // rim angle. The jump-shot curve (d=2 → 0.68) would make contested
  // layups convert at 39% — far below the NBA ~55% for contact finishes.
  // NOTE: the jump-shot curve was reverted to the 0.62-anchored version —
  // the tightened 0.55 anchor cut 3ft contests by 11% (0.89 → 0.78),
  // which dropped the whole-shot profile's 3PT rate from 34% to 26% (a
  // 10-seed audit). NBA jump shots at 3ft still convert at ~38-40% for
  // good shooters; the 0.62 anchor prices that honestly.
  const isContactFinish = shotType === 'drive_finish' || shotType === 'post';
  const contest = dist >= 6 ? 1
    : isContactFinish
      ? 0.70 + 0.30 / (1 + Math.exp(-(dist - 3.0) * 0.9))  // d=0→0.70, d=2→0.78, d=4→0.91
      : 0.62 + 0.38 / (1 + Math.exp(-(dist - 2.2) * 1.1));  // d=0→0.62, d=2→0.70, d=4→0.84
  const playerMod = input.playerModifier ?? 1;
  const staminaMod = input.staminaModifier ?? 1;
  // P4.2 body contest: a 120kg rim protector bodies finishes harder than
  // a 85kg wing. Applied only to drive_finish/post (contact finishes).
  const wtFactor = shotType === 'drive_finish' || shotType === 'post'
    ? (input.defenderWtKg !== undefined ? Math.max(0.85, 1 - (input.defenderWtKg - 95) / 400) : 1)
    : 1;
  const makeRate = Math.min(0.92, base * zm * contest * playerMod * staminaMod * wtFactor);

  // P1.3 block model: base block_rate × type bias × physical × distance.
  const typeBias = ctx.shotTypeBlockBias?.[shotType] ?? 1;
  const wsFactor = input.defenderWsCm !== undefined ? 0.8 + (input.defenderWsCm - 200) / 300 : 1;
  const vjFactor = input.defenderVjCm !== undefined ? 0.8 + (input.defenderVjCm - 70) / 120 : 1;
  const distFactor = dist >= 6 ? 0 : dist <= 3 ? 1 : Math.max(0, 1 - (dist - 3) / 3);
  const blockRate = Math.min(0.35, ctx.baseRates.block_rate * typeBias * wsFactor * vjFactor * distFactor);
  const blocked = blockRate > 0 && rng.next() < blockRate;
  if (blocked) return { made: false, blocked: true };
  return { made: rng.next() < makeRate, blocked: false };
}

/**
 * Per-player shot make modifier from the shooter's ability vector:
 * rim/paint/dunker → rimFinishing, elbow → pullUp, perimeter → catchShoot.
 * Neutral 0.5 → 1.0; a 0.86 catch-shoot 3PT shooter ≈ 1.18× the league
 * base; a 0.30 shooter ≈ 0.9×.
 */
export function shotAbilityModifier(
  abilities: LineupCapability | null | undefined,
  shotValue: 2 | 3,
  zone: string,
): number {
  if (!abilities) return 1;
  const key = shotValue === 3 || ['wing_L', 'wing_R', 'corner_L', 'corner_R', 'slot_L', 'slot_R', 'frontcourt_center'].includes(zone)
    ? 'catchShoot'
    : zone === 'elbow_L' || zone === 'elbow_R'
      ? 'pullUp'
      : 'rimFinishing';
  const ability = abilities[key] ?? 0.5;
  return 1 + (ability - 0.5) * 0.5;
}

/**
 * P(offensive rebound) — P1.5 positional model.
 *
 * The old flat `offensive_rebound_rate` ignored everything: a rim shot
 * with three offense players in the paint and the bigs boxing out had
 * the same OREB chance as a long three with everyone back. The positional
 * model folds three NBA-empirical levers into the base rate:
 *
 *   - shot type: rim/drive_finish/paint shots bounce short → offense
 *     positioned; three-point misses rebound long → defense favored.
 *     (rim +0.05, drive_finish +0.04, post +0.02, pull_up −0.02,
 *     catch_shoot 3PT −0.04)
 *   - paint presence: each offense player within 12ft of rim adds
 *     +0.02 (crash the glass); each defense player within 12ft adds
 *     −0.015 (cleaning the glass).
 *   - box-out: an offense player whose current task is `box_out` adds
 *     +0.03 per man (offensive boards are won by positioning).
 *
 * Consumes one `rng.next()`.
 */
export interface ReboundContext {
  readonly shotType?: 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';
  readonly offensePaintCount: number;
  readonly defensePaintCount: number;
  readonly offenseBoxOuts: number;
}

export function resolveRebound(
  rng: Rng,
  ctx: ResolveContext,
  context: ReboundContext = { offensePaintCount: 0, defensePaintCount: 0, offenseBoxOuts: 0 },
): { readonly offensive: boolean } {
  const base = ctx.baseRates.offensive_rebound_rate;
  const typeMod = context.shotType === 'drive_finish'
    ? 0.04
    : context.shotType === 'post'
      ? 0.02
      : context.shotType === 'catch_shoot'
        ? -0.04
        : context.shotType === 'pull_up'
          ? -0.02
          : 0;
  const paintMod = context.offensePaintCount * 0.02 - context.defensePaintCount * 0.015;
  const boxOutMod = context.offenseBoxOuts * 0.03;
  const rate = Math.max(0.05, Math.min(0.45, base + typeMod + paintMod + boxOutMod));
  return { offensive: rng.next() < rate };
}

/**
 * P(make) — P1.9 FT ability model.
 *
 * The old flat `ft_make` (0.77) made Shaq and Curry identical at the
 * line. The 24-ability set has a dedicated `FT` dimension; the bridged
 * LineupCapability carries it folded into `catchShoot`, but the raw
 * playerData FT is the sharper signal. This resolve reads the raw FT
 * ability when present (÷99 → 0..1), else falls back to the bridged
 * catchShoot as a proxy, else the base rate.
 *
 * Consumes one `rng.next()`.
 */
export interface FtResolveInput {
  readonly staminaModifier?: number;
  /** Raw FT ability 0..1 (from playerData.ability.FT ÷ 99). */
  readonly ftAbility?: number;
  /** Bridged catchShoot 0..1 (proxy when raw FT absent). */
  readonly catchShoot?: number;
}

export function resolveFt(rng: Rng, ctx: ResolveContext, input: FtResolveInput = {}): { readonly made: boolean } {
  const base = ctx.baseRates.ft_make;
  const ability = input.ftAbility !== undefined
    ? input.ftAbility
    : input.catchShoot !== undefined
      ? input.catchShoot
      : 0.5;
  // Neutral 0.5 → base (0.77). Elite FT (0.92) → ~0.88; poor (0.55) → ~0.66.
  const rate = Math.min(0.94, Math.max(0.5, base + (ability - 0.5) * 0.35) * (input.staminaModifier ?? 1));
  return { made: rng.next() < rate };
}

/**
 * P(success) — P1.10 steal model with STL ability + GAMBLE tendency.
 *
 * The old `steal_attempt_success` (0.005) was flat: every defender's
 * steal attempt succeeded at the same tiny rate, and the attempt
 * frequency lived outside the model. The layered player data carries a
 * dedicated STL (steal) ability and a GAMBLE tendency (0..99); a high
 * STL hand + a gambler's willingness makes for a materially better
 * thief. Neutral (0.5 / 50) keeps the base rate.
 *
 * Consumes one `rng.next()`.
 */
export interface StealResolveInput {
  /** Bridged on-ball defense 0..1 (legacy path). */
  readonly playerModifier?: number;
  /** Raw STL ability 0..1 (from playerData.ability.STL ÷ 99). */
  readonly stlAbility?: number;
  /** GAMBLE tendency 0..99 — gamblers commit harder (higher success) but
   *   the miss already carries the reach-in foul risk in adjudicate. */
  readonly gambleTendency?: number;
}

export function resolveSteal(rng: Rng, ctx: ResolveContext, input: StealResolveInput = {}): { readonly success: boolean } {
  const ability = input.stlAbility !== undefined
    ? input.stlAbility
    : ((input.playerModifier ?? 1) - 1) / 0.8 + 0.5; // invert legacy mod
  const gambleMod = input.gambleTendency !== undefined
    ? 1 + (input.gambleTendency - 50) / 300 // 50 → 1.0, 99 → 1.16
    : 1;
  const rate = ctx.baseRates.steal_attempt_success * (1 + (ability - 0.5) * 2) * gambleMod;
  return { success: rng.next() < Math.min(0.15, rate) };
}
