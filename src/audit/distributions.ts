/**
 * Distribution-level statistics aggregator — P0.1.
 *
 * Folds a GameResult event stream into the distributional metrics a
 * hardcore NBA sim must match: shot profile (rim/mid/3), play-type mix,
 * assist/turnover/foul rates, shot-clock timing, and shot-type splits.
 *
 * Every metric is derived from the SAME event stream box-score uses, so
 * it is a pure read — no simulation state is touched.
 */
import type { GameResult, TimelineEvent } from '../sim-utils.js';

// ─── closed unions ──────────────────────────────────────────────────────────

/** Shot location bucket — the modern NBA shot-profile split. */
export type ShotLocation = 'rim' | 'paint' | 'mid' | 'three';

/** Shot method bucket — resolve taxonomy (P1.2 target). */
export type ShotMethod = 'catch_shoot' | 'pull_up' | 'post' | 'drive_finish' | 'other';

/** Possession terminal reason — how possessions end. */
export type PossessionEnd =
  | 'MAKE'
  | 'MISS_DREB'
  | 'MISS_OREB'
  | 'TURNOVER'
  | 'SHOT_CLOCK_VIOLATION'
  | 'SHOOTING_FOUL'
  | 'AND_ONE'
  | 'NON_SHOOTING_FOUL'
  | 'PERIOD_END';

/** Shot-clock band for timing distribution (seconds remaining at release). */
export type ClockBand = '0-4' | '4-10' | '10-15' | '15-18' | '18-24';

/** Foul type taxonomy (P1.8 target). */
export type FoulType = 'shooting' | 'non_shooting' | 'drive' | 'reach' | 'blocking' | 'offensive' | 'other';

// ─── per-game aggregate ─────────────────────────────────────────────────────

export interface DistributionAggregate {
  games: number;
  possessions: number;
  fga: number;
  fgm: number;
  tpa: number;
  tpm: number;
  fta: number;
  ftm: number;
  oreb: number;
  dreb: number;
  assists: number;
  turnovers: number;
  steals: number;
  blocks: number;
  fouls: number;
  scv: number;
  shotLocation: Record<ShotLocation, number>;
  shotMethod: Record<ShotMethod, number>;
  shotLocationMade: Record<ShotLocation, number>;
  possessionEnds: Record<PossessionEnd, number>;
  clockBands: Record<ClockBand, number>;
  foulTypes: Record<FoulType, number>;
  transitionPossessions: number;
  endOfQuarterHeaves: number;
}

// ─── helpers ────────────────────────────────────────────────────────────────

function shotLocationOf(payload: Record<string, unknown>): ShotLocation {
  const zone = payload['zone'];
  if (typeof zone !== 'string') return 'mid';
  switch (zone) {
    case 'rim':
    case 'dunker_L':
    case 'dunker_R':
      return 'rim';
    case 'paint':
      return 'paint';
    case 'elbow_L':
    case 'elbow_R':
    case 'frontcourt_center':
    case 'slot_L':
    case 'slot_R':
      return 'mid';
    case 'wing_L':
    case 'wing_R':
    case 'corner_L':
    case 'corner_R':
      return 'three';
    default:
      return 'mid';
  }
}

function shotMethodOf(payload: Record<string, unknown>): ShotMethod {
  const method = payload['shot_method'];
  if (typeof method === 'string') {
    switch (method) {
      case 'catch_shoot':
      case 'pull_up':
      case 'post':
      case 'drive_finish':
        return method;
      default:
        return 'other';
    }
  }
  // Legacy: derive from catch-window presence.
  const catchWindow = payload['catch_window_seconds'];
  if (typeof catchWindow === 'number' && catchWindow > 0) return 'catch_shoot';
  const drive = payload['drive_finish'];
  if (drive === true) return 'drive_finish';
  return 'other';
}

function clockBandOf(shotClock: unknown): ClockBand {
  const sc = typeof shotClock === 'number' ? shotClock : 24;
  if (sc <= 4) return '0-4';
  if (sc <= 10) return '4-10';
  if (sc <= 15) return '10-15';
  if (sc <= 18) return '15-18';
  return '18-24';
}

function foulTypeOf(e: TimelineEvent): FoulType {
  const payload = e.payload;
  const ft = payload['foul_type'];
  if (typeof ft === 'string') {
    if (ft === 'shooting' || ft === 'non_shooting' || ft === 'drive' || ft === 'reach' || ft === 'blocking' || ft === 'offensive') return ft;
    return 'other';
  }
  if (payload['shooting'] === true) return 'shooting';
  if (e.type === 'FOUL' && payload['and_one'] === true) return 'shooting';
  return 'non_shooting';
}

function possessionEndOf(endReason: string | null): PossessionEnd {
  switch (endReason) {
    case 'MAKE':
    case 'MADE_BASKET':
      return 'MAKE';
    case 'MISS_DREB':
      return 'MISS_DREB';
    case 'MISS_OREB_CONTINUE':
      return 'MISS_OREB';
    case 'TURNOVER':
      return 'TURNOVER';
    case 'SHOT_CLOCK_VIOLATION':
      return 'SHOT_CLOCK_VIOLATION';
    case 'SHOOTING_FOUL':
      return 'SHOOTING_FOUL';
    case 'AND_ONE':
      return 'AND_ONE';
    case 'NON_SHOOTING_FOUL':
      return 'NON_SHOOTING_FOUL';
    case 'PERIOD_END':
      return 'PERIOD_END';
    default:
      return 'MAKE';
  }
}

function newCounts<T extends string>(keys: readonly T[]): Record<T, number> {
  const out = {} as Record<T, number>;
  for (const key of keys) out[key] = 0;
  return out;
}

const SHOT_LOCATIONS: readonly ShotLocation[] = ['rim', 'paint', 'mid', 'three'];
const SHOT_METHODS: readonly ShotMethod[] = ['catch_shoot', 'pull_up', 'post', 'drive_finish', 'other'];
const POSSESSION_ENDS: readonly PossessionEnd[] = ['MAKE', 'MISS_DREB', 'MISS_OREB', 'TURNOVER', 'SHOT_CLOCK_VIOLATION', 'SHOOTING_FOUL', 'AND_ONE', 'NON_SHOOTING_FOUL', 'PERIOD_END'];
const CLOCK_BANDS: readonly ClockBand[] = ['0-4', '4-10', '10-15', '15-18', '18-24'];
const FOUL_TYPES: readonly FoulType[] = ['shooting', 'non_shooting', 'drive', 'reach', 'blocking', 'offensive', 'other'];

export function newDistributionAggregate(): DistributionAggregate {
  return {
    games: 0,
    possessions: 0,
    fga: 0,
    fgm: 0,
    tpa: 0,
    tpm: 0,
    fta: 0,
    ftm: 0,
    oreb: 0,
    dreb: 0,
    assists: 0,
    turnovers: 0,
    steals: 0,
    blocks: 0,
    fouls: 0,
    scv: 0,
    shotLocation: newCounts(SHOT_LOCATIONS),
    shotMethod: newCounts(SHOT_METHODS),
    shotLocationMade: newCounts(SHOT_LOCATIONS),
    possessionEnds: newCounts(POSSESSION_ENDS),
    clockBands: newCounts(CLOCK_BANDS),
    foulTypes: newCounts(FOUL_TYPES),
    transitionPossessions: 0,
    endOfQuarterHeaves: 0,
  };
}

/**
 * Fold one game result into the aggregate.
 */
export function foldGameResult(agg: DistributionAggregate, result: GameResult): void {
  agg.games += 1;
  for (const possession of result.possession_log) {
    agg.possessions += 1;
    if (possession.mode === 'TRANSITION') agg.transitionPossessions += 1;
    const end = possessionEndOf(possession.end_reason);
    agg.possessionEnds[end] += 1;
  }

  // Box-score fields are the canonical source for counting stats that do not
  // have a one-to-one event in the closed foundation catalog.
  for (const player of [...result.box_score.home, ...result.box_score.away]) {
    agg.fta += player.fta;
    agg.ftm += player.ftm;
    agg.oreb += player.oreb;
    agg.dreb += player.dreb;
    agg.turnovers += player.tov;
    agg.steals += player.stl;
    agg.fouls += player.pf;
  }

  for (const e of result.events) {
    switch (e.type) {
      case 'SHOT_RESULT': {
        if (e.payload['blocked'] === true) agg.blocks += 1;
        const payload = e.payload;
        const made = payload['made'] === true;
        agg.fga += 1;
        const value = payload['shot_value'];
        if (value === 3) agg.tpa += 1;
        if (made) {
          agg.fgm += 1;
          if (value === 3) agg.tpm += 1;
          if (typeof payload['assister_id'] === 'string') agg.assists += 1;
        }
        const location = shotLocationOf(payload);
        agg.shotLocation[location] += 1;
        if (made) agg.shotLocationMade[location] += 1;
        agg.shotMethod[shotMethodOf(payload)] += 1;
        agg.clockBands[clockBandOf(payload['shot_clock'])] += 1;
        if (payload['heave'] === true) agg.endOfQuarterHeaves += 1;
        break;
      }
      case 'FOUL': {
        // Foul totals are folded from the box score above; only classify here.
        agg.foulTypes[foulTypeOf(e)] += 1;
        break;
      }
      case 'SHOT_CLOCK_VIOLATION': {
        agg.scv += 1;
        break;
      }
      default:
        break;
    }
  }
}

// ─── rate views ─────────────────────────────────────────────────────────────

export interface DistributionRates {
  readonly fgPct: number;
  readonly tpPct: number;
  readonly ftPct: number;
  readonly rimShare: number;
  readonly midShare: number;
  readonly threeShare: number;
  readonly astPerFgm: number;
  readonly tovRate: number;
  readonly ftr: number;
  readonly orebPct: number;
  readonly stealRate: number;
  readonly blockRate: number;
  readonly possessionLengthSeconds: number;
  readonly scvRate: number;
  readonly catchShootShare: number;
  readonly pullUpShare: number;
  readonly postShare: number;
  readonly driveFinishShare: number;
}

export function distributionRates(agg: DistributionAggregate): DistributionRates {
  const fga = agg.fga || 1;
  const fgm = agg.fgm || 1;
  const poss = agg.possessions || 1;
  return {
    fgPct: agg.fgm / fga,
    tpPct: agg.tpa > 0 ? agg.tpm / agg.tpa : 0,
    ftPct: agg.fta > 0 ? agg.ftm / agg.fta : 0,
    rimShare: (agg.shotLocation.rim + agg.shotLocation.paint) / fga,
    midShare: agg.shotLocation.mid / fga,
    threeShare: agg.shotLocation.three / fga,
    astPerFgm: agg.assists / fgm,
    tovRate: agg.turnovers / poss,
    ftr: agg.fta / fga,
    orebPct: agg.oreb / (agg.oreb + agg.dreb || 1),
    stealRate: agg.steals / poss,
    blockRate: agg.blocks / fga,
    possessionLengthSeconds: 0, // filled by caller (needs liveSeconds)
    scvRate: agg.scv / poss,
    catchShootShare: agg.shotMethod.catch_shoot / fga,
    pullUpShare: agg.shotMethod.pull_up / fga,
    postShare: agg.shotMethod.post / fga,
    driveFinishShare: agg.shotMethod.drive_finish / fga,
  };
}

// ─── NBA reference bands ────────────────────────────────────────────────────

/** NBA 2023-24 league-average reference bands (per 100 possessions or share). */
export interface NbaBand {
  readonly min: number;
  readonly max: number;
  readonly label: string;
}

export const NBA_REFERENCE: Readonly<Record<keyof DistributionRates, NbaBand>> = {
  fgPct: { min: 0.44, max: 0.50, label: 'FG%' },
  tpPct: { min: 0.33, max: 0.40, label: '3PT%' },
  ftPct: { min: 0.75, max: 0.82, label: 'FT%' },
  rimShare: { min: 0.28, max: 0.42, label: 'rim shot share' },
  midShare: { min: 0.15, max: 0.30, label: 'mid-range share' },
  threeShare: { min: 0.35, max: 0.45, label: '3PT shot share' },
  astPerFgm: { min: 0.55, max: 0.70, label: 'assists per FG made' },
  tovRate: { min: 0.10, max: 0.16, label: 'turnovers per possession' },
  ftr: { min: 0.20, max: 0.32, label: 'FT attempts per FGA' },
  orebPct: { min: 0.20, max: 0.32, label: 'offensive rebound %' },
  stealRate: { min: 0.05, max: 0.09, label: 'steals per possession' },
  blockRate: { min: 0.03, max: 0.06, label: 'blocks per FGA' },
  scvRate: { min: 0.0, max: 0.03, label: 'shot-clock violations per possession' },
  possessionLengthSeconds: { min: 13.0, max: 16.0, label: 'possession length (seconds)' },
  catchShootShare: { min: 0.30, max: 0.45, label: 'catch-and-shoot share' },
  pullUpShare: { min: 0.15, max: 0.28, label: 'pull-up share' },
  postShare: { min: 0.03, max: 0.12, label: 'post-up share' },
  driveFinishShare: { min: 0.12, max: 0.25, label: 'drive finish share' },
};

/** Full distribution report for one aggregate. */
export interface DistributionReport {
  readonly games: number;
  readonly rates: DistributionRates;
  readonly deviations: ReadonlyArray<{ readonly key: keyof DistributionRates; readonly value: number; readonly band: NbaBand; readonly pass: boolean }>;
  readonly passed: boolean;
}

/** Build a deterministic NBA-reference report from an aggregate. */
export function distributionReport(agg: DistributionAggregate): DistributionReport {
  const rates = distributionRates(agg);
  const deviations = (Object.keys(NBA_REFERENCE) as Array<keyof DistributionRates>).map((key) => {
    const band = NBA_REFERENCE[key];
    const value = rates[key];
    return { key, value, band, pass: value >= band.min && value <= band.max };
  });
  return { games: agg.games, rates, deviations, passed: deviations.every((row) => row.pass) };
}

/** Fold zero or more game results and return the complete report. */
export function aggregateDistribution(results: Iterable<GameResult>): DistributionReport {
  const aggregate = newDistributionAggregate();
  for (const result of results) foldGameResult(aggregate, result);
  return distributionReport(aggregate);
}
