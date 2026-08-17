import type { GameResult, TimelineEvent } from '../sim-utils.js';
import type { WorldSnapshot } from '../world/snapshot.js';
export type RealityGapCode =
  | 'SCREEN_USE_WITHOUT_SET'
  | 'SCREEN_SET_WITHOUT_PRESSURE'
  | 'SCREEN_SET_TOO_FAR'
  | 'SCREEN_DEFENSE_NOT_EXPOSED'
  | 'DROP_DEFENDER_NOT_AT_RIM'
  | 'SWITCH_COVERAGE_NOT_EXPOSED'
  | 'ADVANCE_STALL'
  | 'HELD_HOLDER_MISMATCH'
  | 'PLAYER_TELEPORT'
  | 'STEAL_OUT_OF_REACH'
  | 'FOUL_OUT_OF_REACH';

export interface RealityGapViolation {
  readonly code: RealityGapCode;
  readonly severity: 'error' | 'warning';
  readonly seq: number | null;
  readonly message: string;
  readonly evidence: Readonly<Record<string, unknown>>;
  readonly likelyRootCause: string;
}

export interface RealityGapReport {
  readonly seed: number;
  readonly violations: readonly RealityGapViolation[];
  readonly counts: Readonly<Record<RealityGapCode, number>>;
  readonly pass: boolean;
}

function payloadString(event: TimelineEvent, key: string): string | null {
  const value = event.payload[key];
  return typeof value === 'string' ? value : null;
}

function distanceFt(a: { readonly x: number; readonly y: number }, b: { readonly x: number; readonly y: number }): number {
  return Math.hypot((a.x - b.x) * 94, (a.y - b.y) * 50);
}

function snapshotAt(result: GameResult, seq: number): WorldSnapshot | null {
  // Prefer the exact seq match; otherwise the PREVIOUS snapshot — the
  // screen's physical frame is the tick before its event when a
  // TURNOVER/steal lands in the same tick (the follow-up snapshot has
  // no tactical render). Fall back to the next snapshot after the event
  // only when no earlier frame exists.
  const snapshots = result.snapshots ?? [];
  const exact = snapshots.find((snapshot) => snapshot.lastEventSeq === seq);
  if (exact) return exact;
  let prior: WorldSnapshot | null = null;
  for (const snapshot of snapshots) {
    const last = snapshot.lastEventSeq ?? -1;
    if (last >= seq) break;
    prior = snapshot;
  }
  if (prior) return prior;
  return snapshots.find((snapshot) => (snapshot.lastEventSeq ?? -1) > seq) ?? null;
}

function add(
  violations: RealityGapViolation[],
  violation: RealityGapViolation,
): void {
  violations.push(violation);
}

function auditScreenLifecycle(result: GameResult, violations: RealityGapViolation[]): void {
  const sets = result.events.filter((event) => event.type === 'SCREEN_SET');
  for (const use of result.events.filter((event) => event.type === 'SCREEN_USE')) {
    const key = `${payloadString(use, 'screener_id')}/${payloadString(use, 'ballHandlerId')}`;
    const prior = sets.some((set) => set.seq < use.seq
      && `${payloadString(set, 'screener_id')}/${payloadString(set, 'ballHandlerId')}` === key);
    if (!prior) {
      add(violations, {
        code: 'SCREEN_USE_WITHOUT_SET', severity: 'error', seq: use.seq,
        message: 'SCREEN_USE 没有对应的先前 SCREEN_SET。',
        evidence: { screener: payloadString(use, 'screener_id'), handler: payloadString(use, 'ballHandlerId') },
        likelyRootCause: '挡拆生命周期被事件层瞬时伪造，或状态完成事实没有先提交。',
      });
    }
  }
  for (const set of sets) {
    // A screen that SETs inside the final half-second of a period is a
    // buzzer-edge remnant: the possession ends on the horn and no USE
    // follows, so there is no defensive response to demand. The audit
    // window is the live screen lifecycle, not the dead-ball edge.
    if (set.clocks && (set.clocks.game <= 0.5 || set.clocks.shot <= 0.5)) continue;
    const screenerId = payloadString(set, 'screener_id');
    const handlerId = payloadString(set, 'ballHandlerId');
    const defenderId = payloadString(set, 'defender_id');
    const frame = snapshotAt(result, set.seq);
    const screener = frame?.players.find((player) => player.jersey === screenerId);
    const handler = frame?.players.find((player) => player.jersey === handlerId);
    const defender = frame?.players.find((player) => player.jersey === defenderId);
    if (!defenderId || !defender) {
      add(violations, {
        code: 'SCREEN_SET_WITHOUT_PRESSURE', severity: 'error', seq: set.seq,
        message: '挡拆成立时没有可识别的持球防守人。',
        evidence: { screener: screenerId, handler: handlerId, defender: defenderId },
        likelyRootCause: '挡拆选择只看进攻阵型，没有把持球防守压力作为前置条件。',
      });
    }
    if (screener && handler && distanceFt(screener, handler) > 6) {
      add(violations, {
        code: 'SCREEN_SET_TOO_FAR', severity: 'error', seq: set.seq,
        message: 'SCREEN_SET 发生时掩护人与持球人距离过大。',
        evidence: { distanceFt: distanceFt(screener, handler), screener: screenerId, handler: handlerId },
        likelyRootCause: '空间目标与完成事实使用了不同的几何尺度，导致事件先于实体接触成立。',
      });
    }
    const defense = frame?.tactical?.screenDefense;
    if (!defense) {
      add(violations, {
        code: 'SCREEN_DEFENSE_NOT_EXPOSED', severity: 'error', seq: set.seq,
        message: '挡拆成立时没有暴露防守覆盖策略。',
        evidence: { screener: screenerId, handler: handlerId, defender: defenderId },
        likelyRootCause: '防守策略只存在于内部目标计算，没有进入挡拆状态和观众快照。',
      });
    } else if (defense.mode === 'DROP' && frame) {
      const dropDefender = defense.screenerDefender
        ? frame.players.find((player) => player.jersey === defense.screenerDefender)
        : undefined;
      const rim = frame.ball.x < 0.5 ? { x: 0.0559, y: 0.5 } : { x: 0.9441, y: 0.5 };
      if (dropDefender && distanceFt(dropDefender, rim) > 20) {
        add(violations, {
          code: 'DROP_DEFENDER_NOT_AT_RIM', severity: 'warning', seq: set.seq,
          message: '策略标记为沉退，但掩护防守人仍远离篮下。',
          evidence: { defender: defense.screenerDefender, rimDistanceFt: distanceFt(dropDefender, rim), mode: defense.mode },
          likelyRootCause: 'DROP 策略没有转化为防守人的实际目标位置。',
        });
      }
    } else if (defense.mode === 'SWITCH' && !defense.switchesAtUse) {
      add(violations, {
        code: 'SWITCH_COVERAGE_NOT_EXPOSED', severity: 'error', seq: set.seq,
        message: '策略标记为换防，但没有声明在使用挡拆时交接。',
        evidence: { mode: defense.mode, switchesAtUse: defense.switchesAtUse },
        likelyRootCause: '换防只是标签，没有进入两名防守人的目标交换状态。',
      });
    }
  }
}

function auditMotion(result: GameResult, violations: RealityGapViolation[]): void {
  const snapshots = result.snapshots ?? [];
  const byHolder = new Map<string, WorldSnapshot[]>();
  for (const snapshot of snapshots) {
    if (snapshot.phase !== 'LIVE' || snapshot.ball.status !== 'held' || !snapshot.ball.holderId) continue;
    const list = byHolder.get(snapshot.ball.holderId) ?? [];
    list.push(snapshot);
    byHolder.set(snapshot.ball.holderId, list);
    const holder = snapshot.players.find((player) => player.jersey === snapshot.ball.holderId);
    if (!holder || !holder.hasBall) {
      add(violations, {
        code: 'HELD_HOLDER_MISMATCH', severity: 'error', seq: snapshot.lastEventSeq,
        message: '球处于 held，但快照中的 holder 没有 hasBall。',
        evidence: { holderId: snapshot.ball.holderId, lastEventType: snapshot.lastEventType },
        likelyRootCause: '球状态和玩家姿态没有在同一完成点提交。',
      });
    }
  }
  for (const [holderId, frames] of byHolder) {
    for (let i = 1; i < frames.length; i += 1) {
      const previous = frames[i - 1]!;
      const current = frames[i]!;
      const player = current.players.find((candidate) => candidate.jersey === holderId);
      const prior = previous.players.find((candidate) => candidate.jersey === holderId);
      if (!player || !prior || current.t_real - previous.t_real > 0.25) continue;
      const jump = distanceFt(player, prior);
      if (jump > 8 && previous.lastEventType !== 'LOOSE_BALL_RECOVER' && current.lastEventType !== 'POSSESSION_GAINED') {
        add(violations, {
          code: 'PLAYER_TELEPORT', severity: 'error', seq: current.lastEventSeq,
          message: '连续快照中的球员发生不可解释的大位移。',
          evidence: { holderId, distanceFt: jump, from: { x: prior.x, y: prior.y }, to: { x: player.x, y: player.y } },
          likelyRootCause: '绝对阵型目标覆盖了连续移动状态，或死球落位逻辑泄漏进 LIVE。',
        });
      }
    }
  }
  for (const event of result.events.filter((candidate) => candidate.type === 'ADVANCE_BACKCOURT')) {
    const holderId = payloadString(event, 'ballHandlerId');
    if (!holderId) continue;
    const disruptive = result.events.find((candidate) => candidate.seq > event.seq
      && (candidate.type === 'STEAL' || candidate.type === 'TURNOVER' || candidate.type === 'LOOSE_BALL_RECOVER' || candidate.type === 'POSSESSION_GAINED'));
    const passedAway = result.events.find((candidate) => candidate.seq > event.seq
      && candidate.type === 'PASS');
    const acted = result.events.find((candidate) => candidate.seq > event.seq
      && (candidate.type === 'SHOT_RELEASE' || candidate.type === 'DRIVE' || candidate.type === 'FOUL'));
    const startIndex = snapshots.findIndex((snapshot) => snapshot.lastEventSeq === event.seq);
    // The snapshot stream does not carry every event seq (events between
    // tick snapshots are coalesced); without a locatable window start the
    // "window" would begin at game tip and the movement check is
    // meaningless. Skip instead of false-flagging.
    if (startIndex < 0 || event.clocks.game > 719.5 || event.clocks.game < 0.5) continue;
    const afterAdvance = snapshots.slice(Math.max(0, startIndex));
    const window: WorldSnapshot[] = [];
    for (const snapshot of afterAdvance) {
      if (disruptive && snapshot.lastEventSeq !== null && snapshot.lastEventSeq >= disruptive.seq) break;
      // A pass is a legitimate end to the advance — the handler moved the
      // ball instead of the body (fast-break kickout), not a stall.
      if (passedAway && snapshot.lastEventSeq !== null && snapshot.lastEventSeq >= passedAway.seq) break;
      if (acted && snapshot.lastEventSeq !== null && snapshot.lastEventSeq >= acted.seq) break;
      if (snapshot.ball.holderId === holderId && snapshot.ball.status === 'held') window.push(snapshot);
      if (window.length >= 40) break;
    }
    // A pass is a legal completion of an advance intent: the handler can
    // cross the line with one dribble and hit a forward outlet before a
    // long body-motion window elapses. The old audit still required 1.4ft
    // of handler displacement after every ADVANCE_BACKCOURT event, so a
    // near-halfcourt recovery followed by a 0.5s kick-ahead pass was
    // misclassified as a freeze (ADVANCE_STALL). Once the ball leaves the
    // handler, the advance obligation is satisfied by ball progression;
    // continue checking only advances that remain held and unacted.
    if (passedAway) continue;
    if (window.length < 5) continue;
    const first = window[0]!.players.find((player) => player.jersey === holderId);
    const last = window[window.length - 1]!.players.find((player) => player.jersey === holderId);
    if (!first || !last || (first.x >= 0.5 && last.x <= 0.5) || (first.x <= 0.5 && last.x >= 0.5)) continue;
    // A stall is NO movement at all. Settling into the frontcourt strike
    // zone can legitimately pull the handler back from midcourt (the
    // attack direction is not inferable from x alone), so direction
    // checks misread 0.42 → 0.29 as a stall.
    const moved = Math.hypot((last.x - first.x) * 94, (last.y - first.y) * 50);
    if (moved < 1.4) {
      add(violations, {
        code: 'ADVANCE_STALL', severity: 'error', seq: event.seq,
        message: '推进事件发生后，持球人没有有效移动。',
        evidence: { holderId, movedFt: moved, fromX: first.x, toX: last.x, fromY: first.y, toY: last.y },
        likelyRootCause: '推进意图被 hold/space 覆盖，或推进目标基于滞后球坐标而非持球人当前位置。',
      });
    }
  }
}
function auditEventContactGeometry(result: GameResult, violations: RealityGapViolation[]): void {
  const snapshots = result.snapshots ?? [];
  const nearby = (event: TimelineEvent, id: string | null): WorldSnapshot['players'][number] | null => {
    if (!id) return null;
    const candidates = snapshots.filter((snapshot) => Math.abs(snapshot.t_real - event.t_real) <= 0.11);
    let best: WorldSnapshot['players'][number] | null = null;
    let bestDt = Infinity;
    for (const snapshot of candidates) {
      const player = snapshot.players.find((candidate) => candidate.jersey === id);
      if (player && Math.abs(snapshot.t_real - event.t_real) < bestDt) {
        best = player;
        bestDt = Math.abs(snapshot.t_real - event.t_real);
      }
    }
    return best;
  };
  for (const event of result.events) {
    if (event.type === 'STEAL') {
      const payload = event.payload;
      const stealer = nearby(event, payloadString(event, 'stealer_id'));
      const victim = nearby(event, payloadString(event, 'victim_id'));
      if (stealer && victim && distanceFt(stealer, victim) > 6) {
        add(violations, {
          code: 'STEAL_OUT_OF_REACH', severity: 'error', seq: event.seq,
          message: '抢断事件发生时，抢断人与失误者不在合理触球距离内。',
          evidence: { stealer: payloadString(event, 'stealer_id'), victim: payloadString(event, 'victim_id'), distanceFt: distanceFt(stealer, victim), midPass: payload['mid_pass'] === true },
          likelyRootCause: '抢断解析只依赖候选事实和随机成功，没有把事件时的身体几何作为最终提交门槛。',
        });
      }
    } else if (event.type === 'FOUL') {
      const payload = event.payload;
      const offender = nearby(event, payloadString(event, 'offender_id'));
      const victim = nearby(event, payloadString(event, 'victim_id'));
      if (offender && victim && distanceFt(offender, victim) > 6) {
        add(violations, {
          code: 'FOUL_OUT_OF_REACH', severity: 'error', seq: event.seq,
          message: '犯规事件发生时，犯规人与被犯规人不在身体接触范围内。',
          evidence: { offender: payloadString(event, 'offender_id'), victim: payloadString(event, 'victim_id'), distanceFt: distanceFt(offender, victim), foulType: payloadString(event, 'foul_type') },
          likelyRootCause: '犯规事实在空间接触之前提交，事件语义与连续位置层脱节。',
        });
      }
    }
  }
}

export function auditGameReality(result: GameResult): RealityGapReport {
  const violations: RealityGapViolation[] = [];
  auditScreenLifecycle(result, violations);
  auditMotion(result, violations);
  auditEventContactGeometry(result, violations);
  const counts = {} as Record<RealityGapCode, number>;
  for (const violation of violations) counts[violation.code] = (counts[violation.code] ?? 0) + 1;
  return { seed: result.meta.seed, violations, counts, pass: violations.every((violation) => violation.severity !== 'error') };
}
