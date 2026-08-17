import { describe, it, expect } from 'vitest';
import { offenseRelationsForPlay } from '../../src/court/play-slots.js';
import playSlotsJson from '../../config/play-slots.json' with { type: 'json' };

const CFG = playSlotsJson as {
  default_halfcourt: Record<string, string>;
  plays: Record<string, Record<string, string>>;
};

const DEFAULT_HALFCOURT = CFG.default_halfcourt;

// Helper: equality of a result map against a plain object (RelationKind values).
function expectMapEqual(actual: Record<string, string>, expected: Record<string, string>): void {
  expect(Object.keys(actual).sort()).toEqual(Object.keys(expected).sort());
  for (const k of Object.keys(expected)) {
    expect(actual[k]).toBe(expected[k]);
  }
}

describe('offenseRelationsForPlay — config-only lookup', () => {
  describe('known play IDs in config', () => {
    it('returns pnr_high overrides merged over default_halfcourt', () => {
      const result = offenseRelationsForPlay('pnr_high');
      expectMapEqual(result, { ...DEFAULT_HALFCOURT, ...CFG.plays['pnr_high'] });
      // Sanity: the override must actually differ from default for at least one slot,
      // otherwise this test proves nothing (override == default).
      expect(result.primary_creator).toBe('ball_handler_pocket');
      expect(DEFAULT_HALFCOURT.primary_creator).toBe('weak_wing');
    });

    it('returns handoff_wing overrides merged over default_halfcourt', () => {
      const result = offenseRelationsForPlay('handoff_wing');
      expectMapEqual(result, { ...DEFAULT_HALFCOURT, ...CFG.plays['handoff_wing'] });
    });

    it('returns iso_clear overrides merged over default_halfcourt', () => {
      const result = offenseRelationsForPlay('iso_clear');
      expectMapEqual(result, { ...DEFAULT_HALFCOURT, ...CFG.plays['iso_clear'] });
    });

    it('returns transition_push overrides merged over default_halfcourt', () => {
      const result = offenseRelationsForPlay('transition_push');
      expectMapEqual(result, { ...DEFAULT_HALFCOURT, ...CFG.plays['transition_push'] });
    });
  });

  describe('unknown play IDs must NOT trigger substring fallback overlays', () => {
    // Each of these IDs contains a substring ('pnr'/'drag'/'handoff'/'iso'/'transition')
    // that the old includes()-based fallback would have matched. Under config-only
    // lookup they must return ONLY the default_halfcourt mapping — no pnr/handoff/
    // iso/transition overlay applied.
    const unknownIds = [
      'pnr_custom',
      'drag_variant',
      'handoff_variant',
      'iso_variant',
      'transition_variant',
    ];

    for (const id of unknownIds) {
      it(`returns default_halfcourt only for unknown id "${id}"`, () => {
        const result = offenseRelationsForPlay(id);
        expectMapEqual(result, DEFAULT_HALFCOURT);
        // Anti-overlay guards: confirm we did NOT pick up any of the four canonical
        // overlays' distinguishing slot value.
        expect(result.primary_creator).toBe('weak_wing'); // default, not pnr/iso/handoff ball_handler_pocket
      });
    }
  });

  describe('nullish play IDs return default_halfcourt only', () => {
    it('returns default_halfcourt for null playId', () => {
      expectMapEqual(offenseRelationsForPlay(null), DEFAULT_HALFCOURT);
    });

    it('returns default_halfcourt for undefined playId', () => {
      expectMapEqual(offenseRelationsForPlay(undefined), DEFAULT_HALFCOURT);
    });

    it('returns default_halfcourt for empty string playId', () => {
      // Empty string is falsy but truthy-fallback differs from null: must still be default.
      expectMapEqual(offenseRelationsForPlay(''), DEFAULT_HALFCOURT);
    });
  });

  describe('isolation / clone behavior', () => {
    it('returns a fresh object on each call (no shared mutation with default)', () => {
      const a = offenseRelationsForPlay('pnr_high');
      const b = offenseRelationsForPlay(null);
      // Mutating a must not leak into the next call's result.
      (a as Record<string, string>).primary_creator = 'mutated';
      const c = offenseRelationsForPlay(null);
      expect(c.primary_creator).toBe(DEFAULT_HALFCOURT.primary_creator);
      // b is a separate object too.
      expect(b).not.toBe(a);
    });
  });
});
