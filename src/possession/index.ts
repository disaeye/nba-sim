/**
 * Possession barrel — episode lifecycle + mode/play selection.
 * The live offense engine is DecisionKernel (src/decision); play selection
 * here is advisory: it picks a playId that DecisionKernel uses for bias.
 */
export type {
  EndReason,
  Play,
  PlaysConfig,
  PlayStep,
  PossessionEpisode,
  PossessionErrorCode,
  PossessionMode,
  StartReason,
} from './types.js';
export { PossessionError } from './types.js';
export { modeSelect } from './mode-select.js';
export { selectPlay } from './play-select.js';
export { createEpisode, defendingTeam, inferEndReason } from './episode.js';
