/**
 * FSM config loader. Static-imports `config/fsm.json` (same pattern as
 * `src/duration/loader.ts`) and casts it to the typed `FsmConfig`.
 *
 * No runtime validation: the schema gate (`config/schemas/fsm.schema.json`
 * via ajv-cli) and the foundation linter (`scripts/check-foundation.mjs`)
 * have already validated structure and cross-references at `npm run
 * foundation:lint` time. The `as FsmConfig` cast is the parse-don't-validate
 * boundary transition — interior code receives typed values.
 *
 * No `@types/node` needed: the static import form avoids `fs.readFileSync`,
 * preserving T11's pinned four-package devDependency manifest.
 */
import fsmConfig from '../../config/fsm.json' with { type: 'json' };

import type { FsmConfig } from './types.js';

/** The parsed `config/fsm.json`, typed as `FsmConfig`. */
export function loadFsmConfig(): FsmConfig {
  return fsmConfig as FsmConfig;
}
