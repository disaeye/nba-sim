export { FOUNDATION_VERSION, validateInput, getCheckpoint, getCheckpointByTime } from './sim-utils.js';

export { simulateGame } from './simulate.js';
export type {
  GameInput,
  GameResult,
  TimelineEvent,
  PossessionEntry,
  SubInstruction,
  Player,
  WorldSnapshot,
} from './simulate.js';
export type { Checkpoint } from './sim-utils.js';
export { computeBoxScore } from './box-score.js';
export type { PlayerBox, BoxScore, RosterMaps } from './box-score.js';

/** Player data system (docs/playerdata-design.md v1.3a). */
export * from './playerdata/index.js';

export { renderFromSnapshots, renderTicks } from './render/index.js';
export type { RenderFrame, RenderPlayer, RenderBall, RenderOptions, StreamTick } from './render/index.js';

export {
  buildCourtDrawSpec,
  shotValueAt,
  zoneFromPoint,
  isThreePointShot,
  isInPaint,
  rimNorm,
  attackRim,
  distFeet,
} from './court/geometry.js';

export { buildSpectatorPackage, narrateEvent, formatBroadcastLine } from './spectator/index.js';
export type { SpectatorPackage, Narration } from './spectator/index.js';
export {
  aggregateDistribution,
  distributionReport,
  distributionRates,
  foldGameResult,
  newDistributionAggregate,
  NBA_REFERENCE,
} from './audit/distributions.js';
export type {
  DistributionAggregate,
  DistributionRates,
  DistributionReport,
  ShotLocation,
  ShotMethod,
  PossessionEnd,
  ClockBand,
  FoulType,
  NbaBand,
} from './audit/distributions.js';
