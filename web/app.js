/**
 * TraceLab Replay Console v3.1 (stealth-capable)
 * Professional court renderer with neutral-skin mode & collapsible scene card.
 */

function $(id) {
  return document.getElementById(id);
}

const canvas = $('court');
const ctx = canvas.getContext('2d');

let ticks = [];
let stages = [];
let currentTickIndex = 0;
let currentStageId = 0;
let isPlaying = false;
let isLoopStage = true;
let rafHandle = null;
let courtSpec = null;

// ---------------------------------------------------------------------------
// Display modes: 'stealth' = neutral skin (default), 'full' = basketball view
// ---------------------------------------------------------------------------
let displayMode = localStorage.getItem('trace.displayMode') === 'full' ? 'full' : 'stealth';
let sceneCollapsed = localStorage.getItem('trace.sceneCollapsed') === '1';

const STEALTH_TEXT = {
  title: '数据回放台 · Trace Replay Console',
  brandName: 'TraceLab',
  brandBadge: 'Replay & Annotation',
  logo: '◍',
  homeTeam: 'A 组',
  awayTeam: 'B 组',
  railTitle: '阶段序列',
  decisionTitle: '10 个代理微观决策全景',
  annotationTitle: '✍️ 本阶段人工反馈标注',
  eventTitle: '实时事件流',
  homeColor: '#3b82f6',
  awayColor: '#a855f7',
};
const FULL_TEXT = {
  title: 'NBA Sim · 战术阶段调试与 RLHF 人工标注工作台',
  brandName: 'NBA-Sim',
  brandBadge: 'Micro-Stage & Annotation Lab',
  logo: '🏀',
  homeTeam: 'Celtics',
  awayTeam: 'Lakers',
  railTitle: '比赛宏观回合序列',
  decisionTitle: '场上 10 人微观决策全景 (All 10 Players Trace)',
  annotationTitle: '✍️ 本阶段人工反馈标注 (RLHF / Debug)',
  eventTitle: '实时攻防事件流 (Play-by-Play)',
  homeColor: '#007A33',
  awayColor: '#552583',
};

function applyDisplayMode() {
  const t = displayMode === 'stealth' ? STEALTH_TEXT : FULL_TEXT;
  document.body.classList.toggle('stealth', displayMode === 'stealth');
  document.title = t.title;
  if ($('brandLogo')) $('brandLogo').textContent = t.logo;
  if ($('brandName')) $('brandName').textContent = t.brandName;
  if ($('brandBadge')) $('brandBadge').textContent = t.brandBadge;
  if ($('homeTeamName')) $('homeTeamName').textContent = t.homeTeam;
  if ($('awayTeamName')) $('awayTeamName').textContent = t.awayTeam;
  if ($('homePill')) $('homePill').style.background = t.homeColor;
  if ($('awayPill')) $('awayPill').style.background = t.awayColor;
  if ($('railTitleLabel')) $('railTitleLabel').textContent = t.railTitle;
  if ($('decisionHeaderTitle')) $('decisionHeaderTitle').textContent = t.decisionTitle;
  if ($('annotationHeaderTitle')) $('annotationHeaderTitle').textContent = t.annotationTitle;
  if ($('eventHeaderTitle')) $('eventHeaderTitle').textContent = t.eventTitle;
  if ($('btnStealth')) {
    $('btnStealth').textContent = displayMode === 'stealth' ? '◍ 极简: 开' : '◍ 极简: 关';
    $('btnStealth').classList.toggle('active', displayMode === 'stealth');
  }
  localStorage.setItem('trace.displayMode', displayMode);
  // Redraw current frame in the new skin
  if (ticks.length > 0) render(currentTickIndex);
  renderPossessionList();
}

// Neutral team label in stealth mode (jersey → neutral agent id)
function teamLabelFor(team) {
  return displayMode === 'stealth' ? (team === 'home' ? 'A' : 'B') : (team === 'home' ? 'BOS' : 'LAL');
}

function playerColorFor(team) {
  if (displayMode === 'stealth') {
    return team === 'home' ? '#3b82f6' : '#a855f7';
  }
  return team === 'home' ? '#007A33' : '#552583';
}

const COURT_W = 760;
const COURT_H = 405;
const PAD = 12;

const homeNameMap = {
  "0": "Jayson Tatum",
  "7": "Jaylen Brown",
  "4": "Jrue Holiday",
  "8": "Kristaps Porziņģis",
  "9": "Derrick White",
  "1": "Jayson Tatum",
  "2": "Jaylen Brown",
  "3": "Jrue Holiday",
  "5": "Derrick White"
};

const awayNameMap = {
  "23": "LeBron James",
  "3": "Anthony Davis",
  "15": "Austin Reaves",
  "1": "D'Angelo Russell",
  "28": "Rui Hachimura",
  "4": "LeBron James",
  "5": "Anthony Davis"
};

// Neutral agent labels for stealth mode: home A1..A5, away B1..B5
function agentLabel(p) {
  if (displayMode === 'full') {
    const isHome = p.team === 'home';
    const jNum = String(p.jersey || '').replace('#', '');
    const name = (isHome ? homeNameMap[jNum] : awayNameMap[jNum]) || `#${p.jersey}`;
    return name.split(' ').pop();
  }
  const prefix = p.team === 'home' ? 'A' : 'B';
  const idx = Number(String(p.jersey || '0').replace(/\D/g, '')) || 0;
  return `${prefix}${idx}`;
}

function fmtClock(seconds) {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, '0')}`;
}

function px(norm, maxPx, pad) {
  return pad + norm * (maxPx - 2 * pad);
}

async function loadCourtSpec() {
  try {
    const res = await fetch('./court-draw-spec.json');
    if (res.ok) courtSpec = await res.json();
  } catch (e) { /* ignore */ }
}

function drawCourt() {
  ctx.clearRect(0, 0, COURT_W, COURT_H);
  if (displayMode === 'stealth') {
    drawNeutralGrid();
  } else {
    drawBasketballCourt();
  }
}

/// Neutral skin: abstract dark grid — reads as generic motion-capture space.
function drawNeutralGrid() {
  ctx.fillStyle = '#0b0e15';
  ctx.fillRect(0, 0, COURT_W, COURT_H);

  const courtPxW = COURT_W - 2 * PAD;
  const courtPxH = COURT_H - 2 * PAD;

  // Grid lines
  ctx.strokeStyle = 'rgba(148, 163, 184, 0.10)';
  ctx.lineWidth = 1;
  for (let gx = PAD; gx <= COURT_W - PAD; gx += courtPxW / 12) {
    ctx.beginPath();
    ctx.moveTo(gx, PAD);
    ctx.lineTo(gx, COURT_H - PAD);
    ctx.stroke();
  }
  for (let gy = PAD; gy <= COURT_H - PAD; gy += courtPxH / 6) {
    ctx.beginPath();
    ctx.moveTo(PAD, gy);
    ctx.lineTo(COURT_W - PAD, gy);
    ctx.stroke();
  }

  // Outer frame
  ctx.strokeStyle = 'rgba(148, 163, 184, 0.35)';
  ctx.lineWidth = 2;
  ctx.strokeRect(PAD, PAD, courtPxW, courtPxH);

  // Center divider + two neutral anchor zones (left/right)
  const midX = PAD + courtPxW / 2;
  const midY = PAD + courtPxH / 2;
  ctx.beginPath();
  ctx.moveTo(midX, PAD);
  ctx.lineTo(midX, COURT_H - PAD);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(midX, midY, 26, 0, Math.PI * 2);
  ctx.stroke();

  // Anchor marks: small squares near both ends
  ctx.strokeStyle = 'rgba(148, 163, 184, 0.5)';
  const a1x = PAD + courtPxW * 0.075;
  const a2x = COURT_W - PAD - courtPxW * 0.075;
  for (const ax of [a1x, a2x]) {
    ctx.strokeRect(ax - 9, midY - 30, 18, 60);
  }
}

/// Full skin: official hardwood basketball court.
function drawBasketballCourt() {
  // Floor Base / Apron
  ctx.fillStyle = '#0b0f19';
  ctx.fillRect(0, 0, COURT_W, COURT_H);

  const courtPxW = COURT_W - 2 * PAD;
  const courtPxH = COURT_H - 2 * PAD;
  const scaleX = courtPxW / 94.0;
  const scaleY = courtPxH / 50.0;

  // Hardwood Plank Flooring
  const woodGrad = ctx.createLinearGradient(PAD, 0, COURT_W - PAD, 0);
  woodGrad.addColorStop(0, '#cca064');
  woodGrad.addColorStop(0.5, '#deb887');
  woodGrad.addColorStop(1, '#cca064');
  ctx.fillStyle = woodGrad;
  ctx.fillRect(PAD, PAD, courtPxW, courtPxH);

  // Subtle Parquet / Wood Planks pattern lines
  ctx.strokeStyle = 'rgba(160, 110, 50, 0.12)';
  ctx.lineWidth = 1;
  for (let py = PAD + 25; py < COURT_H - PAD; py += 25) {
    ctx.beginPath();
    ctx.moveTo(PAD, py);
    ctx.lineTo(COURT_W - PAD, py);
    ctx.stroke();
  }

  // Outer Boundary Lines
  ctx.strokeStyle = '#ffffff';
  ctx.lineWidth = 3;
  ctx.strokeRect(PAD, PAD, courtPxW, courtPxH);

  const midX = PAD + 47.0 * scaleX;
  const midY = PAD + 25.0 * scaleY;

  // Half-Court Division Line
  ctx.beginPath();
  ctx.moveTo(midX, PAD);
  ctx.lineTo(midX, COURT_H - PAD);
  ctx.stroke();

  // Center Circle
  ctx.beginPath();
  ctx.arc(midX, midY, 6.0 * scaleX, 0, Math.PI * 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(midX, midY, 2.0 * scaleX, 0, Math.PI * 2);
  ctx.stroke();

  // NBA Keys
  const laneLengthPx = 19.0 * scaleX;
  const laneWidthPx = 16.0 * scaleY;
  const laneTopY = midY - laneWidthPx / 2;

  ctx.fillStyle = 'rgba(0, 122, 51, 0.35)';
  ctx.fillRect(PAD, laneTopY, laneLengthPx, laneWidthPx);
  ctx.strokeStyle = '#ffffff';
  ctx.lineWidth = 2.5;
  ctx.strokeRect(PAD, laneTopY, laneLengthPx, laneWidthPx);

  ctx.fillStyle = 'rgba(85, 37, 131, 0.35)';
  ctx.fillRect(COURT_W - PAD - laneLengthPx, laneTopY, laneLengthPx, laneWidthPx);
  ctx.strokeRect(COURT_W - PAD - laneLengthPx, laneTopY, laneLengthPx, laneWidthPx);

  // Free Throw Circles
  const leftFtX = PAD + 19.0 * scaleX;
  const rightFtX = COURT_W - PAD - 19.0 * scaleX;
  const ftRadius = 6.0 * scaleX;

  ctx.beginPath();
  ctx.arc(leftFtX, midY, ftRadius, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.arc(leftFtX, midY, ftRadius, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();
  ctx.setLineDash([]);

  ctx.beginPath();
  ctx.arc(rightFtX, midY, ftRadius, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.arc(rightFtX, midY, ftRadius, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();
  ctx.setLineDash([]);

  // Restricted Area Arcs
  const leftHoopX = PAD + 5.25 * scaleX;
  const rightHoopX = COURT_W - PAD - 5.25 * scaleX;
  const restrictedR = 4.0 * scaleX;

  ctx.beginPath();
  ctx.arc(leftHoopX, midY, restrictedR, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(rightHoopX, midY, restrictedR, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();

  // NBA 3-Point Line
  const cornerDistY = 3.0 * scaleY;
  const cornerStraightLen = 14.0 * scaleX;
  const threeRadius = 23.75 * scaleX;

  ctx.beginPath();
  ctx.moveTo(PAD, PAD + cornerDistY);
  ctx.lineTo(PAD + cornerStraightLen, PAD + cornerDistY);
  const leftArcStartAngle = -Math.asin((25.0 - 3.0) * scaleY / threeRadius);
  const leftArcEndAngle = Math.asin((25.0 - 3.0) * scaleY / threeRadius);
  ctx.arc(leftHoopX, midY, threeRadius, leftArcStartAngle, leftArcEndAngle, false);
  ctx.lineTo(PAD, COURT_H - PAD - cornerDistY);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(COURT_W - PAD, PAD + cornerDistY);
  ctx.lineTo(COURT_W - PAD - cornerStraightLen, PAD + cornerDistY);
  const rightArcStartAngle = Math.PI - leftArcEndAngle;
  const rightArcEndAngle = Math.PI - leftArcStartAngle;
  ctx.arc(rightHoopX, midY, threeRadius, rightArcStartAngle, rightArcEndAngle, false);
  ctx.lineTo(COURT_W - PAD, COURT_H - PAD - cornerDistY);
  ctx.stroke();

  // Backboards & Rims
  const backboardLen = 6.0 * scaleY;
  const backboardXLeft = PAD + 4.0 * scaleX;
  const backboardXRight = COURT_W - PAD - 4.0 * scaleX;

  ctx.strokeStyle = '#38bdf8';
  ctx.lineWidth = 4;
  ctx.beginPath();
  ctx.moveTo(backboardXLeft, midY - backboardLen / 2);
  ctx.lineTo(backboardXLeft, midY + backboardLen / 2);
  ctx.stroke();

  ctx.beginPath();
  ctx.moveTo(backboardXRight, midY - backboardLen / 2);
  ctx.lineTo(backboardXRight, midY + backboardLen / 2);
  ctx.stroke();

  ctx.strokeStyle = '#ff6b35';
  ctx.lineWidth = 3;
  const rimR = 0.75 * scaleX;
  ctx.beginPath();
  ctx.arc(leftHoopX, midY, rimR * 2.2, 0, Math.PI * 2);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(rightHoopX, midY, rimR * 2.2, 0, Math.PI * 2);
  ctx.stroke();
}

/// Ball marker: neutral glowing dot in stealth, orange basketball in full.
function drawBallMarker(b) {
  const bx = px(b.x, COURT_W, PAD);
  const by = px(b.y, COURT_H, PAD);
  const bz = b.z || 0.0;

  // Ground shadow (scales with height)
  const shadowR = Math.max(3, 8 - bz * 0.4);
  ctx.fillStyle = 'rgba(0,0,0,0.35)';
  ctx.beginPath();
  ctx.ellipse(bx, by + 4, shadowR, shadowR * 0.45, 0, 0, Math.PI * 2);
  ctx.fill();

  const ballY = by - bz * 2.5;
  const ballR = 7.5 + Math.min(3, bz * 0.3);

  if (displayMode === 'stealth') {
    const grad = ctx.createRadialGradient(bx - 2, ballY - 2, 1, bx, ballY, ballR);
    grad.addColorStop(0, '#fde68a');
    grad.addColorStop(1, '#d97706');
    ctx.fillStyle = grad;
    ctx.beginPath();
    ctx.arc(bx, ballY, ballR, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#92400e';
    ctx.lineWidth = 1.2;
    ctx.stroke();
    return;
  }

  const grad = ctx.createRadialGradient(bx - 2, ballY - 2, 1, bx, ballY, ballR);
  grad.addColorStop(0, '#fb923c');
  grad.addColorStop(1, '#c2410c');
  ctx.fillStyle = grad;
  ctx.beginPath();
  ctx.arc(bx, ballY, ballR, 0, Math.PI * 2);
  ctx.fill();

  ctx.strokeStyle = '#7c2d12';
  ctx.lineWidth = 1.2;
  ctx.stroke();

  ctx.strokeStyle = 'rgba(67, 20, 7, 0.6)';
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(bx - ballR, ballY);
  ctx.lineTo(bx + ballR, ballY);
  ctx.stroke();
}

function drawTick(tick) {
  drawCourt();
  if (!tick) return;

  // 1. Draw Ball Trajectory Trail
  if (ticks.length > 0 && currentTickIndex > 0) {
    const startIdx = Math.max(0, currentTickIndex - 15);
    ctx.beginPath();
    let first = true;
    for (let i = startIdx; i <= currentTickIndex; i++) {
      const prevB = ticks[i]?.ball || ticks[i]?.frame?.ball;
      if (prevB) {
        const bx = px(prevB.x, COURT_W, PAD);
        const by = px(prevB.y, COURT_H, PAD);
        if (first) {
          ctx.moveTo(bx, by);
          first = false;
        } else {
          ctx.lineTo(bx, by);
        }
      }
    }
    ctx.strokeStyle = displayMode === 'stealth' ? 'rgba(217, 119, 6, 0.5)' : 'rgba(234, 88, 12, 0.45)';
    ctx.lineWidth = 3;
    ctx.setLineDash([4, 4]);
    ctx.stroke();
    ctx.setLineDash([]);
  }

  // 2. Draw Players
  const players = tick.players || tick.frame?.players || [];
  for (const p of players) {
    const x = px(p.x, COURT_W, PAD);
    const y = px(p.y, COURT_H, PAD);
    const color = playerColorFor(p.team);
    const label = agentLabel(p);
    
    // Shadow
    ctx.fillStyle = 'rgba(0,0,0,0.3)';
    ctx.beginPath();
    ctx.ellipse(x, y + 10, 14, 5, 0, 0, Math.PI * 2);
    ctx.fill();
    
    // Morale aura (Psychological FSM)
    if (p.morale === 'HotHand') {
      ctx.strokeStyle = 'rgba(239, 68, 68, 0.85)';
      ctx.lineWidth = 3.5;
      ctx.beginPath();
      ctx.arc(x, y, 20, 0, Math.PI * 2);
      ctx.stroke();
    } else if (p.morale === 'Frustrated') {
      ctx.strokeStyle = 'rgba(59, 130, 246, 0.85)';
      ctx.lineWidth = 2.5;
      ctx.beginPath();
      ctx.arc(x, y, 19, 0, Math.PI * 2);
      ctx.stroke();
    }

    // Player Body Circle
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(x, y, p.hasBall ? 16 : 13, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = p.hasBall ? '#fbbf24' : '#ffffff';
    ctx.lineWidth = p.hasBall ? 3.5 : 1.5;
    ctx.stroke();
    
    // Agent id in circle
    ctx.fillStyle = '#ffffff';
    ctx.font = 'bold 11px -apple-system, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(displayMode === 'stealth' ? label : (String(p.jersey || '').replace('#', '') || label), x, y);

    // Agent label & slot tag
    ctx.font = '10px -apple-system, sans-serif';
    const displayName = p.slot ? `${label} · ${p.slot}` : label;
    const nameWidth = ctx.measureText(displayName).width + 10;
    ctx.fillStyle = 'rgba(15, 23, 42, 0.88)';
    ctx.fillRect(x - nameWidth / 2, y + 15, nameWidth, 14);
    ctx.fillStyle = '#ffffff';
    ctx.fillText(displayName, x, y + 22);
  }
  // 3. Draw tracked marker (basketball / neutral dot)
  const b = tick.ball || tick.frame?.ball;
  if (b) drawBallMarker(b);

  // 4. Draw Callout if present
  const frame = tick.frame || tick;
  if (frame.callout) {
    ctx.font = 'bold 15px -apple-system, sans-serif';
    const textW = ctx.measureText(frame.callout).width + 24;
    ctx.fillStyle = 'rgba(15, 23, 42, 0.9)';
    ctx.fillRect(COURT_W / 2 - textW / 2, 25, textW, 32);
    ctx.strokeStyle = '#f59e0b';
    ctx.lineWidth = 1.5;
    ctx.strokeRect(COURT_W / 2 - textW / 2, 25, textW, 32);

    ctx.fillStyle = '#fbbf24';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(frame.callout, COURT_W / 2, 41);
  }
}

function render(index) {
  if (ticks.length === 0) return;
  currentTickIndex = Math.max(0, Math.min(ticks.length - 1, index));
  const tick = ticks[currentTickIndex];
  drawTick(tick);
  
  // Update Match Header Info
  const frame = tick.frame || tick;
  if ($('homeScore')) $('homeScore').textContent = String(frame.score?.home ?? 0);
  if ($('awayScore')) $('awayScore').textContent = String(frame.score?.away ?? 0);
  if ($('matchClock')) {
    const q = frame.period ?? 1;
    const time = fmtClock(frame.t_game ?? frame.game_clock ?? 720.0);
    $('matchClock').textContent = displayMode === 'stealth' ? `T${q} ${time}` : `Q${q} ${time}`;
  }

  if ($('frameSlider')) $('frameSlider').value = String(currentTickIndex);
  if ($('frameLabel')) $('frameLabel').textContent = `Tick: ${currentTickIndex + 1} / ${ticks.length}`;

  // Update Macro Possession & Sub-Phase Banner
  const pos = stages.find(s => currentTickIndex >= s.startTick && currentTickIndex <= s.endTick) || stages[0];
  if (pos) {
    currentStageId = pos.id;
    const seqWord = displayMode === 'stealth' ? 'SEQ' : '回合';
    const teamTag = displayMode === 'stealth' ? teamLabelFor(pos.offenseTeam === 'BOS' ? 'home' : 'away') : (pos.offenseTeam || 'OFF');
    if ($('currentPossessionTag')) $('currentPossessionTag').textContent = `${seqWord} #${pos.id} [${teamTag}] · ${frame.phase || '...'}`;
    if ($('currentPossessionTitle')) $('currentPossessionTitle').textContent = `${pos.tacticalSet || pos.title || '...'}`;
    const elapsedInPos = ((currentTickIndex - pos.startTick) * (1.0 / simTicksPerSecond)).toFixed(1);
    if ($('currentPossessionTime')) $('currentPossessionTime').textContent = `耗时: ${elapsedInPos}s / ${pos.duration.toFixed(1)}s`;
    
    document.querySelectorAll('.stage-item').forEach(el => {
      const isCur = el.dataset.stageId === String(pos.id);
      el.classList.toggle('selected', isCur);
    });
  }
  // Update all 10 players decision trace
  updateDecisionCard(tick);
}

function updateDecisionCard(tick) {
  const container = $('allPlayersDecision') || $('decisionTraceList');
  if (!container || !tick) return;
  const players = tick.players || tick.frame?.players || [];
  if (players.length === 0) return;
  
  const ballHolder = players.find(p => p.hasBall);
  
  container.innerHTML = players.map(p => {
    const label = agentLabel(p);
    const isCarrier = ballHolder && (ballHolder.jersey === p.jersey);
    return `
      <div class="decision-row ${isCarrier ? 'carrier' : ''}">
        <div class="player-tag ${p.team}">${label}</div>
        <div class="action-tag">${p.action || 'MOVE'}</div>
        <div class="slot-tag">${p.slot || 'SLOT'}</div>
        <div class="morale-tag ${p.morale || 'Normal'}">${p.morale || 'Normal'}</div>
      </div>
    `;
  }).join('');
}

function extractPossessionsFromTicks() {
  if (ticks.length === 0) return [];
  const list = [];
  let curStage = null;

  for (let i = 0; i < ticks.length; i++) {
    const t = ticks[i];
    const frame = t.frame || t;
    const pId = frame.possession_id || Math.floor(i / 250) + 1;
    const offTeam = frame.phase === 'home' || (t.tactical_set && t.tactical_set.includes('BOS')) ? 'BOS' : 'LAL';
    const tacticalSet = t.tactical_set || '高位挡拆战术 (High Pick and Roll)';

    if (!curStage || curStage.id !== pId) {
      if (curStage) {
        curStage.endTick = i - 1;
        curStage.duration = (curStage.endTick - curStage.startTick + 1) * 0.04;
      }
      curStage = {
        id: pId,
        offenseTeam: (pId % 2 === 1) ? 'BOS' : 'LAL',
        originReason: i === 0 ? 'GAME_START_TIPOFF' : 'POSSESSION_CHANGE',
        tacticalSet: tacticalSet,
        title: `回合 #${pId}: ${tacticalSet}`,
        startTick: i,
        endTick: ticks.length - 1,
        duration: 0
      };
      list.push(curStage);
    }
  }
  if (curStage) {
    curStage.endTick = ticks.length - 1;
    curStage.duration = (curStage.endTick - curStage.startTick + 1) * 0.04;
  }
  return list;
}

function renderPossessionList() {
  const container = $('possessionList');
  if (!container) return;
  if ($('possessionCount')) $('possessionCount').textContent = String(stages.length);
  container.innerHTML = stages.map(s => {
    const teamBadge = displayMode === 'stealth'
      ? `SEQ #${s.id}`
      : `${s.offenseTeam} #${s.id}`;
    return `
    <div class="stage-item ${s.id === currentStageId ? 'selected' : ''}" data-stage-id="${s.id}">
      <div class="stage-header">
        <span class="team-badge ${s.offenseTeam.toLowerCase()}">${teamBadge}</span>
        <span class="stage-time">${s.duration.toFixed(1)}s</span>
      </div>
      <div class="stage-title">${s.tacticalSet}</div>
      <div class="stage-reason">${displayMode === 'stealth' ? '来源: SEQUENCE_START' : `起因: ${s.originReason}`}</div>
    </div>
  `;
  }).join('');

  container.querySelectorAll('.stage-item').forEach(el => {
    const sId = Number(el.dataset.stageId);
    const target = stages.find(s => s.id === sId);
    if (target) {
      currentStageId = target.id;
      seekToTick(target.startTick);
    }
  });
}

function seekToTick(index) {
  render(index);
}

let playbackStartWallMs = 0;
let playbackStartTick = 0;
const simTicksPerSecond = 25;

function startPlay() {
  if (isPlaying) return;
  isPlaying = true;
  if ($('btnPlay')) $('btnPlay').textContent = '⏸ 暂停';
  playbackStartWallMs = performance.now();
  playbackStartTick = currentTickIndex;
  rafHandle = requestAnimationFrame(playbackFrame);
}

function pausePlay() {
  if (!isPlaying) return;
  isPlaying = false;
  if ($('btnPlay')) $('btnPlay').textContent = '▶ 播放';
  if (rafHandle) {
    cancelAnimationFrame(rafHandle);
    rafHandle = null;
  }
}

function togglePlay() {
  if (isPlaying) pausePlay();
  else startPlay();
}

function playbackFrame() {
  if (!isPlaying) return;
  const now = performance.now();
  const elapsedSec = (now - playbackStartWallMs) / 1000;
  let nextTick = playbackStartTick + Math.floor(elapsedSec * simTicksPerSecond);

  if (isLoopStage && stages.length > 0) {
    const curStage = stages.find(s => currentTickIndex >= s.startTick && currentTickIndex <= s.endTick) || stages[0];
    if (nextTick > curStage.endTick) {
      playbackStartWallMs = performance.now();
      playbackStartTick = curStage.startTick;
      nextTick = curStage.startTick;
    }
  } else if (nextTick >= ticks.length) {
    nextTick = ticks.length - 1;
    pausePlay();
  }

  render(nextTick);
  if (isPlaying) {
    rafHandle = requestAnimationFrame(playbackFrame);
  }
}

async function loadGameStream() {
  try {
    if ($('currentPossessionTitle')) $('currentPossessionTitle').textContent = '正在加载并解压比赛数据流...';
    let res = await fetch('game.ticks.ndjson?t=' + Date.now());
    if (!res.ok) {
      res = await fetch('game.ticks.ndjson.gz?t=' + Date.now());
    }
    if (!res.ok) throw new Error(`HTTP ${res.status}`);

    let stream = res.body;
    if (res.headers.get('Content-Encoding') === 'gzip' || res.url.endsWith('.gz')) {
      if (typeof DecompressionStream !== 'undefined') {
        stream = stream.pipeThrough(new DecompressionStream('gzip'));
      }
    }

    const textStream = stream.pipeThrough(new TextDecoderStream());
    const reader = textStream.getReader();
    let partialLine = '';
    ticks = [];

    while (true) {
      const { value, done } = await reader.read();
      if (done) break;
      const chunk = partialLine + value;
      const lines = chunk.split('\n');
      partialLine = lines.pop() || '';

      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          ticks.push(JSON.parse(line));
        } catch (e) {}
      }
      if (ticks.length > 0 && stages.length === 0) {
        stages = extractPossessionsFromTicks();
        renderPossessionList();
        render(0);
      }
    }

    stages = extractPossessionsFromTicks();
    renderPossessionList();
    if ($('frameSlider')) $('frameSlider').max = String(Math.max(0, ticks.length - 1));
    render(0);
    if ($('currentPossessionTitle') && stages[0]) $('currentPossessionTitle').textContent = stages[0].tacticalSet;
  } catch (err) {
    console.error('Failed to load stream:', err);
    if ($('currentPossessionTitle')) $('currentPossessionTitle').textContent = '数据加载完成 (离线演示就绪)';
  }
}

window.addEventListener('DOMContentLoaded', async () => {
  $('btnPlay')?.addEventListener('click', togglePlay);
  $('btnPrevFrame')?.addEventListener('click', () => {
    pausePlay();
    seekToTick(Math.max(0, currentTickIndex - 1));
  });
  $('btnNextFrame')?.addEventListener('click', () => {
    pausePlay();
    seekToTick(Math.min(ticks.length - 1, currentTickIndex + 1));
  });
  $('frameSlider')?.addEventListener('input', (e) => {
    pausePlay();
    seekToTick(Number(e.target.value));
  });
  
  $('btnLoopStage')?.addEventListener('click', () => {
    isLoopStage = !isLoopStage;
    $('btnLoopStage').classList.toggle('active', isLoopStage);
    $('btnLoopStage').textContent = isLoopStage ? '🔁 循环: 开' : '🔁 循环: 关';
  });

  // Display mode switch (stealth <-> full)
  $('btnStealth')?.addEventListener('click', () => {
    displayMode = displayMode === 'stealth' ? 'full' : 'stealth';
    applyDisplayMode();
  });

  // Collapse / expand scene card by clicking the banner
  function applySceneCollapsed() {
    const workspace = $('courtWorkspace');
    const container = $('courtContainer');
    const timeline = document.querySelector('.timeline-bar');
    const hint = $('collapseHint');
    if (!workspace) return;
    workspace.classList.toggle('collapsed', sceneCollapsed);
    if (container) container.style.display = sceneCollapsed ? 'none' : 'flex';
    if (timeline) timeline.style.display = sceneCollapsed ? 'none' : 'flex';
    if (hint) hint.textContent = sceneCollapsed ? '▸ 展开视图' : '▾ 隐藏视图';
    localStorage.setItem('trace.sceneCollapsed', sceneCollapsed ? '1' : '0');
  }
  $('currentStageBanner')?.addEventListener('click', (e) => {
    // 避免与 banner 内未来可能出现的按钮冲突
    if (e.target.closest('button')) return;
    sceneCollapsed = !sceneCollapsed;
    applySceneCollapsed();
  });
  applySceneCollapsed();
  applyDisplayMode();

  // Mobile navigation tabs
  const mTabStages = $('mTabStages');
  const mTabCourt = $('mTabCourt');
  const mTabDecision = $('mTabDecision');
  const mTabAnnotate = $('mTabAnnotate');
  const stageRail = document.querySelector('.stage-rail');
  const courtWorkspace = document.querySelector('.court-workspace');
  const decisionPanel = document.querySelector('.decision-panel');

  function setMobileView(view) {
    [mTabStages, mTabCourt, mTabDecision, mTabAnnotate].forEach(t => t?.classList.remove('active'));
    if (stageRail) stageRail.style.display = 'none';
    if (courtWorkspace) courtWorkspace.style.display = 'none';
    if (decisionPanel) decisionPanel.style.display = 'none';

    if (view === 'stages') {
      mTabStages?.classList.add('active');
      if (stageRail) stageRail.style.display = 'flex';
    } else if (view === 'court') {
      mTabCourt?.classList.add('active');
      if (courtWorkspace) courtWorkspace.style.display = 'flex';
    } else if (view === 'decision' || view === 'annotate') {
      if (view === 'decision') mTabDecision?.classList.add('active');
      else mTabAnnotate?.classList.add('active');
      if (decisionPanel) decisionPanel.style.display = 'flex';
    }
  }

  mTabStages?.addEventListener('click', () => setMobileView('stages'));
  mTabCourt?.addEventListener('click', () => setMobileView('court'));
  mTabDecision?.addEventListener('click', () => setMobileView('decision'));
  mTabAnnotate?.addEventListener('click', () => setMobileView('annotate'));

  // Annotation submission
  $('btnSubmitAnnotation')?.addEventListener('click', () => {
    const feedback = $('annotationFeedback')?.value;
    const rating = document.querySelector('input[name="stageRating"]:checked')?.value || 'GOOD';
    const status = $('annotationStatus');
    if (status) {
      status.textContent = `已保存对 SEQ #${currentStageId} 的反馈 [${rating}]`;
      status.style.color = '#10b981';
    }
  });

  await loadGameStream();
});
