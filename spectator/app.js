/**
 * Spectator web shell — plays the 0.1s match stream from SpectatorPackage.
 * v0.7: Added action animations (pass arcs, shot flashes, steal bursts, etc.)
 */

/** @typedef {object} StreamTick
 * @property {number} t
 * @property {number} period
 * @property {number} gameClock
 * @property {number} shotClock
 * @property {{home:number,away:number}} score
 * @property {{jersey:string,team:string,x:number,y:number,hasBall:boolean,task?:string,action?:string}[]} players
 * @property {{x:number,y:number}} ball
 * @property {object|undefined} tactical
 * @property {number|null} keyframeIndex
 * @property {string|null} eventType
 * @property {Object<string, unknown>|undefined} eventPayload
 * @property {string|null} callout
 * @property {'low'|'mid'|'high'|null} intensity
 */

/**
 * @typedef {object} SpectatorPackage
 * @property {{ foundation_version: string, seed: number, home_team_id: string, away_team_id: string, home_name?: string, away_name?: string, stream_dt?: number }} meta
 * @property {object[]} frames
 * @property {{ dt: number, tickCount: number, duration: number, ticks: StreamTick[] }} [stream]
 * @property {number[]} broadcastEventSeqs
 * @property {number} event_count
 */

const $ = (id) => {
  const el = document.getElementById(id);
  if (!el) throw new Error(`missing #${id}`);
  return el;
};

const canvas = /** @type {HTMLCanvasElement} */ ($('court'));
const ctx = canvas.getContext('2d');
if (!ctx) throw new Error('2d context unavailable');

/** @type {SpectatorPackage | null} */
let pack = null;
/** @type {StreamTick[]} */
let ticks = [];
let index = 0;
let playing = false;
let timer = 0;
let lastCallout = '—';
/** 最近一次渲染的比分（用于检测得分变化触发脉冲） */
let lastScore = null;
/** 节起始得分缓存：period -> {home, away}，首次扫描后复用 */
const periodStartCache = new Map();
/** URL hash 恢复是否已完成(完成前 render 不写 hash,避免覆盖 #t=N) */
let hashRestored = false;
/** 上次写入 URL hash 的帧号 */
let lastUrlIndex = null;
/** Per-jersey body table from meta.player_bodies (h_cm/wt_kg) for icons. */
let playerBodies = {};
const reduceMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;

// ─── Animation System ───────────────────────────────────────────────────────
/** @type {Animation[]} */
let activeAnims = [];

/**
 * @typedef {object} Animation
 * @property {number} startT    — realClock when animation starts
 * @property {number} duration  — seconds
 * @property {string} type      — transient visual effect type
 * @property {object} data      — type-specific data (positions, jerseys, etc.)
 */

const JUMP_ACTIONS = new Set(['jump', 'shoot', 'shoot_ft']);
const SPRINT_ACTIONS = new Set(['advance', 'drive', 'cut']);
const SPRINT_THRESHOLD_FPS = 14;

// ─── 镜头系统（F）────────────────────────────────────────────────────────────
// 事件触发时短暂聚焦（缩放+平移），结束后 easeOutCubic 恢复 (0,0,1)。
// 简化方案：镜头激活期间（scale !== 1）禁用 hit-test。
let cam = { x: 0, y: 0, scale: 1 };
let camTarget = null; // {x, y, scale} 归一化焦点与目标缩放
let camEndT = 0;      // stream 时间（tick.t），到达后开始恢复
let camBase = null;   // 恢复起点（快照缩放开始时的镜头状态）

const CAM_RECOVER_DURATION = 0.9;
/** 归一化镜头偏移 → 逻辑像素偏移（F）：用插值后的 cam 偏移（-1..1） */
function camFocusPx(w, h) {
  // cam.x/cam.y 已经是 -1..1 的归一化偏移（目标为 (t-0.5)*2），
  // 直接映射到像素。之前误用 camTarget 导致保持阶段瞬跳。
  return { x: cam.x * ((w - 16) / 2), y: cam.y * ((h - 16) / 2) };
}

/**
 * 触发镜头聚焦。camEndT 之前保持目标缩放/焦点；之后 easeOutCubic 恢复。
 * reduceMotion 下禁用（保持 scale 1）。
 * @param {number} now stream 时间
 * @param {{x:number, y:number, scale:number}} target 归一化焦点与缩放
 * @param {number} holdT 保持秒数
 */
function setCameraFocus(now, target, holdT) {
  if (reduceMotion) return;
  if (!camTarget) camBase = { ...cam };
  camTarget = { ...target };
  camEndT = now + holdT;
}

/** 每帧推进镜头状态（now = tick.t，stream 时间） */
function updateCamera(now) {
  if (reduceMotion || !camTarget) return;
  if (now >= camEndT + CAM_RECOVER_DURATION) {
    cam = { x: 0, y: 0, scale: 1 };
    camTarget = null;
    camBase = null;
    return;
  }
  if (now >= camEndT) {
    // 恢复阶段：从 camBase 插值回 (0,0,1)
    const p = Math.min(1, (now - camEndT) / CAM_RECOVER_DURATION);
    const e = easeOutCubic(p);
    cam = {
      x: camBase.x * (1 - e),
      y: camBase.y * (1 - e),
      scale: camBase.scale + (1 - camBase.scale) * e,
    };
    return;
  }
  // 保持阶段：平滑逼近目标（camBase 已快照，避免每次重设导致抖动）
  if (!camBase) camBase = { x: 0, y: 0, scale: 1 };
  const t = Math.min(1, 0.35);
  const c = 1 - t;
  const targetX = (camTarget.x - 0.5) * 2, targetY = (camTarget.y - 0.5) * 2;
  cam = {
    x: cam.x * c + targetX * t,
    y: cam.y * c + targetY * t,
    scale: cam.scale * c + camTarget.scale * t,
  };
}

function animProgress(animation, now) {
  return Math.max(0, Math.min(1, (now - animation.startT) / animation.duration));
}

function easeOutCubic(t) {
  return 1 - Math.pow(1 - t, 3);
}

/**
 * 动画时间缩放（G）：duration 是墙钟秒数、startT 是 stream 秒数，播放进度按
 * (now - startT)/duration 计算——乘以该因子即把动画按回放速度等比压缩。
 * dt 取自 pack.meta.stream_dt / pack.stream.dt，无 pack 时按 1×。
 */
function animTimeScale() {
  const dt = pack?.meta?.stream_dt ?? pack?.stream?.dt ?? 0.1;
  return Math.max(0.5, Math.min(4, dt / 0.1));
}

/** 篮球接缝弧（C）：沿球体画两条正交接缝线（cross 弧线） */
function drawBallSeams(x, y, r, rotate = 0) {
  ctx.save();
  ctx.strokeStyle = 'rgba(90, 45, 15, 0.5)';
  ctx.lineWidth = Math.max(0.8, r * 0.16);
  ctx.beginPath();
  ctx.arc(x, y, r * 0.86, -0.9 + rotate, 0.9 + rotate);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(x, y, r * 0.86, Math.PI - 0.9 + rotate, Math.PI + 0.9 + rotate);
  ctx.stroke();
  ctx.beginPath();
  ctx.ellipse(x, y, r * 0.62, r * 0.28, rotate, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

function jumpLiftFor(jersey, now) {
  let lift = 0;
  for (const animation of activeAnims) {
    if (animation.type !== 'jump' || animation.data.jersey !== jersey) continue;
    const p = animProgress(animation, now);
    lift = Math.max(lift, Math.sin(Math.PI * p) ** 0.72 * (animation.data.maxLift ?? 10));
  }
  return lift;
}

function spawnJump(jersey, player, now, maxLift = 10, duration = 0.62) {
  if (!player) return;
  activeAnims.push({
    startT: now,
    duration: duration * animTimeScale(),
    type: 'jump',
    data: { jersey, x: player.x, y: player.y, maxLift },
  });
}

function shotArcHeight(fromX, toX) {
  const distance = Math.abs(toX - fromX);
  return Math.max(34, Math.min(92, 30 + distance * 95));
}

function rimForShot(tick, fromX) {
  const payload = tick.eventPayload ?? {};
  if (Number.isFinite(Number(payload.target_x)) && Number.isFinite(Number(payload.target_y))) {
    return { x: Number(payload.target_x), y: Number(payload.target_y) };
  }
  const right = fromX >= 0.5;
  const spec = courtSpec?.[right ? 'right' : 'left'];
  return spec?.rim ?? { x: right ? 0.944 : 0.056, y: 0.5 };
}

function shotPoint(animation, progress, w, h, pad) {
  const fx = px(animation.data.fromX, w, pad);
  const fy = px(animation.data.fromY, h, pad);
  const tx = px(animation.data.toX, w, pad);
  const ty = px(animation.data.toY, h, pad);
  // Top-down projection: the ground track is a straight segment. Only the
  // derived lift is curved, which is how a real ballistic arc appears from
  // above rather than as a sideways quadratic curve.
  const x = fx + (tx - fx) * progress;
  const y = fy + (ty - fy) * progress;
  const lift = animation.data.peak * (4 * progress * (1 - progress));
  return { x, y, groundX: x, groundY: y, lift };
}

function playerByJersey(tick, jersey) {
  return (tick?.players ?? []).find((player) => player.jersey === jersey) ?? null;
}

function jumpersForEvent(tick, prev, eventType) {
  const jumpers = [];
  if (eventType === 'JUMP_BALL_TAP') {
    for (const player of tick.players ?? []) {
      if (player.action === 'jump' || player.task === 'jump') jumpers.push(player);
    }
  } else if (eventType === 'SHOT_RELEASE' || eventType === 'FT_ATTEMPT') {
    const shooter = (prev?.players ?? []).find((player) => player.hasBall)
      ?? (tick.players ?? []).find((player) => player.hasBall)
      ?? (tick.players ?? []).find((player) => JUMP_ACTIONS.has(player.action));
    if (shooter) jumpers.push(shooter);
  } else if (eventType === 'REBOUND') {
    const rebounder = (tick.players ?? []).find((player) => player.hasBall)
      ?? (prev?.players ?? []).find((player) => player.hasBall);
    if (rebounder) jumpers.push(rebounder);
  }
  return jumpers;
}

function spawnMovementEffects(tick, prev, eventType = null) {
  if (!prev || reduceMotion) return;
  const now = tick.t ?? 0;
  const dt = Math.max(0.001, (tick.t ?? 0) - (prev.t ?? 0));
  for (const player of tick.players ?? []) {
    const before = playerByJersey(prev, player.jersey);
    if (!before) continue;
    const distanceFt = Math.hypot((player.x - before.x) * 94, (player.y - before.y) * 50);
    const speedFps = distanceFt / dt;
    const action = player.action ?? player.task ?? '';
    if (speedFps >= SPRINT_THRESHOLD_FPS && SPRINT_ACTIONS.has(action)) {
      activeAnims.push({
        startT: now,
        duration: Math.min(0.32, Math.max(0.16, dt * 1.6)) * animTimeScale(),
        type: 'sprint',
        data: {
          jersey: player.jersey,
          fromX: before.x,
          fromY: before.y,
          toX: player.x,
          toY: player.y,
          speedFps,
        },
      });
    }
  }

  for (const player of jumpersForEvent(tick, prev, eventType)) {
    const maxLift = eventType === 'JUMP_BALL_TAP' ? 15 : eventType === 'REBOUND' ? 11 : 10;
    spawnJump(player.jersey, player, now, maxLift, eventType === 'JUMP_BALL_TAP' ? 0.9 : 0.62);
  }
}


/**
 * @param {StreamTick} tick
 * @param {StreamTick|null} prev
 */
function inferredEventType(tick, prev) {
  if (prev?.phase === 'JUMP_BALL' && tick.phase === 'LIVE') return 'JUMP_BALL_TAP';
  if (tick.phase === 'JUMP_BALL' && (!tick.eventType || tick.eventType === 'GAME_START')) return 'JUMP_BALL_SETUP';
  if (!prev && tick.phase === 'LIVE' && (tick.t ?? 0) <= 0.5
      && (tick.players ?? []).some((player) => player.action === 'jump' || player.task === 'jump')) {
    return 'JUMP_BALL_TAP';
  }
  if (tick.eventType) return tick.eventType;
  if (!prev) return null;
  if (prev.ball?.status === 'held' && tick.ball?.status === 'shot') return 'SHOT_RELEASE';
  if (prev.ball?.status === 'shot' && tick.ball?.status === 'loose') return 'SHOT_RESULT';
  if (prev.ball?.status === 'held' && tick.ball?.status === 'pass') return 'PASS';
  return null;
}

/** @param {StreamTick} tick @param {StreamTick|null} prev */
function spawnAnimations(tick, prev) {
  const et = inferredEventType(tick, prev);
  if (!prev) {
    const jumpers = (tick.players ?? []).filter((player) =>
      player.action === 'jump' || player.task === 'jump');
    if (jumpers.length && (tick.t ?? 0) <= 1.0) {
      const maxLift = tick.phase === 'JUMP_BALL' ? 12 : 15;
      const duration = tick.phase === 'JUMP_BALL' ? 0.9 : 0.9;
      for (const player of jumpers) spawnJump(player.jersey, player, 0, maxLift, duration);
    }
    return;
  }
  if (!et) {
    spawnMovementEffects(tick, prev, et);
    return;
  }
  spawnMovementEffects(tick, prev, et);
  const now = tick.t ?? 0;
  const players = tick.players ?? [];

  // PASS: only the flight-start event draws an arc. The completion marker
  // confirms receipt and must not create a second, misleading pass animation.
  // PASS lifecycle is event-driven: flight_start creates one keyed route;
  // flight_complete removes it. A wall-clock timeout remains only as a
  // safety valve for truncated streams and cannot leave stale routes behind.
  if (et === 'PASS' && tick.eventPayload?.note === 'flight_complete') {
    activeAnims = activeAnims.filter((animation) => animation.type !== 'pass');
  }
  if (et === 'PASS' && tick.eventPayload?.note === 'flight_start') {
    const fromX = prev.ball?.x ?? 0.5, fromY = prev.ball?.y ?? 0.5;
    const receiver = tick.eventPayload?.receiver_id
      ? players.find((p) => p.jersey === String(tick.eventPayload.receiver_id))
      : null;
    const toX = receiver?.x ?? tick.ball?.x ?? 0.5;
    const toY = receiver?.y ?? tick.ball?.y ?? 0.5;
    activeAnims = activeAnims.filter((animation) => animation.type !== 'pass');
    activeAnims.push({
      startT: now,
      duration: 0.9 * animTimeScale(),
      type: 'pass',
      data: { fromX, fromY, toX, toY },
    });
  }
  // SHOT_RELEASE: launch a visual 3-D arc. Kernel x/y remains authoritative;
  // the canvas derives height and perspective only for presentation.
  if (et === 'SHOT_RELEASE') {
    const shooter = (prev?.players ?? []).find((player) => player.hasBall)
      ?? players.find((player) => player.hasBall);
    const fromX = shooter?.x ?? tick.ball?.x ?? 0.5;
    const fromY = shooter?.y ?? tick.ball?.y ?? 0.5;
    const rim = rimForShot(tick, fromX);
    activeAnims = activeAnims.filter((animation) => animation.type !== 'shotFlight');
    activeAnims.push({
      startT: now,
      duration: Math.max(0.55, Math.min(1.15, Number(tick.eventPayload?.flight_seconds) || 0.78)) * animTimeScale(),
      type: 'shotFlight',
      data: { fromX, fromY, toX: rim.x, toY: rim.y, peak: shotArcHeight(fromX, rim.x) },
    });
    activeAnims.push({ startT: now, duration: 0.42 * animTimeScale(), type: 'shot', data: { x: fromX, y: fromY } });
  }

  // SHOT_RESULT closes the flight at the rim and adds a distinct net/rim cue.
  if (et === 'SHOT_RESULT') {
    activeAnims = activeAnims.filter((animation) => animation.type !== 'shotFlight');
    const made = (tick.score?.home ?? 0) > (prev.score?.home ?? 0) || (tick.score?.away ?? 0) > (prev.score?.away ?? 0);
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 1.0 * animTimeScale(), type: made ? 'make' : 'miss', data: { x: ballX, y: ballY } });
    // 网动爆点（K）：made 时由渲染层触发 DOM 元素（数据层负责 DOM 得分浮动）
    if (made) window.burstAtRim(ballX, ballY);
  }

  // STEAL: burst effect at steal location
  if (et === 'STEAL') {
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 0.6 * animTimeScale(), type: 'steal', data: { x: ballX, y: ballY } });
    setCameraFocus(now, { x: ballX, y: ballY, scale: 1.3 }, 0.8);
  }

  // TURNOVER: flash at the actual loose-ball location.
  if (et === 'TURNOVER') {
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 0.5 * animTimeScale(), type: 'turnover', data: { x: ballX, y: ballY } });
    setCameraFocus(now, { x: ballX, y: ballY, scale: 1.3 }, 0.8);
  }

  // LOOSE_BALL_RECOVER is a separate physical event. Its ring is anchored at
  // the recoverer, not at the stale turnover location.
  if (et === 'LOOSE_BALL_RECOVER') {
    const recoverer = players.find((p) => p.hasBall)
      ?? players.find((p) => p.jersey === tick.ball?.holderId);
    const x = recoverer?.x ?? tick.ball?.x ?? 0.5;
    const y = recoverer?.y ?? tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 0.7 * animTimeScale(), type: 'rebound', data: { x, y } });
  }

  // REBOUND: ring at rebound location
  if (et === 'REBOUND') {
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 0.7 * animTimeScale(), type: 'rebound', data: { x: ballX, y: ballY } });
  }

  // FOUL: yellow flash
  if (et === 'FOUL') {
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 0.8 * animTimeScale(), type: 'foul', data: { x: ballX, y: ballY } });
  }

  if (et === 'SCREEN_SET' || et === 'SCREEN_USE') {
    const screenerId = String(tick.eventPayload?.screener_id ?? '');
    const screener = players.find((p) => p.jersey === screenerId)
      ?? players.find((p) => p.action === 'screen' || p.task === 'screen');
    if (screener) {
      activeAnims.push({
        startT: now,
        duration: (et === 'SCREEN_SET' ? 1.0 : 0.65) * animTimeScale(),
        type: 'screen',
        // 锚定 jersey：绘制时取球员当前位置，特效跟着人走（加速播放时
        // 球员已移动，固定坐标的特效会留在原地）。
        data: { jersey: screener.jersey, x: screener.x, y: screener.y },
      });
    }
  }

  // DRIVE: motion lines at handler
  if (et === 'DRIVE') {
    const holder = players.find(p => p.hasBall);
    if (holder) {
      activeAnims.push({ startT: now, duration: 0.8 * animTimeScale(), type: 'drive', data: { jersey: holder.jersey, x: holder.x, y: holder.y } });
    }
  }

  // FT_ATTEMPT: free throw indicator
  if (et === 'FT_ATTEMPT') {
    const holder = players.find(p => p.hasBall) ?? players[0];
    if (holder) {
      activeAnims.push({ startT: now, duration: 1.2 * animTimeScale(), type: 'ft', data: { x: holder.x, y: holder.y } });
    }
  }

  // MADE_BASKET_DEAD: score burst + 网动爆点（K）。该帧同时是 made 的
  // SHOT_RESULT（ball shot→loose 且 score 变化），此处补触发。
  // 注意：进球后是死球，球员要步行回发球位置——镜头聚焦会在逐帧
  // 步进/暂停/回跳时停在中间状态，把左半场的球员"推"到画面外，
  // 看起来像全体瞬移。进球后保持全景，让回位走位全程可见。
  if (et === 'MADE_BASKET_DEAD') {
    const ballX = tick.ball?.x ?? 0.5, ballY = tick.ball?.y ?? 0.5;
    activeAnims.push({ startT: now, duration: 1.5 * animTimeScale(), type: 'score', data: { x: ballX, y: ballY, home: tick.score, prevHome: prev.score } });
    window.burstAtRim(ballX, ballY);
  }
}

/**
 * @param {number} now current stream time in seconds
 */
function drawAnimations(w, h, pad, now) {
  activeAnims = activeAnims.filter((a) => {
    const elapsed = now - a.startT;
    if (elapsed > a.duration) return false;
    const progress = Math.max(0, Math.min(1, elapsed / a.duration));
    // Follow the player: if the animation is anchored to a jersey, draw at
    // the player's CURRENT position — a fixed spawn coordinate leaves the
    // effect behind once the player moves (visible when playing fast).
    let fx = a.data.x ?? a.data.toX ?? 0.5;
    let fy = a.data.y ?? a.data.fromY ?? 0.5;
    if (a.data.jersey) {
      const live = (ticks[index]?.players ?? []).find((p) => p.jersey === a.data.jersey);
      if (live) {
        fx = live.x;
        fy = live.y;
      }
    }
    const cx = px(fx, w, pad);
    const cy = px(fy, h, pad);

    switch (a.type) {
      case 'jump':
        break;
      case 'sprint': {
        const fx = px(a.data.fromX, w, pad), fy = px(a.data.fromY, h, pad);
        const tx = px(a.data.toX, w, pad), ty = px(a.data.toY, h, pad);
        ctx.save();
        ctx.globalAlpha = (1 - progress) * 0.28;
        ctx.strokeStyle = '#5a9df0';
        ctx.lineWidth = 1.5;
        ctx.lineCap = 'round';
        for (let line = 0; line < 3; line += 1) {
          const offset = (line - 1) * 3;
          ctx.beginPath();
          ctx.moveTo(fx - offset, fy + offset);
          ctx.lineTo(fx + (tx - fx) * (0.45 + progress * 0.35) - offset,
            fy + (ty - fy) * (0.45 + progress * 0.35) + offset);
          ctx.stroke();
        }
        ctx.restore();
        break;
      }
      case 'pass': {
        const fx = px(a.data.fromX, w, pad), fy = px(a.data.fromY, h, pad);
        const tx = px(a.data.toX, w, pad), ty = px(a.data.toY, h, pad);
        const arcH = 30 * (1 - Math.abs(progress * 2 - 1));
        ctx.save();
        ctx.globalAlpha = 1 - progress * 0.5;
        ctx.strokeStyle = '#c98a00';
        ctx.lineWidth = 2;
        ctx.setLineDash([4, 3]);
        ctx.beginPath();
        ctx.moveTo(fx, fy);
        ctx.quadraticCurveTo((fx + tx) / 2, (fy + ty) / 2 - arcH, tx, ty);
        ctx.stroke();
        ctx.restore();
        break;
      }
      case 'shotFlight': {
        const point = shotPoint(a, progress, w, h, pad);
        const radius = 4.5 + point.lift * 0.045;
        const shadowAlpha = 0.24 * (1 - Math.min(0.75, point.lift / 110));
        ctx.save();
        // Ground shadow communicates height even on the top-down court.
        ctx.globalAlpha = shadowAlpha;
        ctx.fillStyle = '#07100c';
        ctx.beginPath();
        ctx.ellipse(point.groundX, point.groundY, radius * 1.45, Math.max(1.5, radius * 0.35), 0, 0, Math.PI * 2);
        ctx.fill();
        // A short fading trail gives the ball momentum without a flat line.
        ctx.globalAlpha = 0.18 * (1 - progress);
        ctx.strokeStyle = '#d98e1f';
        ctx.lineWidth = Math.max(1, radius * 0.42);
        ctx.lineCap = 'round';
        ctx.beginPath();
        const tail = shotPoint(a, Math.max(0, progress - 0.12), w, h, pad);
        ctx.moveTo(tail.x, tail.y - tail.lift);
        ctx.lineTo(point.x, point.y - point.lift);
        ctx.stroke();
        // Ball: warm highlight plus dark rim for contrast against the court.
        const ballY = point.y - point.lift;
        ctx.globalAlpha = 1;
        ctx.fillStyle = '#c97b2d';
        ctx.beginPath();
        ctx.arc(point.x, ballY, radius, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = '#5d3215';
        ctx.lineWidth = 1.2;
        ctx.stroke();
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.9)';
        ctx.lineWidth = 0.9;
        ctx.beginPath();
        ctx.arc(point.x - radius * 0.12, ballY - radius * 0.1, radius * 0.62, -1.1, 1.15);
        ctx.stroke();
        // 接缝随飞行自旋（C）：两条弧以球心旋转 angle = progress*3π
        drawBallSeams(point.x, ballY, radius, progress * 3 * Math.PI);
        ctx.restore();
        break;
      }
      case 'shot': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        ctx.save();
        ctx.globalAlpha = 1 - progress;
        ctx.strokeStyle = '#c98a00';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(cx, cy, 8 + progress * 25, 0, Math.PI * 2);
        ctx.stroke();
        ctx.restore();
        break;
      }
      case 'make': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        ctx.save();
        const rimPulse = Math.sin(progress * Math.PI);
        ctx.globalAlpha = (1 - progress) * 0.9;
        ctx.strokeStyle = '#2e9e4f';
        ctx.lineWidth = 2 + rimPulse * 3;
        ctx.beginPath();
        ctx.arc(cx, cy, 7 + rimPulse * 9, 0, Math.PI * 2);
        ctx.stroke();
        ctx.globalAlpha = 1 - progress;
        ctx.strokeStyle = 'rgba(255, 255, 255, 0.95)';
        ctx.lineWidth = 1.5;
        ctx.beginPath();
        for (let i = 0; i < 5; i += 1) {
          const dx = (i - 2) * 4;
          ctx.moveTo(cx + dx, cy - 2);
          ctx.lineTo(cx + dx * 0.62, cy + 12 + progress * 8);
        }
        ctx.stroke();
        ctx.fillStyle = '#2e9e4f';
        ctx.font = `bold ${14 + progress * 8}px "IBM Plex Sans", sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText('✓', cx, cy - 24 - progress * 15);
        ctx.restore();
        break;
      }
      case 'miss': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        ctx.save();
        ctx.globalAlpha = 1 - progress;
        ctx.strokeStyle = '#ef4444';
        ctx.lineWidth = 3;
        const s = 12;
        ctx.beginPath();
        ctx.moveTo(cx - s, cy - s); ctx.lineTo(cx + s, cy + s);
        ctx.moveTo(cx + s, cy - s); ctx.lineTo(cx - s, cy + s);
        ctx.stroke();
        ctx.restore();
        break;
      }
      case 'steal': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        ctx.save();
        ctx.globalAlpha = 1 - progress;
        ctx.fillStyle = '#f97316';
        const spikes = 8;
        const outerR = 10 + progress * 20;
        const innerR = outerR * 0.4;
        ctx.beginPath();
        for (let i = 0; i < spikes * 2; i += 1) {
          const r = i % 2 === 0 ? outerR : innerR;
          const angle = (i / (spikes * 2)) * Math.PI * 2;
          const x = cx + Math.cos(angle) * r;
          const y = cy + Math.sin(angle) * r;
          i === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
        }
        ctx.closePath();
        ctx.fill();
        ctx.restore();
        break;
      }
      case 'turnover': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        ctx.save();
        ctx.globalAlpha = (1 - progress) * 0.6;
        ctx.fillStyle = '#ef4444';
        ctx.beginPath();
        ctx.arc(cx, cy, 15 + progress * 10, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
        break;
      }
      case 'rebound': {
        ctx.save();
        ctx.globalAlpha = 1 - progress;
        ctx.strokeStyle = '#2f6fd0';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(cx, cy, 5 + progress * 20, 0, Math.PI * 2);
        ctx.stroke();
        ctx.fillStyle = '#2f6fd0';
        ctx.font = 'bold 10px "IBM Plex Mono", monospace';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText('REB', cx, cy - 15 - progress * 10);
        ctx.restore();
        break;
      }
      case 'foul': {
        ctx.save();
        ctx.globalAlpha = (1 - progress) * 0.7;
        ctx.fillStyle = '#c98a00';
        ctx.font = 'bold 14px "IBM Plex Sans", sans-serif';
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText('⚠ FOUL', cx, cy - 20 - progress * 10);
        ctx.restore();
        break;
      }
      case 'screen': {
        // 掩护盾牌：紫色实心盾 + 高亮描边，让挡拆在画面上清晰可见。
        ctx.save();
        ctx.globalAlpha = 1 - progress * 0.5;
        // 盾形主体
        ctx.fillStyle = '#7c5cd6';
        ctx.beginPath();
        ctx.moveTo(cx, cy - 14);
        ctx.quadraticCurveTo(cx + 12, cy - 10, cx + 10, cy + 2);
        ctx.quadraticCurveTo(cx + 8, cy + 12, cx, cy + 14);
        ctx.quadraticCurveTo(cx - 8, cy + 12, cx - 10, cy + 2);
        ctx.quadraticCurveTo(cx - 12, cy - 10, cx, cy - 14);
        ctx.fill();
        ctx.strokeStyle = '#f8fafc';
        ctx.lineWidth = 1.5;
        ctx.stroke();
        // 底部箭头：掩护方向
        ctx.fillStyle = '#f8fafc';
        ctx.beginPath();
        ctx.moveTo(cx - 5, cy + 4);
        ctx.lineTo(cx + 5, cy + 4);
        ctx.lineTo(cx, cy + 9);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
        break;
      }
      case 'drive': {
        // 突破箭头：三条斜向速度线 + 尾迹，方向指向突破方向。
        ctx.save();
        ctx.globalAlpha = 1 - progress * 0.6;
        ctx.strokeStyle = '#c98a00';
        ctx.lineWidth = 2.2;
        ctx.lineCap = 'round';
        for (let i = 0; i < 3; i += 1) {
          const off = i * 5;
          ctx.beginPath();
          ctx.moveTo(cx - 18 - off * 0.8, cy - 4 + off);
          ctx.lineTo(cx - 4 - off * 0.8, cy - 4 + off);
          ctx.stroke();
        }
        ctx.fillStyle = '#c98a00';
        ctx.beginPath();
        ctx.moveTo(cx - 6, cy - 8);
        ctx.lineTo(cx + 6, cy);
        ctx.lineTo(cx - 6, cy + 8);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
        break;
      }
      case 'ft': {
        ctx.save();
        ctx.globalAlpha = (1 - progress) * 0.5;
        ctx.strokeStyle = '#c98a00';
        ctx.lineWidth = 1;
        ctx.setLineDash([2, 2]);
        ctx.beginPath();
        ctx.arc(cx, cy, 15, 0, Math.PI * 2);
        ctx.stroke();
        ctx.restore();
        break;
      }
      case 'score': {
        if (reduceMotion) break; // H：非必要特效，reduceMotion 时跳过
        const pts = (a.data.home?.home ?? 0) - (a.data.prevHome?.home ?? 0)
          + (a.data.home?.away ?? 0) - (a.data.prevHome?.away ?? 0);
        ctx.save();
        ctx.globalAlpha = Math.max(0, 1 - progress * 1.5);
        ctx.fillStyle = '#2e9e4f';
        ctx.font = `bold ${20 + progress * 6}px "IBM Plex Sans", sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(pts >= 3 ? '+3!' : pts >= 2 ? '+2' : '+1', cx, cy - 30 - progress * 20);
        ctx.restore();
        break;
      }
      default:
        break;
    }
    return true;
  });
}

// ─── UI Elements ────────────────────────────────────────────────────────────

const homeName = $('homeName');
const awayName = $('awayName');
const homePts = $('homePts');
const awayPts = $('awayPts');
const periodEl = $('period');
const gameClock = $('gameClock');
const tacticStage = $('tacticStage');
const tacticName = $('tacticName');
const tacticAction = $('tacticAction');
const assignmentsEl = $('assignments');
const logEl = /** @type {HTMLOListElement} */ ($('log'));
const scrub = /** @type {HTMLInputElement} */ ($('scrub'));
const speed = /** @type {HTMLSelectElement} */ ($('speed'));
const btnPlay = $('btnPlay');
const highOnly = /** @type {HTMLInputElement} */ ($('highOnly'));
const meta = $('meta');
const shotClock = $('shotClock');
const callout = $('callout');
const staminaAlert = $('staminaAlert');
const tip = $('tip');
const ptFloat = $('ptFloat');
const netBurst = $('netBurst');
const loadCover = $('loadCover');
const loadBarFill = $('loadBarFill');
const loadTxt = $('loadTxt');
const courtFrame = $('courtFrame');

// ─── 交互状态（I）────────────────────────────────────────────────────────────
// drawTick 每帧收集球员绘制位置；hit-test 由渲染层默认实现，数据层可覆盖。
/** @type {{jersey:string,team:string,x:number,y:number,r:number}[]} */
let hitRects = [];
/** @type {((clientX:number, clientY:number, rect:DOMRect)=>object|null)|null} */
let courtHitTest = null; // 数据层自定义命中（默认用 window.__hitTest）
/** @type {((on:boolean)=>void)|null} */
let courtCursorFn = null;
/** 当前 hover 命中的球员信息（数据层读此驱动 #playerCard） */
window.__hoverHit = null;
/** 镜头激活时（cam.scale !== 1）hit-test 返回 null */
window.__camActive = () => cam.scale !== 1;
/** 默认 hit-test：遍历 hitRects 找最近（≤ r+6px）球员（I） */
window.__hitTest = (clientX, clientY, canvasRect) => {
  if (cam.scale !== 1 || hitRects.length === 0 || !ticks[index]) return null;
  const rect = canvasRect ?? canvas.getBoundingClientRect();
  // 显示坐标 → 逻辑坐标（940×500 设计系），保证任何显示尺寸命中一致
  const pxPos = (clientX - rect.left) * (COURT_W / rect.width);
  const pyPos = (clientY - rect.top) * (COURT_H / rect.height);
  let best = null, bestD = Infinity;
  for (const h of hitRects) {
    const d = Math.hypot(pxPos - h.x, pyPos - h.y);
    if (d <= h.r + 6 && d < bestD) { best = h; bestD = d; }
  }
  if (!best) return null;
  const tick = ticks[index];
  const player = (tick.players ?? []).find((p) => p.jersey === best.jersey) ?? null;
  if (!player) return null;
  const tac = tick.tactical;
  const assignment = tac?.assignments?.find((a) => String(a.jersey) === String(best.jersey));
  const active = tac?.activeAction && String(tac.activeAction.jersey) === String(best.jersey)
    ? tac.activeAction : null;
  return {
    jersey: best.jersey,
    team: best.team,
    x: best.x,
    y: best.y,
    stm: typeof player.stm === 'number' ? player.stm : null,
    stmMax: typeof player.stmMax === 'number' ? player.stmMax : null,
    task: player.task ?? player.action ?? null,
    action: active?.kind ?? player.action ?? null,
    role: assignment?.role ?? null,
    lane: assignment?.lane ?? null,
  };
};
canvas.addEventListener('pointermove', (ev) => {
  const hit = (courtHitTest ?? window.__hitTest)(ev.clientX, ev.clientY);
  window.__hoverHit = hit;
  if (courtCursorFn) courtCursorFn(Boolean(hit));
});
canvas.addEventListener('pointerleave', () => {
  window.__hoverHit = null;
  if (courtCursorFn) courtCursorFn(false);
});
const possessionEl = $('possession');
const homePeriodPts = $('homePeriodPts');
const awayPeriodPts = $('awayPeriodPts');
const playerCard = $('playerCard');
const pcJersey = $('pcJersey');
const pcName = $('pcName');
const pcStm = $('pcStm');
const pcTask = $('pcTask');
const pcRole = $('pcRole');
function fmtClock(seconds) {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
}

const TACTIC_LABELS = {
  kind: {
    TRANSITION_PUSH: '转换推进', PNR_ROLL: '高位挡拆·顺下', PNR_POP: '高位挡拆·外弹',
    DRIVE_KICK: '突破分球', POST_UP: '低位单打', OFF_BALL_SCREEN: '无球掩护', ISO: '单打', HANDOFF: '手递手',
    PNR: '挡拆配合', BROKEN_PLAY: '临时应变',
  },
  stage: {
    ADVANCE: '推进', SET: '半场落位', SCREEN_APPROACH: '接近挡拆位置', SCREEN_USE: '使用挡拆',
    ADVANTAGE: '扩大进攻优势', TERMINAL: '终结回合',
  },
  action: {
    advance: '推进', pass: '传球', handoff: '手递手', drive: '突破', shoot: '投篮', hold: '持球组织',
    triple_threat: '三威胁', back_to_basket: '背身', pivot: '轴心脚', crossover: '变向', pump_fake: '投篮假动作',
    space: '拉开空间', cut: '顺下切入', relocate: '换位', screen: '设掩护', pressure: '贴身防守',
    tag: '协防补位', weak_side: '弱侧协防', deny: '阻断接球', ball_handler: '持球人',
    on_ball_defend: '持球防守', help: '协防', idle: '等待',
  },
  route: {
    advance_lane: '推进线', drive_lane: '突破线', crossover: '变向线', post_seal: '背身落位',
    screen_angle: '挡拆角度', pin_down: '钉人掩护', roll: '顺下', pop: '外弹', curl: '绕掩护',
    cut: '切入', relocate: '换位', spacing: '空间线', seal: '卡位', pass_lane: '传球线', handoff_lane: '手递手线',
  },
  role: { handler: '持球人', screener: '掩护人', strong_corner: '强侧底角', weak_corner: '弱侧底角', slot: '侧翼接应' },
  lane: { middle: '中路', strong: '强侧', weak: '弱侧', rim: '篮下' },
};

function tacticLabel(group, value) {
  return TACTIC_LABELS[group]?.[value] ?? value ?? '—';
}


/** @type {object | null} */
let courtSpec = null;

async function loadCourtSpec() {
  try {
    const res = await fetch('./court-draw-spec.json');
    if (!res.ok) return;
    courtSpec = await res.json();
  } catch { /* ignore */ }
}

// ─── DPR + 尺寸适配（A）──────────────────────────────────────────────────────
// 逻辑坐标系固定为球场设计尺寸 940×500（与 court-draw-spec 的归一化坐标
// 一致），物理分辨率 = 940×500 × DPR（cap 2）。CSS 负责把 canvas 缩放到
// 任意显示尺寸，因此球员半径/体力槽/动画在所有屏幕上比例一致、不变形。
const COURT_W = 940;
const COURT_H = 500;
let displayW = COURT_W;
let displayH = COURT_H;
let resizeTimer = 0;

function syncCanvasSize() {
  const dpr = Math.min(2, window.devicePixelRatio || 1);
  canvas.width = Math.round(COURT_W * dpr);
  canvas.height = Math.round(COURT_H * dpr);
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  displayW = COURT_W;
  displayH = COURT_H;
  courtCanvas = null; // 缓存失效，下一帧重建
}

window.addEventListener('resize', () => {
  if (resizeTimer) clearTimeout(resizeTimer);
  resizeTimer = window.setTimeout(() => {
    syncCanvasSize();
    renderCourtFrame();
  }, 150);
});

/** 重绘当前帧（含场地贴图），供 hideLoading / resize 等外部调用 */
function renderCourtFrame() {
  if (ticks.length > 0 && index < ticks.length) {
    drawTick(ticks[index]);
  } else {
    drawCourt();
  }
}

function px(n, size, pad) {
  return pad + n * (size - 2 * pad);
}

// ─── 离屏球场缓存（B）────────────────────────────────────────────────────────
/** @type {HTMLCanvasElement|null} 离屏球场图层（地板+着色+线条），尺寸不匹配时重建 */
let courtCanvas = null;

function buildCourtLayer(w, h) {
  const off = document.createElement('canvas');
  off.width = Math.max(1, Math.round(w));
  off.height = Math.max(1, Math.round(h));
  const octx = off.getContext('2d');
  if (!octx) return null;

  // 地板
  octx.fillStyle = '#d9b98c';
  octx.fillRect(0, 0, off.width, off.height);

  // 油漆区与端区分层着色：半透明深色叠在地板上，线框随后压上。
  if (courtSpec) {
    for (const side of ['left', 'right']) {
      const spec = courtSpec[side];
      if (!spec) continue;
      // 油漆区（lane 矩形）
      if (spec.lane) {
        octx.fillStyle = 'rgba(90, 60, 30, 0.16)';
        octx.fillRect(
          px(spec.lane.x, w, 8), px(spec.lane.y, h, 8),
          spec.lane.w * (w - 16), spec.lane.h * (h - 16),
        );
      }
      // 端区：threeArc 与对应边线围成的浅色区域（左右各一）
      if (spec.threeArc && spec.threeArc.length > 0) {
        const pts = spec.threeArc.map((p) => ({ x: px(p.x, w, 8), y: px(p.y, h, 8) }));
        const edgeX = px(side === 'left' ? 0 : 1, w, 8);
        octx.fillStyle = 'rgba(90, 60, 30, 0.07)';
        octx.beginPath();
        const first = pts[0], last = pts[pts.length - 1];
        octx.moveTo(first.x, first.y);
        for (const p of pts) octx.lineTo(p.x, p.y);
        octx.lineTo(edgeX, last.y);
        octx.lineTo(edgeX, first.y);
        octx.closePath();
        octx.fill();
      }
    }
  }

  // 原有线条（drawBasketEnd 内部用全局 ctx 绘制，暂切到离屏上下文）
  const realCtx = ctx;
  window.__courtCtx = octx;
  try {
    // 把 drawBasketEnd/drawCourt 的线条逻辑改在 octx 上执行
    drawCourtLines(w, h, 8);
  } finally {
    window.__courtCtx = null;
  }
  return off;
}

/**
 * 球场线条绘制（供离屏缓存使用，用临时全局 ctx 指向离屏上下文）。
 * 与旧 drawCourt 等价：边界、中圈、两侧篮架。
 */
function drawCourtLines(w, h, pad) {
  const g = window.__courtCtx ?? ctx; // 当前全局 ctx（被 buildCourtLayer 切换为离屏）
  const lineColor = 'rgba(90, 60, 30, 0.55)';
  g.strokeStyle = lineColor;
  g.lineWidth = 1.5;

  // Court boundary
  if (courtSpec && courtSpec.boundary) {
    const b = courtSpec.boundary;
    g.strokeRect(px(b.x, w, pad), px(b.y, h, pad), b.w * (w - 2 * pad), b.h * (h - 2 * pad));
  }

  // Half court line
  if (courtSpec && courtSpec.halfLine) {
    const hl = courtSpec.halfLine;
    g.beginPath();
    g.moveTo(px(hl.x1, w, pad), px(hl.y1, h, pad));
    g.lineTo(px(hl.x2, w, pad), px(hl.y2, h, pad));
    g.stroke();
  }

  // Center circle
  if (courtSpec && courtSpec.centerCircle) {
    const cc = courtSpec.centerCircle;
    g.beginPath();
    g.arc(px(cc.cx, w, pad), px(cc.cy, h, pad),
      cc.r * ((w - 2 * pad) + (h - 2 * pad)) / 2, 0, Math.PI * 2);
    g.stroke();
  }

  drawBasketEnd('left', w, h, pad);
  drawBasketEnd('right', w, h, pad);
}

/**
 * 场地绘制：优先贴离屏缓存（尺寸不匹配或缺失时重建），否则直接绘制。
 * 保持无参可调用（J）：使用当前逻辑尺寸。
 */
function drawCourt() {
  const w = displayW, h = displayH;
  if (!courtCanvas || courtCanvas.width !== Math.round(w) || courtCanvas.height !== Math.round(h)) {
    courtCanvas = buildCourtLayer(w, h);
  }
  if (courtCanvas) {
    ctx.drawImage(courtCanvas, 0, 0, w, h);
    return;
  }
  // 兜底：直接绘制
  ctx.fillStyle = '#d9b98c';
  ctx.fillRect(0, 0, w, h);
  drawCourtLines(w, h, 8);
}

/**
 * Draw one basket end from court-draw-spec (left or right).
 * All spec coordinates are normalized [0,1] on both axes.
 * 绘制到当前 ctx（离屏缓存构建时为离屏上下文，见 buildCourtLayer）。
 */
function drawBasketEnd(side, w, h, pad) {
  if (!courtSpec) return;
  const spec = courtSpec[side];
  if (!spec) return;
  const g = window.__courtCtx ?? ctx; // 离屏缓存构建时为离屏上下文
  const stroke = 'rgba(90, 60, 30, 0.55)';
  const lineW = 1.5;

  // Backboard
  const bb = spec.backboard;
  if (bb) {
    g.strokeStyle = stroke;
    g.lineWidth = lineW;
    g.beginPath();
    g.moveTo(px(bb.x1, w, pad), px(bb.y1, h, pad));
    g.lineTo(px(bb.x2, w, pad), px(bb.y2, h, pad));
    g.stroke();
  }

  // Rim
  const rim = spec.rim;
  const rimR = spec.rimR ?? 0.008;
  if (rim) {
    const rimX = px(rim.x, w, pad);
    const rimY = px(rim.y, h, pad);
    // convert normalized radius to pixels: average of x/y pixel scale
    const rimPx = rimR * ((w - 2 * pad) + (h - 2 * pad)) / 2;
    g.strokeStyle = '#f97316';
    g.lineWidth = 2;
    g.beginPath();
    g.arc(rimX, rimY, Math.max(4, rimPx), 0, Math.PI * 2);
    g.stroke();
  }

  // Restricted area arc
  const ra = spec.restricted;
  if (ra) {
    g.strokeStyle = stroke;
    g.lineWidth = 1;
    g.beginPath();
    g.arc(px(ra.cx, w, pad), px(ra.cy, h, pad),
      ra.r * ((w - 2 * pad) + (h - 2 * pad)) / 2,
      side === 'left' ? -Math.PI / 2 : Math.PI / 2,
      side === 'left' ? Math.PI / 2 : 3 * Math.PI / 2);
    g.stroke();
  }

  // Paint / lane rectangle
  const lane = spec.lane;
  if (lane) {
    g.strokeStyle = stroke;
    g.lineWidth = lineW;
    g.strokeRect(
      px(lane.x, w, pad), px(lane.y, h, pad),
      lane.w * (w - 2 * pad), lane.h * (h - 2 * pad),
    );
  }

  // Free-throw line
  const ft = spec.ftLine;
  if (ft) {
    g.beginPath();
    g.moveTo(px(ft.x1, w, pad), px(ft.y1, h, pad));
    g.lineTo(px(ft.x2, w, pad), px(ft.y2, h, pad));
    g.stroke();
  }

  // Free-throw circle
  const ftc = spec.ftCircle;
  if (ftc) {
    g.beginPath();
    g.arc(px(ftc.cx, w, pad), px(ftc.cy, h, pad),
      ftc.r * ((w - 2 * pad) + (h - 2 * pad)) / 2,
      0, Math.PI * 2);
    g.stroke();
  }

  // Three-point arc
  g.beginPath();
  if (spec.threeArc && spec.threeArc.length > 0) {
    for (let i = 0; i < spec.threeArc.length; i++) {
      const p = spec.threeArc[i];
      const x = px(p.x, w, pad), y = px(p.y, h, pad);
      i === 0 ? g.moveTo(x, y) : g.lineTo(x, y);
    }
  }
  g.stroke();

  // Three-point corner lines (straight segments along baseline)
  if (spec.threeCorners) {
    for (const seg of spec.threeCorners) {
      g.beginPath();
      g.moveTo(px(seg.x1, w, pad), px(seg.y1, h, pad));
      g.lineTo(px(seg.x2, w, pad), px(seg.y2, h, pad));
      g.stroke();
    }
  }

  // Lane hash marks (NBA: 3/10/14/19 ft from baseline, 3ft into the lane)
  if (spec.laneHashMarks) {
    for (const seg of spec.laneHashMarks) {
      g.beginPath();
      g.moveTo(px(seg.x1, w, pad), px(seg.y1, h, pad));
      g.lineTo(px(seg.x2, w, pad), px(seg.y2, h, pad));
      g.stroke();
    }
  }
}

/** @type {StreamTick|null} */
let prevTick = null;

/**
 * @param {StreamTick} tick
 */
function drawTick(tick) {
  // 逻辑尺寸（A）：canvas 物理分辨率 = displayW×displayH × DPR，绘制用逻辑坐标
  const w = displayW, h = displayH;
  const pad = 8;
  updateCamera(tick.t ?? 0);
  drawCourt();

  // 镜头变换（F）：wrap 整个场景绘制。缩放/平移后 clamp 焦点偏移防出界。
  const focusPx = camFocusPx(w, h);
  const camOffX = focusPx.x * cam.scale;
  const camOffY = focusPx.y * cam.scale;
  const maxOffX = Math.max(0, (w / 2) * (1 - 1 / cam.scale));
  const maxOffY = Math.max(0, (h / 2) * (1 - 1 / cam.scale));
  const offX = Math.max(-maxOffX, Math.min(maxOffX, camOffX));
  const offY = Math.max(-maxOffY, Math.min(maxOffY, camOffY));
  ctx.save();
  if (cam.scale !== 1) {
    ctx.translate(w / 2 - offX, h / 2 - offY);
    ctx.scale(cam.scale, cam.scale);
  }

  hitRects = [];
  const players = (tick.players ?? []).map((p) => ({
    p,
    x: px(p.x, w, pad),
    groundY: px(p.y, h, pad),
    lift: jumpLiftFor(p.jersey, tick.t ?? 0),
  }));

  // Top-down depth: farther players first, nearer players last. The model's
  // array order is tactical, not a rendering order.
  players.sort((a, b) => a.groundY - b.groundY
    || Number(Boolean(a.p.hasBall)) - Number(Boolean(b.p.hasBall))
    || String(a.p.jersey).localeCompare(String(b.p.jersey)));

  // 防守对位连线（E）：onBallDefender 与持球者之间极淡连线
  const defId = tick.tactical?.screenDefense?.onBallDefender;
  const holder = players.find(({ p }) => p.hasBall);
  if (defId && holder) {
    const defP = players.find(({ p }) => String(p.jersey) === String(defId));
    if (defP) {
      ctx.save();
      ctx.globalAlpha = 0.12;
      ctx.strokeStyle = '#0b0f14';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 3]);
      ctx.beginPath();
      ctx.moveTo(defP.x, defP.groundY);
      ctx.lineTo(holder.x, holder.groundY);
      ctx.stroke();
      ctx.restore();
    }
  }
  // Tactical route layer: the kernel publishes action-specific waypoints.
  // Draw those paths directly instead of inferring every movement as a
  // generic arrow toward the basket; a curl, pop, screen, and drive have
  // different geometry even when they end on the same half of the floor.
  const tactical = tick.tactical;
  if (tactical && tick.phase === 'LIVE') {
    ctx.save();
    ctx.lineCap = 'round';
    ctx.lineJoin = 'round';
    for (const assignment of (tactical.assignments ?? [])) {
      const route = assignment.route;
      const points = route?.points;
      if (!Array.isArray(points) || points.length < 2) continue;
      const isHandler = String(assignment.jersey) === String(tactical.handler);
      const isActive = tactical.activeAction
        && String(tactical.activeAction.jersey) === String(assignment.jersey);
      const teamColor = tactical.offense === 'home' ? '#2f6fd0' : '#d64550';
      const color = isHandler ? '#c98a00' : teamColor;
      ctx.strokeStyle = color;
      ctx.globalAlpha = isActive ? 0.78 : isHandler ? 0.58 : 0.34;
      ctx.lineWidth = isActive ? 2.2 : 1.4;
      ctx.setLineDash(isActive ? [7, 4] : [5, 5]);
      ctx.beginPath();
      points.forEach((point, pointIndex) => {
        const x = px(point.x, w, pad);
        const y = px(point.y, h, pad);
        if (pointIndex === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.stroke();

      // Waypoint beads make the bend legible at replay speed; the endpoint
      // arrow communicates the intended destination without covering a
      // player's jersey.
      ctx.setLineDash([]);
      for (let pointIndex = 1; pointIndex < points.length - 1; pointIndex += 1) {
        const point = points[pointIndex];
        ctx.globalAlpha = isActive ? 0.7 : 0.28;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(px(point.x, w, pad), px(point.y, h, pad), isActive ? 3 : 2, 0, Math.PI * 2);
        ctx.fill();
      }
      const from = points[points.length - 2];
      const to = points[points.length - 1];
      const tx = px(to.x, w, pad);
      const ty = px(to.y, h, pad);
      const dx = px(to.x - from.x, w - 2 * pad, 0);
      const dy = px(to.y - from.y, h - 2 * pad, 0);
      const length = Math.hypot(dx, dy) || 1;
      const ux = dx / length;
      const uy = dy / length;
      const size = isActive ? 7 : 5;
      ctx.globalAlpha = isActive ? 0.85 : 0.42;
      ctx.fillStyle = color;
      ctx.beginPath();
      ctx.moveTo(tx, ty);
      ctx.lineTo(tx - ux * size - uy * size * 0.65, ty - uy * size + ux * size * 0.65);
      ctx.lineTo(tx - ux * size + uy * size * 0.65, ty - uy * size - ux * size * 0.65);
      ctx.closePath();
      ctx.fill();
    }
    ctx.restore();
  }
 

  for (const { p, x, groundY, lift } of players) {
    const y = groundY - lift;
    // Base radius per player height — the 2D icon scales with the player's
    // real height (0-index: 190cm → 0.5, 225cm → 1.0). The hasBall bump
    // stays on top so the ball carrier is always prominent.
    const body = playerBodies[p.jersey] ?? {};
    const hRatio = (body.h_cm ? Math.max(0.4, Math.min(1.2, (body.h_cm - 160) / 65)) : 0.5);
    const wRatio = (body.wt_kg ? Math.max(0.4, Math.min(1.2, (body.wt_kg - 70) / 55)) : 0.5);
    const r = (p.hasBall ? 14 : 10) + hRatio * 4;
    const baseR = 10 + hRatio * 4;
    const color = p.team === 'home' ? '#2f6fd0' : '#d64550';

    if (prevTick && !reduceMotion) {
      const before = playerByJersey(prevTick, p.jersey);
      if (before) {
        const dt = Math.max(0.001, (tick.t ?? 0) - (prevTick.t ?? 0));
        const vx = (p.x - before.x) * 94 / dt;
        const vy = (p.y - before.y) * 50 / dt;
        const speedFps = Math.hypot(vx, vy);
        if (speedFps >= 4) {
          const len = Math.max(4, Math.min(14, speedFps * 0.5));
          const mag = Math.hypot(p.x - before.x, p.y - before.y) || 1;
          const dx = (p.x - before.x) / mag * len;
          const dy = (p.y - before.y) / mag * len;
          ctx.save();
          ctx.globalAlpha = 0.35;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1.5;
          ctx.lineCap = 'round';
          ctx.beginPath();
          ctx.moveTo(x - dx, groundY - dy);
          ctx.lineTo(x, groundY);
          ctx.stroke();
          ctx.restore();
        }
      }
    }

    if (lift > 0.5) {
      // Keep the shadow attached to the authoritative floor position.
      ctx.save();
      ctx.globalAlpha = Math.max(0.08, 0.24 - lift / 80);
      ctx.fillStyle = '#07100c';
      ctx.beginPath();
      ctx.ellipse(x, groundY, r * (1.25 - Math.min(0.45, lift / 30)), 3, 0, 0, Math.PI * 2);
      ctx.fill();
      ctx.globalAlpha = 0.28;
      ctx.strokeStyle = color;
      ctx.lineWidth = 1;
      ctx.setLineDash([2, 3]);
      ctx.beginPath();
      ctx.moveTo(x, groundY - 4);
      ctx.lineTo(x, y + r);
      ctx.stroke();
      ctx.restore();
    }

    const sprint = activeAnims.find((animation) =>
      animation.type === 'sprint' && animation.data.jersey === p.jersey
      && (tick.t ?? 0) >= animation.startT
      && (tick.t ?? 0) <= animation.startT + animation.duration);
    if (sprint) {
      const sp = animProgress(sprint, tick.t ?? 0);
      const sx = px(sprint.data.fromX, w, pad);
      const sy = px(sprint.data.fromY, h, pad);
      ctx.save();
      ctx.globalAlpha = (1 - sp) * 0.38;
      ctx.strokeStyle = color === '#2f6fd0' ? '#5a9df0' : '#f09098';
      ctx.lineWidth = 2;
      ctx.lineCap = 'round';
    for (let line = 0; line < 3; line += 1) {
      const offset = (line - 1) * 4;
      ctx.beginPath();
      ctx.moveTo(sx - offset, sy + offset);
      ctx.lineTo(x - (x - sx) * (0.25 + line * 0.08), y - (y - sy) * (0.25 + line * 0.08));
      ctx.stroke();
    }
    ctx.restore();
    }

    const action = p.action ?? p.task;
    if ((action === 'drive' || action === 'crossover') && lift <= 0.5) {
      ctx.save();
      ctx.globalAlpha = 0.22;
      ctx.fillStyle = '#c98a00';
      ctx.beginPath();
      ctx.arc(x, y, r + 5, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
    }

    // The engine exposes handling beats directly; the spectator only maps
    // them to the concise court glyph, never inferring them from position.
    const fundamental = {
      triple_threat: '三威胁',
      back_to_basket: '背身',
      pivot: '轴心脚',
      crossover: '变向',
      pump_fake: '假动作',
      shoot: '出手',
      screen: '卡位',
      box_out: '卡位',
    }[action] ?? null;
    if (fundamental) {
      ctx.save();
      ctx.globalAlpha = 0.9;
      ctx.font = '9px "IBM Plex Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'bottom';
      ctx.fillStyle = 'rgba(7,16,12,0.62)';
      const tw = ctx.measureText(fundamental).width;
      ctx.fillRect(x - tw / 2 - 3, y - r - 12, tw + 6, 11);
      ctx.fillStyle = '#f8fafc';
      ctx.fillText(fundamental, x, y - r - 3);
      ctx.restore();
    }

    ctx.beginPath();
    ctx.fillStyle = color;
    // Directional jersey indicator: a clean circle scaled by height with
    // a direction chevron showing movement heading. Body differentiation
    // is communicated through SIZE (height-scaled radius) and a subtle
    // weight ring — not through distorted body shapes that look like
    // dumplings at 12px. The number is the primary read.
    const sizeR = baseR;
    ctx.arc(x, y, sizeR, 0, Math.PI * 2);
    ctx.fill();
    // Weight indicator: a slightly thicker bottom arc for heavier players.
    if (wRatio > 0.6) {
      ctx.save();
      ctx.strokeStyle = color;
      ctx.lineWidth = 2;
      ctx.globalAlpha = 0.7;
      ctx.beginPath();
      ctx.arc(x, y, sizeR, 0.3 * Math.PI, 0.7 * Math.PI);
      ctx.stroke();
      ctx.restore();
    }
    // Accent rings for lift / ball possession.
    ctx.strokeStyle = lift > 0.5 ? '#f8fafc' : 'transparent';
    ctx.lineWidth = lift > 0.5 ? 1.5 : 0;
    if (lift > 0.5) {
      ctx.beginPath();
      ctx.arc(x, y, sizeR + 1.5, 0, Math.PI * 2);
      ctx.stroke();
    }
    if (p.hasBall) {
      ctx.strokeStyle = '#c98a00';
      ctx.lineWidth = 2.5;
      ctx.beginPath();
      ctx.arc(x, y, sizeR + 2.5, 0, Math.PI * 2);
      ctx.stroke();
    }

    ctx.font = 'bold 11px "IBM Plex Mono", monospace';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    if (p.hasBall) {
      // Use a high-contrast label treatment for the holder. The ball is a
      // separate badge beside the player, never painted over this number.
      ctx.lineWidth = 3;
      ctx.strokeStyle = '#0b0f14';
      ctx.strokeText(p.jersey, x, y);
      ctx.fillStyle = '#f8fafc';
    } else {
      ctx.fillStyle = '#0b0f14';
    }
    ctx.fillText(p.jersey, x, y);

    // 体力条 (§8): 槽宽随球员半径自适应（移动端显示尺寸小，需等比放大）。
    // 只在低体力（≤60%）时显示，避免 10 个细条干扰球场辨识；持球人始终显示。
    if (typeof p.stm === 'number' && p.stm >= 0 && typeof p.stmMax === 'number' && p.stmMax > 0
        && (p.hasBall || p.stm / p.stmMax <= 0.6)) {
      const stmRatio = Math.max(0, Math.min(1, p.stm / p.stmMax));
      const barW = Math.max(20, r * 1.8);
      const barH = Math.max(3, r * 0.28);
      const barY = y + r + 5;
      ctx.fillStyle = 'rgba(7,16,12,0.55)';
      ctx.fillRect(x - barW / 2 - 0.5, barY - 0.5, barW + 1, barH + 1);
      ctx.fillStyle = stmRatio > 0.6 ? '#3fae5a' : stmRatio > 0.35 ? '#d9a514' : '#d64550';
      ctx.fillRect(x - barW / 2, barY, barW * stmRatio, barH);
    }

    // hit-test 数据收集（I）：记录绘制位置（逻辑坐标）
    hitRects.push({ jersey: p.jersey, team: p.team, x, y, r: r + 2 });
  }

  // During a shot, the visual flight owns the ball. Drawing the kernel's
  // ground-plane coordinate as a second ball would flatten the trajectory.
  const shotFlightActive = tick.ball.status === 'shot'
    && activeAnims.some((animation) => animation.type === 'shotFlight');
  if (!shotFlightActive) {
    const holder = tick.ball.status === 'held'
      ? players.find(({ p }) => p.jersey === tick.ball.holderId)
      : null;
    const ballX = holder
      ? holder.x + (holder.x < w / 2 ? 1 : -1) * 19
      : px(tick.ball.x, w, pad);
    const ballY = holder ? holder.y - 13 : px(tick.ball.y, h, pad);
    // 篮球纹理（C）：接缝弧线
    ctx.beginPath();
    ctx.fillStyle = '#c97b2d';
    ctx.arc(ballX, ballY, 5, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#5d3215';
    ctx.lineWidth = 1;
    ctx.stroke();
    drawBallSeams(ballX, ballY, 5);
  }

  drawAnimations(w, h, pad, tick.t ?? 0);
  ctx.restore();
}

/**
 * @param {number} i
 */
function render(i) {
  if (ticks.length === 0) return;
  // Backward stepping (index > i) is a jump too: the camera animation
  // interpolates by stream time (tick.t), which does not move backwards —
  // a paused/stepped-back frame would keep the frozen zoom. Any
  // non-consecutive move snaps the camera and direction lines.
  const jumped = Math.abs(index - i) > 1 || i < index;
  if (jumped) {
    resetVisualHistory();
    // Prevent stale prevTick from drawing a direction line across the
    // entire court when seeking/jumping (backward play, play-by-play
    // click). prevTick is rebuilt from the new position on the next
    // consecutive frame.
    prevTick = null;
    // Seeking past an active camera hold (score/steal zoom) would leave
    // the view stuck zoomed; snap back to the full-court view.
    if (camTarget) {
      cam = { x: 0, y: 0, scale: 1 };
      camTarget = null;
      camEndT = 0;
      camBase = null;
    }
  }
  index = Math.max(0, Math.min(i, ticks.length - 1));
  const tick = ticks[index];
  if (!tick) return;

  spawnAnimations(tick, prevTick);
  prevTick = tick;
  drawTick(tick);
  homePts.textContent = String(tick.score.home);
  awayPts.textContent = String(tick.score.away);
  periodEl.textContent = `Q${tick.period}`;
  const gc = tick.gameClock ?? tick.t_game ?? 0;
  gameClock.textContent = fmtClock(gc);
  shotClock.textContent = `24s · ${Math.max(0, tick.shotClock ?? 0).toFixed(1)}`;

  // 节分（E）：当前节起始分差 → #homePeriodPts / #awayPeriodPts。
  const periodStart = periodStartScore(tick.period);
  const ptsHome = Math.max(0, (tick.score.home ?? 0) - (periodStart?.home ?? 0));
  const ptsAway = Math.max(0, (tick.score.away ?? 0) - (periodStart?.away ?? 0));
  if (typeof window.setPeriodPts === 'function') window.setPeriodPts(String(ptsHome), String(ptsAway));
  // 控球指示（E）：当前持球方 → #possession。
  const possTeam = typeof window.getPossessionTeam === 'function' ? window.getPossessionTeam() : null;
  possessionEl.textContent = possTeam === 'home' ? '● 主队进攻' : possTeam === 'away' ? '● 客队进攻' : '';

  // 比分脉冲（E）：score 相对上一帧变化时 flashScore + 得分浮动。
  // 约定：burstAtRim 由渲染层在 SHOT_RESULT 触发，这里不重复调用。
  const curScore = tick.score ?? { home: 0, away: 0 };
  if (lastScore !== null) {
    const dh = (curScore.home ?? 0) - (lastScore.home ?? 0);
    const da = (curScore.away ?? 0) - (lastScore.away ?? 0);
    const diff = dh + da;
    if (diff > 0) {
      const team = dh > 0 ? 'home' : 'away';
      if (typeof window.flashScore === 'function') window.flashScore(team);
      if (typeof window.setFloatingPoints === 'function'
        && (typeof window.ptFloatHidden !== 'function' || window.ptFloatHidden())) {
        window.setFloatingPoints(`+${diff}`);
      }
    }
  }
  lastScore = curScore;

  // URL 状态（F）：hash 恢复完成后，帧号变化时写回 #t=N。
  if (hashRestored && lastUrlIndex !== index) {
    lastUrlIndex = index;
    history.replaceState(null, '', `#t=${index}`);
  }

  if (tick.callout) {
    lastCallout = tick.callout;
    callout.classList.toggle('high', tick.intensity === 'high');
  }
  callout.textContent = lastCallout;
  // 体力换人提示 (§8.6): STATE_NOTE stamina events surface as a banner.
  const staminaNote = tick.eventType === 'STATE_NOTE' && tick.eventPayload
    && typeof tick.eventPayload.note === 'string' && tick.eventPayload.note.startsWith('stamina');
  if (staminaNote) {
    const forced = tick.eventPayload.note === 'stamina_forced_sub';
    staminaAlert.textContent = `${forced ? '⚠ 强制换人提示' : '建议换人'}：#${tick.eventPayload.player_id ?? '?'} 体力 ${tick.eventPayload.stm ?? '?'}`;
    staminaAlert.classList.toggle('forced', forced);
    staminaAlert.hidden = false;
  } else {
    staminaAlert.hidden = true;
  }
  const tactical = tick.tactical;
  if (tactical) {
    tacticStage.textContent = tacticLabel('stage', tactical.stage);
    const defense = tactical.screenDefense;
    const defenseLabel = defense
      ? ` · 防守：${defense.mode === 'DROP' ? '沉退' : '换防'}${defense.onBallDefender ? `（#${defense.onBallDefender}）` : ''}`
      : '';
    tacticName.textContent = `${tactical.offense === 'home' ? '主队' : '客队'} · ${tacticLabel('kind', tactical.kind)}${defenseLabel}`;
    const active = tactical.activeAction;
    tacticAction.textContent = active
      ? `#${active.jersey} ${tacticLabel('action', active.kind)} · ${tacticLabel('stage', active.stage)} · ${active.elapsedSeconds.toFixed(1)}秒 / 起手 ${active.windupSeconds.toFixed(1)}秒`
      : `持球人 #${tactical.handler} · 等待下一动作`;
    assignmentsEl.innerHTML = '';
    if (defense) {
      const row = document.createElement('div');
      row.className = 'assignment defense-row';
      row.innerHTML = `<span class="a-role">挡拆防守</span><span class="a-detail">${defense.mode === 'DROP' ? '沉退护框' : '换防跟人'}${defense.screenerDefender ? ` · #${defense.screenerDefender}` : ''}</span>`;
      assignmentsEl.appendChild(row);
    }
    for (const assignment of tactical.assignments) {
      const row = document.createElement('div');
      row.className = `assignment${assignment.jersey === tactical.handler ? ' handler' : ''}`;
      const stmPlayer = tick.players?.find((p) => p.jersey === assignment.jersey);
      const stmText = stmPlayer && typeof stmPlayer.stm === 'number' && stmPlayer.stm >= 0
        ? ` <span class="a-stm">${Math.round(stmPlayer.stm)}%</span>`
        : '';
      const routeText = assignment.route?.kind ? ` · ${tacticLabel('route', assignment.route.kind)}` : '';
      row.innerHTML = `
        <span class="a-jersey">#${assignment.jersey}</span>
        <span class="a-role">${tacticLabel('role', assignment.role)}</span>
        <span class="a-detail">${tacticLabel('action', assignment.action)}${assignment.targetJersey ? ` →#${assignment.targetJersey}` : ''}${routeText}</span>
        <span class="a-stm-wrap">${stmText}</span>`;
      assignmentsEl.appendChild(row);
    }
  }
  scrub.value = String(index);
  highlightLogByEventSeq(selectedEventSeq !== null ? selectedEventSeq : tick.eventSeq ?? null);
}

let selectedEventSeq = null;
function highlightLogByEventSeq(eventSeq) {
  const items = logEl.querySelectorAll('li');
  items.forEach((li) => {
    li.classList.toggle('active', eventSeq !== null && Number(li.dataset.eventSeq) === eventSeq);
  });
}

/**
 * 节起始得分（E）：找到该 period 的第一个 tick，返回其 score，结果缓存。
 * ticks 按时间顺序排列，首个同 period 帧即节开始帧。
 */
function periodStartScore(period) {
  if (periodStartCache.has(period)) return periodStartCache.get(period);
  let start = null;
  for (const tick of ticks) {
    if (tick.period === period) { start = tick.score ?? { home: 0, away: 0 }; break; }
  }
  start ??= { home: 0, away: 0 };
  periodStartCache.set(period, start);
  return start;
}

/** 日志行分级着色（C）：失误/抢断→high，投篮/命中/篮→made，犯规→foul */
function logLineClass(line) {
  if (line.includes('失误') || line.includes('抢断')) return 'high';
  if (line.includes('犯规')) return 'foul';
  if (line.includes('投') || line.includes('命中') || line.includes('篮')) return 'made';
  return '';
}

/** 日志虚拟化渲染上限：超出后只渲染首尾各 250 行 + 中间省略标记 */
const LOG_RENDER_LIMIT = 500;
const LOG_EDGE = 250;

function buildLog() {
  if (!pack) return;
  logEl.innerHTML = '';
  const onlyHigh = highOnly.checked;
  const summary = pack.broadcastSummary?.length ? pack.broadcastSummary : null;
  const source = summary ?? (pack.broadcast ?? []).map((text, eventSeq) => ({ text, eventSeq, period: 0, gameClock: 0 }));
  const lines = source.map((entry) => ({ line: entry.text, eventSeq: entry.eventSeq, period: entry.period, gameClock: entry.gameClock }))
    .filter(({ line }) => line.length > 0)
    .filter(({ line }) => !onlyHigh || line.includes('失误') || line.includes('投') || line.includes('犯规') || line.includes('抢断') || line.includes('篮板') || line.includes('过半场') || line.includes('突破') || line.includes('出界'));
  const appendRow = (li, eventSeq, period, gameClock, cls) => {
    if (cls) li.classList.add(cls);
    li.addEventListener('click', () => {
      stop();
      selectedEventSeq = eventSeq;
      let frameIndex = ticks.findIndex((tick) => tick.eventSeq === eventSeq);
      if (frameIndex < 0 && period > 0) {
        frameIndex = ticks.findIndex((tick) => tick.period === period && tick.gameClock <= gameClock);
      }
      render(frameIndex >= 0 ? frameIndex : 0);
      // 点击日志自动滚动（D）：滚动到行居中。
      const rowTop = li.getBoundingClientRect().top - logEl.getBoundingClientRect().top + logEl.scrollTop;
      logEl.scrollTop = rowTop - logEl.clientHeight / 2;
    });
    logEl.appendChild(li);
  };
  if (lines.length > LOG_RENDER_LIMIT) {
    // 首尾各 250 行 + 中间省略标记
    const head = lines.slice(0, LOG_EDGE);
    const tail = lines.slice(lines.length - LOG_EDGE);
    for (const { line, eventSeq, period, gameClock } of head) {
      const li = document.createElement('li');
      li.textContent = line;
      li.dataset.eventSeq = String(eventSeq);
      appendRow(li, eventSeq, period, gameClock, logLineClass(line));
    }
    const el = document.createElement('li');
    el.className = 'ellipsis';
    el.textContent = `… 中间 ${lines.length - LOG_RENDER_LIMIT} 行省略 …`;
    logEl.appendChild(el);
    for (const { line, eventSeq, period, gameClock } of tail) {
      const li = document.createElement('li');
      li.textContent = line;
      li.dataset.eventSeq = String(eventSeq);
      appendRow(li, eventSeq, period, gameClock, logLineClass(line));
    }
  } else {
    for (const { line, eventSeq, period, gameClock } of lines) {
      const li = document.createElement('li');
      li.textContent = line;
      li.dataset.eventSeq = String(eventSeq);
      appendRow(li, eventSeq, period, gameClock, logLineClass(line));
    }
  }
}
function loadPackage(data) {
  pack = data;
  ticks = extractTicks(data);
  index = 0;
  resetVisualHistory();
  periodStartCache.clear();
  lastScore = null;
  scrub.max = String(Math.max(0, ticks.length - 1));
  selectedEventSeq = null;
  if (data.meta) {
    homeName.textContent = data.meta.home_name ?? data.meta.home_team_id ?? 'HOME';
    awayName.textContent = data.meta.away_name ?? data.meta.away_team_id ?? 'AWAY';
    meta.textContent = `foundation ${data.meta.foundation_version} · seed ${data.meta.seed} · ${ticks.length} frames`;
    // Per-jersey body dimensions for icon differentiation.
    playerBodies = (data.meta.player_bodies ?? {});
  }
  buildLog();
  render(0);
}

function resetVisualHistory() {
  activeAnims = [];
  prevTick = null;
  lastCallout = '—';
  cam = { x: 0, y: 0, scale: 1 };
  camTarget = null;
  camEndT = 0;
  camBase = null;
}

function stop() {
  playing = false;
  btnPlay.textContent = 'Play';
  if (timer) { clearTimeout(timer); timer = 0; }
}

function tickPlay() {
  if (!playing || ticks.length === 0) return;
  if (index >= ticks.length - 1) { stop(); return; }
  render(index + 1);
  const mult = Number(speed.value) || 1;
  const dt = pack?.meta?.stream_dt ?? pack?.stream?.dt ?? 0.1;
  const delay = Math.max(4, (dt * 1000) / mult);
  timer = window.setTimeout(tickPlay, delay);
}

function play() {
  if (ticks.length === 0) return;
  if (index >= ticks.length - 1) index = 0;
  selectedEventSeq = null;
  playing = true;
  btnPlay.textContent = 'Pause';
  tickPlay();
}

function extractTicks(data) {
  if (data.stream?.ticks?.length) return data.stream.ticks;
  return (data.frames ?? []).map((f, i) => ({
    t: i * 100,
    period: f.period,
    gameClock: f.gameClock,
    shotClock: f.shotClock,
    score: f.score,
    players: f.players,
    ball: f.ball,
    tactical: f.tactical,
    keyframeIndex: i,
    eventType: f.eventType,
    callout: data.broadcast?.[i] ?? f.narration?.cn ?? null,
    intensity: f.narration?.intensity ?? null,
  }));
  index = 0;
  resetVisualHistory();
  scrub.max = String(Math.max(0, ticks.length - 1));

  if (data.meta) {
    homeName.textContent = data.meta.home_name ?? data.meta.home_team_id ?? 'HOME';
    awayName.textContent = data.meta.away_name ?? data.meta.away_team_id ?? 'AWAY';
    meta.textContent = `foundation ${data.meta.foundation_version} · seed ${data.meta.seed} · ${ticks.length} frames`;
    // Per-jersey body dimensions for icon differentiation.
    playerBodies = (data.meta.player_bodies ?? {});
  }

  buildLog();
  render(0);
}

// ─── Event Handlers ─────────────────────────────────────────────────────────

/** 从 URL hash 恢复帧号（F）：#t=N 有效时跳到该帧，初次加载后调用 */
function restoreFromHash() {
  hashRestored = true;
  const m = location.hash.match(/#t=(\d+)/);
  if (!m) {
    // 无 hash：记录当前帧（index=0）作为初始状态
    lastUrlIndex = index;
    history.replaceState(null, '', `#t=${index}`);
    return;
  }
  const n = Number(m[1]);
  if (Number.isFinite(n) && ticks.length > 0) {
    selectedEventSeq = null;
    render(Math.max(0, Math.min(n, ticks.length - 1)));
  }
}

/** SportVU 回放（I）：加载 sportvu-replay.json 整包，失败时提示 */
async function loadSportvuReplay() {
  try {
    const response = await fetch('sportvu-replay.json');
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    loadPackage(data);
    restoreFromHash();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    meta.textContent = `Unable to load sportvu-replay.json: ${message}`;
    if (typeof window.setTip === 'function') window.setTip(`加载失败：${message}`);
    console.error('spectator: sportvu replay load failed', error);
  }
}

btnPlay.addEventListener('click', () => { playing ? stop() : play(); });
$('btnStepBack').addEventListener('click', () => { stop(); selectedEventSeq = null; render(index - 1); });
$('btnStepFwd').addEventListener('click', () => { stop(); selectedEventSeq = null; render(index + 1); });
scrub.addEventListener('input', () => { stop(); selectedEventSeq = null; render(Number(scrub.value)); });

highOnly.addEventListener('change', () => { buildLog(); });

$('file').addEventListener('change', async (ev) => {
  const input = /** @type {HTMLInputElement} */ (ev.target);
  if (!input.files?.[0]) return;
  try {
    const data = JSON.parse(await input.files[0].text());
    loadPackage(data);
    restoreFromHash();
    // If a sibling .ticks.ndjson was exported alongside, load it too.
    const name = input.files[0].name.replace(/\.json$/, '');
    const twin = `${name}.ticks.ndjson`;
    if (input.files.length > 1) {
      const f = Array.from(input.files).find((x) => x.name === twin);
      if (f) {
        const lines = (await f.text()).split('\n').filter((l) => l.trim().length > 0);
        ticks = lines.map((l) => JSON.parse(l));
        pack = { ...pack, stream: { ...(pack?.stream ?? {}), tickCount: ticks.length, ticks } };
        index = 0;
        resetVisualHistory();
        periodStartCache.clear();
        lastScore = null;
        scrub.max = String(Math.max(0, ticks.length - 1));
        buildLog();
        render(0);
        restoreFromHash();
      }
    }
  } catch (e) {
    const message = e instanceof Error ? e.message : String(e);
    meta.textContent = `Error: ${message}`;
    if (typeof window.setTip === 'function') window.setTip(`加载失败：${message}`);
  }
});
async function loadDemoGame() {
  try {
    // Slim package: meta/box/court/broadcast without the render tick stream.
    const response = await fetch('game.json?t=' + Date.now());
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const data = await response.json();
    loadPackage(data);
    // Render ticks stream separately (NDJSON, gzip-negotiated by the server).
    const ticksUrl = data.stream?.ticksUrl ?? 'game.ticks.ndjson';
    await loadTicksFromUrl(ticksUrl, data);
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    meta.textContent = `Unable to load game.json: ${message}`;
    if (typeof window.setTip === 'function') window.setTip(`加载失败：${message}`);
    console.error('spectator: game.json load failed', error);
  }
}

/**
 * 内存优化：把原始 tick 压缩成渲染最小集。
 * 原始行 ~2.3KB（含 zone/phase/t_game/keyframeIndex 等渲染不用的字段），
 * 34185 帧全量在 JS 堆里会膨胀到 300MB+。这里只保留 drawTick/render
 * 实际读取的字段，players 每项只留渲染必需项，体积约省 60-70%。
 */
function slimTick(t) {
  const players = (t.players ?? []).map((p) => ({
    jersey: p.jersey,
    team: p.team,
    x: p.x,
    y: p.y,
    hasBall: p.hasBall,
    action: p.action,
    task: p.task,
    stm: p.stm,
    stmMax: p.stmMax,
  }));
  return {
    t: t.t,
    period: t.period,
    gameClock: t.gameClock,
    shotClock: t.shotClock,
    phase: t.phase,
    score: t.score,
    players,
    ball: t.ball,
    eventType: t.eventType,
    eventSeq: t.eventSeq,
    eventPayload: t.eventPayload,
    callout: t.callout,
    intensity: t.intensity,
    tactical: t.tactical,
  };
}

/**
 * Stream NDJSON ticks line-by-line (server sends gzip automatically).
 * Each line is one tick object; a trailing whitespace-only line is ignored.
 * 流式加载（A）：边读边解析，每攒 200 行上报一次进度；完成后由
 * window.loadTicksDone() 收尾（渲染层内部 render(0)）。
 */
async function loadTicksFromUrl(url, data) {
  // Prefer the .gz twin — 8MB vs 76MB over the public internet, and
  // avoids nginx/proxy timeout on the raw stream. The browser's
  // DecompressionStream handles inflation; this is supported in all
  // modern browsers (Chrome 80+, Firefox 113+, Safari 16.4+).
  // Build the .gz twin URL: insert ".gz" before any cache-buster query,
  // e.g. "x.ndjson?v=123" → "x.ndjson.gz?v=123". Putting .gz after the
  // query ("x.ndjson?v=123.gz") hits a non-existent path → 404 →
  // DecompressionStream "incorrect header check".
  const qIdx = url.indexOf('?');
  const base = qIdx >= 0 ? url.slice(0, qIdx) : url;
  const query = qIdx >= 0 ? url.slice(qIdx) : '';
  const gzUrl = base.endsWith('.ndjson') ? base + '.gz' + query : url;
  // Append the per-request cache-buster WITHOUT producing a double '?':
  // the ticksUrl already carries a regeneration cache-buster (?v=<mtime>),
  // so this t=<epoch> must join with '&' when a query already exists. A
  // bare '?t=' would yield "x.ndjson.gz?v=123?t=456" — a malformed URL
  // that the CDN treats as one opaque key (defeating both busters) and
  // some servers reject.
  const withBust = (u) => (u.includes('?') ? `${u}&t=${Date.now()}` : `${u}?t=${Date.now()}`);
  let res;
  let stream;
  try {
    res = await fetch(withBust(gzUrl));
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    stream = res.body.pipeThrough(new DecompressionStream('gzip'));
  } catch {
    // Fallback: raw NDJSON (server negotiates gzip transparently).
    res = await fetch(withBust(url));
    if (!res.ok) throw new Error(`HTTP ${res.status} for ${url}`);
    stream = res.body;
  }
  const totalBytes = Number(res.headers.get('Content-Length')) || null;
  const reader = stream.getReader();
  const decoder = new TextDecoder();
  let buf = '';
  const all = [];
  let received = 0;
  const progressEvery = 200;
  const report = () => {
    if (typeof window.showLoading !== 'function') return;
    if (totalBytes) {
      const percent = Math.min(100, (received / totalBytes) * 100);
      window.showLoading(percent, `已加载 ${all.length} 帧`);
    } else {
      window.showLoading(0, `已解析 ${all.length} 帧`);
    }
  };
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    received += value.byteLength;
    buf += decoder.decode(value, { stream: true });
    let nl;
    while ((nl = buf.indexOf('\n')) >= 0) {
      const line = buf.slice(0, nl);
      buf = buf.slice(nl + 1);
      if (line.length > 0 && !/^\s*$/.test(line)) all.push(slimTick(JSON.parse(line)));
    }
    if (all.length % progressEvery === 0) report();
  }
  if (buf.length > 0 && !/^\s*$/.test(buf)) all.push(slimTick(JSON.parse(buf)));
  report();
  ticks = all;
  pack = { ...pack, stream: { ...(pack?.stream ?? {}), tickCount: all.length, ticks } };
  index = 0;
  resetVisualHistory();
  periodStartCache.clear();
  lastScore = null;
  scrub.max = String(Math.max(0, ticks.length - 1));
  meta.textContent = `foundation ${data?.meta?.foundation_version ?? ''} · seed ${data?.meta?.seed ?? ''} · ${ticks.length} frames`;
  buildLog();
  if (typeof window.loadTicksDone === 'function') {
    window.loadTicksDone();
  } else {
    render(0);
  }
  restoreFromHash();
}

$('btnDemo').addEventListener('click', () => { void loadDemoGame(); });
$('btnSportvu').addEventListener('click', () => { void loadSportvuReplay(); });
$('btnFullscreen').addEventListener('click', () => {
  try {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void document.documentElement.requestFullscreen();
  } catch { /* fullscreen 可能被策略拒绝，忽略 */ }
});
void loadDemoGame();

window.addEventListener('keydown', (ev) => {
  if (ev.target instanceof HTMLInputElement || ev.target instanceof HTMLSelectElement) return;
  switch (ev.key) {
    case ' ': ev.preventDefault(); playing ? stop() : play(); break;
    case 'ArrowLeft': stop(); render(index - 1); break;
    case 'ArrowRight': stop(); render(index + 1); break;
    case 'Home': stop(); render(0); break;
    case 'End': stop(); render(ticks.length - 1); break;
    case '1': speed.value = '0.5'; break;
    case '2': speed.value = '1'; break;
    case '3': speed.value = '2'; break;
    case '4': speed.value = '4'; break;
    case '5': speed.value = '16'; break;
    case 'l':
    case 'L': highOnly.checked = !highOnly.checked; buildLog(); break;
    case 'f':
    case 'F': {
      try {
        if (document.fullscreenElement) void document.exitFullscreen();
        else void document.documentElement.requestFullscreen();
      } catch { /* fullscreen 可能被策略拒绝，忽略 */ }
      break;
    }
  }
});

// ─── 移动端手势（touch）────────────────────────────────────────────────────
// tap → 播放/暂停；左滑 → 上一帧；右滑 → 下一帧。双指/滚动由浏览器接管。
// 判定阈值：位移 ≥ 40px 且时间 ≤ 500ms 算滑动；否则 < 300ms 抬起算 tap。
let touchStartX = 0;
let touchStartY = 0;
let touchStartT = 0;
let touchMoved = false;
window.__suppressTap = false;

const courtFrameEl = $('courtFrame');

courtFrameEl.addEventListener('touchstart', (ev) => {
  if (ev.touches.length !== 1) return;
  const t = ev.touches[0];
  touchStartX = t.clientX;
  touchStartY = t.clientY;
  touchStartT = Date.now();
  touchMoved = false;
}, { passive: true });

courtFrameEl.addEventListener('touchmove', (ev) => {
  if (ev.touches.length !== 1) return;
  const t = ev.touches[0];
  const dx = t.clientX - touchStartX;
  const dy = t.clientY - touchStartY;
  if (Math.abs(dx) > 12 || Math.abs(dy) > 12) touchMoved = true;
}, { passive: true });

courtFrameEl.addEventListener('touchend', (ev) => {
  if (ev.changedTouches.length !== 1) return;
  const t = ev.changedTouches[0];
  const dx = t.clientX - touchStartX;
  const dy = t.clientY - touchStartY;
  const dt = Date.now() - touchStartT;

  if (touchMoved && dt <= 500 && Math.abs(dx) >= 40 && Math.abs(dx) > Math.abs(dy) * 1.5) {
    // 横向滑动：单帧步进
    ev.preventDefault();
    window.__suppressTap = false;
    stop();
    selectedEventSeq = null;
    render(index + (dx < 0 ? 1 : -1));
    return;
  }
  if (!touchMoved && dt < 300) {
    // 轻点：命中球员时（卡片已处理）不切换播放；否则播放/暂停
    if (window.__suppressTap) { window.__suppressTap = false; return; }
    playing ? stop() : play();
  }
}, { passive: false });


/** 显示加载进度覆盖层（数据层边解析边调用） */
window.showLoading = (percent, text) => {
  loadCover.hidden = false;
  loadBarFill.style.width = `${Math.max(0, Math.min(100, percent))}%`;
  if (text != null) loadTxt.textContent = String(text);
};

/** 隐藏加载覆盖层并重绘场地 */
window.hideLoading = () => {
  loadCover.hidden = true;
  renderCourtFrame();
};

/** 得分浮动 "+2"（1.1 秒后隐藏；数据层用 ptFloatHidden 防重复触发） */
window.setFloatingPoints = (text, cssClass) => {
  ptFloat.textContent = String(text);
  ptFloat.className = cssClass ? `pt-float ${cssClass}` : 'pt-float';
  ptFloat.hidden = false;
  window.setTimeout(() => { ptFloat.hidden = true; }, 1100);
};

/** 篮下白色网动爆点（0.65 秒后隐藏），xNorm/yNorm 为 [0,1] 归一化坐标 */
window.burstAtRim = (xNorm, yNorm) => {
  netBurst.style.left = `${Math.max(0, Math.min(1, xNorm)) * 100}%`;
  netBurst.style.top = `${Math.max(0, Math.min(1, yNorm)) * 100}%`;
  netBurst.hidden = false;
  window.setTimeout(() => { netBurst.hidden = true; }, 650);
};

/** 当前持球方：基于 tick.ball.holderId 与 players 的 hasBall/球衣匹配 */
window.getPossessionTeam = () => {
  const tick = ticks[index];
  if (!tick) return null;
  const holderId = tick.ball?.holderId;
  if (holderId != null) {
    const holder = (tick.players ?? []).find((p) => String(p.jersey) === String(holderId));
    if (holder?.team) return holder.team;
  }
  const hasBall = (tick.players ?? []).find((p) => p.hasBall);
  return hasBall?.team ?? null;
};

/** 写入节比分文本 */
window.setPeriodPts = (homeStr, awayStr) => {
  homePeriodPts.textContent = homeStr ?? '';
  awayPeriodPts.textContent = awayStr ?? '';
};

/** 重绘场地帧 */
window.renderCourtFrame = renderCourtFrame;

/** 给 #homePts / #awayPts 加 .pulse 500ms 后移除 */
window.flashScore = (team) => {
  const el = team === 'away' ? awayPts : homePts;
  el.classList.remove('pulse');
  // 强制 reflow 以便重复触发动画
  void el.offsetWidth;
  el.classList.add('pulse');
  window.setTimeout(() => el.classList.remove('pulse'), 500);
};

/** #ptFloat 当前是否隐藏 */
window.ptFloatHidden = () => ptFloat.hidden;

/** 返回 ticks[i] 或 null（数据层查找同 eventSeq 帧） */
window.peekTick = (i) => (i >= 0 && i < ticks.length ? ticks[i] : null);

/** 流加载完成回调：隐藏加载层并 render(0) */
window.loadTicksDone = () => {
  window.hideLoading();
  render(0);
};

/** prefers-reduced-motion 是否匹配 */
window.hasReducedMotion = () => reduceMotion;

/**
 * 注册球场交互（契约 12）：hitTest(clientX, clientY, canvasRect) -> 球员信息
 * 或 null；cursorFn(bool) 设置 canvas.style.cursor。hitTest 传 null 用默认
 * 实现（基于 hitRects）；数据层也可读 window.__hoverHit。
 */
window.setCourtInteractive = (hitTest, cursorFn) => {
  courtHitTest = typeof hitTest === 'function' ? hitTest : null;
  courtCursorFn = typeof cursorFn === 'function' ? cursorFn : null;
  if (courtCursorFn) canvas.style.cursor = 'pointer';
  else canvas.style.cursor = '';
};

/** 设置 .tip 元素文本 */
window.setTip = (text) => { tip.textContent = text ?? ''; };

/** 当前 tick 的 stream 时间（无 tick 返回 0） */
window.getNow = () => ticks[index]?.t ?? 0;

/** 当前 tick 比分 */
window.getScore = () => {
  const tick = ticks[index];
  return { home: tick?.score?.home ?? 0, away: tick?.score?.away ?? 0 };
};

syncCanvasSize();
void loadCourtSpec().then(() => { drawCourt(); });

// ─── 球员交互卡（H）──────────────────────────────────────────────────────────
// 渲染层维护 window.__hoverHit（pointermove/pointerleave 已挂载），数据层只
// 负责把命中信息渲染进 #playerCard 并做防溢出定位。setCourtInteractive 注册
// 光标回调（hitTest 传 null 用渲染层默认实现，容错未注册的情况）。
function updatePlayerCard(hit) {
  if (!hit) {
    playerCard.hidden = true;
    return;
  }
  pcJersey.textContent = `#${hit.jersey}`;
  pcName.textContent = hit.team === 'home' ? (pack?.meta?.home_name ?? '主队') : (pack?.meta?.away_name ?? '客队');
  if (typeof hit.stm === 'number') {
    pcStm.textContent = Number.isFinite(hit.stmMax) && hit.stmMax > 0
      ? `${Math.round(hit.stm)} / ${Math.round(hit.stmMax)}`
      : String(Math.round(hit.stm));
  } else {
    pcStm.textContent = '—';
  }
  pcTask.textContent = hit.task ? tacticLabel('action', hit.task) : '—';
  pcRole.textContent = hit.role ? tacticLabel('role', hit.role) : '—';
  playerCard.hidden = false;
}

if (typeof window.setCourtInteractive === 'function') {
  window.setCourtInteractive(null, (on) => {
    if (on) canvas.style.cursor = 'pointer';
    else canvas.style.cursor = '';
  });
}

// 读渲染层维护的命中结果驱动卡片；指针移动时同时记录位置用于防溢出定位。
// 移动端（coarse pointer）：tap 切换卡片，定位在球员位置而非手指位置，
// 避免手指遮挡；卡片显示后自动隐藏于 3s 后，轻点其他位置关闭。
// 移动端（coarse pointer）判定：触摸设备或窄视口都走 tap 底部条路径。
// 只用 maxTouchPoints 判定在桌面触屏笔记本上会误判为移动端，所以窄视口
// 优先；headless/真机移动端都能稳定命中。
const coarsePointer = window.innerWidth <= 900
  || window.matchMedia('(pointer: coarse)').matches
  || (navigator.maxTouchPoints || 0) > 0;
let cardHideTimer = 0;

function positionCardForHit(hit, ev) {
  void playerCard.offsetWidth; // 强制 reflow，确保首次显示时能测出卡片尺寸
  const vw = window.innerWidth;
  const vh = window.innerHeight;
  const x = Math.min(vw - playerCard.offsetWidth - 10, Math.max(10, ev.clientX + 14));
  const y = Math.min(vh - playerCard.offsetHeight - 10, Math.max(10, ev.clientY + 14));
  playerCard.style.left = `${x}px`;
  playerCard.style.top = `${y}px`;
}

canvas.addEventListener('pointermove', (ev) => {
  if (coarsePointer) return; // 触摸设备不跟随 hover
  const hit = window.__hoverHit ?? null;
  if (!hit) { playerCard.hidden = true; return; }
  playerCard.classList.remove('bottom-sheet');
  playerCard.style.right = 'auto';
  playerCard.style.bottom = 'auto';
  updatePlayerCard(hit);
  positionCardForHit(hit, ev);
});
canvas.addEventListener('pointerleave', () => {
  if (coarsePointer) return;
  playerCard.hidden = true;
});

if (coarsePointer) {
  // 移动端：轻点球员 → 底部信息条显示球员数据（fixed 在控制条上方，
  // 不遮挡球场）；再点同球员/点空白 → 隐藏。命中球员时抑制播放手势。
  // 底部条复用 #playerCard，但位置固定在控制条上方，横向撑满。
  let tappedJersey = null;
  canvas.addEventListener('pointerdown', (ev) => {
    const hit = window.__hoverHit ?? null;
    if (hit && hit.jersey === tappedJersey) {
      playerCard.hidden = true;
      tappedJersey = null;
      window.__suppressTap = true;
      return;
    }
    if (hit) {
      updatePlayerCard(hit);
      // 底部信息条：固定在视口底部（控制条上方 8px），全宽，不跟随手指
      playerCard.classList.add('bottom-sheet');
      playerCard.style.left = '10px';
      playerCard.style.right = '10px';
      playerCard.style.width = 'auto';
      playerCard.style.top = 'auto';
      const bar = document.querySelector('.transport')?.getBoundingClientRect();
      const bottomGap = bar ? bar.height + 16 : 80;
      playerCard.style.bottom = `${bottomGap}px`;
      tappedJersey = hit.jersey;
      window.__suppressTap = true;
      if (cardHideTimer) window.clearTimeout(cardHideTimer);
      cardHideTimer = window.setTimeout(() => { playerCard.hidden = true; tappedJersey = null; }, 3000);
    } else {
      playerCard.hidden = true;
      tappedJersey = null;
    }
  });
}
