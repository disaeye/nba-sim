/**
 * Pins §2.2/§2.5 possession budget: nominal shares, conflict compression by
 * Fit, 92% renormalization, and the all-low-demand dispersion case.
 */
import { describe, it, expect } from 'vitest';

import { allocatePossessionShares, totalActualShare } from '../../src/playerdata/possession.js';
import { OFFENSE_ROLES, POSSESSION } from '../../src/playerdata/tables.js';
import type { OffenseRoleId } from '../../src/playerdata/types.js';

const B = (roles: OffenseRoleId[], fits: number[]) =>
  roles.map((role, i) => ({ role, fit: fits[i] ?? 60 }));

describe('allocatePossessionShares (§2.2)', () => {
  it('no conflicts → nominal shares renormalized to 0.92', () => {
    // primary_creator 30 + secondary_creator 22 + floor_spacer 13 + movement_shooter 12 + cutter 12 = 89
    const shares = allocatePossessionShares(B(
      ['primary_creator', 'secondary_creator', 'floor_spacer', 'movement_shooter', 'cutter'],
      [90, 80, 70, 60, 50],
    ));
    expect(totalActualShare(shares)).toBeCloseTo(0.92, 9);
    expect(shares[0]!.actual).toBeCloseTo((30 / 89) * 0.92, 9);
    expect(shares[4]!.actual).toBeCloseTo((12 / 89) * 0.92, 9);
    for (const s of shares) expect(s.compressed).toBe(false);
  });

  it('PC×PC conflict compresses the lower-Fit creator', () => {
    const shares = allocatePossessionShares(B(
      ['primary_creator', 'primary_creator', 'floor_spacer', 'movement_shooter', 'cutter'],
      [90, 55, 70, 60, 50],
    ));
    expect(totalActualShare(shares)).toBeCloseTo(0.92, 9);
    // The fit-55 creator is compressed (×0.8), the fit-90 one is not.
    expect(shares[1]!.compressed).toBe(true);
    expect(shares[0]!.compressed).toBe(false);
    expect(shares[1]!.actual).toBeLessThan(shares[1]!.nominal * 0.92);
    // Higher-fit creator's actual share exceeds its nominal share.
    expect(shares[0]!.actual).toBeGreaterThan(shares[0]!.nominal * 0.92);
  });

  it('processes conflicts in ascending lower-Fit order', () => {
    // Two conflicts: PC×PC (players 0,1) and PC×ISO (players 0,2).
    const shares = allocatePossessionShares(B(
      ['primary_creator', 'primary_creator', 'iso_scorer', 'floor_spacer', 'cutter'],
      [90, 80, 50, 60, 40],
    ));
    // Player 2 (fit 50) is the lowest-Fit member of PC×ISO → compressed.
    // Player 1 (fit 80) is the lower-Fit member of PC×PC → compressed.
    expect(shares[2]!.compressed).toBe(true);
    expect(shares[1]!.compressed).toBe(true);
    expect(shares[0]!.compressed).toBe(false);
    expect(totalActualShare(shares)).toBeCloseTo(0.92, 9);
  });

  it('all-low-demand lineup disperses possession ("全民皆兵")', () => {
    const shares = allocatePossessionShares(B(
      ['transition_runner', 'transition_runner', 'transition_runner', 'transition_runner', 'transition_runner'],
      [60, 60, 60, 60, 60],
    ));
    expect(totalActualShare(shares)).toBeCloseTo(0.92, 9);
    for (const s of shares) {
      expect(s.actual).toBeCloseTo(0.92 / 5, 9);
      expect(s.compressed).toBe(false);
    }
  });

  it('empty input → empty output', () => {
    expect(allocatePossessionShares([])).toEqual([]);
  });

  it('demand values match the §2.2 table', () => {
    expect(OFFENSE_ROLES.primary_creator.demand).toBe(30);
    expect(OFFENSE_ROLES.iso_scorer.demand).toBe(26);
    expect(OFFENSE_ROLES.secondary_creator.demand).toBe(22);
    expect(OFFENSE_ROLES.hub.demand).toBe(20);
    expect(OFFENSE_ROLES.post_scorer.demand).toBe(18);
    expect(OFFENSE_ROLES.interior_finisher.demand).toBe(15);
    expect(OFFENSE_ROLES.floor_spacer.demand).toBe(13);
    expect(OFFENSE_ROLES.cutter.demand).toBe(12);
    expect(OFFENSE_ROLES.movement_shooter.demand).toBe(12);
    expect(OFFENSE_ROLES.transition_runner.demand).toBe(10);
  });

  it('uses the documented 0.92 reserve and 0.8 compression step', () => {
    expect(POSSESSION.totalShare).toBeCloseTo(0.92, 9);
    expect(POSSESSION.compressionStep).toBeCloseTo(0.8, 9);
    expect(POSSESSION.vacancyEfficiency).toBeCloseTo(0.85, 9);
    expect(POSSESSION.mismatchEfficiency).toBeCloseTo(0.8, 9);
  });
});
