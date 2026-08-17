/**
 * 球权两段式分配 (§2.2) — possession budget for one five-man unit.
 *
 * Stage 1: nominal share = demand / Σdemand (per-role demand values).
 * Stage 2: conflict compression — possession conflicts (initiator-class
 * duplicates + compress-kind chemistry) are resolved by Fit: the lower-Fit
 * member of each conflict gets share ×= 0.8 (⌛), processed in ascending
 * lower-Fit order.
 * Final: renormalized to 0.92 — the 8% reserve is the buffer for random
 * events and vacancy penalties.
 */
import { possessionConflicts } from './chemistry.js';
import { OFFENSE_ROLES, POSSESSION } from './tables.js';
import type { OffenseRoleId, PossessionShare } from './types.js';

export interface BudgetPlayer {
  readonly role: OffenseRoleId;
  readonly fit: number;
}

/**
 * Allocate possession shares for a five-man offense-role assignment.
 * Returns one entry per input player, in input order.
 */
export function allocatePossessionShares(players: readonly BudgetPlayer[]): readonly PossessionShare[] {
  if (players.length === 0) return [];

  // Stage 1: nominal shares from demand values.
  const demands = players.map((p) => OFFENSE_ROLES[p.role].demand);
  const totalDemand = demands.reduce((a, b) => a + b, 0);
  if (totalDemand <= 0) {
    throw new Error('allocatePossessionShares: zero total demand');
  }
  const nominal = demands.map((d) => d / totalDemand);
  const shares = [...nominal];
  const compressed = players.map(() => false);

  // Stage 2: conflict compression, ascending by lower-Fit member.
  const conflicts = possessionConflicts(players.map((p) => p.role))
    .map((c) => ({ ...c, keyFit: Math.min(players[c.a]!.fit, players[c.b]!.fit) }))
    .sort((x, y) => x.keyFit - y.keyFit);
  for (const conflict of conflicts) {
    const a = players[conflict.a]!;
    const b = players[conflict.b]!;
    // Fit 低者回调（ties → lower index, stable).
    const victim = a.fit <= b.fit ? conflict.a : conflict.b;
    shares[victim] = shares[victim]! * POSSESSION.compressionStep;
    compressed[victim] = true;
  }

  // Final: renormalize to 0.92 (8% reserve).
  const sum = shares.reduce((a, b) => a + b, 0);
  const scale = POSSESSION.totalShare / sum;
  return players.map((p, i) => ({
    index: i,
    role: p.role,
    fit: p.fit,
    nominal: nominal[i]!,
    actual: shares[i]! * scale,
    compressed: compressed[i]!,
  }));
}

/** Sum of actual shares — always 0.92 after allocation. */
export function totalActualShare(shares: readonly PossessionShare[]): number {
  return shares.reduce((a, s) => a + s.actual, 0);
}
