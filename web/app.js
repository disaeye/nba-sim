/**
 * NBA-Sim Micro-Stage & Annotation Workbench v2.0
 * Focus on Stage-by-Stage Play Debugging, Decision Tracing, and Human Feedback Annotation
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
let playInterval = null;
let courtSpec = null;
let courtCanvas = null;

const COURT_W = 940;
const COURT_H = 500;
const PAD = 8;

const homeNameMap = {
  "0": "Jayson Tatum",
  "7": "Jaylen Brown",
  "4": "Jrue Holiday",
  "8": "Kristaps Porziņģis",
  "9": "Derrick White"
};

const awayNameMap = {
  "23": "LeBron James",
  "3": "Anthony Davis",
  "15": "Austin Reaves",
  "1": "D'Angelo Russell",
  "28": "Rui Hachimura"
};

function fmtClock(seconds) {
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${String(s).padStart(2, '0')}`;
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
  // Draw base court floor
  ctx.fillStyle = '#c89968';
  ctx.fillRect(0, 0, COURT_W, COURT_H);
  
  // Outer boundary line
  ctx.strokeStyle = '#5d3215';
  ctx.lineWidth = 2.5;
  ctx.strokeRect(PAD, PAD, COURT_W - 2 * PAD, COURT_H - 2 * PAD);
  
  // Mid-court line & center circle
  ctx.beginPath();
  ctx.moveTo(COURT_W / 2, PAD);
  ctx.lineTo(COURT_W / 2, COURT_H - PAD);
  ctx.stroke();
  
  ctx.beginPath();
  ctx.arc(COURT_W / 2, COURT_H / 2, 60, 0, Math.PI * 2);
  ctx.stroke();
  
  // Key Areas / Paint
  ctx.fillStyle = 'rgba(0, 122, 51, 0.15)'; // Left Celtics paint
  ctx.fillRect(PAD, COURT_H / 2 - 80, 190, 160);
  ctx.strokeRect(PAD, COURT_H / 2 - 80, 190, 160);
  
  ctx.fillStyle = 'rgba(85, 37, 131, 0.15)'; // Right Lakers paint
  ctx.fillRect(COURT_W - PAD - 190, COURT_H / 2 - 80, 190, 160);
  ctx.strokeRect(COURT_W - PAD - 190, COURT_H / 2 - 80, 190, 160);
  
  // Left & Right Hoops
  ctx.fillStyle = '#ff6b35';
  ctx.beginPath();
  ctx.arc(PAD + 52, COURT_H / 2, 8, 0, Math.PI * 2);
  ctx.fill();
  ctx.beginPath();
  ctx.arc(COURT_W - PAD - 52, COURT_H / 2, 8, 0, Math.PI * 2);
  ctx.fill();
}

function drawTick(tick) {
  drawCourt();
  if (!tick) return;
  
  // Draw Players
  const players = tick.players || [];
  for (const p of players) {
    const x = px(p.x, COURT_W, PAD);
    const y = px(p.y, COURT_H, PAD);
    const isHome = p.team === 'home';
    const color = isHome ? '#007A33' : '#552583';
    const name = (isHome ? homeNameMap[p.jersey] : awayNameMap[p.jersey]) || `#${p.jersey}`;
    
    // Shadow
    ctx.fillStyle = 'rgba(0,0,0,0.25)';
    ctx.beginPath();
    ctx.ellipse(x, y + 4, 14, 6, 0, 0, Math.PI * 2);
    ctx.fill();
    
    // Player Body Circle
    ctx.fillStyle = color;
    ctx.beginPath();
    ctx.arc(x, y, p.hasBall ? 16 : 13, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = p.hasBall ? '#fbbf24' : '#ffffff';
    ctx.lineWidth = p.hasBall ? 3 : 1.5;
    ctx.stroke();
    
    // Jersey Number
    ctx.fillStyle = '#ffffff';
    ctx.font = 'bold 11px -apple-system, sans-serif';
    ctx.textAlign = 'center';
    ctx.textBaseline = 'middle';
    ctx.fillText(p.jersey, x, y);
    
    // Name Tag
    ctx.fillStyle = 'rgba(15, 23, 42, 0.85)';
    const nameWidth = ctx.measureText(name.split(' ').pop()).width + 10;
    ctx.fillRect(x - nameWidth / 2, y + 15, nameWidth, 14);
    ctx.fillStyle = '#ffffff';
    ctx.font = '10px -apple-system, sans-serif';
    ctx.fillText(name.split(' ').pop(), x, y + 22);
  }
  
  // Draw Ball
  const b = tick.ball;
  if (b) {
    const bx = px(b.x, COURT_W, PAD);
    const by = px(b.y, COURT_H, PAD);
    ctx.fillStyle = 'rgba(0,0,0,0.3)';
    ctx.beginPath();
    ctx.ellipse(bx, by + 3, 7, 3, 0, 0, Math.PI * 2);
    ctx.fill();
    
    ctx.fillStyle = '#ea580c';
    ctx.beginPath();
    ctx.arc(bx, by, 7.5, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#7c2d12';
    ctx.lineWidth = 1;
    ctx.stroke();
  }
}

function render(index) {
  if (ticks.length === 0) return;
  currentTickIndex = Math.max(0, Math.min(ticks.length - 1, index));
  const tick = ticks[currentTickIndex];
  drawTick(tick);
  
  // Update Match Header Info
  $('homeScore').textContent = String(tick.score?.home ?? 0);
  $('awayScore').textContent = String(tick.score?.away ?? 0);
  $('matchClock').textContent = `Q${tick.period ?? 1} ${fmtClock(tick.gameClock ?? 720.0)}`;
  
  $('frameSlider').value = String(currentTickIndex);
  $('frameLabel').textContent = `Tick: ${currentTickIndex + 1} / ${ticks.length}`;
  
  // Update Stage Banner
  const stage = stages.find(s => currentTickIndex >= s.startTick && currentTickIndex <= s.endTick) || stages[0];
  if (stage) {
    currentStageId = stage.id;
    $('currentStageType').textContent = stage.type;
    $('currentStageTitle').textContent = stage.title;
    const elapsedInStage = ((currentTickIndex - stage.startTick) * 0.1).toFixed(1);
    $('currentStageTime').textContent = `阶段用时: ${elapsedInStage}s / ${stage.duration.toFixed(1)}s`;
    
    document.querySelectorAll('.stage-item').forEach(el => {
      const isCur = el.dataset.stageId === String(stage.id);
      el.classList.toggle('selected', isCur);
    });
  }
  
  // Update all 10 players decision trace
  updateDecisionCard(tick);
}

function updateDecisionCard(tick) {
  const container = $('allPlayersDecision');
  if (!container || !tick || !tick.players) return;
  
  const pMeta = pack?.meta?.players_meta || {};
  const ballHolder = tick.players.find(p => p.hasBall);
  const trace = tick.decisionTrace;
  
  container.innerHTML = tick.players.map(p => {
    const name = pMeta[p.jersey]?.name || `#${p.jersey}`;
    const isHolder = p.hasBall;
    let reason = p.team === 'home' ? '进攻落位与拉开空间' : '紧贴对位人，阻断传球路线';
    
    if (isHolder && trace) {
      reason = `【持球决策】${trace.reason}`;
    } else if (p.action === 'SCREEN') {
      reason = '【掩护】为持球人设立高位刚体挡拆';
    } else if (p.action === 'CUT') {
      reason = '【空切】观察到防守人盲区，空切篮下';
    } else if (p.action === 'DEFEND') {
      reason = '【防守】保持防守滑步，卡在对手与篮筐中线';
    }
    
    const teamColor = p.team === 'home' ? '#007A33' : '#552583';
    
    return `
      <div class="player-decision-row ${isHolder ? 'has-ball' : ''}">
        <div class="p-head">
          <div class="p-name">
            <span class="pill" style="width:8px;height:8px;border-radius:50%;background:${teamColor};display:inline-block;"></span>
            <b>#${p.jersey} ${name}</b>
            ${isHolder ? '<span style="color:#fbbf24;font-size:10px;margin-left:4px;">🏀 持球核心</span>' : ''}
          </div>
          <span class="p-action-tag" style="${isHolder ? 'background:#b45309;color:#fff;' : ''}">${p.action || 'MOVE'}</span>
        </div>
        <div class="p-reason">${reason}</div>
      </div>
    `;
  }).join('');
}

function extractStagesFromTicks(allTicks) {
  const list = [];
  let cur = null;
  for (let i = 0; i < allTicks.length; i++) {
    const t = allTicks[i];
    const sId = t.stageId ?? 0;
    if (!cur || cur.id !== sId) {
      if (cur) {
        cur.endTick = i - 1;
        cur.duration = (cur.endTick - cur.startTick + 1) * 0.1;
        list.push(cur);
      }
      cur = {
        id: sId,
        type: t.stageType || 'OFFENSE_SET',
        title: t.stageTitle || `阶段 #${sId + 1}`,
        startTick: i,
        endTick: i,
        duration: 0.1
      };
    }
  }
  if (cur) {
    cur.endTick = allTicks.length - 1;
    cur.duration = (cur.endTick - cur.startTick + 1) * 0.1;
    list.push(cur);
  }
  return list;
}

function renderStageList() {
  const rail = $('stageList');
  $('stageCount').textContent = String(stages.length);
  rail.innerHTML = stages.map(s => `
    <div class="stage-item ${s.id === currentStageId ? 'selected' : ''}" data-stage-id="${s.id}">
      <div class="head">
        <span class="type-badge">${s.type}</span>
        <span class="duration">${s.duration.toFixed(1)}s</span>
      </div>
      <div class="title">${s.title}</div>
    </div>
  `).join('');
  
  rail.querySelectorAll('.stage-item').forEach(el => {
    el.addEventListener('click', () => {
      const sId = Number(el.dataset.stageId);
      const target = stages.find(s => s.id === sId);
      if (target) {
        currentStageId = target.id;
        render(target.startTick);
      }
    });
  });
}

function renderEventStream(allTicks) {
  const stream = $('eventStream');
  const events = [];
  for (let i = 0; i < allTicks.length; i++) {
    const t = allTicks[i];
    if (t.callout && (!events.length || events[events.length - 1].callout !== t.callout)) {
      events.push({ tick: i, clock: fmtClock(t.gameClock), callout: t.callout, type: t.eventType });
    }
  }
  stream.innerHTML = events.map(e => `
    <div class="event-item" data-tick="${e.tick}">
      <span class="etime">${e.clock}</span>
      <span>${e.callout}</span>
    </div>
  `).join('');
  
  stream.querySelectorAll('.event-item').forEach(el => {
    el.addEventListener('click', () => {
      render(Number(el.dataset.tick));
    });
  });
}

// Local Persistence for Annotation Dataset
function saveAnnotationFeedback() {
  const curStage = stages.find(s => s.id === currentStageId) || stages[0];
  const rating = document.querySelector('input[name="stageRating"]:checked')?.value || 'GOOD';
  const feedbackText = $('annotationFeedback').value.trim();
  
  if (!feedbackText && rating !== 'GOOD') {
    $('annotationStatus').textContent = '⚠️ 请输入具体问题描述以帮助决策函数改进';
    $('annotationStatus').style.color = '#f59e0b';
    return;
  }
  
  const annotationRecord = {
    id: `ann_${Date.now()}`,
    timestamp: new Date().toISOString(),
    stage_id: curStage.id,
    stage_type: curStage.type,
    stage_title: curStage.title,
    duration_s: curStage.duration,
    rating,
    feedback: feedbackText,
    cur_tick: currentTickIndex,
    decision_trace: ticks[currentTickIndex]?.decisionTrace || null
  };
  
  // Store in LocalStorage array
  try {
    const history = JSON.parse(localStorage.getItem('nba_sim_annotations') || '[]');
    history.push(annotationRecord);
    localStorage.setItem('nba_sim_annotations', JSON.stringify(history));
    
    $('annotationStatus').textContent = `✅ 标注已保存！(已沉淀 ${history.length} 条调优样本)`;
    $('annotationStatus').style.color = '#10b981';
    $('annotationFeedback').value = '';
    setTimeout(() => { $('annotationStatus').textContent = ''; }, 3000);
  } catch (err) {
    $('annotationStatus').textContent = '❌ 保存失败: ' + err.message;
    $('annotationStatus').style.color = '#ef4444';
  }
}

async function loadGameStream() {
  try {
    const res = await fetch('game.ticks.ndjson?t=' + Date.now());
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    const text = await res.text();
    const lines = text.trim().split('\n');
    ticks = lines.map(l => {
      const raw = JSON.parse(l);
      const f = raw.frame || raw;
      return {
        t: f.t,
        period: f.period,
        gameClock: f.game_clock ?? f.t_game ?? f.gameClock ?? 720.0,
        shotClock: f.shot_clock ?? f.shotClock ?? 24.0,
        score: f.score || { home: 0, away: 0 },
        players: f.players || [],
        ball: f.ball,
        callout: f.callout,
        eventType: f.event_type || f.eventType,
        stageId: f.stage_id,
        stageType: f.stage_type,
        stageTitle: f.stage_title,
        decisionTrace: f.decision_trace
      };
    });
    
    stages = extractStagesFromTicks(ticks);
    renderStageList();
    renderEventStream(ticks);
    $('frameSlider').max = String(ticks.length - 1);
    render(0);
  } catch (e) {
    console.error('Failed to load stream:', e);
  }
}

// Playback Control Loop
function togglePlay() {
  isPlaying = !isPlaying;
  $('btnPlay').textContent = isPlaying ? '⏸ 暂停' : '▶ 播放';
  if (isPlaying) {
    playInterval = setInterval(() => {
      const curStage = stages.find(s => s.id === currentStageId);
      let next = currentTickIndex + 1;
      if (isLoopStage && curStage) {
        if (next > curStage.endTick) next = curStage.startTick;
      } else if (next >= ticks.length) {
        next = 0;
      }
      render(next);
    }, 100);
  } else {
    clearInterval(playInterval);
  }
}
// Event Listeners
$('btnPlay').addEventListener('click', togglePlay);
$('btnPrevFrame').addEventListener('click', () => { if (isPlaying) togglePlay(); render(currentTickIndex - 1); });
$('btnNextFrame').addEventListener('click', () => { if (isPlaying) togglePlay(); render(currentTickIndex + 1); });
$('btnLoopStage').addEventListener('click', () => {
  isLoopStage = !isLoopStage;
  $('btnLoopStage').textContent = isLoopStage ? '🔁 单阶段循环: 开' : '▶ 顺序连播: 开';
  $('btnLoopStage').classList.toggle('active', isLoopStage);
});
$('frameSlider').addEventListener('input', (e) => {
  if (isPlaying) togglePlay();
  render(Number(e.target.value));
});
$('btnSubmitAnnotation').addEventListener('click', saveAnnotationFeedback);

// Mobile Tab Switcher Logic
const mTabs = [
  { btn: 'mTabStages', panel: '.stage-rail' },
  { btn: 'mTabCourt', panel: '.court-viewport' },
  { btn: 'mTabDecision', panel: '.decision-panel' },
  { btn: 'mTabAnnotate', panel: '.annotation-box' }
];

mTabs.forEach(({ btn, panel }) => {
  $(btn)?.addEventListener('click', () => {
    mTabs.forEach(t => $(t.btn)?.classList.remove('active'));
    $(btn)?.classList.add('active');
    
    // Show target section on mobile
    document.querySelectorAll('.stage-rail, .court-viewport, .decision-panel').forEach(el => el.classList.remove('mobile-active'));
    if (panel === '.annotation-box') {
      document.querySelector('.decision-panel')?.classList.add('mobile-active');
      document.querySelector('.annotation-box')?.scrollIntoView({ behavior: 'smooth' });
    } else {
      document.querySelector(panel)?.classList.add('mobile-active');
    }
  });
});
// Bootstrap
loadCourtSpec().then(() => {
  loadGameStream();
});

