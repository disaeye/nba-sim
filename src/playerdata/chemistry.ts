/**
 * 化学反应 (§2.4) — decision-tree branch-probability modifiers (±10%), NOT
 * numeric buffs. Every synergy/clash pair is data; the lookup returns active
 * effects for a five-man lineup pair of role lists.
 */
import { CHEMISTRY_CLASH, CHEMISTRY_SYNERGY, DUPLICATE_CLASSES } from './tables.js';
import type { ChemistryEffect, DefenseRoleId, OffenseRoleId, RoleId } from './types.js';

function countRoles(roles: readonly RoleId[]): Map<RoleId, number> {
  const counts = new Map<RoleId, number>();
  for (const role of roles) {
    counts.set(role, (counts.get(role) ?? 0) + 1);
  }
  return counts;
}

/**
 * Active chemistry effects for one lineup's role assignment.
 *
 * `offense` = the five offense-role slots, `defense` = the five defense-role
 * slots. Returns:
 * - explicit synergy pairs (8 groups),
 * - explicit clash pairs (7 groups),
 * - duplicate-class rules (终结/发起/防守赌博 repeats) not already covered
 *   by an explicit same-role entry (持球核心×持球核心 and 无球射手≥3 have
 *   explicit entries).
 */
export function chemistryEffects(
  offense: readonly OffenseRoleId[],
  defense: readonly DefenseRoleId[],
): readonly ChemistryEffect[] {
  const roles: RoleId[] = [...offense, ...defense];
  const counts = countRoles(roles);
  const effects: ChemistryEffect[] = [];
  const seen = new Set<string>();

  const push = (e: ChemistryEffect): void => {
    if (seen.has(e.id)) return;
    seen.add(e.id);
    effects.push(e);
  };

  // Explicit pair rules.
  for (const pair of [...CHEMISTRY_SYNERGY, ...CHEMISTRY_CLASH]) {
    const [a, b] = pair.roles;
    if (a === undefined) continue;
    if (b === undefined) {
      // Count-based rule (FS×3).
      if ((counts.get(a) ?? 0) >= DUPLICATE_CLASSES.spacerCount) {
        push({ id: pair.id, roles: [a], channel: pair.channel, modifier: pair.modifier, note: pair.note });
      }
      continue;
    }
    if (a === b) {
      if ((counts.get(a) ?? 0) >= 2) {
        push({ id: pair.id, roles: [a, b], channel: pair.channel, modifier: pair.modifier, note: pair.note });
      }
      continue;
    }
    if ((counts.get(a) ?? 0) >= 1 && (counts.get(b) ?? 0) >= 1) {
      push({ id: pair.id, roles: [a, b], channel: pair.channel, modifier: pair.modifier, note: pair.note });
    }
  }

  // Duplicate-class general rules (§2.4): same-role repeats not already
  // covered by an explicit entry (primary_creator has PC×PC; floor_spacer
  // has FS×3).
  const explicitSameRole = new Set<RoleId>(['primary_creator', 'floor_spacer']);
  const classRules: ReadonlyArray<{ roles: readonly RoleId[]; channel: ChemistryEffect['channel']; modifier: number; note: string }> = [
    { roles: DUPLICATE_CLASSES.initiator, channel: 'initiation', modifier: 0, note: '发起类重复 → 球权冲突（§2.5 压缩）' },
    { roles: DUPLICATE_CLASSES.finishing, channel: 'paint_clog', modifier: 0.9, note: '终结类重复 → 油漆区拥堵 −10%' },
    { roles: DUPLICATE_CLASSES.gambling, channel: 'vacuum_penalty', modifier: 1.1, note: '防守赌博类重复 → 防线真空 +10%' },
  ];
  for (const rule of classRules) {
    for (const role of rule.roles) {
      const n = counts.get(role) ?? 0;
      if (n >= 2 && !explicitSameRole.has(role)) {
        push({ id: `DUP:${role}`, roles: [role], channel: rule.channel, modifier: rule.modifier, note: rule.note });
      }
    }
  }

  return effects;
}

/**
 * Whether a lineup has any possession-conflict (compress-kind) chemistry —
 * feeds the §2.2 two-stage allocation. Duplicate initiator-class roles
 * count too.
 */
export function possessionConflicts(offense: readonly OffenseRoleId[]): ReadonlyArray<{ a: number; b: number }> {
  const conflicts: Array<{ a: number; b: number }> = [];
  // Explicit compress pairs.
  for (const pair of CHEMISTRY_CLASH) {
    if (pair.kind !== 'compress') continue;
    const [ra, rb] = pair.roles as readonly OffenseRoleId[];
    if (ra === undefined || rb === undefined) continue;
    const ia = offense.indexOf(ra);
    const ib = offense.indexOf(rb);
    if (ia === -1 || ib === -1) continue;
    if (ia !== ib) {
      conflicts.push({ a: Math.min(ia, ib), b: Math.max(ia, ib) });
    } else if (ia === ib && ia !== -1) {
      // Same role twice requires two distinct players (PC×PC with two PCs).
      const second = offense.indexOf(ra, ia + 1);
      if (second !== -1) conflicts.push({ a: ia, b: second });
    }
  }
  // Duplicate initiator-class roles (ISO×ISO, HUB×HUB).
  const counts = new Map<OffenseRoleId, number>();
  for (const role of offense) counts.set(role, (counts.get(role) ?? 0) + 1);
  for (const role of DUPLICATE_CLASSES.initiator) {
    const n = counts.get(role as OffenseRoleId) ?? 0;
    if (n >= 2 && role !== 'primary_creator') {
      const first = offense.indexOf(role as OffenseRoleId);
      const second = offense.indexOf(role as OffenseRoleId, first + 1);
      if (first !== -1 && second !== -1) {
        conflicts.push({ a: Math.min(first, second), b: Math.max(first, second) });
      }
    }
  }
  // Dedup by sorted index pair.
  const seen = new Set<string>();
  return conflicts.filter((c) => {
    const key = `${c.a}:${c.b}`;
    if (seen.has(key)) return false;
    seen.add(key);
    return true;
  });
}
