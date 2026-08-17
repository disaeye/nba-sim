/**
 * Identity types — LineupPackage, RoleBinding, matchup map, and the typed
 * errors raised by `bindRoles` / `retargetOnBall`.
 *
 * The runtime authority for the player/role model is
 * `docs/foundation/identity-roles.md`; the JSON authority for the lineup
 * shape is `config/schemas/lineup-package.schema.json`. If this file ever
 * disagrees with either, the docs + schema win.
 */
// ─── closed-set unions ──────────────────────────────────────────────────────

/**
 * The five offense role slots a play declares. Frozen at v0.1.0; adding a
 * slot is a `foundation_version` major bump. See
 * `docs/foundation/identity-roles.md` §Role Slots.
 */
export type OffenseSlot =
  | 'primary_creator'
  | 'secondary_creator'
  | 'screener'
  | 'spacer_strong'
  | 'spacer_weak';

/**
 * The five defense live slots. `on_ball` is special-cased by the live
 * retarget rule (it always tracks `ball.holderId`); the other four are
 * computed once per possession and held stable until the next bind.
 */
export type DefenseSlot =
  | 'on_ball'
  | 'deny'
  | 'help'
  | 'tag'
  | 'weak_side';

// ─── lineup package ─────────────────────────────────────────────────────────

/**
 * Priority-ordered role lists consumed by `bindRoles` at possession start.
 * Every jersey in any list MUST also appear in `LineupPackage.players`
 * (enforced by `scripts/check-foundation.mjs` since JSON Schema draft-07
 * cannot express cross-array subset constraints portably).
 */
export interface UsageProfile {
  readonly creator: readonly string[];
  readonly screener: readonly string[];
  readonly spacer: readonly string[];
}

// ─── lineup roles and capabilities ─────────────────────────────────────────

/**
 * Functional role carried by a player in one five-man unit. These are
 * lineup responsibilities, not fixed basketball positions: the same jersey
 * may carry different responsibilities in another LineupPackage.
 */
export type LineupRole =
  | 'initiator'
  | 'connector'
  | 'movement_shooter'
  | 'spot_up_shooter'
  | 'roll_big'
  | 'pop_big'
  | 'post_hub'
  | 'rim_runner'
  | 'point_of_attack'
  | 'rim_protector'
  | 'roamer';

/** Capability vector used by the tactical system, normalized to 0..1. */
export interface LineupCapability {
  readonly creation: number;
  readonly pullUp: number;
  readonly catchShoot: number;
  readonly rimFinishing: number;
  readonly passing: number;
  readonly screening: number;
  readonly rolling: number;
  readonly popping: number;
  readonly postPlay: number;
  readonly cutting: number;
  readonly handleSecurity: number;
  readonly transition: number;
  /** On-ball pressure: deny tightness, contest quality, steal activity. */
  readonly onBallDefense: number;
  /** Help/weak-side positioning: sag discipline, recover speed, rim protection. */
  readonly helpDefense: number;
}

/** A player's package-local responsibility and execution profile. */
export interface LineupRoleAssignment {
  readonly jersey: string;
  readonly roles: readonly LineupRole[];
  readonly capabilities: LineupCapability;
}

/** Optional tactical identity for one lineup package. */
export interface LineupIdentity {
  readonly primary: LineupRole;
  readonly assignments: readonly LineupRoleAssignment[];
  readonly preferredSystems?: readonly string[];
}

/**
 * The five on-court jerseys for one team plus that unit's preferred role
 * priority. Role priority lives on the package, not on the player.
 */
export interface LineupPackage {
  readonly id: string;
  /** Exactly five unique jersey strings. Order has no semantics. */
  readonly players: readonly string[];
  readonly usageProfile: UsageProfile;
  /** Package-local functional roles; absent packages use the legacy profile. */
  readonly lineupIdentity?: LineupIdentity;
}

// ─── role binding ───────────────────────────────────────────────────────────

/**
 * Maps each play's offense slot to a jersey for one possession. Computed
 * by `bindRoles(package, play)` and consumed by the play runner + resolver
 * for the lifetime of the possession. Lives only as long as that
 * possession; a new possession triggers a fresh bind.
 */
export interface RoleBinding {
  readonly primary_creator: string;
  readonly secondary_creator: string;
  readonly screener: string;
  readonly spacer_strong: string;
  readonly spacer_weak: string;
}

// ─── play contract ──────────────────────────────────────────────────────────

/**
 * The subset of `config/plays.json` needed by `bindRoles`: the play's id
 * and its declared offense slots. All five v0.1.0 plays declare the same
 * five slots; the contract is parameterized so a future minor bump can
 * add a play that omits `screener` or `secondary_creator`.
 */
export interface PlayContract {
  readonly id: string;
  readonly slots: readonly OffenseSlot[];
}

// ─── matchup map ────────────────────────────────────────────────────────────

/**
 * holderId to defenderId, 1:1 by lineup index for Phase 1. Built at
 * possession start and rebuilt (or confirmed) whenever the on_ball slot
 * must retarget. The runtime construct owned by the possession engine;
 * `GameState` does NOT carry a matchup map.
 */
export type MatchupMap = Readonly<Record<string, string>>;

// ─── typed errors ───────────────────────────────────────────────────────────

/**
 * Discriminator for identity errors. Stable strings let callers branch
 * on `err.code` rather than parsing the message.
 */
export type IdentityErrorCode =
  | 'IDENTITY_SLOT_NOT_FILLED'
  | 'IDENTITY_DUPLICATE_JERSEY'
  | 'IDENTITY_JERSEY_NOT_IN_PACKAGE'
  | 'IDENTITY_NO_HOLDER'
  | 'IDENTITY_NO_DEFENDER'
  | 'IDENTITY_NO_POSSESSION'
  | 'IDENTITY_PACKAGE_NOT_FOUND'
  | 'IDENTITY_LINEUP_LENGTH_MISMATCH';

/**
 * Typed error raised by `bindRoles` / `getOnBallDefender` /
 * `retargetOnBall` when an invariant is violated. Carries a stable
 * `code` discriminator so callers can branch without parsing prose.
 */
export class IdentityError extends Error {
  readonly code: IdentityErrorCode;

  constructor(code: IdentityErrorCode, message: string) {
    super(message);
    this.name = 'IdentityError';
    this.code = code;
  }
}
