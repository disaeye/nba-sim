/**
 * Resolve module barrel.
 *
 * Re-exports the typed contract surface and the per-check resolve
 * functions plus the FT sequence emitter. Consumers should import from
 * `'../resolve/index.js'` rather than reaching into individual files.
 */
export type {
  BaseRates,
  FtSequenceConfig,
  ResolveConfig,
  ResolveContext,
  ResolveResult,
  SanityBands,
} from './types.js';
export { loadResolveConfig, makeResolveContext } from './loader.js';
export {
  resolveDrive,
  resolveFt,
  resolveHandoff,
  resolvePass,
  resolveRebound,
  resolveShot,
  shotAbilityModifier,
  resolveSteal,
} from './checks.js';
export { runFtSequence } from './ft-sequence.js';
