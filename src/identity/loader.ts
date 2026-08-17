/**
 * Loads LineupPackage definitions from `config/demo-game.json`. The JSON
 * file is the foundation authority — each package is schema-validated by
 * `config/schemas/lineup-package.schema.json` via the foundation linter
 * (`scripts/check-foundation.mjs`). The `as LineupPackage[]` cast is the
 * parse-don't-validate boundary; the linter gate has already run by the
 * time any kernel code imports this module.
 *
 * Static JSON import (not `fs.readFileSync`) so this module stays free
 * of `@types/node` — preserving the T11 pinned devDependency manifest.
 * Pattern matches `src/duration/loader.ts`.
 */
import demoGame from '../../config/demo-game.json' with { type: 'json' };

import type { LineupPackage } from './types.js';
import { IdentityError } from './types.js';

/**
 * All LineupPackages defined for one team in `config/demo-game.json`.
 * Returns a fresh array so callers may mutate without aliasing the
 * parsed JSON module cache.
 */
export function loadLineupPackages(teamId: 'home' | 'away'): LineupPackage[] {
  const team = teamId === 'home' ? demoGame.home_team : demoGame.away_team;
  return team.lineup_packages as LineupPackage[];
}

/**
 * Look up a single LineupPackage by id. Throws the typed
 * `IDENTITY_PACKAGE_NOT_FOUND` error when no package matches.
 */
export function loadLineupPackage(teamId: 'home' | 'away', packageId: string): LineupPackage {
  const packages = loadLineupPackages(teamId);
  const found = packages.find((p) => p.id === packageId);
  if (found === undefined) {
    throw new IdentityError(
      'IDENTITY_PACKAGE_NOT_FOUND',
      `loadLineupPackage: package "${packageId}" not found for team "${teamId}" (available: ${packages.map((p) => p.id).join(', ')})`,
    );
  }
  return found;
}
