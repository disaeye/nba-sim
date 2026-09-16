(() => {
  let wasmPromise = null;
  function ensureWasm() {
    if (!wasmPromise) {
      wasmPromise = (async () => {
        try {
          const mod = await import("./wasm/nba_wasm.js");
          await mod.default();
          return mod;
        } catch {
          return null;
        }
      })();
    }
    return wasmPromise;
  }
  ensureWasm();

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
    ppp: [0.9, 1.15],
    medianPossession: [8, 18],
    passesPerPossession: [1, 5],
    fgPct: [0.4, 0.52],
    threeShare: [0.25, 0.48],
    offensiveReboundPct: [0.18, 0.34],
    turnoverRate: [0.08, 0.2],
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
      playerRadius: finite(
        rules.player_radius_ft,
        DEFAULT_RULES.player_radius_ft,
      ),
      minPlayerSeparation: finite(
        rules.min_player_separation_ft,
        DEFAULT_RULES.player_radius_ft * 2,
      ),
      separationSafetyMargin: finite(rules.separation_safety_margin_ft, 0),
      maxPlayerSpeed: finite(
        rules.max_player_speed_ftps,
        DEFAULT_RULES.max_player_speed_ftps,
      ),
      maxPlayerAccel: finite(
        rules.max_player_accel_ftps2,
        DEFAULT_RULES.max_player_accel_ftps2,
      ),
      ballMaxSpeed: finite(
        rules.ball_max_speed_ftps,
        DEFAULT_RULES.ball_max_speed_ftps,
      ),
      rimShotDistance: finite(
        rules.rim_shot_distance_ft,
        DEFAULT_RULES.rim_shot_distance_ft,
      ),
      threePointDistance: finite(
        rules.three_point_distance_ft,
        DEFAULT_RULES.three_point_distance_ft,
      ),
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
    engineViolations: [],
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
    courtMode: "half",
    lastFrameJson: -1,
  };

  window.__nbaDebug = state;
  window.__nbaDebugReady = true;
  function el(tag, className, text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = String(text);
    return node;
  }
  // `esc` 已移除：全部渲染路径改用 `el()` + textContent（DOM 自动转义）。
  function finite(value, fallback = 0) {
    return Number.isFinite(Number(value)) ? Number(value) : fallback;
  }
  function clamp(value, lo, hi) {
    return Math.min(hi, Math.max(lo, value));
  }
  function pct(value) {
    return Number.isFinite(value) ? `${(value * 100).toFixed(1)}%` : "—";
  }
  function one(value) {
    return Number.isFinite(value) ? value.toFixed(1) : "—";
  }
  function two(value) {
    return Number.isFinite(value) ? value.toFixed(2) : "—";
  }
  function timeClock(seconds) {
    const value = Math.max(0, finite(seconds));
    return `${String(Math.floor(value / 60)).padStart(2, "0")}:${String(Math.floor(value % 60)).padStart(2, "0")}`;
  }
  function timeWithTenths(seconds) {
    const value = Math.max(0, finite(seconds));
    return `${timeClock(value)}.${Math.floor((value % 1) * 10)}`;
  }
  function tickStep() {
    return (
      finite(
        runtimeRules(state.ticks[state.idx] || {}).tickSeconds,
        DEFAULT_RULES.tick_seconds,
      ) || DEFAULT_RULES.tick_seconds
    );
  }
  function eventNames(tick) {
    if (Array.isArray(tick.events) && tick.events.length) return tick.events;
    return tick.eventType ? [tick.eventType] : [];
  }

  const EVENT_NAME_ZH = {
    TIPOFF: "争顶跳球",
    TIPOFF_SECURED: "获得球权",
    CONTACT_BUMP: "身体对抗",
    ACTION_WINDOW_SHIFT: "战术推进",
    PHASE_TRANSITION: "战术流转",
    PASS: "传球配合",
    PASS_RECEIVED: "接球就绪",
    SHOT_RELEASE: "投篮出手",
    SCORE: "进球得分",
    SHOT_MISS: "投篮打铁",
    REBOUND: "争抢篮板",
    STEAL: "防守抢断",
    OUT_OF_BOUNDS: "出界停表",
    VIOLATION: "违例判罚",
    FOUL: "犯规吹罚",
    SCREEN_CONTACT: "设立掩护",
    POSSESSION_SUMMARY: "回合总结",
    TURNOVER: "进攻失误",
    BLOCK: "盖帽封盖",
  };
  function getEventNameZh(name) {
    return EVENT_NAME_ZH[name] || name;
  }

  // 动态岛核心高光事件白名单（过滤 CONTACT_BUMP 等底层物理微小碰擦杂音）
  const HIGHLIGHT_EVENTS = new Set([
    "SCORE",
    "SHOT_RELEASE",
    "SHOT_MISS",
    "REBOUND",
    "STEAL",
    "BLOCK",
    "FOUL",
    "TURNOVER",
    "SCREEN_CONTACT",
    "OUT_OF_BOUNDS",
    "TIPOFF",
    "TIPOFF_SECURED",
  ]);

  const EVENT_ICONS = {
    SCORE: "🏀",
    SHOT_RELEASE: "🎯",
    SHOT_MISS: "💥",
    REBOUND: "🛡️",
    STEAL: "⚡",
    BLOCK: "🚫",
    FOUL: "⚠️",
    TURNOVER: "🔄",
    SCREEN_CONTACT: "🧱",
    OUT_OF_BOUNDS: "🛑",
    TIPOFF: "⏱️",
    TIPOFF_SECURED: "✋",
  };

  let overlayTimer = null;

  function eventClass(name) {
    const upper = String(name).toUpperCase();
    if (upper.includes("SCORE") || upper === "DRIVE_SCORE") return "tag-score";
    if (
      upper.includes("SHOT") ||
      upper.includes("DRIVE_MISS") ||
      upper.includes("DRIVE_STOPPED")
    )
      return "tag-shot";
    if (
      upper.includes("FOUL") ||
      upper.includes("VIOLATION") ||
      upper.includes("OUT_OF_BOUNDS")
    )
      return "tag-violation";
    if (upper.includes("REBOUND")) return "tag-rebound";
    if (upper.includes("PASS")) return "tag-pass";
    if (upper.includes("DRIVE")) return "tag-drive";
    return "";
  }
  async function fetchText(url, options) {
    const response = await fetch(url, options);
    const text = await response.text();
    if (!response.ok)
      throw new Error(text || `${response.status} ${response.statusText}`);
    return text;
  }
  async function fetchDefaultRules(updateEditor) {
    let rules;
    try {
      const wasm = await ensureWasm();
      if (wasm && wasm.getDefaultRulesJson) {
        rules = JSON.parse(wasm.getDefaultRulesJson());
      } else {
        rules = JSON.parse(await fetchText("/api/rules"));
      }
    } catch (error) {
      try {
        rules = JSON.parse(await fetchText("/api/rules"));
      } catch {
        throw new Error(`规则加载失败：${error.message}`);
      }
    }
    state.rules = rules;
    state.rulesLoaded = true;
    if (updateEditor) $("rulesEditor").value = JSON.stringify(rules, null, 2);
    return rules;
  }

  async function runSimulation(withRules = null) {
    stopPlayback();
    // 移动端体验：模拟开始时视口保持在球场核心区域，杜绝下滚遮挡
    window.scrollTo({ top: 0, behavior: "smooth" });
    const parsedSeed = Number.parseInt($("seedInput").value, 10);
    const seed = Number.isFinite(parsedSeed) ? parsedSeed : 42;
    const scope = $("scopeInput").value;
    setRunStatus("模拟运行中…");
    setControlsBusy(true);
    try {
      if (!withRules) await fetchDefaultRules(false);
      const wasm = await ensureWasm();
      let responseText;
      if (wasm && wasm.simulateToNdjson) {
        let rulesJson = null;
        if (withRules) {
          rulesJson = JSON.stringify(withRules);
        } else if (state.rules) {
          rulesJson = JSON.stringify(state.rules);
        }
        responseText = wasm.simulateToNdjson(BigInt(seed), scope, rulesJson);
      } else {
        responseText = withRules
          ? await fetchText("/api/simulate", {
              method: "POST",
              headers: { "Content-Type": "application/json" },
              body: JSON.stringify({ seed, scope, rules: withRules }),
            })
          : await fetchText(
              `/api/simulate?seed=${encodeURIComponent(seed)}&scope=${encodeURIComponent(scope)}`,
            );
      }
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
    let engineViolations = [];
    for (const line of text.split(/\r?\n/)) {
      if (!line.trim()) continue;
      try {
        const record = JSON.parse(line);
        // C6.6：流末 run_summary 携带引擎官方违规（非 tick 帧，不入回放缓存）。
        if (record && record.run_summary) {
          engineViolations = Array.isArray(record.violations)
            ? record.violations
            : [];
          continue;
        }
        parsed.push(record);
      } catch (error) {
        console.warn("Skipping malformed stream line", error);
      }
    }
    if (!parsed.length) throw new Error("流中没有可解析的帧");
    state.engineViolations = engineViolations;
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
      const holderId =
        tick.ball?.holderId ||
        (tick.players || []).find((player) => player.hasBall)?.id ||
        tick.debug?.player;
      const holder =
        holderId &&
        (tick.players || []).find((player) => player.id === holderId);
      if (holderId)
        possessionTeams.set(id, holder?.team || tick.possession_team || "home");

      const names = eventNames(tick);
      names.forEach((name) => events.push({ index, name, tick }));
      let possession = possessionMap.get(id);
      if (!possession) {
        possession = {
          id,
          start: index,
          end: index,
          duration: 0,
          passes: 0,
          shots: 0,
          scores: 0,
          rebounds: 0,
          drives: 0,
          driveScores: 0,
          driveMisses: 0,
          violations: 0,
          fouls: 0,
          events: [],
          retainedRebound: false,
        };
        possessionMap.set(id, possession);
      }
      possession.end = index;
      possession.events.push(...names);
      possession.passes += names.filter((name) => name === "PASS").length;
      possession.shots += names.filter(
        (name) => name === "SHOT_RELEASE",
      ).length;
      possession.scores += names.filter(
        (name) => name === "SCORE" || name === "DRIVE_SCORE",
      ).length;
      possession.drives += names.filter(
        (name) => name === "DRIVE_INITIATED",
      ).length;
      possession.driveScores += names.filter(
        (name) => name === "DRIVE_SCORE",
      ).length;
      possession.driveMisses += names.filter(
        (name) => name === "DRIVE_MISS" || name === "DRIVE_STOPPED",
      ).length;
      possession.rebounds += names.filter((name) => name === "REBOUND").length;
      possession.violations += names.filter((name) =>
        /VIOLATION|OUT_OF_BOUNDS/.test(name),
      ).length;
      possession.fouls += names.filter((name) => /FOUL/.test(name)).length;
      if (tick.debug) decisions.push({ index, debug: tick.debug });

      for (const name of names) {
        const frameEvents = Array.isArray(tick.event_log) ? tick.event_log : [];
        const domainEvent = frameEvents.find((event) => event.kind === name);
        const payload = domainEvent?.data || null;
        if (name === "SHOT_RELEASE") {
          const eventId = domainEvent?.event_id ?? null;
          const shooterId =
            payload?.ShotRelease?.shooter_id ||
            tick.debug?.player ||
            previous?.ball?.holderId ||
            holderId ||
            "unknown";
          const shooter =
            (tick.players || []).find((player) => player.id === shooterId) ||
            (previous?.players || []).find((player) => player.id === shooterId);
          const team = shooter?.team || tick.possession_team || "home";
          const rules = runtimeRules(tick);
          const origin = payload?.ShotRelease?.pos;
          const x = Number.isFinite(Number(origin?.[0]))
            ? Number(origin[0])
            : finite(tick.ball?.x) * rules.courtWidth;
          const y = Number.isFinite(Number(origin?.[1]))
            ? Number(origin[1])
            : finite(tick.ball?.y) * rules.courtHeight;
          const hoop =
            team === "home"
              ? { x: rules.rightHoopX, y: rules.hoopY }
              : { x: rules.leftHoopX, y: rules.hoopY };
          const distance = Math.hypot(x - hoop.x, y - hoop.y);
          const shot = {
            index,
            eventId,
            possessionId: id,
            team,
            shooter: shooterId,
            x,
            y,
            distance,
            three:
              payload?.ShotRelease?.is_three ??
              distance >= rules.threePointDistance,
            makeProbability: payload?.ShotRelease?.make_probability ?? null,
            contestLevel: payload?.ShotRelease?.contest_level ?? null,
            made: null,
            resolved: false,
            points: 0,
          };
          pendingShots.push(shot);
          shots.push(shot);
        } else if (name === "SCORE" || name === "SHOT_MISS") {
          // C6.3：按引擎因果链配对（SCORE/SHOT_MISS 的 parent_event_id 指向
          // SHOT_RELEASE 的 event_id，见引擎 causal_parent_of "shot" 槽位）。
          // 原 FIFO 配对在"出手→篮板→再出手"交错时会错配，且跨回合残留。
          // 仅当旧流缺失 parent_event_id 时退化为 FIFO（遗留文件兼容）。
          const parentId = domainEvent?.parent_event_id ?? null;
          const shotIndex =
            parentId == null
              ? pendingShots.length > 0
                ? 0
                : -1
              : pendingShots.findIndex(
                  (candidate) => candidate.eventId === parentId,
                );
          if (shotIndex >= 0) {
            const shot = pendingShots.splice(shotIndex, 1)[0];
            shot.made = name === "SCORE";
            shot.resolved = true;
            if (shot.made) {
              const priorScore = previous?.score;
              const homeDelta =
                finite(tick.score?.home) - finite(priorScore?.home);
              const awayDelta =
                finite(tick.score?.away) - finite(priorScore?.away);
              shot.points = Math.max(
                0,
                shot.team === "home" ? homeDelta : awayDelta,
              );
            }
          }
        }
      }
      previous = tick;
    });

    const possessions = [...possessionMap.values()].sort(
      (a, b) => a.start - b.start,
    );
    for (const possession of possessions) {
      possession.duration = Math.max(
        0,
        finite(ticks[possession.end].t) - finite(ticks[possession.start].t),
      );
      for (let index = possession.start; index <= possession.end; index += 1) {
        if (
          eventNames(ticks[index]).includes("REBOUND") &&
          ticks[index].possession_id === ticks[index - 1]?.possession_id
        ) {
          possession.retainedRebound = true;
        }
      }
    }

    const resolvedShots = shots.filter((shot) => shot.resolved);
    const makes = resolvedShots.filter((shot) => shot.made);
    const threeAttempts = resolvedShots.filter((shot) => shot.three);
    const rebounds = events.filter((event) => event.name === "REBOUND");
    const finalScore = ticks[ticks.length - 1].score || {};
    const points = finite(finalScore.home) + finite(finalScore.away);
    const passCount = events.filter((event) => event.name === "PASS").length;
    const foulCount = events.filter((event) => /FOUL/.test(event.name)).length;
    const violationCount = events.filter((event) =>
      /VIOLATION|OUT_OF_BOUNDS/.test(event.name),
    ).length;
    const interceptCount = events.filter(
      (event) => event.name === "PASS_INTERCEPT_OPPORTUNITY",
    ).length;
    const turnoverPossessions = possessions.filter((possession) =>
      possession.events.some((name) =>
        /VIOLATION|OUT_OF_BOUNDS|PASS_INTERCEPT_OPPORTUNITY/.test(name),
      ),
    ).length;
    const medianPossession = median(
      possessions.map((possession) => possession.duration),
    );
    // C6.2：offensiveReboundPct/turnoverRate/foulRate 此前面板引用了但
    // stats 从未提供，三行永远显示 "—"（假数据静默）。补齐计算。
    const orbPossessions = possessions.filter(
      (possession) => possession.retainedRebound,
    ).length;
    const stats = {
      offensiveReboundPct: possessions.length
        ? orbPossessions / possessions.length
        : NaN,
      turnoverRate: possessions.length
        ? turnoverPossessions / possessions.length
        : NaN,
      foulRate: possessions.length ? foulCount / possessions.length : NaN,
      ppp: possessions.length ? points / possessions.length : NaN,
      medianPossession,
      passesPerPossession: possessions.length
        ? passCount / possessions.length
        : NaN,
      fgPct: resolvedShots.length ? makes.length / resolvedShots.length : NaN,
      threeShare: resolvedShots.length
        ? threeAttempts.length / resolvedShots.length
        : NaN,
      points,
      attempts: resolvedShots.length,
      makes: makes.length,
      threeAttempts: threeAttempts.length,
      rebounds: rebounds.length,
      drives: possessions.reduce(
        (total, possession) => total + possession.drives,
        0,
      ),
      driveScores: possessions.reduce(
        (total, possession) => total + possession.driveScores,
        0,
      ),
      fouls: foulCount,
      violations: violationCount,
      intercepts: interceptCount,
      maxPossession: Math.max(
        0,
        ...possessions.map((possession) => possession.duration),
      ),
    };
    // C6.6：异常面板只渲染引擎官方违规（gap.md §16.3：调试视图是投影，
    // 不允许 UI 重算语义）。原 detectAnomalies 是第二套口径（容差与引擎
    // 不一致：*1.05 vs speed_tolerance_ftps、自创 LONG_POSSESSION>35s 等）。
    const anomalies = state.engineViolations.map((violation) => ({
      index: finite(violation.tick_index),
      kind: `${violation.rule}${violation.severity ? " · " + violation.severity : ""}`,
      detail: violation.detail || "",
    }));
    return {
      events,
      possessions,
      possessionTeams,
      shots,
      decisions,
      anomalies,
      stats,
    };
  }
  function median(values) {
    const sorted = values.filter(Number.isFinite).sort((a, b) => a - b);
    if (!sorted.length) return NaN;
    const middle = Math.floor(sorted.length / 2);
    return sorted.length % 2
      ? sorted[middle]
      : (sorted[middle - 1] + sorted[middle]) / 2;
  }

  function renderStats(stats) {
    const rows = [
      ["每回合得分 (PPP)", stats.ppp, "0.90 – 1.15", BASELINES.ppp, two],
      [
        "回合耗时中位数",
        stats.medianPossession,
        "8 – 18 秒",
        BASELINES.medianPossession,
        (value) => `${one(value)} 秒`,
      ],
      [
        "每回合传球次数",
        stats.passesPerPossession,
        "1 – 5 次",
        BASELINES.passesPerPossession,
        two,
      ],
      ["投篮命中率 (FG%)", stats.fgPct, "40 – 52%", BASELINES.fgPct, pct],
      [
        "三分出手占比 (3P%)",
        stats.threeShare,
        "25 – 48%",
        BASELINES.threeShare,
        pct,
      ],
      [
        "进攻篮板率 (ORB%)",
        stats.offensiveReboundPct,
        "18 – 34%",
        BASELINES.offensiveReboundPct,
        pct,
      ],
      [
        "失误率 (TOV%)",
        stats.turnoverRate,
        "8 – 20%",
        BASELINES.turnoverRate,
        pct,
      ],
      ["每回合犯规率", stats.foulRate, "6 – 32%", BASELINES.foulRate, pct],
    ];
    const statsTable = $("statsTable");
    statsTable.replaceChildren();

    // 统计表头
    const headerRow = document.createElement("div");
    headerRow.className = "stat-row stat-header";
    headerRow.style.fontWeight = "700";
    headerRow.style.color = "var(--text-muted)";
    headerRow.style.fontSize = "10.5px";
    headerRow.style.textTransform = "uppercase";
    headerRow.style.letterSpacing = "0.5px";
    headerRow.innerHTML =
      '<span>统计指标</span><span style="text-align:right">本场数据</span><span style="text-align:right">标准区间</span><span></span>';
    statsTable.appendChild(headerRow);

    for (const [name, value, baseline, range, format] of rows) {
      const status = metricStatus(value, range[0], range[1]);
      const row = document.createElement("div");
      row.className = "stat-row";
      const nameEl = document.createElement("span");
      nameEl.className = "stat-name";
      nameEl.textContent = name;
      const valueEl = document.createElement("span");
      valueEl.className = "stat-value";
      valueEl.textContent = format(value);
      const baselineEl = document.createElement("span");
      baselineEl.className = "stat-baseline";
      baselineEl.textContent = baseline;
      const indicator = document.createElement("i");
      indicator.className = `stat-indicator stat-${status}`;
      row.append(nameEl, valueEl, baselineEl, indicator);
      statsTable.appendChild(row);
    }
    const summary = document.createElement("div");
    summary.className = "stats-summary";
    const summaryA = document.createElement("span");
    summaryA.textContent = `总投篮 ${stats.attempts} 次 · 命中 ${stats.makes} 球 · 三分出手 ${stats.threeAttempts} 次`;
    const summaryB = document.createElement("span");
    summaryB.textContent = `突破 ${stats.drives} 次 · 禁区终结 ${stats.driveScores} 次 · 篮板 ${stats.rebounds} 个 · 犯规 ${stats.fouls} 次`;
    summary.append(summaryA, summaryB);
    statsTable.appendChild(summary);
  }
  function metricStatus(value, lo, hi) {
    if (!Number.isFinite(value)) return "warn";
    const margin = (hi - lo) * 0.6;
    if (value >= lo && value <= hi) return "good";
    if (value >= lo - margin && value <= hi + margin) return "warn";
    return "bad";
  }

  function getEventDetail(event) {
    const tick = event.tick;
    const name = event.name;
    switch (name) {
      case "TIPOFF":
        return "中圈垂直抛球，比赛正式开始";
      case "TIPOFF_SECURED":
        return tick.callout || "跳球点拍争顶成功";
      case "CONTACT_BUMP":
        return "身体对抗碰撞 / 防守贴防阻截";
      case "ACTION_WINDOW_SHIFT":
        return "动作窗口时钟推进";
      case "PHASE_TRANSITION":
        return `阶段流转 ➔ ${tick.phase || ""}`;
      case "PASS":
        return "持球人传球转移出球";
      case "PASS_RECEIVED":
        return "队友稳妥接球";
      case "SHOT_RELEASE":
        return "投篮出手！";
      case "SCORE":
        return "球进！得分生效！";
      case "SHOT_MISS":
        return "投篮不中，争抢篮板";
      case "REBOUND":
        return "争顶抢下篮板球";
      case "STEAL":
        return "防守截断球路抢断！";
      case "OUT_OF_BOUNDS":
        return tick.callout && tick.callout.includes("出界")
          ? tick.callout
          : "无球跑位触碰边线（归位中）";
      case "VIOLATION":
        return tick.callout || "违例发生";
      case "FOUL":
        return tick.callout || "裁判吹罚犯规";
      case "SCREEN_CONTACT":
        return "设立掩护，发生身体挡人接触";
      case "POSSESSION_SUMMARY":
        return `回合结束总结 · #${tick.possession_id}`;
      default:
        return tick.callout || tick.phase || "";
    }
  }

  function renderTimeline() {
    const names = [...new Set(state.events.map((event) => event.name))];
    if (state.filters.size === 0) {
      // 默认排除高频物理底层事件（如单纯的每 tick 碰撞与动作窗口微调），默认呈现比赛核心技术与战术事件
      const defaultHidden = new Set(["CONTACT_BUMP", "ACTION_WINDOW_SHIFT"]);
      names.forEach((name) => {
        if (!defaultHidden.has(name)) state.filters.add(name);
      });
    }
    const filterBox = $("eventFilters");
    filterBox.replaceChildren();
    for (const name of names) {
      const button = el(
        "button",
        `filter-chip ${state.filters.has(name) ? "active" : ""}`,
      );
      button.dataset.filter = name;
      button.textContent = getEventNameZh(name);
      button.addEventListener("click", () => {
        if (state.filters.has(name)) state.filters.delete(name);
        else state.filters.add(name);
        renderTimeline();
      });
      filterBox.append(button);
    }
    const visible = state.events.filter((event) => {
      if (!state.filters.has(event.name)) return false;
      // 物理层 BoundaryCross 针对无球踩线不属于失误，只有持球出界才展示在默认技术统计中
      if (
        event.name === "OUT_OF_BOUNDS" &&
        event.tick.callout &&
        !event.tick.callout.includes("出界")
      )
        return false;
      return true;
    });
    $("timelineCount").textContent = `共 ${visible.length} 条赛事事件`;
    const fragment = document.createDocumentFragment();
    for (const event of visible) {
      const row = document.createElement("div");
      row.className = `event-row ${event.index === state.idx ? "current-tick" : ""}`;
      row.dataset.index = String(event.index);

      const topRow = el("div", "event-row-top");
      const meta = el("div", "event-meta");
      meta.append(
        el("span", "event-clock-badge", timeClock(event.tick.t_game)),
        el(
          "span",
          "event-possession-badge",
          `第 ${event.tick.possession_id ?? "—"} 回合`,
        ),
      );
      const tag = el(
        "span",
        `event-tag ${eventClass(event.name)}`,
        getEventNameZh(event.name),
      );
      topRow.append(meta, tag);

      const detail = el("div", "event-detail-text", getEventDetail(event));
      row.append(topRow, detail);

      row.addEventListener("click", () => seek(event.index));
      fragment.append(row);
      state.eventElements.push(row);
    }
    $("timeline").replaceChildren(fragment);
    if (!visible.length)
      $("timeline").replaceChildren(
        el("div", "empty-state", "当前过滤器没有事件"),
      );
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
      if (state.playing) {
        // 关键修复：仅在 timeline 容器内部滚动，杜绝整页 window 向下翻卷离开球场！
        const timeline = $("timeline");
        if (timeline && timeline.scrollHeight > timeline.clientHeight) {
          const rowTop = current.offsetTop - timeline.offsetTop;
          const target =
            rowTop - timeline.clientHeight / 2 + current.clientHeight / 2;
          timeline.scrollTop = Math.max(0, target);
        }
      }
    }
  }
  function renderAnomalies() {
    const count = state.anomalies.length;
    const badge = $("anomalyBadge");
    badge.textContent = count ? `● ${count} anomalies` : "● 0 anomalies";
    badge.className = `anomaly-badge ${count > 10 ? "anomaly-danger" : count ? "anomaly-warn" : "anomaly-ok"}`;
    // DOM API 构建（textContent 赋值，无 HTML 拼接）。
    const list = $("anomalyList");
    const bindSeek = (button) =>
      button.addEventListener("click", () =>
        seek(Number(button.dataset.index)),
      );
    if (count) {
      const fragment = document.createDocumentFragment();
      for (const item of state.anomalies) {
        const button = document.createElement("button");
        button.className = "anomaly-item";
        button.dataset.index = String(item.index);
        button.textContent = `#${item.index} · ${item.kind} · ${item.detail}`;
        bindSeek(button);
        fragment.appendChild(button);
      }
      list.replaceChildren(fragment);
    } else {
      const empty = document.createElement("span");
      empty.className = "muted-label";
      empty.textContent = "引擎不变量未报告违规。";
      list.replaceChildren(empty);
    }
    $("streamSummary").textContent = count
      ? `引擎报告 ${count} 条违规（点击定位）`
      : "流已加载 · 引擎不变量 0 违规";
  }
  function updateReadouts(source) {
    $("eventReadout").textContent = `共 ${state.events.length} 条事件`;
    $("streamSummary").textContent =
      `${source} · ${state.possessions.length} possessions · ${state.shots.length} shots`;
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
    $("tickReadout").textContent =
      `${state.idx.toLocaleString()} / ${state.ticks.length.toLocaleString()} 帧`;
    $("progressTime").textContent = timeWithTenths(tick.t);
    $("progressPossession").textContent =
      `第 ${tick.possession_id ?? "—"} 回合`;
    if (forceJson || !state.playing || state.idx % 3 === 0)
      renderFrameJson(tick);
    renderDecision(tick);
  }
  function updateHud(tick) {
    const homeTeam = tick.home_team || {};
    const awayTeam = tick.away_team || {};
    if ($("frameLabel")) $("frameLabel").textContent = `frame ${state.idx}`;
    $("homeTeamName").textContent =
      homeTeam.short_name || homeTeam.name || "HOME";
    $("awayTeamName").textContent =
      awayTeam.short_name || awayTeam.name || "AWAY";
    $("tacticalSet").textContent = tick.tactical_set || "—";
    $("phaseLabel").textContent = String(tick.phase || "—")
      .replaceAll("_", " ")
      .toUpperCase();
    const allNames = eventNames(tick);
    const highlightNames = allNames.filter((name) =>
      HIGHLIGHT_EVENTS.has(name),
    );
    const chip = $("eventChip");
    const overlay = $("eventOverlayChip");
    if (highlightNames.length) {
      const icon = EVENT_ICONS[highlightNames[0]] || "⚡";
      const txt = `${icon} ${highlightNames.map(getEventNameZh).join(" · ")}`;
      if (chip) {
        chip.textContent = txt;
        chip.classList.add("active");
      }
      if (overlay) {
        overlay.textContent = txt;
        overlay.classList.add("active");
        if (overlayTimer) clearTimeout(overlayTimer);
        overlayTimer = setTimeout(() => {
          overlay.classList.remove("active");
        }, 2200);
      }
    }
    $("possessionLabel").textContent = `第 ${tick.possession_id ?? "—"} 回合`;
    const newHome = finite(tick.score?.home).toFixed(0);
    const newAway = finite(tick.score?.away).toFixed(0);
    if (
      $("homeScore").textContent !== newHome &&
      $("homeScore").textContent !== ""
    ) {
      $("homeScore").classList.remove("score-pulse");
      void $("homeScore").offsetWidth;
      $("homeScore").classList.add("score-pulse");
    }
    if (
      $("awayScore").textContent !== newAway &&
      $("awayScore").textContent !== ""
    ) {
      $("awayScore").classList.remove("score-pulse");
      void $("awayScore").offsetWidth;
      $("awayScore").classList.add("score-pulse");
    }
    $("homeScore").textContent = newHome;
    $("awayScore").textContent = newAway;
    $("periodLabel").textContent = `第 ${tick.period || 1} 节`;
    $("gameClock").textContent = timeClock(tick.gameClock ?? tick.t_game);
    const sc = finite(tick.shotClock);
    $("shotClock").textContent = one(tick.shotClock);
    $("shotClock").classList.toggle("urgent-shot-clock", sc <= 5 && sc > 0);
    $("flowLabel").textContent = String(tick.game_flow || "LIVE")
      .replaceAll("Ball", "")
      .toUpperCase();
    $("foulsReadout").textContent =
      `${tick.team_fouls_home ?? 0} / ${tick.team_fouls_away ?? 0}`;
    $("freeThrows").textContent = String(tick.free_throws_remaining ?? 0);
    $("intensityReadout").textContent = tick.intensity || "—";
    const callout = tick.callout || "—";
    if ($("calloutText").textContent !== callout) {
      $("calloutText").textContent = callout;
      if (callout !== "—") {
        $("calloutText").classList.remove("callout-flash");
        void $("calloutText").offsetWidth;
        $("calloutText").classList.add("callout-flash");
      }
    }
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
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const rules = runtimeRules(tick);

    // 标准 NBA 全场物理世界等比例严密映射 (Zero Distortion Uniform Scaling)
    // Canvas: 1000 x 560
    // 外圈 Apron 缓冲区: 30px (带底线球队标识)
    // 比赛场内有效尺寸: 940px x 500px (10px = 1英尺, 严格 94:50 物理长宽比)
    const originX = 30;
    const originY = 30;
    const scale = 10.0;

    const point = (ftX, ftY) => ({
      x: originX + ftX * scale,
      y: originY + ftY * scale,
    });

    ctx.clearRect(0, 0, canvas.width, canvas.height);

    // 1. 赛场外围环带 (Arena Apron / Perimeter)
    ctx.fillStyle = "#0a0d12";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // 底线外侧主客队文字 (Visitor / Home Lettering)
    ctx.save();
    ctx.font = "900 16px 'Plus Jakarta Sans', -apple-system, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    // 左侧底线外客队字样
    ctx.save();
    ctx.translate(15, 280);
    ctx.rotate(-Math.PI / 2);
    ctx.fillStyle = "rgba(245, 158, 11, 0.45)";
    ctx.fillText("VISITOR", 0, 0);
    ctx.restore();

    // 右侧底线外主队字样
    ctx.save();
    ctx.translate(985, 280);
    ctx.rotate(Math.PI / 2);
    ctx.fillStyle = "rgba(16, 185, 129, 0.45)";
    ctx.fillText("HOME", 0, 0);
    ctx.restore();

    // 边线外侧副标
    ctx.font = "700 9.5px 'Plus Jakarta Sans', -apple-system, sans-serif";
    ctx.fillStyle = "rgba(255, 255, 255, 0.2)";
    ctx.fillText("NBA SIMULATION ARENA", 500, 15);
    ctx.restore();

    // 2. 比赛主场地高级浅色枫木地板 (Playing Surface: 940 x 500)
    const floorGrad = ctx.createLinearGradient(30, 30, 970, 530);
    floorGrad.addColorStop(0, "#dfcca6");
    floorGrad.addColorStop(0.5, "#d2be97");
    floorGrad.addColorStop(1, "#dac59f");
    ctx.fillStyle = floorGrad;
    ctx.fillRect(30, 30, 940, 500);

    // 枫木拼板纵向缝隙细纹
    ctx.strokeStyle = "rgba(0, 0, 0, 0.025)";
    ctx.lineWidth = 1;
    for (let x = 50; x < 970; x += 20) {
      ctx.beginPath();
      ctx.moveTo(x, 30);
      ctx.lineTo(x, 530);
      ctx.stroke();
    }

    // 3. 禁区与油漆区 (Paint / Key: 19ft x 16ft -> 190px x 160px)
    // 客队禁区柔光底色 (左)
    ctx.fillStyle = "rgba(245, 158, 11, 0.12)";
    ctx.fillRect(30, 200, 190, 160);
    // 主队禁区柔光底色 (右)
    ctx.fillStyle = "rgba(16, 185, 129, 0.12)";
    ctx.fillRect(780, 200, 190, 160);

    // 4. 白色标准球场地线 (Standard Court Boundary & Lines)
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 2.0;

    // 主边界线 (94 x 50 ft -> 940 x 500 px)
    ctx.strokeRect(30, 30, 940, 500);

    // 中线 (Half Court Line, ftX = 47 -> x = 500)
    ctx.beginPath();
    ctx.moveTo(500, 30);
    ctx.lineTo(500, 530);
    ctx.stroke();

    // 中圈 (Center Circle, 半径 6 ft = 60px; 内圈半径 2 ft = 20px)
    ctx.beginPath();
    ctx.arc(500, 280, 60, 0, Math.PI * 2);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(500, 280, 20, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.6)";
    ctx.stroke();
    ctx.strokeStyle = "#ffffff";

    // 绘制标准禁区、三分线与篮筐
    drawKey(ctx, false);
    drawKey(ctx, true);
    drawThreePointLine(ctx, false);
    drawThreePointLine(ctx, true);
    drawHoop(ctx, 30 + rules.leftHoopX * 10, 280, false);
    drawHoop(ctx, 30 + rules.rightHoopX * 10, 280, true);

    // 轨迹绘制
    drawTrails(ctx, point);

    // 5. 球员渲染
    state.hitPlayers.length = 0;
    const playerRadius = 15; // 严格人体防守圆柱体比例

    for (const player of tick.players || []) {
      if (player.onCourt === false) continue;
      const playerPoint = point(
        finite(player.x) * rules.courtWidth,
        finite(player.y) * rules.courtHeight,
      );
      state.hitPlayers.push({ player, x: playerPoint.x, y: playerPoint.y });

      // 战术路线 (Play-art route)
      if (player.target_x !== undefined && player.target_y !== undefined) {
        const targetPt = point(
          finite(player.target_x) * rules.courtWidth,
          finite(player.target_y) * rules.courtHeight,
        );
        const dist = Math.hypot(
          targetPt.x - playerPoint.x,
          targetPt.y - playerPoint.y,
        );
        if (dist > 18) {
          ctx.save();
          ctx.setLineDash([4, 4]);
          ctx.strokeStyle =
            player.team === "home"
              ? "rgba(16, 185, 129, 0.45)"
              : "rgba(245, 158, 11, 0.45)";
          ctx.lineWidth = 1.5;
          ctx.beginPath();
          ctx.moveTo(playerPoint.x, playerPoint.y);
          ctx.lineTo(targetPt.x, targetPt.y);
          ctx.stroke();
          ctx.beginPath();
          ctx.arc(targetPt.x, targetPt.y, 4, 0, Math.PI * 2);
          ctx.fillStyle =
            player.team === "home"
              ? "rgba(16, 185, 129, 0.7)"
              : "rgba(245, 158, 11, 0.7)";
          ctx.fill();
          ctx.restore();
        }
      }

      // 球员地面阴影
      ctx.beginPath();
      ctx.ellipse(
        playerPoint.x,
        playerPoint.y + 2,
        playerRadius * 0.9,
        playerRadius * 0.45,
        0,
        0,
        Math.PI * 2,
      );
      ctx.fillStyle = "rgba(0, 0, 0, 0.25)";
      ctx.fill();

      // 持球人高亮能量环 (温和律动光波)
      if (player.hasBall) {
        const nowPulse = (Math.sin(performance.now() / 300) + 1) * 0.5;
        const pulseR = playerRadius + 4 + nowPulse * 2.5;
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, pulseR, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(245, 158, 11, ${0.45 + nowPulse * 0.4})`;
        ctx.lineWidth = 2.0;
        ctx.stroke();
      }

      // 球员身体圆环 (主队翠绿 / 客队暖金)
      const teamColor = player.team === "home" ? "#10b981" : "#f59e0b";
      ctx.beginPath();
      ctx.arc(playerPoint.x, playerPoint.y, playerRadius, 0, Math.PI * 2);
      ctx.fillStyle = "#11161f";
      ctx.fill();
      ctx.strokeStyle = teamColor;
      ctx.lineWidth = 2.5;
      ctx.stroke();

      // 背号 (纯白高对比度粗体)
      ctx.font = "700 11px 'JetBrains Mono', monospace";
      ctx.fillStyle = "#ffffff";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const num =
        player.number === undefined
          ? String(player.id || "")
          : String(player.number);
      ctx.fillText(num, playerPoint.x, playerPoint.y + 0.5);
    }

    // 6. 篮球渲染 (严格三维投射与地面阴影)
    if (tick.ball) {
      const ballPoint = point(
        finite(tick.ball.x) * rules.courtWidth,
        finite(tick.ball.y) * rules.courtHeight,
      );
      const ballRadius = 6.5;
      const ballZ = finite(tick.ball.z);
      const heightOffset = Math.min(30, ballZ * 2.8);
      const ballCenterY = ballPoint.y - heightOffset;

      // 地面阴影
      if (heightOffset > 2) {
        ctx.beginPath();
        ctx.ellipse(
          ballPoint.x,
          ballPoint.y,
          Math.max(2, ballRadius * (1 - heightOffset / 60)),
          Math.max(1, ballRadius * 0.5 * (1 - heightOffset / 60)),
          0,
          0,
          Math.PI * 2,
        );
        ctx.fillStyle = "rgba(0, 0, 0, 0.35)";
        ctx.fill();
      }

      // 篮球球体高光
      ctx.beginPath();
      ctx.arc(ballPoint.x, ballCenterY, ballRadius, 0, Math.PI * 2);
      const bGrad = ctx.createRadialGradient(
        ballPoint.x - 2,
        ballCenterY - 2,
        1,
        ballPoint.x,
        ballCenterY,
        ballRadius,
      );
      bGrad.addColorStop(0, "#f97316");
      bGrad.addColorStop(0.7, "#ea580c");
      bGrad.addColorStop(1, "#9a3412");
      ctx.fillStyle = bGrad;
      ctx.fill();
      ctx.strokeStyle = "#431407";
      ctx.lineWidth = 0.8;
      ctx.stroke();

      // 进球篮筐光波
      const hasScoreEvent =
        (tick.event || "").includes("MADE") ||
        (tick.event || "").includes("3PT") ||
        (tick.event || "").includes("2PT");
      if (hasScoreEvent) {
        const hoopX =
          tick.ball && tick.ball.x > 0.5
            ? 30 + rules.rightHoopX * 10
            : 30 + rules.leftHoopX * 10;
        ctx.beginPath();
        ctx.arc(hoopX, 280, 22, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(44, 229, 155, 0.85)";
        ctx.lineWidth = 2.5;
        ctx.stroke();
      }
    }
  }

  function drawHoop(ctx, hoopX, hoopY, right) {
    // 篮板 (Backboard: 宽 6 ft = 60px, 厚 4px, 距底线 4 ft = 40px)
    const boardX = right ? 970 - 40 : 30 + 40;
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 3.5;
    ctx.beginPath();
    ctx.moveTo(boardX, hoopY - 30);
    ctx.lineTo(boardX, hoopY + 30);
    ctx.stroke();

    // 篮板连接支架 (Stanchion)
    ctx.strokeStyle = "rgba(255, 255, 255, 0.4)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(right ? 970 : 30, hoopY);
    ctx.lineTo(boardX, hoopY);
    ctx.stroke();

    // 篮筐 (Rim: 直径 1.5 ft = 15px, 橙红色)
    ctx.strokeStyle = "#f97316";
    ctx.lineWidth = 2.0;
    ctx.beginPath();
    ctx.arc(hoopX, hoopY, 7.5, 0, Math.PI * 2);
    ctx.stroke();
  }

  function drawKey(ctx, right) {
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 2.0;

    // 禁区外框 (Key / Paint: 19 ft x 16 ft -> 190px x 160px)
    const keyX = right ? 780 : 30;
    ctx.strokeRect(keyX, 200, 190, 160);

    // 罚球圈 (Free Throw Circle: 顶端在 19 ft 处，半径 6 ft = 60px)
    const ftCenterX = right ? 780 : 220;
    ctx.beginPath();
    ctx.arc(
      ftCenterX,
      280,
      60,
      right ? Math.PI / 2 : -Math.PI / 2,
      right ? (3 * Math.PI) / 2 : Math.PI / 2,
    );
    ctx.stroke();

    // 罚球圈虚线半圆 (进入禁区的一侧)
    ctx.save();
    ctx.setLineDash([6, 6]);
    ctx.beginPath();
    ctx.arc(
      ftCenterX,
      280,
      60,
      right ? -Math.PI / 2 : Math.PI / 2,
      right ? Math.PI / 2 : (3 * Math.PI) / 2,
    );
    ctx.stroke();
    ctx.restore();
  }

  function drawThreePointLine(ctx, right) {
    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 2.0;

    // 篮筐中心: Y = 280, 距底线 5.25 ft = 52.5px
    const hoopX = right ? 917.5 : 82.5;
    const hoopY = 280;
    const r3pt = 237.5; // 23.75 ft x 10 = 237.5 px

    // 底角三分线 (距离边线 3 ft = 30px, Y = 60 和 Y = 500)
    // 底角长度 14 ft = 140px (从底线延伸 140px)
    const cornerBreakX = right ? 970 - 140 : 30 + 140;
    const baselineX = right ? 970 : 30;

    // 上侧底角直线
    ctx.beginPath();
    ctx.moveTo(baselineX, 60);
    ctx.lineTo(cornerBreakX, 60);
    ctx.stroke();

    // 下侧底角直线
    ctx.beginPath();
    ctx.moveTo(baselineX, 500);
    ctx.lineTo(cornerBreakX, 500);
    ctx.stroke();

    // 三分大圆弧 (以篮筐为圆心，半径 237.5px)
    // 严格计算圆弧相交角度: sin(theta) = (280 - 60) / 237.5 = 220 / 237.5
    const angleDelta = Math.asin(220 / 237.5);
    ctx.beginPath();
    if (right) {
      ctx.arc(hoopX, hoopY, r3pt, Math.PI - angleDelta, Math.PI + angleDelta);
    } else {
      ctx.arc(hoopX, hoopY, r3pt, -angleDelta, angleDelta);
    }
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
      const ballPoint = point(
        finite(ball.x) * rules.courtWidth,
        finite(ball.y) * rules.courtHeight,
      );
      if (index === start) ctx.moveTo(ballPoint.x, ballPoint.y);
      else ctx.lineTo(ballPoint.x, ballPoint.y);
    }
    ctx.strokeStyle = "rgba(255,235,188,.32)";
    ctx.lineWidth = 1.4;
    ctx.stroke();
  }

  function renderDecision(tick) {
    const previous = [...state.decisions]
      .reverse()
      .find((item) => item.index <= state.idx);
    const debug = tick.debug || previous?.debug;
    const panel = $("decisionPanel");
    panel.replaceChildren();
    if (!debug) {
      const empty = el("div", "empty-state", "当前帧之前没有决策追踪");
      empty.append(
        el("br"),
        el(
          "small",
          null,
          "跳到一个决策帧，或点击事件流中的 PASS / SHOT_RELEASE",
        ),
      );
      panel.replaceChildren(empty);
      return;
    }
    const utilities = debug.utilities || [];
    const probabilities = debug.probabilities || [];
    const maxUtility = Math.max(
      0.001,
      ...utilities.map((item) => Math.abs(finite(item.utility))),
    );
    const chosen = debug.chosen || "";
    const decisionIndex =
      state.decisions.find((item) => item.debug === debug)?.index ?? state.idx;
    const section = (title, ...children) => {
      const box = el("div", "decision-section");
      box.append(el("div", "decision-title", title), ...children);
      return box;
    };
    const scoreRows = (items, value, scale) => {
      if (!items.length) return el("span", "muted-label", "无数据");
      const box = el("div");
      for (const item of items) {
        const number = finite(value(item));
        const selected = item.kind === chosen;
        const row = el("div", "score-bar-row");
        const track = el("span", "score-bar-track");
        const fill = el("i", `score-bar-fill ${selected ? "chosen" : ""}`);
        fill.style.width = `${clamp(Math.abs(number) * scale, 1, 100)}%`;
        track.append(fill);
        row.append(
          el("span", "score-bar-label", item.kind),
          track,
          el("span", "score-bar-value", number.toFixed(3)),
        );
        box.appendChild(row);
      }
      return box;
    };
    const flagSection = (flags) => {
      if (!flags.length) return el("span", "muted-label", "无约束惩罚");
      const box = el("div");
      for (const flag of flags) {
        const row = el("div", "flag-row");
        row.append(
          el("span", null, flag.constraint),
          el("span", null, flag.reason),
          el("b", null, two(finite(flag.penalty))),
        );
        box.appendChild(row);
      }
      return box;
    };
    const chips = (items, className, emptyText) => {
      const box = el("div", "chips");
      if (items?.length) {
        for (const item of items)
          box.append(el("span", `chip ${className}`, item));
      } else {
        box.append(el("span", "muted-label", emptyText));
      }
      return box;
    };
    const head = el("div", "decision-head");
    head.append(
      el("strong", null, chosen || "—"),
      el("span", null, `${debug.player || "—"} · frame ${decisionIndex}`),
    );
    panel.append(
      head,
      section(
        "Utility ranking",
        scoreRows(utilities, (item) => item.utility, 100 / maxUtility),
      ),
      section(
        "Softmax probability",
        scoreRows(probabilities, (item) => item.prob, 100),
      ),
      section(
        "Hard blockers",
        chips(debug.blocked, "bad", "没有被硬约束剔除的候选"),
      ),
      section("Constraint flags", flagSection(debug.flags || [])),
      section("Active constraints", chips(debug.active_constraints, "", "—")),
      section(
        "Enforcement feedback",
        chips(debug.enforcement, "warn", "本帧没有执行意图"),
      ),
    );
    const nextButton = el(
      "button",
      "text-button decision-next",
      "跳到下一决策帧 →",
    );
    nextButton.id = "nextDecisionButton";
    nextButton.addEventListener("click", () => {
      const next = state.decisions.find((item) => item.index > state.idx);
      if (next) seek(next.index);
      else setRunStatus("已经是最后一个决策帧");
    });
    panel.appendChild(nextButton);
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
      ctx.arc(
        shot.x * scaleX,
        shot.y * scaleY,
        shot.three ? 5 : 4,
        0,
        Math.PI * 2,
      );
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
      [
        `Mid ${rim}–16 ft`,
        (shot) => shot.distance >= rim && shot.distance < 16,
      ],
      [`Long 16–${three} ft`, (shot) => shot.distance >= 16 && !shot.three],
      [`Three ${three}+ ft`, (shot) => shot.three],
    ];
    // DOM API 构建（textContent 赋值，无 HTML 拼接）。
    const zoneTable = $("zoneTable");
    const zoneFragment = document.createDocumentFragment();
    for (const [name, predicate] of zones) {
      const subset = state.shots.filter(predicate);
      const made = subset.filter((shot) => shot.made === true).length;
      const row = document.createElement("div");
      row.className = "zone-row";
      const label = document.createElement("strong");
      label.textContent = name;
      const value = document.createElement("span");
      value.textContent = `${made}/${subset.length} · ${subset.length ? pct(made / subset.length) : "—"}`;
      row.append(label, value);
      zoneFragment.appendChild(row);
    }
    zoneTable.replaceChildren(zoneFragment);
  }

  function seek(index) {
    if (!state.ticks.length) return;
    stopPlayback();
    state.idx = clamp(
      Math.round(Number(index) || 0),
      0,
      state.ticks.length - 1,
    );
    state.accumulator = 0;
    renderCurrent(true);
  }
  function step(delta) {
    stopPlayback();
    seek(state.idx + delta);
  }
  function setControlsBusy(busy) {
    $("runButton").disabled = busy;
    $("fileInput").disabled = busy;
    $("runRulesButton").disabled = busy;
    $("loadRulesButton").disabled = busy;
    document.querySelectorAll("[data-preset]").forEach((button) => {
      button.disabled = busy;
    });
    document.querySelectorAll(".transport-group button").forEach((button) => {
      button.disabled = busy;
    });
    document.querySelectorAll(".speed-group button").forEach((button) => {
      button.disabled = busy;
    });
    $("jumpButton").disabled = busy;
    $("progressInput").disabled = busy;
    if (busy) stopPlayback();
  }
  function possessionJump(direction) {
    if (!state.possessions.length) return;
    const starts = state.possessions.map((possession) => possession.start);
    let current = 0;
    for (let index = 0; index < starts.length; index += 1)
      if (starts[index] <= state.idx) current = index;
    seek(starts[clamp(current + direction, 0, starts.length - 1)]);
  }
  function togglePlayback() {
    if (!state.ticks.length) return;
    state.playing = !state.playing;
    state.previousRaf = 0;
    updatePlayButton();
    if (state.playing && !state.rafId)
      state.rafId = requestAnimationFrame(playbackFrame);
  }
  function stopPlayback() {
    state.playing = false;
    updatePlayButton();
  }
  function updatePlayButton() {
    $("playButton").textContent = state.playing ? "Ⅱ" : "▶";
  }
  function playbackFrame(now) {
    state.rafId = requestAnimationFrame(playbackFrame);
    if (!state.playing || !state.ticks.length) return;
    if (!state.previousRaf) state.previousRaf = now;
    const elapsed = Math.min(0.25, (now - state.previousRaf) / 1000);
    state.previousRaf = now;
    state.accumulator += (elapsed * state.speed) / tickStep();
    while (state.accumulator >= 1 && state.idx < state.ticks.length - 1) {
      state.idx += 1;
      state.accumulator -= 1;
    }
    if (state.idx >= state.ticks.length - 1) {
      state.playing = false;
      updatePlayButton();
    }
    renderCurrent(false);
  }

  async function loadDefaultRules() {
    try {
      await fetchDefaultRules(true);
      setRulesStatus("默认 GameRules 已载入");
    } catch (error) {
      setRulesStatus(error.message, true);
    }
  }
  function setRulesStatus(text, error = false) {
    $("rulesStatus").textContent = text;
    $("rulesStatus").className = `rules-status ${error ? "error" : "success"}`;
  }
  async function runEditedRules() {
    let rules;
    try {
      rules = JSON.parse($("rulesEditor").value);
    } catch (error) {
      setRulesStatus(`JSON 错误：${error.message}`, true);
      return;
    }
    setRulesStatus("规则已解析，模拟运行中…");
    const ok = await runSimulation(rules);
    if (ok)
      setRulesStatus(`完成：${state.ticks.length.toLocaleString()} ticks`);
  }
  function applyPreset(name) {
    let rules;
    try {
      rules = JSON.parse($("rulesEditor").value || "{}");
    } catch {
      rules = { ...state.rules };
    }
    if (name === "default") rules = { ...state.rules };
    if (name === "fast") {
      rules.tick_seconds = 0.02;
      rules.decision_interval_seconds = Math.min(
        finite(rules.decision_interval_seconds, 0.8),
        0.4,
      );
    }
    if (name === "contest") {
      rules.shot_contest_sensitivity = Math.min(
        0.85,
        finite(rules.shot_contest_sensitivity, 0.22) * 1.8,
      );
      rules.contact_margin_ft = Math.max(
        0.6,
        finite(rules.contact_margin_ft, 0.6) * 1.5,
      );
    }
    $("rulesEditor").value = JSON.stringify(rules, null, 2);
    setRulesStatus(`已载入 ${name.toUpperCase()} 预设，点击应用并重跑`);
  }

  function wire() {
    $("runButton").addEventListener("click", () => runSimulation());
    $("fileInput").addEventListener("change", async (event) => {
      const file = event.target.files?.[0];
      if (!file) return;
      try {
        loadStream(await file.text(), file.name);
        setRunStatus(`${state.ticks.length.toLocaleString()} ticks`);
      } catch (error) {
        setRunStatus("文件读取失败", true);
        $("streamSummary").textContent = error.message;
      }
    });
    $("anomalyBadge").addEventListener("click", () => {
      const panel = $("anomalyPanel");
      const open = panel.hidden;
      panel.hidden = !open;
      $("anomalyBadge").setAttribute("aria-expanded", String(open));
    });
    document.querySelectorAll(".tab-button").forEach((button) =>
      button.addEventListener("click", () => {
        state.currentTab = button.dataset.tab;
        document
          .querySelectorAll(".tab-button")
          .forEach((item) => item.classList.toggle("active", item === button));
        document.querySelectorAll(".tab-pane").forEach((pane) => {
          const active = pane.id === `tab-${state.currentTab}`;
          pane.hidden = !active;
          pane.classList.toggle("active", active);
        });
        if (state.currentTab === "shots") renderShotMap();
      }),
    );
    $("prevPossessionButton").addEventListener("click", () =>
      possessionJump(-1),
    );
    $("nextPossessionButton").addEventListener("click", () =>
      possessionJump(1),
    );
    $("stepBackButton").addEventListener("click", () => step(-1));
    $("stepForwardButton").addEventListener("click", () => step(1));
    $("playButton").addEventListener("click", togglePlayback);
    $("progressInput").addEventListener("input", (event) =>
      seek(event.target.value),
    );
    $("jumpButton").addEventListener("click", () => seek($("jumpInput").value));
    document.querySelectorAll(".speed-group button").forEach((button) =>
      button.addEventListener("click", () => {
        state.speed = Number(button.dataset.speed);
        document
          .querySelectorAll(".speed-group button")
          .forEach((item) => item.classList.toggle("active", item === button));
      }),
    );
    $("loadRulesButton").addEventListener("click", loadDefaultRules);
    $("runRulesButton").addEventListener("click", runEditedRules);
    document
      .querySelectorAll("[data-preset]")
      .forEach((button) =>
        button.addEventListener("click", () =>
          applyPreset(button.dataset.preset),
        ),
      );
    $("copyFrameButton").addEventListener("click", async () => {
      try {
        await navigator.clipboard.writeText($("frameJson").textContent);
        $("copyFrameButton").textContent = "已复制";
        setTimeout(() => {
          $("copyFrameButton").textContent = "复制当前帧";
        }, 1200);
      } catch {
        setRunStatus("浏览器拒绝访问剪贴板", true);
      }
    });

    const courtViewBtn = $("courtViewBtn");
    if (courtViewBtn) {
      courtViewBtn.addEventListener("click", () => {
        state.courtMode = state.courtMode === "half" ? "full" : "half";
        const icon = $("courtViewIcon");
        const label = $("courtViewLabel");
        if (icon) icon.textContent = state.courtMode === "half" ? "🔍" : "🌐";
        if (label)
          label.textContent =
            state.courtMode === "half" ? "半场特写" : "全场鸟瞰";
        if (state.ticks && state.ticks[state.idx]) {
          drawCourt(state.ticks[state.idx]);
        }
      });
    }

    const quickBtn = $("quickSimBtn");
    if (quickBtn) quickBtn.addEventListener("click", () => runSimulation());

    const openSet = $("openSettingsBtn");
    const closeSet = $("closeSettingsBtn");
    const sheet = $("settingsSheet");
    if (openSet && sheet) {
      openSet.addEventListener("click", () => {
        sheet.hidden = false;
      });
    }
    if (closeSet && sheet) {
      closeSet.addEventListener("click", () => {
        sheet.hidden = true;
      });
    }
    if (sheet) {
      sheet.addEventListener("click", (e) => {
        if (e.target === sheet) sheet.hidden = true;
      });
    }

    const applyRun = $("applyAndRunBtn");
    if (applyRun && sheet) {
      applyRun.addEventListener("click", () => {
        sheet.hidden = true;
        runSimulation();
      });
    }

    const randomBtn = $("randomSeedBtn");
    if (randomBtn) {
      randomBtn.addEventListener("click", () => {
        $("seedInput").value = String(Math.floor(Math.random() * 90000) + 1000);
      });
    }

    const uploadTrigger = $("uploadTriggerBtn");
    if (uploadTrigger) {
      uploadTrigger.addEventListener("click", () => $("fileInput").click());
    }

    document.querySelectorAll(".scope-chip").forEach((chip) => {
      chip.addEventListener("click", () => {
        document
          .querySelectorAll(".scope-chip")
          .forEach((c) => c.classList.remove("active"));
        chip.classList.add("active");
        $("scopeInput").value = chip.dataset.scope;
      });
    });
    function handlePointer(clientX, clientY, canvas) {
      const rect = canvas.getBoundingClientRect();
      const x = ((clientX - rect.left) * canvas.width) / rect.width;
      const y = ((clientY - rect.top) * canvas.height) / rect.height;
      const hit = state.hitPlayers.find(
        (item) => Math.hypot(item.x - x, item.y - y) < 26,
      );
      const tooltip = $("playerTooltip");
      if (!hit) {
        tooltip.hidden = true;
        return;
      }
      const player = hit.player;
      tooltip.hidden = false;
      const isMobile = window.innerWidth <= 640;
      if (isMobile) {
        tooltip.style.left = "50%";
        tooltip.style.transform = "translateX(-50%)";
        tooltip.style.top = "8px";
      } else {
        tooltip.style.transform = "none";
        tooltip.style.left = `${Math.min(rect.width - 165, (x / canvas.width) * rect.width + 12)}px`;
        tooltip.style.top = `${Math.max(4, (y / canvas.height) * rect.height - 35)}px`;
      }
      tooltip.replaceChildren(
        el("strong", null, `${player.id} · #${player.jersey}`),
        el("span", null, `${player.action} · ${player.slot}`),
        el(
          "span",
          null,
          `stamina ${one(player.stm)}/${one(player.stmMax)} · ${player.morale}`,
        ),
      );
    }
    $("courtCanvas").addEventListener("mousemove", (event) => {
      handlePointer(event.clientX, event.clientY, event.currentTarget);
    });
    $("courtCanvas").addEventListener(
      "touchstart",
      (event) => {
        if (event.touches.length === 1) {
          handlePointer(
            event.touches[0].clientX,
            event.touches[0].clientY,
            event.currentTarget,
          );
        }
      },
      { passive: true },
    );
    $("courtCanvas").addEventListener(
      "touchmove",
      (event) => {
        if (event.touches.length === 1) {
          handlePointer(
            event.touches[0].clientX,
            event.touches[0].clientY,
            event.currentTarget,
          );
        }
      },
      { passive: true },
    );
    $("courtCanvas").addEventListener("mouseleave", () => {
      $("playerTooltip").hidden = true;
    });
    $("courtCanvas").addEventListener("touchend", () => {
      setTimeout(() => {
        $("playerTooltip").hidden = true;
      }, 2500);
    });
    document.addEventListener("keydown", (event) => {
      if (event.target.matches("input, textarea, select")) return;
      if (event.code === "Space") {
        event.preventDefault();
        togglePlayback();
      }
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
