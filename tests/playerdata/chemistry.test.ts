/**
 * Pins §2.4 chemistry: all 8 synergy pairs, all 7 clash pairs, duplicate
 * class rules, and the spacer ≥3 structural rule.
 */
import { describe, it, expect } from 'vitest';

import { chemistryEffects, possessionConflicts } from '../../src/playerdata/chemistry.js';
import { CHEMISTRY_SYNERGY, CHEMISTRY_CLASH, DUPLICATE_CLASSES } from '../../src/playerdata/tables.js';
import type { DefenseRoleId, OffenseRoleId } from '../../src/playerdata/types.js';

const O = (roles: OffenseRoleId[]): OffenseRoleId[] => roles;
const D = (roles: DefenseRoleId[]): DefenseRoleId[] => roles;

describe('catalog completeness (§2.4)', () => {
  it('enumerates 8 synergy groups', () => {
    expect(CHEMISTRY_SYNERGY).toHaveLength(8);
  });

  it('enumerates 7 clash groups', () => {
    expect(CHEMISTRY_CLASH).toHaveLength(7);
  });
});

describe('chemistryEffects — synergy', () => {
  it('持球核心 × 无球射手: help_recovery −10%', () => {
    const eff = chemistryEffects(O(['primary_creator', 'floor_spacer', 'cutter', 'movement_shooter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    const e = eff.find((x) => x.id === 'PC×FS');
    expect(e?.channel).toBe('help_recovery');
    expect(e?.modifier).toBeCloseTo(0.9, 9);
  });

  it('持球核心 × 空切终结: dime_cut +10%', () => {
    const eff = chemistryEffects(O(['primary_creator', 'cutter', 'floor_spacer', 'movement_shooter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'PC×CUT')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('副攻手 × 持球核心: relief_target +10%', () => {
    const eff = chemistryEffects(O(['secondary_creator', 'primary_creator', 'floor_spacer', 'cutter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'SC×PC')?.channel).toBe('relief_target');
  });

  it('组织枢纽 × 空切终结: cut_target +10%', () => {
    const eff = chemistryEffects(O(['hub', 'cutter', 'primary_creator', 'floor_spacer', 'movement_shooter']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'HUB×CUT')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('组织枢纽 × 跑动射手: move_catch +10%', () => {
    const eff = chemistryEffects(O(['hub', 'movement_shooter', 'primary_creator', 'floor_spacer', 'cutter']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'HUB×MS')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('单打得分手 × 无球射手: iso_space +10%', () => {
    const eff = chemistryEffects(O(['iso_scorer', 'floor_spacer', 'primary_creator', 'cutter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'ISO×FS')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('背身轴心 × 跑动射手: spacer_open +10%', () => {
    const eff = chemistryEffects(O(['post_scorer', 'movement_shooter', 'primary_creator', 'floor_spacer', 'cutter']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'PS×MS')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('盯人箭头 × 护筐锚: recovery_penalty −10%', () => {
    const eff = chemistryEffects(O(['primary_creator', 'floor_spacer', 'cutter', 'movement_shooter', 'hub']), D(['poa_stopper', 'rim_anchor', 'switch_wing', 'rotator', 'post_defender']));
    expect(eff.find((x) => x.id === 'POA×RA')?.modifier).toBeCloseTo(0.9, 9);
  });
});

describe('chemistryEffects — clash', () => {
  it('持球核心 × 持球核心 is a compress conflict', () => {
    const eff = chemistryEffects(O(['primary_creator', 'primary_creator', 'floor_spacer', 'cutter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    const e = eff.find((x) => x.id === 'PC×PC');
    expect(e).toBeDefined();
  });

  it('持球核心 × 单打得分手 is a compress conflict', () => {
    const eff = chemistryEffects(O(['primary_creator', 'iso_scorer', 'floor_spacer', 'cutter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'PC×ISO')).toBeDefined();
  });

  it('持球核心 × 组织枢纽: initiation −10% each', () => {
    const eff = chemistryEffects(O(['primary_creator', 'hub', 'floor_spacer', 'cutter', 'movement_shooter']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    const e = eff.find((x) => x.id === 'PC×HUB');
    expect(e?.channel).toBe('initiation');
    expect(e?.modifier).toBeCloseTo(0.9, 9);
  });

  it('空切终结 × 内线终结: paint_clog −10%', () => {
    const eff = chemistryEffects(O(['cutter', 'interior_finisher', 'primary_creator', 'floor_spacer', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'CUT×IF')?.modifier).toBeCloseTo(0.9, 9);
  });

  it('空切终结 × 背身轴心: paint_clog −10%', () => {
    const eff = chemistryEffects(O(['cutter', 'post_scorer', 'primary_creator', 'floor_spacer', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'CUT×PS')?.modifier).toBeCloseTo(0.9, 9);
  });

  it('纠缠者 × 协防扫荡: vacuum_penalty +10%', () => {
    const eff = chemistryEffects(O(['primary_creator', 'floor_spacer', 'cutter', 'movement_shooter', 'hub']), D(['pest', 'roamer', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'PEST×ROAM')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('无球射手 ≥3: initiation_eff 0.85 (single emission)', () => {
    const eff = chemistryEffects(O(['primary_creator', 'floor_spacer', 'floor_spacer', 'floor_spacer', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    const fs = eff.filter((x) => x.id === 'FS×3');
    expect(fs).toHaveLength(1);
    expect(fs[0]!.modifier).toBeCloseTo(0.85, 9);
  });
});

describe('chemistryEffects — duplicate class rules', () => {
  it('终结类重复 (内线×2) → paint_clog', () => {
    const eff = chemistryEffects(O(['primary_creator', 'interior_finisher', 'interior_finisher', 'floor_spacer', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'DUP:interior_finisher')?.channel).toBe('paint_clog');
  });

  it('发起类重复 (ISO×2) → compress conflict', () => {
    const eff = chemistryEffects(O(['iso_scorer', 'iso_scorer', 'floor_spacer', 'cutter', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'DUP:iso_scorer')).toBeDefined();
    expect(possessionConflicts(O(['iso_scorer', 'iso_scorer', 'floor_spacer', 'cutter', 'hub'])).length).toBeGreaterThan(0);
  });

  it('防守赌博类重复 (纠缠者×2) → vacuum_penalty', () => {
    const eff = chemistryEffects(O(['primary_creator', 'floor_spacer', 'cutter', 'movement_shooter', 'hub']), D(['pest', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    expect(eff.find((x) => x.id === 'DUP:pest')?.modifier).toBeCloseTo(1.1, 9);
  });

  it('does not double-emit explicit same-role rules (PC, FS)', () => {
    // Two PC → one explicit PC×PC; two spacers → no FS×3 at all.
    const eff = chemistryEffects(O(['primary_creator', 'primary_creator', 'floor_spacer', 'floor_spacer', 'hub']), D(['poa_stopper', 'pest', 'switch_wing', 'rotator', 'rim_anchor']));
    const pc = eff.filter((x) => x.id === 'PC×PC');
    const fs = eff.filter((x) => x.id === 'FS×3');
    const dup = eff.filter((x) => x.id.startsWith('DUP:'));
    expect(pc).toHaveLength(1);
    expect(fs).toHaveLength(0);
    // PC and FS are covered by explicit entries — no duplicate-class rules fire.
    expect(dup).toHaveLength(0);
  });
});

describe('possessionConflicts', () => {
  it('detects PC×PC and PC×ISO but not neutral pairs', () => {
    expect(possessionConflicts(O(['primary_creator', 'primary_creator', 'floor_spacer', 'cutter', 'hub']))).toHaveLength(1);
    expect(possessionConflicts(O(['primary_creator', 'iso_scorer', 'floor_spacer', 'cutter', 'hub']))).toHaveLength(1);
    expect(possessionConflicts(O(['primary_creator', 'secondary_creator', 'floor_spacer', 'cutter', 'hub']))).toHaveLength(0);
  });

  it('uses the class tables from §2.4', () => {
    expect(DUPLICATE_CLASSES.initiator).toContain('primary_creator');
    expect(DUPLICATE_CLASSES.finishing).toEqual(['cutter', 'interior_finisher', 'post_scorer']);
    expect(DUPLICATE_CLASSES.gambling).toEqual(['pest', 'roamer']);
    expect(DUPLICATE_CLASSES.spacerCount).toBe(3);
  });
});
