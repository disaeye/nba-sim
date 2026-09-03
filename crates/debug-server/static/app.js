(function () {
const DEFAULT_RULES = {
  tick_seconds: 0.04,
  max_player_speed_ftps: 22,
  max_player_accel_ftps2: 35,
  player_radius_ft: 1.8,
  min_player_separation_ft: 3.6,
  ball_max_speed_ftps: 85,
  rim_shot_distance_ft: 8,
  three_point_distance_ft: 23.75,
  court_width_ft: 94,
  court_height_ft: 50,
  hoop_left_x_ft: 5.25,
  hoop_right_x_ft: 88.75,
  hoop_y_ft: 25,
};

function courtFromRules(rules) {
  return {
    width: finite(rules?.court_width_ft, DEFAULT_RULES.court_width_ft),
    height: finite(rules?.court_height_ft, DEFAULT_RULES.court_height_ft),
    leftHoopX: finite(rules?.hoop_left_x_ft, DEFAULT_RULES.hoop_left_x_ft),
    rightHoopX: finite(rules?.hoop_right_x_ft, DEFAULT_RULES.hoop_right_x_ft),
    hoopY: finite(rules?.hoop_y_ft, DEFAULT_RULES.hoop_y_ft),
  };
}
const BASELINES = {
  ppp: [0.90, 1.15],
  medianPossession: [8, 18],
  passesPerPossession: [1, 5],
  fgPct: [0.40, 0.52],
  threeShare: [0.25, 0.48],
  offensiveReboundPct: [0.18, 0.34],
  turnoverRate: [0.08, 0.20],
  foulRate: [0.06, 0.32],
};

function runtimeRules(tick) {
  const rules = tick?.rules || state?.rules || DEFAULT_RULES;
  const court = courtFromRules(rules);
  return {
    tickSeconds: finite(rules.tick_seconds, DEFAULT_RULES.tick_seconds),
    courtWidth: court.width,
    courtHeight: court.height,
    leftHoopX: court.leftHoopX,
    rightHoopX: court.rightHoopX,
    hoopY: court.hoopY,
    playerRadius: finite(rules.player_radius_ft, DEFAULT_RULES.player_radius_ft),
    minPlayerSeparation: finite(rules.min_player_separation_ft, DEFAULT_RULES.player_radius_ft * 2),
    separationSafetyMargin: finite(rules.separation_safety_margin_ft, 0),
    maxPlayerSpeed: finite(rules.max_player_speed_ftps, DEFAULT_RULES.max_player_speed_ftps),
    maxPlayerAccel: finite(rules.max_player_accel_ftps2, DEFAULT_RULES.max_player_accel_ftps2),
    ballMaxSpeed: finite(rules.ball_max_speed_ftps, DEFAULT_RULES.ball_max_speed_ftps),
    rimShotDistance: finite(rules.rim_shot_distance_ft, DEFAULT_RULES.rim_shot_distance_ft),
    threePointDistance: finite(rules.three_point_distance_ft, DEFAULT_RULES.three_point_distance_ft),
  };
}
const $ = (id) => document.getElementById(id);

const state = {
  ticks: [],
  events: [],
  possessions: [],
  possessionTeams: new Map(),
  shots: [],
  decisions: [],
  anomalies: [],
  rules: { ...DEFAULT_RULES },
  rulesLoaded: false,
  filters: new Set(),
  eventElements: [],
  hitPlayers: [],
  analytics: null,
  idx: 0,
  playing: false,
  speed: 1,
  rafId: null,
  previousRaf: 0,
  accumulator: 0,
  currentTab: "timeline",
  lastFrameJson: -1,
};

window.__nbaDebug = state;
window.__nbaDebugReady = true;
function esc(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#039;");
}
function finite(value, fallback = 0) {
  return Number.isFinite(Number(value)) ? Number(value) : fallback;
}
function clamp(value, lo, hi) { return Math.min(hi, Math.max(lo, value)); }
function pct(value) { return Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—"; }
function one(value) { return Number.isFinite(value) ? value.toFixed(1) : "—"; }
function two(value) { return Number.isFinite(value) ? value.toFixed(2) : "—"; }
function timeClock(seconds) {
  const value = Math.max(0, finite(seconds));
  return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(Math.floor(value % 60)).padStart(2, "0")}`;
}
function timeWithTenths(seconds) {
  const value = Math.max(0, finite(seconds));
  return `${timeClock(value)}.${Math.floor((value % 1) * 10)}`;
}
function tickStep() { return finite(runtimeRules(state.ticks[state.idx] || {}).tickSeconds, DEFAULT_RULES.tick_seconds) || DEFAULT_RULES.tick_seconds; }
function eventNames(tick) {
  if (Array.isArray(tick.events) && tick.events.length) return tick.events;
  return tick.eventType ? [tick.eventType] : [];
}
function eventClass(name) {
  const upper = String(name).toUpperCase();
  if (upper.includes("SCORE") || upper === "DRIVE_SCORE") return "tag-score";
  if (upper.includes("SHOT") || upper.includes("DRIVE_MISS") || upper.includes("DRIVE_STOPPED")) return "tag-shot";
  if (upper.includes("FOUL") || upper.includes("VIOLATION") || upper.includes("OUT_OF_BOUNDS")) return "tag-violation";
  if (upper.includes("REBOUND")) return "tag-rebound";
  if (upper.includes("PASS")) return "tag-pass";
  if (upper.includes("DRIVE")) return "tag-drive";
  return "";
}
  async function fetchText(url, options) {
    const response = await fetch(url, options);
    const text = await response.text();
    if (!response.ok) throw new Error(text || `${response.status} ${response.statusText}`);
    return text;
  }
  async function fetchDefaultRules(updateEditor) {
    const rules = JSON.parse(await fetchText("/api/rules"));
    state.rules = rules;
    state.rulesLoaded = true;
    if (updateEditor) $("rulesEditor").value = JSON.stringify(rules, null, 2);
    return rules;
  }

  async function runSimulation(withRules = null) {
    stopPlayback();
    const parsedSeed = Number.parseInt($("seedInput").value, 10);
    const seed = Number.isFinite(parsedSeed) ? parsedSeed : 42;
    const scope = $("scopeInput").value;
    setRunStatus("模拟运行中…");
    setControlsBusy(true);
    try {
      if (!withRules) await fetchDefaultRules(false);
      const responseText = withRules
        ? await fetchText("/api/simulate", {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ seed, scope, rules: withRules })
          })
        : await fetchText(`/api/simulate?seed=${encodeURIComponent(seed)}&scope=${encodeURIComponent(scope)}`);
      if (withRules) state.rules = { ...state.rules, ...withRules };
      loadStream(responseText, `seed ${seed} · ${scope}`);
      setRunStatus(`${state.ticks.length.toLocaleString()} ticks`);
      return true;
    } catch (error) {
      setRunStatus("运行失败", true);
      $("streamSummary").textContent = error.message;
      console.error(error);
      return false;
    } finally {
      setControlsBusy(false);
    }
  }

  function loadStream(text, source = "NDJSON") {
    const parsed = [];
    for (const line of text.split(/\r?\n/)) {
      if (!line.trim()) continue;
      try { parsed.push(JSON.parse(line)); }
      catch (error) { console.warn("Skipping malformed stream line", error); }
    }
    if (!parsed.length) throw new Error("流中没有可解析的帧");
    state.ticks = parsed;
    state.idx = 0;
    state.accumulator = 0;
    state.previousRaf = 0;
    state.lastFrameJson = -1;
    state.rules = { ...state.rules, ...(parsed[0].rules || {}) };
    state.analytics = buildAnalytics(parsed);
    state.events = state.analytics.events;
    state.possessions = state.analytics.possessions;
    state.possessionTeams = state.analytics.possessionTeams;
    state.shots = state.analytics.shots;
    state.decisions = state.analytics.decisions;
    state.anomalies = state.analytics.anomalies;
    state.filters.clear();
    renderStats(state.analytics.stats);
    renderTimeline();
    renderAnomalies();
    renderShotMap();
    updateReadouts(source);
    $("progressInput").max = String(Math.max(0, parsed.length - 1));
    $("progressInput").value = "0";
    $("jumpInput").max = String(Math.max(0, parsed.length - 1));
    $("progressEnd").textContent = timeWithTenths(parsed[parsed.length - 1].t);
    renderCurrent(true);
    state.playing = false;
    updatePlayButton();
  }

  function buildAnalytics(ticks) {
    const events = [];
    const possessionMap = new Map();
    const possessionTeams = new Map();
    const decisions = [];
    const shots = [];
    const pendingShots = [];
    let previous = null;

    ticks.forEach((tick, index) => {
      const id = tick.possession_id ?? 0;
      const holderId = tick.ball?.holderId
        || (tick.players || []).find((player) => player.hasBall)?.id
        || tick.debug?.player;
      const holder = holderId && (tick.players || []).find((player) => player.id === holderId);
      if (holderId) possessionTeams.set(id, holder?.team || tick.possession_team || "home");

      const names = eventNames(tick);
      names.forEach((name) => events.push({ index, name, tick }));
      let possession = possessionMap.get(id);
      if (!possession) {
        possession = {
          id, start: index, end: index, duration: 0,
          passes: 0, shots: 0, scores: 0, rebounds: 0,
          drives: 0, driveScores: 0, driveMisses: 0,
          violations: 0, fouls: 0, events: [], retainedRebound: false
        };
        possessionMap.set(id, possession);
      }
      possession.end = index;
      possession.events.push(...names);
      possession.passes += names.filter((name) => name === "PASS").length;
      possession.shots += names.filter((name) => name === "SHOT_RELEASE").length;
      possession.scores += names.filter((name) => name === "SCORE" || name === "DRIVE_SCORE").length;
      possession.drives += names.filter((name) => name === "DRIVE_INITIATED").length;
      possession.driveScores += names.filter((name) => name === "DRIVE_SCORE").length;
      possession.driveMisses += names.filter((name) => name === "DRIVE_MISS" || name === "DRIVE_STOPPED").length;
      possession.rebounds += names.filter((name) => name === "REBOUND").length;
      possession.violations += names.filter((name) => /VIOLATION|OUT_OF_BOUNDS/.test(name)).length;
      possession.fouls += names.filter((name) => /FOUL/.test(name)).length;
      if (tick.debug) decisions.push({ index, debug: tick.debug });

      for (const name of names) {
        const frameEvents = Array.isArray(tick.event_log) ? tick.event_log : [];
        const domainEvent = frameEvents.find((event) => event.kind === name);
        const payload = domainEvent?.data || null;
        if (name === "SHOT_RELEASE") {
          const shooterId = payload?.ShotRelease?.shooter_id
            || tick.debug?.player
            || previous?.ball?.holderId
            || holderId
            || "unknown";
          const shooter = (tick.players || []).find((player) => player.id === shooterId)
            || (previous?.players || []).find((player) => player.id === shooterId);
          const team = shooter?.team || tick.possession_team || "home";
          const rules = runtimeRules(tick);
          const origin = payload?.ShotRelease?.pos;
          const x = Number.isFinite(Number(origin?.[0])) ? Number(origin[0]) : finite(tick.ball?.x) * rules.courtWidth;
          const y = Number.isFinite(Number(origin?.[1])) ? Number(origin[1]) : finite(tick.ball?.y) * rules.courtHeight;
          const hoop = team === "home"
            ? { x: rules.rightHoopX, y: rules.hoopY }
            : { x: rules.leftHoopX, y: rules.hoopY };
          const distance = Math.hypot(x - hoop.x, y - hoop.y);
          const shot = {
            index, possessionId: id, team, shooter: shooterId, x, y, distance,
            three: payload?.ShotRelease?.is_three ?? distance >= rules.threePointDistance,
            makeProbability: payload?.ShotRelease?.make_probability ?? null,
            contestLevel: payload?.ShotRelease?.contest_level ?? null,
            made: null, resolved: false, points: 0
          };
          pendingShots.push(shot);
          shots.push(shot);
        } else if ((name === "SCORE" || name === "SHOT_MISS") && pendingShots.length) {
          const shot = pendingShots.shift();
          shot.made = name === "SCORE";
          shot.resolved = true;
          if (shot.made) {
            const priorScore = previous?.score;
            const homeDelta = finite(tick.score?.home) - finite(priorScore?.home);
            const awayDelta = finite(tick.score?.away) - finite(priorScore?.away);
            shot.points = Math.max(0, shot.team === "home" ? homeDelta : awayDelta);
          }
        }
      }
      previous = tick;
    });

    const possessions = [...possessionMap.values()].sort((a, b) => a.start - b.start);
    for (const possession of possessions) {
      possession.duration = Math.max(0, finite(ticks[possession.end].t) - finite(ticks[possession.start].t));
      for (let index = possession.start; index <= possession.end; index += 1) {
        if (eventNames(ticks[index]).includes("REBOUND") && ticks[index].possession_id === ticks[index - 1]?.possession_id) {
          possession.retainedRebound = true;
        }
      }
    }

    const resolvedShots = shots.filter((shot) => shot.resolved);
    const makes = resolvedShots.filter((shot) => shot.made);
    const threeAttempts = resolvedShots.filter((shot) => shot.three);
    const rebounds = events.filter((event) => event.name === "REBOUND");
    const retainedRebounds = rebounds.filter((event) => event.tick.possession_id === ticks[event.index - 1]?.possession_id);
    const finalScore = ticks[ticks.length - 1].score || {};
    const points = finite(finalScore.home) + finite(finalScore.away);
    const passCount = events.filter((event) => event.name === "PASS").length;
    const foulCount = events.filter((event) => /FOUL/.test(event.name)).length;
    const violationCount = events.filter((event) => /VIOLATION|OUT_OF_BOUNDS/.test(event.name)).length;
    const interceptCount = events.filter((event) => event.name === "PASS_INTERCEPT_OPPORTUNITY").length;
    const turnoverPossessions = possessions.filter((possession) => possession.events.some((name) => /VIOLATION|OUT_OF_BOUNDS|PASS_INTERCEPT_OPPORTUNITY/.test(name))).length;
    const medianPossession = median(possessions.map((possession) => possession.duration));
    const stats = {
      ppp: possessions.length ? points / possessions.length : NaN,
      medianPossession,
      passesPerPossession: possessions.length ? passCount / possessions.length : NaN,
      fgPct: resolvedShots.length ? makes.length / resolvedShots.length : NaN,
      threeShare: resolvedShots.length ? threeAttempts.length / resolvedShots.length : NaN,
      points, attempts: resolvedShots.length, makes: makes.length,
      threeAttempts: threeAttempts.length, rebounds: rebounds.length,
      drives: possessions.reduce((total, possession) => total + possession.drives, 0),
      driveScores: possessions.reduce((total, possession) => total + possession.driveScores, 0),
      fouls: foulCount, violations: violationCount, intercepts: interceptCount,
      maxPossession: Math.max(0, ...possessions.map((possession) => possession.duration))
    };
    return {
      events, possessions, possessionTeams, shots, decisions,
      anomalies: detectAnomalies(ticks, events, possessions), stats
    };
  }
  function median(values) {
    const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
    if (!sorted.length) return NaN;
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
  }

  function detectAnomalies(ticks, events, possessions) {
    const anomalies = [];
    let stalledTicks = 0;
    // 与引擎 L1 PLAYER_SEPARATION 同口径：min_sep × 0.5（invariants/lib.rs）。
    const overlapLimit = Math.max(
      .0001,
      (ticks[0] ? runtimeRules(ticks[0]).minPlayerSeparation : 1.8) * .5
    );
    const add = (index, kind, detail) => {
      if (anomalies.length < 300) anomalies.push({ index, kind, detail });
    };
    for (let index = 0; index < ticks.length; index += 1) {
      const tick = ticks[index];
      const previous = ticks[index - 1];
      const rules = runtimeRules(tick);
      const dt = Math.max(.0001, finite(tick.t) - finite(previous?.t, finite(tick.t) - rules.tickSeconds));
      const players = (tick.players || []).filter((p) => p.onCourt !== false);
      const homeCount = players.filter((p) => p.team === "home").length;
      const awayCount = players.filter((p) => p.team === "away").length;
      if (homeCount !== 5 || awayCount !== 5) {
        add(index, "ILLEGAL_LINEUP", `在场人数异常: Home=${homeCount}, Away=${awayCount}`);
      }
      for (const player of players) {
        if (player.x < -.01 || player.x > 1.01 || player.y < -.01 || player.y > 1.01) {
          add(index, "PLAYER_OUT_OF_BOUNDS", `${player.id} (${one(player.x)}, ${one(player.y)})`);
        }
        const old = (previous?.players || []).find((candidate) => candidate.id === player.id);
        if (old) {
          const speed = Math.hypot((player.x - old.x) * rules.courtWidth, (player.y - old.y) * rules.courtHeight) / dt;
          if (speed > rules.maxPlayerSpeed * 1.05) add(index, "PLAYER_SPEED", `${player.id} ${speed.toFixed(1)} ft/s`);
        }
      }
      for (let a = 0; a < players.length; a += 1) {
        for (let b = a + 1; b < players.length; b += 1) {
          const distance = Math.hypot((players[a].x - players[b].x) * rules.courtWidth, (players[a].y - players[b].y) * rules.courtHeight);
          if (distance < overlapLimit) add(index, "PLAYER_OVERLAP", `${players[a].id}/${players[b].id} ${distance.toFixed(2)} ft`);
        }
      }
      if (previous?.ball && tick.ball) {
        const ballSpeed = Math.hypot(
          (tick.ball.x - previous.ball.x) * rules.courtWidth,
          (tick.ball.y - previous.ball.y) * rules.courtHeight,
          tick.ball.z - previous.ball.z
        ) / dt;
        if (ballSpeed > rules.ballMaxSpeed * 1.05) add(index, "BALL_SPEED", `${ballSpeed.toFixed(1)} ft/s`);
      }
      for (const enforcement of tick.debug?.enforcement || []) {
        if (/ILLEGAL_FLOW|OUT_OF_BOUNDS|GAME_CLOCK_EXPIRED/.test(enforcement)) add(index, "ENFORCEMENT", enforcement);
      }
      if (tick.game_flow === "LiveBall" && finite(tick.shotClock) <= .001) stalledTicks += 1;
      else stalledTicks = 0;
      if (stalledTicks === Math.max(5, Math.ceil(1 / rules.tickSeconds))) add(index, "CLOCK_STALL", "shot clock is zero while LiveBall");
    }
    for (const possession of possessions) {
      if (possession.duration > 35) add(possession.end, "LONG_POSSESSION", `${possession.duration.toFixed(1)} s`);
    }
    return anomalies;
  }
  function renderStats(stats) {
    const rows = [
      ["Points per possession", stats.ppp, "0.90 – 1.15", BASELINES.ppp, two],
      ["Median possession", stats.medianPossession, "8 – 18 s", BASELINES.medianPossession, (value) => `${one(value)} s`],
      ["Passes / possession", stats.passesPerPossession, "1 – 5", BASELINES.passesPerPossession, two],
      ["Field-goal percentage", stats.fgPct, "40 – 52%", BASELINES.fgPct, pct],
      ["3PT attempt share", stats.threeShare, "25 – 48%", BASELINES.threeShare, pct],
      ["Offensive rebound rate", stats.offensiveReboundPct, "18 – 34%", BASELINES.offensiveReboundPct, pct],
      ["Turnover proxy rate", stats.turnoverRate, "8 – 20%", BASELINES.turnoverRate, pct],
      ["Fouls / possession", stats.foulRate, "6 – 32%", BASELINES.foulRate, pct],
    ];
    $("statsTable").innerHTML = rows.map(([name, value, baseline, range, format]) => {
      const status = metricStatus(value, range[0], range[1]);
      return `<div class="stat-row"><span class="stat-name">${name}</span><span class="stat-value">${format(value)}</span><span class="stat-baseline">${baseline}</span><i class="stat-indicator stat-${status}"></i></div>`;
    }).join("") + `<div class="stats-summary"><span>${stats.attempts} FGA · ${stats.makes} FGM · ${stats.threeAttempts} 3PA</span><span>${stats.drives} DRV · ${stats.driveScores} FIN · ${stats.rebounds} REB · ${stats.fouls} FOUL</span></div>`;
  }
  function metricStatus(value, lo, hi) {
    if (!Number.isFinite(value)) return "warn";
    const margin = (hi - lo) * .6;
    if (value >= lo && value <= hi) return "good";
    if (value >= lo - margin && value <= hi + margin) return "warn";
    return "bad";
  }

  function renderTimeline() {
    const names = [...new Set(state.events.map((event) => event.name))];
    names.forEach((name) => state.filters.add(name));
    $("eventFilters").innerHTML = names.map((name) => `<button class="filter-button ${state.filters.has(name) ? "active" : ""}" data-filter="${esc(name)}">${esc(name)}</button>`).join("");
    $("eventFilters").querySelectorAll("[data-filter]").forEach((button) => button.addEventListener("click", () => {
      const name = button.dataset.filter;
      if (state.filters.has(name)) state.filters.delete(name); else state.filters.add(name);
      renderTimeline();
    }));
    const visible = state.events.filter((event) => state.filters.has(event.name));
    $("timelineCount").textContent = `${visible.length} events`;
    const fragment = document.createDocumentFragment();
    state.eventElements = [];
    for (const event of visible) {
      const row = document.createElement("div");
      row.className = "event-row";
      row.dataset.index = String(event.index);
      row.innerHTML = `<span class="event-time">${timeClock(event.tick.t_game)}</span><span class="event-possession">#${esc(event.tick.possession_id)}</span><span class="event-tag ${eventClass(event.name)}">${esc(event.name)}</span><span class="event-detail">${esc(event.tick.callout || event.tick.phase || "")}</span>`;
      row.addEventListener("click", () => seek(event.index));
      fragment.appendChild(row);
      state.eventElements.push(row);
    }
    $("timeline").replaceChildren(fragment);
    if (!visible.length) $("timeline").innerHTML = `<div class="empty-state">当前过滤器没有事件</div>`;
    updateTimelineCursor();
  }
  function updateTimelineCursor() {
    let current = null;
    for (const row of state.eventElements) {
      const rowIdx = Number(row.dataset.index);
      const isPassed = rowIdx <= state.idx;
      row.classList.toggle("passed", isPassed);
      row.classList.remove("current");
      if (isPassed) current = row;
    }
    if (current) {
      current.classList.add("current");
      if (state.playing) current.scrollIntoView({ block: "nearest" });
    }
  }
  function renderAnomalies() {
    const count = state.anomalies.length;
    const badge = $("anomalyBadge");
    badge.textContent = count ? `● ${count} anomalies` : "● 0 anomalies";
    badge.className = `anomaly-badge ${count > 10 ? "anomaly-danger" : count ? "anomaly-warn" : "anomaly-ok"}`;
    $("anomalyList").innerHTML = count
      ? state.anomalies.map((item) => `<button class="anomaly-item" data-index="${item.index}">#${item.index} · ${esc(item.kind)} · ${esc(item.detail)}</button>`).join("")
      : `<span class="muted-label">当前流没有检测到客户端时空/执行异常。</span>`;
    $("anomalyList").querySelectorAll("[data-index]").forEach((item) => item.addEventListener("click", () => seek(Number(item.dataset.index))));
    $("streamSummary").textContent = count ? "检测到需要回看的运行时异常" : "流已加载 · 时空和执行不变量未触发红灯";
  }
  function updateReadouts(source) {
    $("eventReadout").textContent = `${state.events.length} events`;
    $("streamSummary").textContent = `${source} · ${state.possessions.length} possessions · ${state.shots.length} shots`;
  }
  function setRunStatus(text, error = false) {
    $("runStatus").textContent = text;
    $("runStatus").style.color = error ? "var(--red)" : "";
  }

  function renderCurrent(forceJson = false) {
    const tick = state.ticks[state.idx];
    if (!tick) return;
    updateHud(tick);
    drawCourt(tick);
    updateTimelineCursor();
    $("progressInput").value = String(state.idx);
    $("jumpInput").value = String(state.idx);
    if ($("frameLabel")) $("frameLabel").textContent = `frame ${state.idx}`;
    $("tickReadout").textContent = `${state.idx.toLocaleString()} / ${state.ticks.length.toLocaleString()} ticks`;
    $("progressTime").textContent = timeWithTenths(tick.t);
    $("progressPossession").textContent = `POS #${tick.possession_id ?? "—"}`;
    if (forceJson || !state.playing || state.idx % 3 === 0) renderFrameJson(tick);
    renderDecision(tick);
  }
  function updateHud(tick) {
    const homeTeam = tick.home_team || {};
    const awayTeam = tick.away_team || {};
    if ($("frameLabel")) $("frameLabel").textContent = `frame ${state.idx}`;
    $("homeTeamName").textContent = homeTeam.short_name || homeTeam.name || "HOME";
    $("awayTeamName").textContent = awayTeam.short_name || awayTeam.name || "AWAY";
    $("tacticalSet").textContent = tick.tactical_set || "—";
    $("phaseLabel").textContent = String(tick.phase || "—").replaceAll("_", " ").toUpperCase();
    const names = eventNames(tick);
    const chip = $("eventChip");
    if (chip) {
      if (names.length) {
        chip.textContent = names.join(" · ");
        chip.style.display = "";
      } else {
        chip.style.display = "none";
      }
    }
    $("possessionLabel").textContent = `POS #${tick.possession_id ?? "—"}`;
    $("homeScore").textContent = finite(tick.score?.home).toFixed(0);
    $("awayScore").textContent = finite(tick.score?.away).toFixed(0);
    $("periodLabel").textContent = `Q${tick.period || 1}`;
    $("gameClock").textContent = timeClock(tick.gameClock ?? tick.t_game);
    $("shotClock").textContent = one(tick.shotClock);
    $("shotClock").style.color = finite(tick.shotClock) <= 5 ? "var(--red)" : "";
    $("flowLabel").textContent = String(tick.game_flow || "LIVE").replaceAll("Ball", "").toUpperCase();
    $("foulsReadout").textContent = `${tick.team_fouls_home ?? 0} / ${tick.team_fouls_away ?? 0}`;
    $("freeThrows").textContent = String(tick.free_throws_remaining ?? 0);
    $("intensityReadout").textContent = tick.intensity || "—";
    $("calloutText").textContent = tick.callout || "—";
    const dead = /Dead|Free|Quarter|Half|GameEnd/.test(String(tick.game_flow));
    $("flowLabel").style.color = dead ? "var(--away)" : "";
  }

  function renderFrameJson(tick) {
    if (state.lastFrameJson === state.idx) return;
    $("frameJson").textContent = JSON.stringify(tick, null, 2);
    state.lastFrameJson = state.idx;
  }

  function drawCourt(tick) {
    const canvas = $("courtCanvas");
    const ctx = canvas.getContext("2d");
    const rules = runtimeRules(tick);
    const point = (x, y) => ({ x: 10 + x * (940 / rules.courtWidth), y: 10 + y * (500 / rules.courtHeight) });
    const leftHoopX = rules.leftHoopX;
    const rightHoopX = rules.rightHoopX;
    const hoopY = rules.hoopY;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    const background = ctx.createLinearGradient(0, 0, canvas.width, canvas.height);
    background.addColorStop(0, "#c1aa80"); background.addColorStop(.5, "#ad966e"); background.addColorStop(1, "#c7b183");
    ctx.fillStyle = background; ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "rgba(255,255,255,.06)"; ctx.fillRect(10, 10, 940, 500);
    ctx.strokeStyle = "rgba(247,239,214,.82)"; ctx.lineWidth = 1.5; ctx.strokeRect(10, 10, 940, 500);
    ctx.beginPath(); ctx.moveTo(480, 10); ctx.lineTo(480, 510); ctx.stroke();
    ctx.beginPath(); ctx.arc(480, 260, 60, 0, Math.PI * 2); ctx.stroke();
    drawKey(ctx, false, rules); drawKey(ctx, true, rules); drawThreePointLine(ctx, false, rules); drawThreePointLine(ctx, true, rules);
    drawHoop(ctx, point(leftHoopX, hoopY), false); drawHoop(ctx, point(rightHoopX, hoopY), true);
    ctx.setLineDash([4, 4]); ctx.strokeStyle = "rgba(247,239,214,.32)";
    ctx.beginPath(); ctx.moveTo(10, 40); ctx.lineTo(950, 40); ctx.moveTo(10, 480); ctx.lineTo(950, 480); ctx.stroke(); ctx.setLineDash([]);
    drawTrails(ctx, point);
    state.hitPlayers = [];
    for (const player of tick.players || []) {
      if (player.onCourt === false) continue;
      const playerPoint = point(finite(player.x) * rules.courtWidth, finite(player.y) * rules.courtHeight);
      state.hitPlayers.push({ player, x: playerPoint.x, y: playerPoint.y });
      const color = player.team === "home" ? "#2ce59b" : "#f5bd45";
      const dark = player.team === "home" ? "#087b57" : "#a96f15";
      const radius = 17;
      if (player.hasBall) {
        ctx.beginPath(); ctx.arc(playerPoint.x, playerPoint.y, radius + 7, 0, Math.PI * 2); ctx.strokeStyle = "rgba(255,255,255,.92)"; ctx.lineWidth = 2; ctx.stroke();
      }
      ctx.beginPath(); ctx.arc(playerPoint.x, playerPoint.y, radius, 0, Math.PI * 2); ctx.fillStyle = dark; ctx.fill(); ctx.strokeStyle = color; ctx.lineWidth = 2; ctx.stroke();
      ctx.fillStyle = "#07130f"; ctx.font = "700 10px IBM Plex Mono, monospace"; ctx.textAlign = "center"; ctx.textBaseline = "middle"; ctx.fillText(player.jersey || "?", playerPoint.x, playerPoint.y + 1);
      ctx.fillStyle = "rgba(7,19,15,.82)"; ctx.font = "8px IBM Plex Mono, monospace"; ctx.fillText(player.slot || player.action || "", playerPoint.x, playerPoint.y + 28);
      const stamina = clamp(finite(player.stm) / Math.max(1, finite(player.stmMax, 100)), 0, 1);
      ctx.beginPath(); ctx.arc(playerPoint.x, playerPoint.y, radius + 3, -Math.PI / 2, -Math.PI / 2 + stamina * Math.PI * 2); ctx.strokeStyle = stamina > .45 ? "rgba(255,255,255,.72)" : "#ff6f7e"; ctx.lineWidth = 2; ctx.stroke();
    }
    if (tick.ball) {
      const ballPoint = point(finite(tick.ball.x) * rules.courtWidth, finite(tick.ball.y) * rules.courtHeight);
      const z = Math.max(0, finite(tick.ball.z));
      ctx.beginPath(); ctx.ellipse(ballPoint.x, ballPoint.y, 7 + z * .14, 3 + z * .05, 0, 0, Math.PI * 2); ctx.fillStyle = "rgba(20,18,12,.3)"; ctx.fill();
      ctx.beginPath(); ctx.arc(ballPoint.x, ballPoint.y - z * 2.6, 5.5 + z * .06, 0, Math.PI * 2); ctx.fillStyle = "#e87530"; ctx.fill(); ctx.strokeStyle = "#ffd08c"; ctx.lineWidth = 1; ctx.stroke();
    }
    const holderTeam = (tick.players || []).find((player) => player.hasBall)?.team;
    const attacksRight = (holderTeam || state.possessionTeams.get(tick.possession_id) || "home") === "home";
    const attackHoop = point(attacksRight ? rightHoopX : leftHoopX, hoopY);
    ctx.beginPath(); ctx.arc(attackHoop.x, attackHoop.y, 11 + Math.sin(finite(tick.t) * 4) * 2, 0, Math.PI * 2); ctx.strokeStyle = attacksRight ? "rgba(44,229,155,.72)" : "rgba(245,189,69,.72)"; ctx.lineWidth = 2; ctx.stroke();
  }
  function drawHoop(ctx, center, right) {
    ctx.strokeStyle = "rgba(247,239,214,.9)"; ctx.lineWidth = 1.5;
    ctx.beginPath(); ctx.arc(center.x, center.y, 7.5, 0, Math.PI * 2); ctx.stroke();
    ctx.beginPath(); ctx.moveTo(center.x + (right ? 12 : -12), center.y - 20); ctx.lineTo(center.x + (right ? 12 : -12), center.y + 20); ctx.stroke();
  }
  function drawKey(ctx, right, rules) {
    const hoopX = right ? rules.rightHoopX * 10 + 10 : rules.leftHoopX * 10 + 10;
    const y = rules.hoopY * 10 + 10;
    const keyWidth = 190;
    const keyHeight = 190;
    ctx.strokeStyle = "rgba(247,239,214,.8)";
    ctx.lineWidth = 1.5;
    ctx.strokeRect(right ? 760 : 10, y - keyHeight / 2, keyWidth, keyHeight);
    ctx.beginPath();
    ctx.arc(hoopX, y, 60, right ? Math.PI / 2 : -Math.PI / 2, right ? 3 * Math.PI / 2 : Math.PI / 2);
    ctx.stroke();
  }
  function drawThreePointLine(ctx, right, rules) {
    const hoopX = (right ? rules.rightHoopX : rules.leftHoopX) * 10 + 10;
    const hoopY = rules.hoopY * 10 + 10;
    const radius = rules.threePointDistance * 10;
    ctx.strokeStyle = "rgba(247,239,214,.72)";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    ctx.arc(hoopX, hoopY, radius, right ? Math.PI / 2 : -Math.PI / 2, right ? 3 * Math.PI / 2 : Math.PI / 2);
    ctx.stroke();
    ctx.beginPath();
    if (right) { ctx.moveTo(950, 40); ctx.lineTo(950, 480); }
    else { ctx.moveTo(10, 40); ctx.lineTo(10, 480); }
    ctx.stroke();
  }
  function drawTrails(ctx, point) {
    if (state.ticks.length < 2) return;
    const start = Math.max(0, state.idx - 70);
    ctx.beginPath();
    for (let index = start; index <= state.idx; index += 1) {
      const tick = state.ticks[index];
      const ball = tick.ball;
      if (!ball) continue;
      const rules = runtimeRules(tick);
      const ballPoint = point(finite(ball.x) * rules.courtWidth, finite(ball.y) * rules.courtHeight);
      if (index === start) ctx.moveTo(ballPoint.x, ballPoint.y); else ctx.lineTo(ballPoint.x, ballPoint.y);
    }
    ctx.strokeStyle = "rgba(255,235,188,.32)";
    ctx.lineWidth = 1.4;
    ctx.stroke();
  }

  function renderDecision(tick) {
    const previous = [...state.decisions].reverse().find((item) => item.index <= state.idx);
    const debug = tick.debug || previous?.debug;
    if (!debug) {
      $("decisionPanel").innerHTML = `<div class="empty-state">当前帧之前没有决策追踪<br><small>跳到一个决策帧，或点击事件流中的 PASS / SHOT_RELEASE</small></div>`;
      return;
    }
    const utilities = debug.utilities || [];
    const probabilities = debug.probabilities || [];
    const maxUtility = Math.max(.001, ...utilities.map((item) => Math.abs(finite(item.utility))));
    const chosen = debug.chosen || "";
    const flagRows = (debug.flags || []).map((flag) => `<div class="flag-row"><span>${esc(flag.constraint)}</span><span>${esc(flag.reason)}</span><b>${two(finite(flag.penalty))}</b></div>`).join("") || `<span class="muted-label">无约束惩罚</span>`;
    const decisionIndex = state.decisions.find((item) => item.debug === debug)?.index ?? state.idx;
    const rows = (items, value, scale) => items.length ? items.map((item) => {
      const number = finite(value(item));
      const selected = item.kind === chosen;
      return `<div class="score-bar-row"><span class="score-bar-label">${esc(item.kind)}</span><span class="score-bar-track"><i class="score-bar-fill ${selected ? "chosen" : ""}" style="width:${clamp(Math.abs(number) * scale, 1, 100)}%"></i></span><span class="score-bar-value">${number.toFixed(3)}</span></div>`;
    }).join("") : `<span class="muted-label">无数据</span>`;
    $("decisionPanel").innerHTML = `
      <div class="decision-head"><strong>${esc(chosen || "—")}</strong><span>${esc(debug.player || "—")} · frame ${decisionIndex}</span></div>
      <div class="decision-section"><div class="decision-title">Utility ranking</div>${rows(utilities, (item) => item.utility, 100 / maxUtility)}</div>
      <div class="decision-section"><div class="decision-title">Softmax probability</div>${rows(probabilities, (item) => item.prob, 100)}</div>
      <div class="decision-section"><div class="decision-title">Hard blockers</div><div class="chips">${debug.blocked?.length ? debug.blocked.map((item) => `<span class="chip bad">${esc(item)}</span>`).join("") : `<span class="muted-label">没有被硬约束剔除的候选</span>`}</div></div>
      <div class="decision-section"><div class="decision-title">Constraint flags</div>${flagRows}</div>
      <div class="decision-section"><div class="decision-title">Active constraints</div><div class="chips">${debug.active_constraints?.length ? debug.active_constraints.map((item) => `<span class="chip">${esc(item)}</span>`).join("") : `<span class="muted-label">—</span>`}</div></div>
      <div class="decision-section"><div class="decision-title">Enforcement feedback</div><div class="chips">${debug.enforcement?.length ? debug.enforcement.map((item) => `<span class="chip warn">${esc(item)}</span>`).join("") : `<span class="muted-label">本帧没有执行意图</span>`}</div></div>
      <button id="nextDecisionButton" class="text-button decision-next">跳到下一决策帧 →</button>`;
    $("nextDecisionButton").addEventListener("click", () => {
      const next = state.decisions.find((item) => item.index > state.idx);
      if (next) seek(next.index); else setRunStatus("已经是最后一个决策帧");
    });
  }

  function renderShotMap() {
    const canvas = $("shotCanvas");
    const ctx = canvas.getContext("2d");
    const rules = runtimeRules(state.ticks[0] || {});
    const scaleX = canvas.width / rules.courtWidth;
    const scaleY = canvas.height / rules.courtHeight;
    const leftHoopX = rules.leftHoopX;
    const rightHoopX = rules.rightHoopX;
    const hoopY = rules.hoopY;
    ctx.fillStyle = "#b8a27d";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.strokeStyle = "rgba(247,239,214,.86)";
    ctx.lineWidth = 1.2;
    for (const hoopX of [leftHoopX, rightHoopX]) {
      ctx.beginPath();
      ctx.arc(hoopX * scaleX, hoopY * scaleY, 7 * scaleX, 0, Math.PI * 2);
      ctx.stroke();
    }
    for (const shot of state.shots) {
      ctx.beginPath();
      ctx.arc(shot.x * scaleX, shot.y * scaleY, shot.three ? 5 : 4, 0, Math.PI * 2);
      if (shot.made === true) {
        ctx.fillStyle = "#2ce59b";
        ctx.fill();
        ctx.strokeStyle = "#bffff0";
      } else if (shot.made === false) {
        ctx.strokeStyle = "#ff6f7e";
        ctx.lineWidth = 1.5;
      } else {
        ctx.strokeStyle = "#f5bd45";
        ctx.lineWidth = 1.2;
      }
      if (shot.made !== true) ctx.stroke();
    }
    const rim = rules.rimShotDistance;
    const three = rules.threePointDistance;
    const zones = [
      [`Rim < ${rim} ft`, (shot) => shot.distance < rim],
      [`Mid ${rim}–16 ft`, (shot) => shot.distance >= rim && shot.distance < 16],
      [`Long 16–${three} ft`, (shot) => shot.distance >= 16 && !shot.three],
      [`Three ${three}+ ft`, (shot) => shot.three]
    ];
    $("zoneTable").innerHTML = zones.map(([name, predicate]) => {
      const subset = state.shots.filter(predicate);
      const made = subset.filter((shot) => shot.made === true).length;
      return `<div class="zone-row"><strong>${name}</strong><span>${made}/${subset.length} · ${subset.length ? pct(made / subset.length) : "—"}</span></div>`;
    }).join("") || `<div class="empty-state">暂无出手</div>`;
  }

  function seek(index) {
    if (!state.ticks.length) return;
    stopPlayback();
    state.idx = clamp(Math.round(Number(index) || 0), 0, state.ticks.length - 1);
    state.accumulator = 0;
    renderCurrent(true);
  }
  function step(delta) { stopPlayback(); seek(state.idx + delta); }
  function setControlsBusy(busy) {
    $("runButton").disabled = busy;
    $("fileInput").disabled = busy;
    $("runRulesButton").disabled = busy;
    $("loadRulesButton").disabled = busy;
    document.querySelectorAll("[data-preset]").forEach((button) => { button.disabled = busy; });
    document.querySelectorAll(".transport-group button").forEach((button) => { button.disabled = busy; });
    document.querySelectorAll(".speed-group button").forEach((button) => { button.disabled = busy; });
    $("jumpButton").disabled = busy;
    $("progressInput").disabled = busy;
    if (busy) stopPlayback();
  }
  function possessionJump(direction) {
    if (!state.possessions.length) return;
    const starts = state.possessions.map((possession) => possession.start);
    let current = 0;
    for (let index = 0; index < starts.length; index += 1) if (starts[index] <= state.idx) current = index;
    seek(starts[clamp(current + direction, 0, starts.length - 1)]);
  }
  function togglePlayback() {
    if (!state.ticks.length) return;
    state.playing = !state.playing;
    state.previousRaf = 0;
    updatePlayButton();
    if (state.playing && !state.rafId) state.rafId = requestAnimationFrame(playbackFrame);
  }
  function stopPlayback() { state.playing = false; updatePlayButton(); }
  function updatePlayButton() { $("playButton").textContent = state.playing ? "Ⅱ" : "▶"; }
  function playbackFrame(now) {
    state.rafId = requestAnimationFrame(playbackFrame);
    if (!state.playing || !state.ticks.length) return;
    if (!state.previousRaf) state.previousRaf = now;
    const elapsed = Math.min(.25, (now - state.previousRaf) / 1000);
    state.previousRaf = now;
    state.accumulator += elapsed * state.speed / tickStep();
    while (state.accumulator >= 1 && state.idx < state.ticks.length - 1) {
      state.idx += 1; state.accumulator -= 1;
    }
    if (state.idx >= state.ticks.length - 1) { state.playing = false; updatePlayButton(); }
    renderCurrent(false);
  }

  async function loadDefaultRules() {
    try {
      await fetchDefaultRules(true);
      setRulesStatus("默认 GameRules 已载入");
    } catch (error) { setRulesStatus(error.message, true); }
  }
  function setRulesStatus(text, error = false) {
    $("rulesStatus").textContent = text;
    $("rulesStatus").className = `rules-status ${error ? "error" : "success"}`;
  }
  async function runEditedRules() {
    let rules;
    try { rules = JSON.parse($("rulesEditor").value); }
    catch (error) { setRulesStatus(`JSON 错误：${error.message}`, true); return; }
    setRulesStatus("规则已解析，模拟运行中…");
    const ok = await runSimulation(rules);
    if (ok) setRulesStatus(`完成：${state.ticks.length.toLocaleString()} ticks`);
  }
  function applyPreset(name) {
    let rules;
    try { rules = JSON.parse($("rulesEditor").value || "{}"); } catch { rules = { ...state.rules }; }
    if (name === "default") rules = { ...state.rules };
    if (name === "fast") {
      rules.tick_seconds = .02;
      rules.decision_interval_seconds = Math.min(finite(rules.decision_interval_seconds, .8), .4);
    }
    if (name === "contest") {
      rules.shot_contest_sensitivity = Math.min(.85, finite(rules.shot_contest_sensitivity, .22) * 1.8);
      rules.contact_margin_ft = Math.max(.6, finite(rules.contact_margin_ft, .6) * 1.5);
    }
    $("rulesEditor").value = JSON.stringify(rules, null, 2);
    setRulesStatus(`已载入 ${name.toUpperCase()} 预设，点击应用并重跑`);
  }

  function wire() {
    $("runButton").addEventListener("click", () => runSimulation());
    $("fileInput").addEventListener("change", async (event) => {
      const file = event.target.files?.[0]; if (!file) return;
      try { loadStream(await file.text(), file.name); setRunStatus(`${state.ticks.length.toLocaleString()} ticks`); }
      catch (error) { setRunStatus("文件读取失败", true); $("streamSummary").textContent = error.message; }
    });
    $("anomalyBadge").addEventListener("click", () => {
      const panel = $("anomalyPanel"); const open = panel.hidden;
      panel.hidden = !open; $("anomalyBadge").setAttribute("aria-expanded", String(open));
    });
    document.querySelectorAll(".tab-button").forEach((button) => button.addEventListener("click", () => {
      state.currentTab = button.dataset.tab;
      document.querySelectorAll(".tab-button").forEach((item) => item.classList.toggle("active", item === button));
      document.querySelectorAll(".tab-pane").forEach((pane) => {
        const active = pane.id === `tab-${state.currentTab}`;
        pane.hidden = !active; pane.classList.toggle("active", active);
      });
      if (state.currentTab === "shots") renderShotMap();
    }));
    $("prevPossessionButton").addEventListener("click", () => possessionJump(-1));
    $("nextPossessionButton").addEventListener("click", () => possessionJump(1));
    $("stepBackButton").addEventListener("click", () => step(-1));
    $("stepForwardButton").addEventListener("click", () => step(1));
    $("playButton").addEventListener("click", togglePlayback);
    $("progressInput").addEventListener("input", (event) => seek(event.target.value));
    $("jumpButton").addEventListener("click", () => seek($("jumpInput").value));
    document.querySelectorAll(".speed-group button").forEach((button) => button.addEventListener("click", () => {
      state.speed = Number(button.dataset.speed);
      document.querySelectorAll(".speed-group button").forEach((item) => item.classList.toggle("active", item === button));
    }));
    $("loadRulesButton").addEventListener("click", loadDefaultRules);
    $("runRulesButton").addEventListener("click", runEditedRules);
    document.querySelectorAll("[data-preset]").forEach((button) => button.addEventListener("click", () => applyPreset(button.dataset.preset)));
    $("copyFrameButton").addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText($("frameJson").textContent);
        $("copyFrameButton").textContent = "已复制";
        setTimeout(() => { $("copyFrameButton").textContent = "复制当前帧"; }, 1200);
      } catch { setRunStatus("浏览器拒绝访问剪贴板", true); }
    });
    $("courtCanvas").addEventListener("mousemove", (event) => {
      const canvas = event.currentTarget;
      const rect = canvas.getBoundingClientRect();
      const x = (event.clientX - rect.left) * canvas.width / rect.width;
      const y = (event.clientY - rect.top) * canvas.height / rect.height;
      const hit = state.hitPlayers.find((item) => Math.hypot(item.x - x, item.y - y) < 23);
      const tooltip = $("playerTooltip");
      if (!hit) { tooltip.hidden = true; return; }
      const player = hit.player;
      tooltip.hidden = false;
      tooltip.style.left = `${Math.min(rect.width - 165, (x / canvas.width) * rect.width + 12)}px`;
      tooltip.style.top = `${Math.max(4, (y / canvas.height) * rect.height - 35)}px`;
      tooltip.innerHTML = `<strong>${esc(player.id)} · #${esc(player.jersey)}</strong><span>${esc(player.action)} · ${esc(player.slot)}</span><span>stamina ${one(player.stm)}/${one(player.stmMax)} · ${esc(player.morale)}</span>`;
    });
    $("courtCanvas").addEventListener("mouseleave", () => { $("playerTooltip").hidden = true; });
    document.addEventListener("keydown", (event) => {
      if (event.target.matches("input, textarea, select")) return;
      if (event.code === "Space") { event.preventDefault(); togglePlayback(); }
      if (event.key === "ArrowLeft") step(-1);
      if (event.key === "ArrowRight") step(1);
      if (event.key === "[" || event.key === "{") possessionJump(-1);
      if (event.key === "]" || event.key === "}") possessionJump(1);
    });
  }

  async function boot() {
    wire();
    await loadDefaultRules();
    await runSimulation();
  }
  boot().catch((error) => {
    setRunStatus("启动失败", true);
    $("streamSummary").textContent = error.message;
    console.error(error);
  });
})();
