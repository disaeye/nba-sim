import type { TimelineEvent, GameResult } from '../sim-utils.js';
import type { WorldSnapshot } from '../world/snapshot.js';
import type { RealityScanFinding } from './reality-scan.js';

function text(event: TimelineEvent, key: string): string | null {
  const value = event.payload[key];
  return typeof value === 'string' ? value : null;
}

function number(event: TimelineEvent, key: string): number | null {
  const value = event.payload[key];
  return typeof value === 'number' ? value : null;
}

function boolean(event: TimelineEvent, key: string): boolean | null {
  const value = event.payload[key];
  return typeof value === 'boolean' ? value : null;
}

function snapshotFor(result: GameResult, event: TimelineEvent): WorldSnapshot | null {
  return (result.snapshots ?? []).find((snapshot) => snapshot.lastEventSeq === event.seq) ?? null;
}

function playerAt(result: GameResult, event: TimelineEvent, jersey: string | null): WorldSnapshot['players'][number] | null {
  if (!jersey) return null;
  return snapshotFor(result, event)?.players.find((player) => player.jersey === jersey) ?? null;
}

function context(events: readonly TimelineEvent[], index: number): Readonly<Record<string, unknown>> {
  const previous = events[index - 1];
  const next = events[index + 1];
  return {
    previous: previous ? { seq: previous.seq, type: previous.type, payload: previous.payload } : null,
    current: events[index] ? { seq: events[index]!.seq, type: events[index]!.type } : null,
    next: next ? { seq: next.seq, type: next.type, payload: next.payload } : null,
  };
}

function add(
  findings: RealityScanFinding[],
  result: GameResult,
  events: readonly TimelineEvent[],
  index: number,
  code: string,
  message: string,
  evidence: Readonly<Record<string, unknown>>,
  likelyRootCause: string,
  layer: RealityScanFinding['layer'] = 'causal',
): void {
  const event = events[index]!;
  findings.push({
    layer,
    code,
    severity: 'error',
    seed: result.meta.seed,
    seq: event.seq,
    message,
    evidence: { ...context(events, index), ...evidence },
    likelyRootCause,
  });
}

interface StreamState {
  holderId: string | null;
  possessionTeam: 'home' | 'away' | null;
  pendingPass: { passerId: string; receiverId: string; startedSeq: number } | null;
  pendingShot: { shooterId: string; shotValue: number; releasedSeq: number } | null;
  pendingFoulShots: { shooterId: string; attempts: number; nextAttempt: number } | null;
  pendingScreen: { screenerId: string; handlerId: string; setSeq: number } | null;
  requiresInbound: boolean;
  lastSteal: { stealerId: string; victimId: string; seq: number } | null;
  lastMiss: { shooterId: string; seq: number } | null;
}

function initialState(): StreamState {
  return {
    holderId: null,
    possessionTeam: null,
    pendingPass: null,
    pendingShot: null,
    pendingFoulShots: null,
    pendingScreen: null,
    requiresInbound: false,
    lastSteal: null,
    lastMiss: null,
  };
}

function ensureLiveAction(
  state: StreamState,
  findings: RealityScanFinding[],
  result: GameResult,
  events: readonly TimelineEvent[],
  index: number,
  action: TimelineEvent,
): void {
  if (state.requiresInbound) {
    add(findings, result, events, index, 'LIVE_ACTION_BEFORE_INBOUND',
      `${action.type} 在死球后没有先经过 INBOUND_START/INBOUND_TOUCH。`,
      { action: action.type, holderId: state.holderId },
      '死球结束没有进入明确的 inbound 状态，或 live action 被错误地沿用了上一回合。');
  }
}

export function scanEventStream(result: GameResult): RealityScanFinding[] {
  const findings: RealityScanFinding[] = [];
  const events = result.events;
  const state = initialState();
  let gameStarted = false;
  let firstPossession = false;
  for (let index = 0; index < events.length; index += 1) {
    const event = events[index]!;
    const prior = events[index - 1];
    const next = events[index + 1];
    const payloadTeam = event.payload['team'];
    const team = payloadTeam === 'home' || payloadTeam === 'away' ? payloadTeam : null;

    if (event.type === 'GAME_START') {
      if (gameStarted) add(findings, result, events, index, 'DUPLICATE_GAME_START', 'GAME_START 被重复发出。', {}, '初始化事件被放进了可重复执行的回合循环。', 'integrity');
      gameStarted = true;
    } else if (!gameStarted) {
      add(findings, result, events, index, 'EVENT_BEFORE_GAME_START', `${event.type} 出现在 GAME_START 之前。`, {}, '事件生成器绕过了统一的比赛生命周期入口。', 'integrity');
    }

    if (event.type === 'JUMP_BALL_TAP') {
      const openingTip = prior?.type === 'GAME_START';
      const overtimeTip = prior?.type === 'PERIOD_START' && event.period >= 5;
      if (firstPossession && !openingTip && !overtimeTip) add(findings, result, events, index, 'JUMP_BALL_OUT_OF_CONTEXT', '跳球事件不在开局或死球跳球上下文中。', { priorType: prior?.type, period: event.period }, '跳球事实没有被限制在合法的 dead-ball/jump-ball 状态。');
    }

    if (event.type === 'POSSESSION_GAINED') {
      const playerId = text(event, 'player_id');
      if (!playerId || !team) add(findings, result, events, index, 'POSSESSION_GAINED_MALFORMED', 'POSSESSION_GAINED 缺少合法球员或球队。', { playerId, team }, '持球权完成事实没有携带完整的控制者身份。');
      if (firstPossession && state.holderId === playerId && state.possessionTeam === team && !state.requiresInbound) {
        add(findings, result, events, index, 'DUPLICATE_POSSESSION_GAINED', '同一持球人和球队在未发生球权变化时重复获得持球权。', { playerId, team }, '球权事件被重复提交，或恢复事件与 POSSESSION_GAINED 重复表达同一事实。');
      }
      state.holderId = playerId;
      state.possessionTeam = team;
      state.requiresInbound = false;
      firstPossession = true;
      continue;
    }

    if (event.type === 'INBOUND_START') {
      if (state.pendingShot || state.pendingPass) add(findings, result, events, index, 'INBOUND_DURING_FLIGHT', '球仍在传球或投篮飞行中却开始发球。', {}, '死球/飞行球状态没有在事件边界处完成。');
      state.requiresInbound = true;
      // The inbounder holds the ball at the line — the following inbound
      // pass flight starts from their hands, so the replay tracks them
      // as the holder until the receiver touches the ball.
      const inbounder = text(event, 'inbounder_id');
      if (inbounder) state.holderId = inbounder;
      continue;
    }

    if (event.type === 'INBOUND_TOUCH') {
      const receiver = text(event, 'receiver_id');
      // Real-pass inbound: INBOUND_START → PASS(flight_start) →
      // PASS(flight_complete) → INBOUND_TOUCH. The touch must be preceded
      // by the matching inbound start, possibly with the flight between.
      const startOk = prior?.type === 'INBOUND_START'
        || (prior?.type === 'PASS'
          && text(prior, 'note') === 'flight_complete'
          && state.requiresInbound);
      if (!startOk) add(findings, result, events, index, 'INBOUND_TOUCH_WITHOUT_START', 'INBOUND_TOUCH 没有紧邻对应的 INBOUND_START。', { receiver }, '发球生命周期被拆散，或事件顺序在状态层之外被重排。');
      state.holderId = receiver;
      state.requiresInbound = true;
      continue;
    }

    if (event.type === 'PASS') {
      const passer = text(event, 'passer_id');
      const receiver = text(event, 'receiver_id');
      const note = text(event, 'note');
      if (!passer || !receiver || passer === receiver) add(findings, result, events, index, 'PASS_INVALID_ACTORS', '传球双方身份缺失或相同。', { passer, receiver }, '传球意图没有经过合法的队友目标选择。');
      if (note === 'flight_start') {
        if (state.pendingPass) add(findings, result, events, index, 'PASS_OVERLAP', '上一传球尚未完成就开始下一次传球。', { pending: state.pendingPass }, '传球完成事实未提交，决策层却重新取得了持球权。');
        if (state.holderId !== passer) add(findings, result, events, index, 'PASSER_NOT_HOLDER', '传球发起人不是当前持球人。', { passer, holderId: state.holderId }, '球权状态与传球动作决策脱节。');
        state.pendingPass = { passerId: passer!, receiverId: receiver!, startedSeq: event.seq };
        state.holderId = null;
      } else if (note === 'flight_complete') {
        const pending = state.pendingPass;
        if (!pending || pending.passerId !== passer || pending.receiverId !== receiver) add(findings, result, events, index, 'PASS_COMPLETE_WITHOUT_START', '传球完成事件没有对应的同一传球发起事件。', { passer, receiver, pending }, '传球事件同时承担开始和完成语义，但没有稳定的飞行状态。');
        state.pendingPass = null;
        state.holderId = receiver;
      }
      continue;
    }

    if (event.type === 'HANDOFF') {
      const giver = text(event, 'giver_id');
      const receiver = text(event, 'receiver_id');
      if (!giver || !receiver || giver === receiver) add(findings, result, events, index, 'HANDOFF_INVALID_ACTORS', '手递手双方身份缺失或相同。', { giver, receiver }, '手递手意图没有经过合法的队友目标选择。');
      if (state.holderId !== giver) add(findings, result, events, index, 'HANDOFF_GIVER_NOT_HOLDER', '手递手给球人不是当前持球人。', { giver, holderId: state.holderId }, '球权状态没有在手递手开始前锁定。');
      state.holderId = receiver;
      continue;
    }

    if (event.type === 'ADVANCE_BACKCOURT' || event.type === 'CROSS_HALF' || event.type === 'DRIVE') {
      ensureLiveAction(state, findings, result, events, index, event);
      const handler = text(event, 'ballHandlerId');
      if (!handler || handler !== state.holderId) add(findings, result, events, index, `${event.type}_NOT_HOLDER`, `${event.type} 的持球人不是事件流当前 holder。`, { handler, holderId: state.holderId }, '动作决策使用了过期 holder 或球权变化没有向意图层传播。');
      const player = playerAt(result, event, handler);
      if (player && !player.hasBall) add(findings, result, events, index, `${event.type}_PLAYER_NOT_HOLDING`, `${event.type} 发生时快照中的球员没有球。`, { handler, action: player.action, x: player.x, y: player.y }, '语义动作事件早于实体持球完成，或快照提交顺序错误。', 'spatial');
      continue;
    }

    if (event.type === 'SCREEN_SET') {
      const screener = text(event, 'screener_id');
      const handler = text(event, 'ballHandlerId') ?? text(event, 'screen_target_id');
      const handlerPlayer = playerAt(result, event, handler);
      const screenerPlayer = playerAt(result, event, screener);
      if (!screener || screener === handler || (handler && handler === state.holderId && screenerPlayer?.team === handlerPlayer?.team)) {
        if (!screener || screener === handler) add(findings, result, events, index, 'SCREEN_SET_INVALID_PAIR', '掩护人与目标身份缺失或相同。', { screener, handler }, '掩护意图没有绑定两个不同的进攻球员。');
      }
      if (handler && state.holderId !== handler) add(findings, result, events, index, 'SCREEN_HANDLER_NOT_HOLDER', '挡拆目标不是事件流当前持球人。', { handler, holderId: state.holderId }, '战术计划使用了过期的 ball handler。');
      if (screenerPlayer && handlerPlayer && screenerPlayer.team !== handlerPlayer.team) add(findings, result, events, index, 'SCREEN_CROSS_TEAM', '掩护人与持球人不是同队。', { screener, handler, screenerTeam: screenerPlayer.team, handlerTeam: handlerPlayer.team }, '进攻掩护和防守匹配被错误地混合到同一个执行计划。');
      if (handler) state.pendingScreen = { screenerId: screener!, handlerId: handler, setSeq: event.seq };
      continue;
    }

    if (event.type === 'SCREEN_USE') {
      const screener = text(event, 'screener_id');
      const handler = text(event, 'ballHandlerId');
      const screen = state.pendingScreen;
      if (!screen || screen.screenerId !== screener || screen.handlerId !== handler) add(findings, result, events, index, 'SCREEN_USE_WITHOUT_CONTEXT', 'SCREEN_USE 没有对应当前回合和同一对球员的 SCREEN_SET。', { screener, handler, pendingScreen: screen }, '挡拆使用事件没有消费实际已建立的掩护状态。', 'tactical');
      state.pendingScreen = null;
      continue;
    }

    if (event.type === 'SHOT_RELEASE') {
      const shooter = text(event, 'shooter_id');
      const shotValue = number(event, 'shot_value');
      if (state.pendingShot) add(findings, result, events, index, 'SHOT_RELEASE_OVERLAP', '上一投尚未结算就再次出手。', { pendingShot: state.pendingShot }, '投篮飞行状态被新的决策覆盖。');
      if (!shooter || shooter !== state.holderId) add(findings, result, events, index, 'SHOT_SHOOTER_NOT_HOLDER', '出手球员不是当前持球人。', { shooter, holderId: state.holderId }, '出手事件与球权状态脱节。');
      if (shotValue !== 2 && shotValue !== 3) add(findings, result, events, index, 'SHOT_VALUE_INVALID', '出手分值不是 2 或 3。', { shotValue }, '投篮区域到 shot value 的映射产生了非法值。');
      state.pendingShot = { shooterId: shooter ?? '?', shotValue: shotValue ?? 0, releasedSeq: event.seq };
      continue;
    }

    if (event.type === 'SHOT_RESULT') {
      const shooter = text(event, 'shooter_id');
      const shotValue = number(event, 'shot_value');
      const made = boolean(event, 'made');
      if (!state.pendingShot || state.pendingShot.shooterId !== shooter || state.pendingShot.shotValue !== shotValue) add(findings, result, events, index, 'SHOT_RESULT_WITHOUT_RELEASE', '投篮结果没有匹配同一球员和分值的 SHOT_RELEASE。', { shooter, shotValue, pendingShot: state.pendingShot }, '投篮完成事件丢失、重复或引用了错误的球权上下文。');
      if (made === null) add(findings, result, events, index, 'SHOT_RESULT_MISSING_MADE', 'SHOT_RESULT 缺少 made 布尔值。', {}, '结果解析边界没有保证投篮结果字段完整。');
      if (made === true && next?.type !== 'MADE_BASKET_DEAD' && next?.type !== 'FOUL') add(findings, result, events, index, 'MADE_SHOT_NOT_TERMINATED', '命中后没有立即进入得分死球或 and-one 犯规处理。', { nextType: next?.type }, '命中结果没有驱动合法的回合终结状态。');
      if (made === false) state.lastMiss = { shooterId: shooter ?? '?', seq: event.seq };
      state.pendingShot = null;
      state.holderId = null;
      continue;
    }

    if (event.type === 'REBOUND') {
      const rebounder = text(event, 'rebounder_id');
      const offensive = boolean(event, 'offensive');
      const validContext = state.lastMiss !== null || prior?.type === 'FT_SEQUENCE_END';
      if (!validContext) add(findings, result, events, index, 'REBOUND_WITHOUT_MISS', '篮板事件没有对应先前的投篮不中或罚球序列结束。', { rebounder, offensive }, '篮板结果脱离了投篮结果，或罚球后篮板未被纳入事件上下文。');
      if (offensive === null || !rebounder) add(findings, result, events, index, 'REBOUND_MALFORMED', '篮板事件缺少 rebounder 或 offensive。', { rebounder, offensive }, '篮板完成事实没有携带完整的归属信息。');
      state.holderId = rebounder;
      state.possessionTeam = team;
      state.lastMiss = null;
      state.requiresInbound = false;
      continue;
    }

    if (event.type === 'LOOSE_BALL_RECOVER') {
      const recoverer = text(event, 'recoverer_id');
      if (!recoverer || !team) add(findings, result, events, index, 'LOOSE_RECOVER_MALFORMED', '松球恢复缺少球员或球队。', { recoverer, team }, '松球完成事实没有携带控制者身份。');
      if (state.lastMiss) state.lastMiss = null;
      state.holderId = recoverer;
      state.possessionTeam = team;
      state.requiresInbound = false;
      continue;
    }

    if (event.type === 'STEAL') {
      const stealer = text(event, 'stealer_id');
      const victim = text(event, 'victim_id');
      const midPass = boolean(event, 'mid_pass') === true;
      // A mid-pass steal consumes the in-flight pass before awarding the
      // turnover to the interceptor. The candidate victim is the receiver.
      if (midPass && state.pendingPass && state.pendingPass.receiverId === victim) {
        state.pendingPass = null;
      }
      if (!stealer || !victim || stealer === victim) add(findings, result, events, index, 'STEAL_INVALID_ACTORS', '抢断双方身份缺失或相同。', { stealer, victim }, '抢断候选没有绑定有效的攻防双方。');
      if (!midPass && state.holderId !== victim) add(findings, result, events, index, 'STEAL_VICTIM_NOT_HOLDER', '被抢断球员不是事件流当前持球人。', { victim, holderId: state.holderId }, '抢断候选使用了过期持球人。');
      state.lastSteal = { stealerId: stealer ?? '?', victimId: victim ?? '?', seq: event.seq };
      state.holderId = stealer;
      continue;
    }

    if (event.type === 'TURNOVER') {
      const player = text(event, 'player_id');
      const stealer = text(event, 'stealer_id');
      if (stealer && (!state.lastSteal || state.lastSteal.stealerId !== stealer || state.lastSteal.victimId !== player)) add(findings, result, events, index, 'TURNOVER_STEAL_MISMATCH', '带 stealer_id 的失误没有匹配同一抢断事实。', { player, stealer, lastSteal: state.lastSteal }, '抢断和失误由不同状态路径分别提交，导致事件因果断裂。');
      const stealerPlayer = playerAt(result, event, stealer);
      state.holderId = stealer;
      state.possessionTeam = stealerPlayer?.team ?? state.possessionTeam;
      state.pendingPass = null;
      state.pendingShot = null;
      state.requiresInbound = false;
      state.lastSteal = null;
      continue;
    }

    if (event.type === 'FOUL') {
      const shooting = boolean(event, 'shooting');
      const attempts = number(event, 'free_throws_awarded');
      if (shooting === true && (attempts === null || attempts < 1)) add(findings, result, events, index, 'SHOOTING_FOUL_NO_FT', 'shooting foul 没有产生合法罚球数。', { attempts }, '犯规结果和罚球序列之间的规则映射缺失。');
      // Any foul that awards FTs (shooting OR non-shooting bonus/intentional)
      // seeds the FT sequence expectation. The FT_START handler verifies the
      // shooter/attempts match regardless of foul type.
      if (attempts && attempts > 0) state.pendingFoulShots = { shooterId: text(event, 'victim_id') ?? '?', attempts, nextAttempt: 1 };
      continue;
    }

    if (event.type === 'FT_START') {
      const shooter = text(event, 'shooter_id');
      const attempts = number(event, 'attempts');
      // The foul carries the provisional 2/3 FTs (whistled at release); a
      // made shot settles to an and-one 1-FT sequence. Accept both.
      const ftaMatches = state.pendingFoulShots !== null
        && state.pendingFoulShots.shooterId === shooter
        && (state.pendingFoulShots.attempts === attempts
          || (attempts === 1 && state.pendingFoulShots.attempts >= 2));
      if (!state.pendingFoulShots || !ftaMatches) add(findings, result, events, index, 'FT_START_WITHOUT_FOUL', 'FT_START 没有对应同一球员和次数的 shooting foul。', { shooter, attempts, pendingFoulShots: state.pendingFoulShots }, '罚球序列被独立启动，未由犯规完成事实驱动。');
      continue;
    }

    if (event.type === 'FT_ATTEMPT' || event.type === 'FT_RESULT') {
      const shooter = text(event, 'shooter_id');
      const attempt = number(event, 'attempt_number');
      const expected = state.pendingFoulShots?.nextAttempt ?? null;
      if (!state.pendingFoulShots || shooter !== state.pendingFoulShots.shooterId || attempt !== expected) add(findings, result, events, index, 'FT_ATTEMPT_SEQUENCE_INVALID', `${event.type} 不符合当前罚球序列。`, { shooter, attempt, expected, pendingFoulShots: state.pendingFoulShots }, '罚球 attempt 状态没有对应当前罚球序列。');
      if (event.type === 'FT_RESULT') {
        const made = boolean(event, 'made');
        if (made === false) state.lastMiss = { shooterId: shooter ?? '?', seq: event.seq };
        if (state.pendingFoulShots) state.pendingFoulShots = { ...state.pendingFoulShots, nextAttempt: Math.min(state.pendingFoulShots.attempts + 1, state.pendingFoulShots.nextAttempt + 1) };
      }
      continue;
    }

    if (event.type === 'FT_SEQUENCE_END') {
      const expected = state.pendingFoulShots?.attempts ?? null;
      const nextAttempt = state.pendingFoulShots?.nextAttempt ?? null;
      if (expected !== null && (nextAttempt === null || nextAttempt < 2)) add(findings, result, events, index, 'FT_SEQUENCE_WITHOUT_ATTEMPT', '罚球序列结束前没有完成任何合法罚球结果。', { expected, nextAttempt }, '罚球序列结束条件没有消费罚球动作。');
      state.pendingFoulShots = null;
      state.requiresInbound = false;
      continue;
    }

    if (event.type === 'MADE_BASKET_DEAD') {
      if (prior?.type !== 'SHOT_RESULT' && prior?.type !== 'FT_RESULT') add(findings, result, events, index, 'MADE_BASKET_WITHOUT_SCORE', '得分死球没有紧邻命中结果。', { priorType: prior?.type }, '得分死球事件脱离了实际得分完成点。');
      state.requiresInbound = true;
      continue;
    }

    if (event.type === 'PERIOD_END' || event.type === 'GAME_END') {
      // A period boundary legally truncates an in-flight possession. The
      // next period starts from a dead ball, so no pass/shot/FT lifecycle may
      // leak across the boundary into the following inbound.
      state.pendingPass = null;
      state.pendingShot = null;
      state.pendingFoulShots = null;
      state.pendingScreen = null;
      state.holderId = null;
      state.requiresInbound = true;
    }
  }
  return findings;
}
