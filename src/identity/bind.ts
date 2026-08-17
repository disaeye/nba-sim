/**
 * bindRoles — maps a LineupPackage's usageProfile onto a play's offense
 * slots for one possession. See `docs/foundation/identity-roles.md`
 * §RoleBinding and `docs/foundation/possession-plays.md` §Bind Algorithm.
 *
 * Algorithm (frozen at v0.1.0):
 *   1. primary_creator    ← first creator not yet assigned
 *   2. secondary_creator  ← creators[1] ?? creators[0]
 *   3. screener           ← first screener not yet assigned
 *   4. spacer_strong      ← first spacer not yet assigned
 *   5. spacer_weak        ← next spacer not yet assigned
 *   6. fill any unassigned slot from the remaining players
 *   7. validate: all 5 filled, no jersey in 2 slots, all ∈ package.players
 *
 * Step 2 deliberately does NOT consult the assigned set — it is a literal
 * "second creator, or first if only one". When only one creator exists,
 * step 2 produces a duplicate and step 7 throws IDENTITY_DUPLICATE_JERSEY,
 * surfacing the incompatibility between the package and a play that needs
 * two distinct creators. Every demo-game package has two creators, so the
 * happy path never trips this.
 *
 * Pure: returns a new RoleBinding; never mutates `pkg` or `play`.
 */
import type { LineupPackage, PlayContract, RoleBinding } from './types.js';
import { IdentityError } from './types.js';

/** Canonical slot order — drives iteration in step 6 and validation. */
const SLOT_ORDER = [
  'primary_creator',
  'secondary_creator',
  'screener',
  'spacer_strong',
  'spacer_weak',
] as const;

/**
 * Bind `pkg.usageProfile` onto `play.slots`, returning a fresh
 * `RoleBinding`. Throws `IdentityError` (with a stable `code`) on any
 * invariant violation.
 */
export function bindRoles(pkg: LineupPackage, play: PlayContract): RoleBinding {
  void play; // play.slots is the contract for which slots exist; v0.1.0 plays all use the same 5.

  const slots: Partial<Record<(typeof SLOT_ORDER)[number], string>> = {};
  const assigned = new Set<string>();
  const { creator, screener, spacer } = pkg.usageProfile;

  // Step 1: primary_creator ← first creator not yet assigned.
  for (const jersey of creator) {
    if (!assigned.has(jersey)) {
      slots.primary_creator = jersey;
      assigned.add(jersey);
      break;
    }
  }

  // Step 2: secondary_creator ← creators[1] ?? creators[0].
  // (Literal spec — no assigned check. Validation catches the duplicate
  // when only one creator exists; this is how a package/play mismatch
  // surfaces rather than silently degrading.)
  const secondaryCandidate = creator[1] ?? creator[0];
  if (secondaryCandidate !== undefined) {
    slots.secondary_creator = secondaryCandidate;
    assigned.add(secondaryCandidate);
  }

  // Step 3: screener ← first screener not yet assigned.
  takeFirstAvailable(screener, assigned, (j) => { slots.screener = j; });

  // Step 4 + 5: spacer_strong and spacer_weak from the spacer list.
  takeFirstAvailable(spacer, assigned, (j) => { slots.spacer_strong = j; });
  takeFirstAvailable(spacer, assigned, (j) => { slots.spacer_weak = j; });

  // Step 6: fill any unassigned slot from the remaining players.
  const remaining = pkg.players.filter((p) => !assigned.has(p));
  let remIdx = 0;
  for (const slot of SLOT_ORDER) {
    if (slots[slot] === undefined) {
      const next = remaining[remIdx];
      if (next === undefined) {
        // noUncheckedIndexedAccess: remaining[remIdx] is string | undefined.
        // Throwing here is the named "not enough players" scenario, not
        // defensive bloat.
        throw new IdentityError(
          'IDENTITY_SLOT_NOT_FILLED',
          `bindRoles: cannot fill slot "${slot}" — package "${pkg.id}" has only ${pkg.players.length} players and not enough unassigned to cover all 5 slots`,
        );
      }
      slots[slot] = next;
      assigned.add(next);
      remIdx += 1;
    }
  }

  // Step 7: validate (safety net for hand-crafted inputs).
  return validateAndBuild(slots, pkg);
}

/**
 * Iterate `list`, invoking `onPick` for the first element not in
 * `assigned`, then adding it to `assigned`. No-op when the list is
 * exhausted. Used by steps 3–5 to share the "first eligible" rule.
 */
function takeFirstAvailable(
  list: readonly string[],
  assigned: Set<string>,
  onPick: (jersey: string) => void,
): void {
  for (const jersey of list) {
    if (!assigned.has(jersey)) {
      onPick(jersey);
      assigned.add(jersey);
      return;
    }
  }
}

/**
 * Resolve the partial slots into a complete `RoleBinding`, throwing on
 * any invariant violation. Reads each slot via `requireSlot` so the
 * `string | undefined` from `Partial<...>` is narrowed by runtime check
 * rather than `!`.
 */
function validateAndBuild(
  slots: Partial<Record<(typeof SLOT_ORDER)[number], string>>,
  pkg: LineupPackage,
): RoleBinding {
  const primaryCreator = requireSlot(slots, 'primary_creator', pkg);
  const secondaryCreator = requireSlot(slots, 'secondary_creator', pkg);
  const screener = requireSlot(slots, 'screener', pkg);
  const spacerStrong = requireSlot(slots, 'spacer_strong', pkg);
  const spacerWeak = requireSlot(slots, 'spacer_weak', pkg);

  const binding: RoleBinding = {
    primary_creator: primaryCreator,
    secondary_creator: secondaryCreator,
    screener,
    spacer_strong: spacerStrong,
    spacer_weak: spacerWeak,
  };

  // No jersey in two slots.
  const seen = new Set<string>();
  for (const slot of SLOT_ORDER) {
    const jersey = binding[slot];
    if (seen.has(jersey)) {
      throw new IdentityError(
        'IDENTITY_DUPLICATE_JERSEY',
        `bindRoles: jersey "${jersey}" appears in multiple slots for package "${pkg.id}"`,
      );
    }
    seen.add(jersey);
  }

  // Every bound jersey must be a member of package.players.
  for (const slot of SLOT_ORDER) {
    const jersey = binding[slot];
    if (!pkg.players.includes(jersey)) {
      throw new IdentityError(
        'IDENTITY_JERSEY_NOT_IN_PACKAGE',
        `bindRoles: jersey "${jersey}" in slot "${slot}" is not in package "${pkg.id}" players [${pkg.players.join(', ')}]`,
      );
    }
  }

  return binding;
}

/** Narrow a partial slot to `string`, throwing the named error if undefined. */
function requireSlot(
  slots: Partial<Record<(typeof SLOT_ORDER)[number], string>>,
  slot: (typeof SLOT_ORDER)[number],
  pkg: LineupPackage,
): string {
  const value = slots[slot];
  if (value === undefined) {
    throw new IdentityError(
      'IDENTITY_SLOT_NOT_FILLED',
      `bindRoles: slot "${slot}" is unfilled for package "${pkg.id}"`,
    );
  }
  return value;
}
