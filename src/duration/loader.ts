// Static JSON import. tsconfig has `resolveJsonModule: true` +
// `esModuleInterop: true`, so the default export is the parsed object.
// The `with { type: 'json' }` import attribute is the modern ESM form
// and is required when this module is loaded by Node's native ESM
// loader (Node 22+); vitest/vite accepts it as a no-op hint.
import durationConfig from '../../config/duration.json' with { type: 'json' };

import type { DurationConfig } from './types.js';

/**
 * The parsed `config/duration.json`, typed as `DurationConfig`.
 *
 * The JSON file is the foundation authority — validated by
 * `config/schemas/duration.schema.json` via `ajv-cli` and cross-checked
 * by `scripts/check-foundation.mjs` (todo 9). This loader does no runtime
 * validation; the cast is safe because the schema gate already ran in
 * `npm run foundation:lint`.
 *
 * The static-import form (rather than `fs.readFileSync`) keeps this
 * module dependency-free at the type-check level — no `@types/node`
 * needed, which preserves T11's pinned devDependency manifest.
 */
export function loadDurationConfig(): DurationConfig {
  return durationConfig as DurationConfig;
}
