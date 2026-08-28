/**
 * NBA-Sim Micro-Stage & Annotation Workbench v3.0
 * Professional 2D NBA Court Renderer + Dynamic Tactical Plays & Decision Trace
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

const COURT_W = 940;
const COURT_H = 500;
const PAD = 15;

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

  // Outer Boundary Lines (Official 2-inch solid white lines)
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

  // Center Circle (6 ft radius inner, 2 ft radius center jump circle)
  ctx.beginPath();
  ctx.arc(midX, midY, 6.0 * scaleX, 0, Math.PI * 2);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(midX, midY, 2.0 * scaleX, 0, Math.PI * 2);
  ctx.stroke();

  // NBA Keys: 16 ft wide (y: 17ft to 33ft, height 16ft), 19 ft long (from baseline to FT line)
  const laneLengthPx = 19.0 * scaleX;
  const laneWidthPx = 16.0 * scaleY;
  const laneTopY = midY - laneWidthPx / 2;

  // Left Paint (Celtics Green Fill)
  ctx.fillStyle = 'rgba(0, 122, 51, 0.35)';
  ctx.fillRect(PAD, laneTopY, laneLengthPx, laneWidthPx);
  ctx.strokeStyle = '#ffffff';
  ctx.lineWidth = 2.5;
  ctx.strokeRect(PAD, laneTopY, laneLengthPx, laneWidthPx);

  // Right Paint (Lakers Purple Fill)
  ctx.fillStyle = 'rgba(85, 37, 131, 0.35)';
  ctx.fillRect(COURT_W - PAD - laneLengthPx, laneTopY, laneLengthPx, laneWidthPx);
  ctx.strokeRect(COURT_W - PAD - laneLengthPx, laneTopY, laneLengthPx, laneWidthPx);

  // Free Throw Circles (6 ft radius at 19 ft from baseline)
  const leftFtX = PAD + 19.0 * scaleX;
  const rightFtX = COURT_W - PAD - 19.0 * scaleX;
  const ftRadius = 6.0 * scaleX;

  // Left FT Circle (Solid towards midcourt, dashed towards baseline)
  ctx.beginPath();
  ctx.arc(leftFtX, midY, ftRadius, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.arc(leftFtX, midY, ftRadius, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();
  ctx.setLineDash([]);

  // Right FT Circle
  ctx.beginPath();
  ctx.arc(rightFtX, midY, ftRadius, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();
  ctx.setLineDash([6, 6]);
  ctx.beginPath();
  ctx.arc(rightFtX, midY, ftRadius, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();
  ctx.setLineDash([]);

  // NBA Restricted Area Arcs (4 ft radius from hoop center)
  const leftHoopX = PAD + 5.25 * scaleX;
  const rightHoopX = COURT_W - PAD - 5.25 * scaleX;
  const restrictedR = 4.0 * scaleX;

  ctx.beginPath();
  ctx.arc(leftHoopX, midY, restrictedR, -Math.PI / 2, Math.PI / 2, false);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(rightHoopX, midY, restrictedR, Math.PI / 2, Math.PI * 1.5, false);
  ctx.stroke();

  // Official NBA 3-Point Line:
  // Corner 3: 22 ft from hoop (3 ft from sideline, extends 14 ft from baseline)
  // Arc: 23.75 ft radius from hoop center
  const cornerDistY = 3.0 * scaleY;
  const cornerStraightLen = 14.0 * scaleX;
  const threeRadius = 23.75 * scaleX;

  // Left 3PT Line
  ctx.beginPath();
  ctx.moveTo(PAD, PAD + cornerDistY);
  ctx.lineTo(PAD + cornerStraightLen, PAD + cornerDistY);
  const leftArcStartAngle = -Math.asin((25.0 - 3.0) * scaleY / threeRadius);
  const leftArcEndAngle = Math.asin((25.0 - 3.0) * scaleY / threeRadius);
  ctx.arc(leftHoopX, midY, threeRadius, leftArcStartAngle, leftArcEndAngle, false);
  ctx.lineTo(PAD, COURT_H - PAD - cornerDistY);
  ctx.stroke();

  // Right 3PT Line
  ctx.beginPath();
  ctx.moveTo(COURT_W - PAD, PAD + cornerDistY);
  ctx.lineTo(COURT_W - PAD - cornerStraightLen, PAD + cornerDistY);
  const rightArcStartAngle = Math.PI - leftArcEndAngle;
  const rightArcEndAngle = Math.PI - leftArcStartAngle;
  ctx.arc(rightHoopX, midY, threeRadius, rightArcStartAngle, rightArcEndAngle, false);
  ctx.lineTo(COURT_W - PAD, COURT_H - PAD - cornerDistY);
  ctx.stroke();

  // Backboards (6 ft wide, 4 ft from baseline) & Rims (18-inch diameter, 5.25 ft from baseline)
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

  // Orange Rims
  ctx.strokeStyle = '#ff6b35';
  ctx.lineWidth = 3;
  const rimR = 0.75 * scaleX; // 9 inch radius
  ctx.beginPath();
  ctx.arc(leftHoopX, midY, rimR * 2.2, 0, Math.PI * 2);
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(rightHoopX, midY, rimR * 2.2, 0, Math.PI * 2);
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
    ctx.strokeStyle = 'rgba(234, 88, 12, 0.45)';
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
    const isHome = p.team === 'home';
    const color = isHome ? '#007A33' : '#552583';
    const jNum = String(p.jersey || '').replace('#', '').replace('A', '').replace('H', '');
    const name = (isHome ? homeNameMap[jNum] : awayNameMap[jNum]) || `#${p.jersey}`;
    
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
    
    // Jersey Number
    ctx.fillStyle = '#ffffff';
    ctx.font = 'bold 11px -apple-system, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(jNum || p.jersey, x, y);
    
    // Player Name Tag & Tactical Slot Label
    ctx.font = '10px -apple-system, sans-serif';
    const displayName = p.slot ? `${name.split(' ').pop()} (${p.slot})` : name.split(' ').pop();
    const nameWidth = ctx.measureText(displayName).width + 10;
    ctx.fillStyle = 'rgba(15, 23, 42, 0.88)';
    ctx.fillRect(x - nameWidth / 2, y + 15, nameWidth, 14);
    ctx.fillStyle = '#ffffff';
    ctx.fillText(displayName, x, y + 22);
  }
  
  // 3. Draw Basketball with 3D Height Shadow
  const b = tick.ball || tick.frame?.ball;
  if (b) {
    const bx = px(b.x, COURT_W, PAD);
    const by = px(b.y, COURT_H, PAD);
    const bz = b.z || 0.0;
    
    // Ground Shadow (scales with height)
    const shadowR = Math.max(3, 8 - bz * 0.4);
    ctx.fillStyle = 'rgba(0,0,0,0.35)';
    ctx.beginPath();
    ctx.ellipse(bx, by + 4, shadowR, shadowR * 0.45, 0, 0, Math.PI * 2);
    ctx.fill();
    
    // Ball Body (height offset on y axis)
    const ballY = by - bz * 2.5;
    const ballR = 7.5 + Math.min(3, bz * 0.3);
    
    // Ball Gradient
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
    
    // Ball Seams
    ctx.strokeStyle = 'rgba(67, 20, 7, 0.6)';
    ctx.lineWidth = 1;
    ctx.beginPath();
    ctx.moveTo(bx - ballR, ballY);
    ctx.lineTo(bx + ballR, ballY);
    ctx.stroke();
  }

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
  if ($('matchClock')) $('matchClock').textContent = `Q${frame.period ?? 1} ${fmtClock(frame.t_game ?? frame.game_clock ?? 720.0)}`;
  
  if ($('frameSlider')) $('frameSlider').value = String(currentTickIndex);
  if ($('frameLabel')) $('frameLabel').textContent = `Tick: ${currentTickIndex + 1} / ${ticks.length}`;
  
  // Update Macro Possession & Sub-Phase Banner
  const pos = stages.find(s => currentTickIndex >= s.startTick && currentTickIndex <= s.endTick) || stages[0];
  if (pos) {
    currentStageId = pos.id;
    if ($('currentPossessionTag')) $('currentPossessionTag').textContent = `回合 #${pos.id} [${pos.offenseTeam || 'OFF'}] · ${frame.phase || tick.possessionSubPhase || '战术组织'}`;
    if ($('currentPossessionTitle')) $('currentPossessionTitle').textContent = `${pos.tacticalSet || pos.title || '战术推进'}`;
    const elapsedInPos = ((currentTickIndex - pos.startTick) * (1.0 / simTicksPerSecond)).toFixed(1);
    if ($('currentPossessionTime')) $('currentPossessionTime').textContent = `起因: ${pos.originReason || '发球'} | 回合耗时: ${elapsedInPos}s / ${pos.duration.toFixed(1)}s`;
    
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
    const isHome = p.team === 'home';
    const jNum = String(p.jersey || '').replace('#', '').replace('A', '').replace('H', '');
    const name = (isHome ? homeNameMap[jNum] : awayNameMap[jNum]) || `#${p.jersey}`;
    const isCarrier = ballHolder && (ballHolder.jersey === p.jersey);
    return `
      <div class="decision-row ${isCarrier ? 'carrier' : ''}">
        <div class="player-tag ${p.team}">#${jNum} ${name}</div>
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
  container.innerHTML = stages.map(s => `
    <div class="stage-item ${s.id === currentStageId ? 'selected' : ''}" data-stage-id="${s.id}">
      <div class="stage-header">
        <span class="team-badge ${s.offenseTeam.toLowerCase()}">${s.offenseTeam} #${s.id}</span>
        <span class="stage-time">${s.duration.toFixed(1)}s</span>
      </div>
      <div class="stage-title">${s.tacticalSet}</div>
      <div class="stage-reason">起因: ${s.originReason}</div>
    </div>
  `).join('');

  container.querySelectorAll('.stage-item').forEach(el => {
    el.addEventListener('click', () => {
      const sId = Number(el.dataset.stageId);
      const target = stages.find(s => s.id === sId);
      if (target) {
        currentStageId = target.id;
        seekToTick(target.startTick);
      }
    });
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
    $('btnLoopStage').textContent = isLoopStage ? '🔁 单阶段循环: 开' : '🔁 单阶段循环: 关';
  });

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
      status.textContent = `已成功保存对 回合 #${currentStageId} 的反馈 [${rating}]！`;
      status.style.color = '#10b981';
      setTimeout(() => { status.textContent = ''; }, 3000);
    }
  });

  await loadGameStream();
});
