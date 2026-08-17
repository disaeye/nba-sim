/**
 * Resolve contract types — the runtime view of `config/resolve.json`.
 * v0.7.0: added zone_modifiers + block_rate for spatial shot resolution.
 */
import type { TeamId } from '../state/types.js';

export interface BaseRates {
  readonly pass_success: number;
  readonly handoff_success: number;
  readonly drive_success: number;
  readonly shot_make_2pt: number;
  readonly shot_make_3pt: number;
  readonly ft_make: number;
  readonly steal_attempt_success: number;
  readonly foul_on_drive_rate: number;
  readonly offensive_rebound_rate: number;
  readonly block_rate: number;
}

/** P1.2 shot-type taxonomy base rates — NBA-empirical by method. */
export interface ShotTypeRates {
  readonly catch_shoot_2pt: number;
  readonly catch_shoot_3pt: number;
  readonly pull_up_2pt: number;
  readonly pull_up_3pt: number;
  readonly post_2pt: number;
  readonly drive_finish_2pt: number;
  readonly other_2pt: number;
}

/** P1.3 block-rate multiplier per shot type (rim shots blockable, 3PT not). */
export interface ShotTypeBlockBias {
  readonly drive_finish: number;
  readonly post: number;
  readonly catch_shoot: number;
  readonly pull_up: number;
  readonly other: number;
}

export interface SanityBands {
  readonly fg_pct_min: number;
  readonly fg_pct_max: number;
  readonly tp_pct_min: number;
  readonly tp_pct_max: number;
  readonly score_per_team_min: number;
  readonly score_per_team_max: number;
  readonly oreb_pct_min: number;
  readonly oreb_pct_max: number;
}

export interface ResolveConfig {
  readonly foundation_version: string;
  readonly model: string;
  readonly base_rates: BaseRates;
  readonly shot_type_rates: ShotTypeRates;
  readonly shot_type_block_bias: ShotTypeBlockBias;
  readonly sanity_bands: SanityBands;
  readonly ft_taxonomy_v0_1: {
    readonly allowed: readonly string[];
    readonly forbidden: readonly string[];
  };
  readonly bonus_rule: {
    readonly team_fouls_per_period_threshold: number;
    readonly description: string;
  };
}

export interface ResolveContext {
  readonly baseRates: BaseRates;
  /** Per-zone FG% modifiers (NBA-empirical, normalized to rim≈1.0). */
  readonly zoneModifiers: Record<string, number>;
  /** Per-zone 3PT modifiers (NBA-empirical, corner≈1.08, wings flat 1.0).
   *   Three-point attempts MUST use this table, never zoneModifiers. */
  readonly zoneModifiers3pt: Record<string, number>;
  /** P1.2 shot-type taxonomy base rates. */
  readonly shotTypeRates: ShotTypeRates;
  /** P1.3 block-rate multiplier per shot type. */
  readonly shotTypeBlockBias: ShotTypeBlockBias;
}

export type ResolveResult =
  | { readonly success: boolean }
  | { readonly success: boolean; readonly fouled: boolean }
  | { readonly made: boolean; readonly blocked: boolean }
  | { readonly offensive: boolean };

export interface FtSequenceConfig {
  readonly shooterId: string;
  readonly team: TeamId;
  readonly attempts: 1 | 2 | 3;
  readonly andOne: boolean;
}
