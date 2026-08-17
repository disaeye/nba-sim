/**
 * Resolve config loader — mirrors `src/duration/loader.ts`.
 *
 * Static JSON import → typed cast. The foundation linter
 * (`scripts/check-foundation.mjs`) runs ajv validation at install time,
 * so the cast at kernel runtime is safe.
 */
import resolveConfigJson from '../../config/resolve.json' with { type: 'json' };
import type { ResolveConfig, ResolveContext } from './types.js';
import type { BaseRates } from './types.js';

// Snake_case from JSON → camelCase for runtime types
interface ResolveConfigRaw {
  foundation_version: string;
  model: string;
  base_rates: BaseRates;
  shot_type_rates?: import('./types.js').ShotTypeRates;
  shot_type_block_bias?: import('./types.js').ShotTypeBlockBias;
  sanity_bands: Record<string, number>;
  bonus_rule: Record<string, unknown>;
  zone_modifiers?: Record<string, number>;
  zone_modifiers_3pt?: Record<string, number>;
}

const raw = resolveConfigJson as unknown as ResolveConfigRaw;

export function loadResolveConfig(): ResolveConfig {
  return {
    foundation_version: raw.foundation_version,
    model: raw.model,
    base_rates: raw.base_rates,
    shot_type_rates: raw.shot_type_rates ?? {
      catch_shoot_2pt: 0.46, catch_shoot_3pt: 0.38, pull_up_2pt: 0.40, pull_up_3pt: 0.33,
      post_2pt: 0.45, drive_finish_2pt: 0.58, other_2pt: 0.42,
    },
    shot_type_block_bias: raw.shot_type_block_bias ?? {
      drive_finish: 1.4, post: 1.2, catch_shoot: 0.7, pull_up: 0.8, other: 1.0,
    },
    sanity_bands: {
      fg_pct_min: raw.sanity_bands.fg_pct_min ?? 0.25,
      fg_pct_max: raw.sanity_bands.fg_pct_max ?? 0.65,
      tp_pct_min: raw.sanity_bands.tp_pct_min ?? 0.20,
      tp_pct_max: raw.sanity_bands.tp_pct_max ?? 0.55,
      score_per_team_min: raw.sanity_bands.score_per_team_min ?? 60,
      score_per_team_max: raw.sanity_bands.score_per_team_max ?? 160,
      oreb_pct_min: raw.sanity_bands.oreb_pct_min ?? 0.1,
      oreb_pct_max: raw.sanity_bands.oreb_pct_max ?? 0.4,
    },
    ft_taxonomy_v0_1: { allowed: [], forbidden: [] },
    bonus_rule: raw.bonus_rule as ResolveConfig['bonus_rule'],
  };
}

export function makeResolveContext(config: ResolveConfig): ResolveContext {
  return {
    baseRates: config.base_rates,
    shotTypeRates: config.shot_type_rates,
    shotTypeBlockBias: config.shot_type_block_bias,
    zoneModifiers: raw.zone_modifiers ?? {
      // Paint 0.85→0.92: a shot from inside 8ft is a layup/short-roll finish
      // — NBA converts ~62% there; 0.85 combined with the contest curve
      // priced rim attempts at 38% (measured), starving the drive game the
      // spatial layer works to create.
      rim: 1.00, paint: 0.92, elbow_L: 0.72, elbow_R: 0.72,
      wing_L: 0.65, wing_R: 0.65, corner_L: 0.80, corner_R: 0.80,
      slot_L: 0.70, slot_R: 0.70, frontcourt_center: 0.68,
      backcourt: 0.05,
    },
    zoneModifiers3pt: raw.zone_modifiers_3pt ?? {
      wing_L: 1.00, wing_R: 1.00, corner_L: 1.08, corner_R: 1.08,
      slot_L: 1.00, slot_R: 1.00, frontcourt_center: 0.98,
      backcourt: 0.05,
    },
  };
}
