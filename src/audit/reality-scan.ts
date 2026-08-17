import type { GameResult, TimelineEvent } from '../sim-utils.js';
import type { WorldSnapshot } from '../world/snapshot.js';
import { auditGameReality, type RealityGapViolation } from './reality-gap.js';
import { scanEventStream } from './event-stream.js';
import { foldGameResult, newDistributionAggregate, NBA_REFERENCE, distributionRates, type DistributionAggregate } from './distributions.js';
export type RealityScanLayer = 'integrity' | 'causal' | 'spatial' | 'tactical' | 'statistical';

export interface RealityScanFinding {
  readonly layer: RealityScanLayer;
  readonly code: string;
  readonly severity: 'error' | 'warning';
  readonly seed: number;
  readonly seq: number | null;
  readonly message: string;
  readonly evidence: Readonly<Record<string, unknown>>;
  readonly likelyRootCause: string;
}

export interface RealityMetric {
  readonly value: number;
  readonly band: { readonly min: number; readonly max: number };
  readonly pass: boolean;
  readonly unit: string;
}

export interface RealityScanReport {
  readonly seeds: readonly number[];
  readonly games: number;
  /** Event-level and statistical findings; any error drives passed=false. */
  readonly findings: readonly RealityScanFinding[];
  /** Aggregate metrics used for diagnostics and statistical findings. */
  readonly diagnostics: Readonly<Record<string, RealityMetric>>;
  readonly passed: boolean;
}

const BANDS = {
  pace_per_team: { min: 97, max: 103, unit: 'possessions/48/team' },
  mean_possession_length_seconds: { min: 13.5, max: 15.5, unit: 'live seconds' },
  transition_share: { min: 0.12, max: 0.18, unit: 'share' },
  fg_pct: { min: 0.25, max: 0.65, unit: 'share' },
  score_per_team: { min: 80, max: 160, unit: 'points/team' },
  oreb_pct: { min: 0.18, max: 0.32, unit: 'share' },
  scv_rate: { min: 0, max: 0.12, unit: 'share' },
  turnover_rate: { min: 0.08, max: 0.22, unit: 'share' },
} as const;

type MetricKey = keyof typeof BANDS;

function findingFromGap(seed: number, violation: RealityGapViolation): RealityScanFinding {
  const statistical = new Set(['ADVANCE_STALL']);
  const tactical = new Set([
    'SCREEN_SET_WITHOUT_PRESSURE',
    'SCREEN_SET_TOO_FAR',
    'SCREEN_DEFENSE_NOT_EXPOSED',
    'DROP_DEFENDER_NOT_AT_RIM',
    'SWITCH_COVERAGE_NOT_EXPOSED',
  ]);
  const spatial = new Set(['PLAYER_TELEPORT']);
  const layer: RealityScanLayer = statistical.has(violation.code)
    ? 'statistical'
    : tactical.has(violation.code)
      ? 'tactical'
      : spatial.has(violation.code)
        ? 'spatial'
        : 'causal';
  return { ...violation, seed, layer };
}

function add(
  findings: RealityScanFinding[],
  seed: number,
  layer: RealityScanLayer,
  code: string,
  severity: 'error' | 'warning',
  seq: number | null,
  message: string,
  evidence: Readonly<Record<string, unknown>>,
  likelyRootCause: string,
): void {
  findings.push({ layer, code, severity, seed, seq, message, evidence, likelyRootCause });
}

function auditTimeline(result: GameResult, findings: RealityScanFinding[]): void {
  let previous: TimelineEvent | null = null;
  for (const event of result.events) {
    if (previous && event.seq <= previous.seq) {
      add(findings, result.meta.seed, 'integrity', 'EVENT_SEQ_NOT_MONOTONIC', 'error', event.seq,
        '事件序号不是严格递增的。', { previousSeq: previous.seq, seq: event.seq },
        '事件提交路径存在重复提交或多个状态层各自分配序号。');
    }
    if (previous && event.t_real < previous.t_real) {
      add(findings, result.meta.seed, 'integrity', 'EVENT_REAL_TIME_REVERSED', 'error', event.seq,
        '事件真实时间倒退。', { previous: previous.t_real, current: event.t_real },
        '语义事件时间戳没有使用同一个连续时钟提交。');
    }
    if (previous && event.period === previous.period && event.clocks.game > previous.clocks.game + 0.01) {
      add(findings, result.meta.seed, 'integrity', 'GAME_CLOCK_REVERSED', 'error', event.seq,
        '同一节内比赛时钟反向增加。', { previous: previous.clocks.game, current: event.clocks.game },
        '死球或事件折叠错误地修改了 LIVE game clock。');
    }
    previous = event;
  }
}

function distanceFt(a: { readonly x: number; readonly y: number }, b: { readonly x: number; readonly y: number }): number {
  return Math.hypot((a.x - b.x) * 94, (a.y - b.y) * 50);
}

function auditSnapshots(result: GameResult, findings: RealityScanFinding[]): void {
  const snapshots = result.snapshots ?? [];
  let previous: WorldSnapshot | null = null;
  let crowdedFrames = 0;
  let liveFrames = 0;
  for (const snapshot of snapshots) {
    if (previous && snapshot.t_real < previous.t_real) {
      add(findings, result.meta.seed, 'integrity', 'SNAPSHOT_TIME_REVERSED', 'error', snapshot.lastEventSeq,
        '连续世界快照的 real clock 倒退。', { previous: previous.t_real, current: snapshot.t_real },
        '快照写入发生在状态推进之前或存在重复时间源。');
    }
    if (snapshot.phase === 'LIVE') {
      liveFrames += 1;
      if (snapshot.players.length !== 10) {
        add(findings, result.meta.seed, 'integrity', 'LIVE_PLAYER_COUNT_INVALID', 'error', snapshot.lastEventSeq,
          'LIVE 快照不是完整十人场。', { count: snapshot.players.length, t_real: snapshot.t_real },
          '换人、死球落位或阵容裁剪泄漏到了 LIVE 连续状态。');
      }
      const holders = snapshot.players.filter((player) => player.hasBall);
      if (snapshot.ball.status === 'held' && (holders.length !== 1 || holders[0]?.jersey !== snapshot.ball.holderId)) {
        add(findings, result.meta.seed, 'causal', 'BALL_HOLDER_STATE_MISMATCH', 'error', snapshot.lastEventSeq,
          'held 球的 holderId、玩家 hasBall 和快照状态不一致。', {
            holderId: snapshot.ball.holderId,
            hasBall: holders.map((player) => player.jersey),
            t_real: snapshot.t_real,
          }, '球状态、玩家姿态和持球权没有在同一个完成点提交。');
      }
      for (const team of ['home', 'away'] as const) {
        const players = snapshot.players.filter((player) => player.team === team);
        let minDistance = Infinity;
        for (let i = 0; i < players.length; i += 1) {
          for (let j = i + 1; j < players.length; j += 1) {
            minDistance = Math.min(minDistance, distanceFt(players[i]!, players[j]!));
          }
        }
        if (minDistance < 0.5) crowdedFrames += 1;
      }
    }
    previous = snapshot;
  }
  if (liveFrames > 0 && crowdedFrames / liveFrames > 0.05) {
    add(findings, result.meta.seed, 'spatial', 'TEAM_SPACING_COLLAPSE', 'warning', null,
      '超过 5% 的 LIVE 快照出现同队球员小于 0.5 英尺的重叠。',
      { crowdedFrames, liveFrames, fraction: crowdedFrames / liveFrames },
      '多个球员被写入相同绝对阵型目标，或身体接触分离没有在 retarget 后生效。');
  }
}

interface Aggregate {
  games: number;
  possessions: number;
  transition: number;
  liveSeconds: number;
  fgm: number;
  fga: number;
  oreb: number;
  dreb: number;
  scv: number;
  turnovers: number;
  teamPaces: number[];
  teamScores: number[];
}

function newAggregate(): Aggregate {
  return {
    games: 0, possessions: 0, transition: 0, liveSeconds: 0,
    fgm: 0, fga: 0, oreb: 0, dreb: 0, scv: 0, turnovers: 0,
    teamPaces: [], teamScores: [],
  };
}

function foldAggregate(aggregate: Aggregate, result: GameResult): void {
  aggregate.games += 1;
  const periodCount = new Set(result.events.map((event) => event.period)).size;
  const gameMinutes = 48 + Math.max(0, periodCount - 4) * 5;
  let homePossessions = 0;
  let awayPossessions = 0;
  for (const possession of result.possession_log) {
    aggregate.possessions += 1;
    if (possession.offensive_team_id === 'home') homePossessions += 1;
    else awayPossessions += 1;
    if (possession.mode === 'TRANSITION') aggregate.transition += 1;
    if (possession.end_reason === 'SHOT_CLOCK_VIOLATION') aggregate.scv += 1;
    if (possession.end_reason === 'TURNOVER') aggregate.turnovers += 1;
  }
  aggregate.liveSeconds += Math.min(4, periodCount) * 720 + Math.max(0, periodCount - 4) * 300;
  aggregate.teamPaces.push((homePossessions / gameMinutes) * 48, (awayPossessions / gameMinutes) * 48);
  let homeScore = 0;
  let awayScore = 0;
  for (const player of result.box_score.home) {
    homeScore += player.points;
    aggregate.fgm += player.fgm;
    aggregate.fga += player.fga;
    aggregate.oreb += player.oreb;
    aggregate.dreb += player.dreb;
  }
  for (const player of result.box_score.away) {
    awayScore += player.points;
    aggregate.fgm += player.fgm;
    aggregate.fga += player.fga;
    aggregate.oreb += player.oreb;
    aggregate.dreb += player.dreb;
  }
  aggregate.teamScores.push(homeScore, awayScore);
}

function mean(values: readonly number[]): number {
  return values.length === 0 ? Number.NaN : values.reduce((sum, value) => sum + value, 0) / values.length;
}

function makeMetric(value: number, key: MetricKey): RealityMetric {
  const band = BANDS[key];
  return { value, band: { min: band.min, max: band.max }, pass: value >= band.min && value <= band.max, unit: band.unit };
}

function aggregateMetrics(aggregate: Aggregate): Readonly<Record<MetricKey, RealityMetric>> {
  return {
    pace_per_team: makeMetric(mean(aggregate.teamPaces), 'pace_per_team'),
    mean_possession_length_seconds: makeMetric(aggregate.possessions > 0 ? aggregate.liveSeconds / aggregate.possessions : Number.NaN, 'mean_possession_length_seconds'),
    transition_share: makeMetric(aggregate.possessions > 0 ? aggregate.transition / aggregate.possessions : Number.NaN, 'transition_share'),
    fg_pct: makeMetric(aggregate.fga > 0 ? aggregate.fgm / aggregate.fga : Number.NaN, 'fg_pct'),
    score_per_team: makeMetric(mean(aggregate.teamScores), 'score_per_team'),
    oreb_pct: makeMetric(aggregate.oreb + aggregate.dreb > 0 ? aggregate.oreb / (aggregate.oreb + aggregate.dreb) : Number.NaN, 'oreb_pct'),
    scv_rate: makeMetric(aggregate.possessions > 0 ? aggregate.scv / aggregate.possessions : Number.NaN, 'scv_rate'),
    turnover_rate: makeMetric(aggregate.possessions > 0 ? aggregate.turnovers / aggregate.possessions : Number.NaN, 'turnover_rate'),
  };
}

function statisticalFindings(
  metrics: Readonly<Record<MetricKey, RealityMetric>>,
  seed: number,
): RealityScanFinding[] {
  const findings: RealityScanFinding[] = [];
  const rootCauses: Partial<Record<MetricKey, string>> = {
    pace_per_team: '决策节奏过快、终结过早或持球回合被错误切碎。',
    mean_possession_length_seconds: '推进、停顿和执行阶段的时间模型没有形成真实回合长度。',
    transition_share: '攻防转换触发条件或半场落位状态机与真实比赛分布不一致。',
    fg_pct: '出手区域、压力修正或命中率基线与事件生成分布不一致。',
    score_per_team: '回合长度、出手频率或得分终结概率共同造成比赛总产量偏移。',
    oreb_pct: '投篮结束、篮板归属或进攻篮板继续回合的概率链不一致。',
    scv_rate: '持球动作未能及时终结，watchdog、shot-clock 或推进状态存在停滞。',
    turnover_rate: '传球、持球推进、抢断和回合结束原因的概率链不一致。',
  };
  for (const key of Object.keys(metrics) as MetricKey[]) {
    const metric = metrics[key];
    if (metric.pass) continue;
    add(findings, seed, 'statistical', `METRIC_${key.toUpperCase()}`, 'error', null,
      `${key} 超出真实性区间。`, { metric: key, value: metric.value, band: metric.band, unit: metric.unit },
      rootCauses[key] ?? '该统计指标的源事件链与真实性目标不一致。');
  }
  return findings;
}
function distributionFindings(
  agg: DistributionAggregate,
  seed: number,
): RealityScanFinding[] {
  const findings: RealityScanFinding[] = [];
  const rates = distributionRates(agg);
  for (const key of Object.keys(NBA_REFERENCE) as Array<keyof typeof NBA_REFERENCE>) {
    const band = NBA_REFERENCE[key];
    const value = rates[key];
    if (value === 0 && key !== 'scvRate') continue; // no data yet (e.g. no shot_method tags)
    if (value >= band.min && value <= band.max) continue;
    findings.push({
      layer: 'statistical',
      code: `DIST_${key.toUpperCase()}`,
      severity: 'error',
      seed,
      seq: null,
      message: `${band.label} 超出 NBA 参考区间。`,
      evidence: { metric: key, value, band: { min: band.min, max: band.max } },
      likelyRootCause: '事件流的投篮/传球/篮板/犯规概率链与真实分布不一致。',
    });
  }
  return findings;
}

export function scanReality(results: Iterable<GameResult>): RealityScanReport {
  const findings: RealityScanFinding[] = [];
  const aggregate = newAggregate();
  const distribution = newDistributionAggregate();
  const seeds: number[] = [];
  for (const result of results) {
    seeds.push(result.meta.seed);
    auditTimeline(result, findings);
    auditSnapshots(result, findings);
    findings.push(...scanEventStream(result));
    for (const violation of auditGameReality(result).violations) {
      if (findings.length < 2000) findings.push(findingFromGap(result.meta.seed, violation));
    }
    foldAggregate(aggregate, result);
    foldGameResult(distribution, result);
  }
  const diagnostics = aggregateMetrics(aggregate);
  const statisticalSeed = seeds.length === 1 ? (seeds[0] ?? 0) : 0;
  findings.push(...statisticalFindings(diagnostics, statisticalSeed));
  // P0.1 distribution-level validation against NBA reference bands.
  findings.push(...distributionFindings(distribution, statisticalSeed));
  return {
    seeds,
    games: seeds.length,
    findings,
    diagnostics,
    passed: findings.every((finding) => finding.severity !== 'error'),
  };
}
