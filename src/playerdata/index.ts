/**
 * playerdata — 球员数据体系 (docs/playerdata-design.md v1.3a, sole authority).
 *
 * Layered player data: 体测 (physical, 8) → 能力 (ability, 24) / 倾向
 * (tendency, 9) / 判断 (awareness, 4) → 属性 (attribute, 7, observed), plus
 * the role/Fit/chemistry/possession systems, season growth, stamina, morale,
 * the generator, the event-stream consumption queries, and the bridge to the
 * kernel's LineupCapability.
 */
export * from './types.js';
export * from './tables.js';
export * from './normalize.js';
export * from './aggregate.js';
export * from './observation.js';
export * from './scout.js';
export * from './fit.js';
export * from './chemistry.js';
export * from './possession.js';
export * from './generate.js';
export * from './growth.js';
export * from './stamina.js';
export * from './morale.js';
export * from './consume.js';
export * from './bridge.js';
