import type { RoleBinding } from '../identity/types.js';
import playSlotsJson from '../../config/play-slots.json' with { type: 'json' };
import type { RelationKind } from './relations.js';

export type OffenseSlotName =
  | 'primary_creator'
  | 'secondary_creator'
  | 'screener'
  | 'spacer_strong'
  | 'spacer_weak';

type OffenseMap = Record<OffenseSlotName, RelationKind>;

interface PlaySlotsFile {
  readonly default_halfcourt: OffenseMap;
  readonly plays: Readonly<Record<string, Partial<OffenseMap>>>;
  readonly defense_order: readonly RelationKind[];
}

const CFG = playSlotsJson as PlaySlotsFile;

const OFFENSE_KEYS: readonly OffenseSlotName[] = [
  'primary_creator',
  'secondary_creator',
  'screener',
  'spacer_strong',
  'spacer_weak',
] as const;

export function offenseRelationsForPlay(playId: string | null | undefined): OffenseMap {
  const base = { ...CFG.default_halfcourt };
  if (!playId) return base;
  const override = CFG.plays[playId];
  if (!override) return base;
  return { ...base, ...override };
}

export function jerseyToOffenseRelation(
  binding: RoleBinding,
  playId: string | null | undefined,
  jersey: string,
): RelationKind | null {
  const map = offenseRelationsForPlay(playId);
  for (const key of OFFENSE_KEYS) {
    if (binding[key] === jersey) return map[key];
  }
  return null;
}

export function defenseRelationAt(index: number): RelationKind {
  const order = CFG.defense_order;
  return order[index % order.length] ?? 'defend_deny';
}

