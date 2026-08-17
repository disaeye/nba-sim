/**
 * Duration config contract — mirrors the runtime subset of
 * config/duration.json needed by the sampler. The JSON file (validated by
 * config/schemas/duration.schema.json) is the sole authority; if this
 * interface ever disagrees with the schema, the schema wins.
 */

/**
 * One sampling distribution entry. v0.1.0 freezes `distribution: uniform`
 * as the only permitted value (the `trunc_normal` enum slot is reserved
 * for a future minor-version bump and rejected by the foundation linter).
 */
export interface DurationEntry {
  readonly id: string;
  readonly distribution: 'uniform' | 'trunc_normal';
  readonly min: number;
  readonly max: number;
}

/**
 * The runtime-facing duration config. Only the fields actually consumed
 * by `sampleDuration` are declared; the remaining config/duration.json
 * fields (`clocks`, `made_fg_stoppage`, `start_reason_bias`, etc.) are
 * owned by other kernel modules and read separately.
 */
export interface DurationConfig {
  readonly foundation_version: string;
  readonly resolution_seconds: number;
  readonly quantize_rule: string;
  readonly durations: readonly DurationEntry[];
}

/**
 * Discriminator for `DurationNotFoundError`. Stable string lets consumers
 * branch on `err.code` without parsing the message.
 */
export type DurationErrorCode = 'DURATION_NOT_FOUND';

/**
 * Typed error raised when `sampleDuration` is asked for an `id` that is
 * not in `config.durations`. Carries the offending `id` so the caller can
 * surface it in a structured error report.
 */
export class DurationNotFoundError extends Error {
  readonly code: DurationErrorCode = 'DURATION_NOT_FOUND';
  readonly id: string;

  constructor(id: string) {
    super(`sampleDuration: id "${id}" not found in durations catalog`);
    this.name = 'DurationNotFoundError';
    this.id = id;
  }
}
