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
      ["Points per possession", stats.ppp, "0.90 – 1.15", BASELINES.ppp, two],
      [
        "Median possession",
        stats.medianPossession,
        "8 – 18 s",
        BASELINES.medianPossession,
        (value) => `${one(value)} s`,
      ],
      [
        "Passes / possession",
        stats.passesPerPossession,
        "1 – 5",
        BASELINES.passesPerPossession,
        two,
      ],
      ["Field-goal percentage", stats.fgPct, "40 – 52%", BASELINES.fgPct, pct],
      [
        "3PT attempt share",
        stats.threeShare,
        "25 – 48%",
        BASELINES.threeShare,
        pct,
      ],
      [
        "Offensive rebound rate",
        stats.offensiveReboundPct,
        "18 – 34%",
        BASELINES.offensiveReboundPct,
        pct,
      ],
      [
        "Turnover proxy rate",
        stats.turnoverRate,
        "8 – 20%",
        BASELINES.turnoverRate,
        pct,
      ],
      [
        "Fouls / possession",
        stats.foulRate,
        "6 – 32%",
        BASELINES.foulRate,
        pct,
      ],
    ];
    // C6.2：DOM API 构建（textContent 赋值，无 HTML 拼接）。
    const statsTable = $("statsTable");
    statsTable.replaceChildren();
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
    summaryA.textContent = `${stats.attempts} FGA · ${stats.makes} FGM · ${stats.threeAttempts} 3PA`;
    const summaryB = document.createElement("span");
    summaryB.textContent = `${stats.drives} DRV · ${stats.driveScores} FIN · ${stats.rebounds} REB · ${stats.fouls} FOUL`;
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
        `filter-button ${state.filters.has(name) ? "active" : ""}`,
      );
      button.dataset.filter = name;
      button.textContent = name;
      button.addEventListener("click", () => {
        if (state.filters.has(name)) state.filters.delete(name);
        else state.filters.add(name);
        renderTimeline();
      });
      filterBox.appendChild(button);
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
    $("timelineCount").textContent = `${visible.length} events`;
    const fragment = document.createDocumentFragment();
    for (const event of visible) {
      const row = document.createElement("div");
      row.className = `event-row ${event.index === state.idx ? "current" : ""}`;
      row.dataset.index = String(event.index);
      const detail = getEventDetail(event);
      row.append(
        el("span", "event-time", timeClock(event.tick.t_game)),
        el("span", "event-possession", `#${event.tick.possession_id}`),
        el("span", `event-tag ${eventClass(event.name)}`, event.name),
        el("span", "event-detail", detail),
      );
      row.addEventListener("click", () => seek(event.index));
      fragment.appendChild(row);
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
    $("eventReadout").textContent = `${state.events.length} events`;
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
      `${state.idx.toLocaleString()} / ${state.ticks.length.toLocaleString()} ticks`;
    $("progressTime").textContent = timeWithTenths(tick.t);
    $("progressPossession").textContent = `POS #${tick.possession_id ?? "—"}`;
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
    const names = eventNames(tick);
    const chip = $("eventChip");
    const overlay = $("eventOverlayChip");
    if (names.length) {
      const txt = names.join(" · ");
      if (chip) {
        chip.textContent = txt;
        chip.classList.add("active");
      }
      if (overlay) {
        overlay.textContent = txt;
        overlay.classList.add("active");
      }
    } else {
      if (chip) chip.classList.remove("active");
      if (overlay) overlay.classList.remove("active");
    }
    $("possessionLabel").textContent = `POS #${tick.possession_id ?? "—"}`;
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
    $("periodLabel").textContent = `Q${tick.period || 1}`;
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
    const ctx = canvas.getContext("2d");
    const rules = runtimeRules(tick);
    const point = (x, y) => ({
      x: 10 + x * (940 / rules.courtWidth),
      y: 10 + y * (500 / rules.courtHeight),
    });
    const leftHoopX = rules.leftHoopX;
    const rightHoopX = rules.rightHoopX;
    const hoopY = rules.hoopY;
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    const background = ctx.createLinearGradient(
      0,
      0,
      canvas.width,
      canvas.height,
    );
    background.addColorStop(0, "#dfcca6");
    background.addColorStop(0.5, "#ceba92");
    background.addColorStop(1, "#d8c39e");
    ctx.fillStyle = background;
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = "rgba(255,255,255,.05)";
    ctx.fillRect(10, 10, 940, 500);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.88)";
    ctx.lineWidth = 1.5;
    ctx.strokeRect(10, 10, 940, 500);
    ctx.beginPath();
    ctx.moveTo(480, 10);
    ctx.lineTo(480, 510);
    ctx.stroke();
    ctx.beginPath();
    ctx.arc(480, 260, 60, 0, Math.PI * 2);
    ctx.stroke();
    drawKey(ctx, false, rules);
    drawKey(ctx, true, rules);
    drawThreePointLine(ctx, false, rules);
    drawThreePointLine(ctx, true, rules);
    drawHoop(ctx, point(leftHoopX, hoopY), false);
    drawHoop(ctx, point(rightHoopX, hoopY), true);
    ctx.setLineDash([4, 4]);
    ctx.strokeStyle = "rgba(247,239,214,.32)";
    ctx.beginPath();
    ctx.moveTo(10, 40);
    ctx.lineTo(950, 40);
    ctx.moveTo(10, 480);
    ctx.lineTo(950, 480);
    ctx.stroke();
    ctx.setLineDash([]);
    drawTrails(ctx, point);
    // C6.5：命中列表每帧重建（原实现只 push 从不重置，长时间播放无界增长）。
    state.hitPlayers.length = 0;
    for (const player of tick.players || []) {
      if (player.onCourt === false) continue;
      const playerPoint = point(
        finite(player.x) * rules.courtWidth,
        finite(player.y) * rules.courtHeight,
      );
      state.hitPlayers.push({ player, x: playerPoint.x, y: playerPoint.y });

      // 2K 风格战术路线与目标站位标识（Play-art route & spacing spot）
      if (
        player.target_x !== undefined &&
        player.target_x !== null &&
        player.target_y !== undefined &&
        player.target_y !== null
      ) {
        const targetPt = point(
          finite(player.target_x) * rules.courtWidth,
          finite(player.target_y) * rules.courtHeight,
        );
        const distToTarget = Math.hypot(
          targetPt.x - playerPoint.x,
          targetPt.y - playerPoint.y,
        );
        if (distToTarget > 6) {
          ctx.save();
          // 1. 战术跑位虚线/箭头
          ctx.beginPath();
          ctx.moveTo(playerPoint.x, playerPoint.y);
          ctx.lineTo(targetPt.x, targetPt.y);
          ctx.strokeStyle =
            player.team === "home"
              ? "rgba(44, 229, 155, 0.35)"
              : "rgba(245, 189, 69, 0.35)";
          ctx.lineWidth = 1.6;
          ctx.setLineDash([4, 4]);
          ctx.stroke();
          ctx.setLineDash([]);

          // 2. 目标落位点（2K 圆环准星）
          ctx.beginPath();
          ctx.arc(targetPt.x, targetPt.y, 7, 0, Math.PI * 2);
          ctx.strokeStyle =
            player.team === "home"
              ? "rgba(44, 229, 155, 0.6)"
              : "rgba(245, 189, 69, 0.6)";
          ctx.lineWidth = 1.4;
          ctx.stroke();

          // 3. 槽位缩写标识
          if (player.slot) {
            const abbr =
              player.slot.replace(/([a-z])/g, "").slice(0, 3) ||
              player.slot.slice(0, 2);
            ctx.fillStyle =
              player.team === "home"
                ? "rgba(44, 229, 155, 0.75)"
                : "rgba(245, 189, 69, 0.75)";
            ctx.font = "600 7px IBM Plex Mono, monospace";
            ctx.textAlign = "center";
            ctx.textBaseline = "middle";
            ctx.fillText(abbr, targetPt.x, targetPt.y);
          }
          ctx.restore();
        }
      }
      const color = player.team === "home" ? "#2ce59b" : "#f5bd45";
      const dark = player.team === "home" ? "#087b57" : "#a96f15";
      const radius = 17;
      const action = String(player.action || "").toUpperCase();
      const isShooting =
        action.includes("SHOT") ||
        action.includes("JUMP") ||
        action.includes("PULLUP") ||
        action.includes("HOOK") ||
        action.includes("STEPBACK");
      const isDriving =
        action.includes("DRIVE") ||
        action.includes("LAYUP") ||
        action.includes("DUNK") ||
        action.includes("FLOATER") ||
        action.includes("PENETRATE");
      const isPassing = action.includes("PASS");
      const isContesting =
        action.includes("CONTEST") ||
        action.includes("BLOCK") ||
        action.includes("CLOSEOUT");
      const isScreening = action.includes("SCREEN");
      const isBoxOut = action.includes("BOX_OUT") || action.includes("BOXOUT");
      const isPostUp = action.includes("POST");
      const isTripleThreat = action.includes("TRIPLE");
      const isTakeCharge = action.includes("CHARGE");
      const isDive = action.includes("DIVE");
      const isSlide = action.includes("SLIDE") || action.includes("PRESSURE");
      const isSprinting =
        Math.hypot(player.vx || 0, player.vy || 0) > 12.0 ||
        action.includes("FAST") ||
        action.includes("TRANSITION") ||
        action.includes("CUT") ||
        action.includes("CRASH");

      // 动作1：投篮/起跳与滞空光环脉冲
      if (isShooting) {
        ctx.beginPath();
        ctx.arc(
          playerPoint.x,
          playerPoint.y,
          radius + 11 + Math.sin(finite(tick.t) * 16) * 3,
          0,
          Math.PI * 2,
        );
        ctx.strokeStyle = "rgba(255, 105, 50, 0.88)";
        ctx.lineWidth = 3;
        ctx.setLineDash([4, 2]);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      // 动作2：突破/攻筐/暴扣动量尾羽
      if (isDriving) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 8, 0, Math.PI * 2);
        ctx.strokeStyle = "#ff3b30";
        ctx.lineWidth = 2.5;
        ctx.stroke();
        const vx = Number(player.vx || player.target_x - player.x || 0);
        const vy = Number(player.vy || player.target_y - player.y || 0);
        if (Math.hypot(vx, vy) > 0.1) {
          const angle = Math.atan2(vy, vx);
          ctx.beginPath();
          ctx.moveTo(
            playerPoint.x - Math.cos(angle) * (radius + 2),
            playerPoint.y - Math.sin(angle) * (radius + 2),
          );
          ctx.lineTo(
            playerPoint.x - Math.cos(angle) * (radius + 20),
            playerPoint.y - Math.sin(angle) * (radius + 20),
          );
          ctx.strokeStyle = "rgba(255, 69, 58, 0.75)";
          ctx.lineWidth = 3;
          ctx.stroke();
        }
      }
      // 动作3：掩护设立（方形刚体防御屏障）
      if (isScreening) {
        ctx.save();
        ctx.strokeStyle = "rgba(255, 214, 10, 0.9)";
        ctx.lineWidth = 2.4;
        ctx.strokeRect(
          playerPoint.x - radius - 5,
          playerPoint.y - radius - 5,
          (radius + 5) * 2,
          (radius + 5) * 2,
        );
        ctx.restore();
      }
      // 动作4：篮下卡位推搡（后方背身弧面波纹）
      if (isBoxOut) {
        ctx.save();
        const facingAngle = Math.atan2(
          Number(player.facing_y || 0),
          Number(player.facing_x || 1),
        );
        const backAngle = facingAngle + Math.PI;
        ctx.beginPath();
        ctx.arc(
          playerPoint.x,
          playerPoint.y,
          radius + 8,
          backAngle - Math.PI / 3,
          backAngle + Math.PI / 3,
        );
        ctx.strokeStyle = "rgba(255, 149, 0, 0.85)";
        ctx.lineWidth = 3.5;
        ctx.stroke();
        ctx.restore();
      }
      // 动作5：低位背身单打（背身推搡盾形）
      if (isPostUp) {
        ctx.save();
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 6, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(175, 82, 222, 0.85)";
        ctx.lineWidth = 2.2;
        ctx.setLineDash([3, 3]);
        ctx.stroke();
        ctx.setLineDash([]);
        ctx.restore();
      }
      // 动作6：三威胁试探步（金色专注环与脚步探针）
      if (isTripleThreat) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 5, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(255, 214, 10, 0.75)";
        ctx.lineWidth = 2.0;
        ctx.setLineDash([2, 3]);
        ctx.stroke();
        ctx.setLineDash([]);
      }
      // 动作7：外线防守滑步与造撞人
      if (isSlide || isTakeCharge) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 7, 0, Math.PI * 2);
        ctx.strokeStyle = isTakeCharge
          ? "rgba(255, 59, 48, 0.9)"
          : "rgba(88, 86, 214, 0.75)";
        ctx.lineWidth = 2.2;
        ctx.stroke();
      }
      // 动作8：扑防扬手起跳与封盖
      if (isContesting) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 9, 0, Math.PI * 2);
        ctx.strokeStyle = "#af52de";
        ctx.lineWidth = 2.5;
        ctx.stroke();
      }
      // 动作9：鱼跃扑地抢球
      if (isDive) {
        ctx.save();
        ctx.beginPath();
        ctx.ellipse(
          playerPoint.x,
          playerPoint.y,
          radius + 10,
          radius + 4,
          0,
          0,
          Math.PI * 2,
        );
        ctx.strokeStyle = "rgba(50, 215, 75, 0.85)";
        ctx.lineWidth = 2.0;
        ctx.stroke();
        ctx.restore();
      }
      // 动作10：传球与切入冲刺
      if (isPassing) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 6, 0, Math.PI * 2);
        ctx.strokeStyle = "#5ac8fa";
        ctx.lineWidth = 2;
        ctx.stroke();
      } else if (isSprinting && !isDriving) {
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, radius + 4, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(255, 255, 255, 0.4)";
        ctx.lineWidth = 1.5;
        ctx.stroke();
      }

      // 肢体朝向与探步投影矢量 (Facing / Jab Limb Vector)
      if (player.facing_x !== undefined && player.facing_y !== undefined) {
        const fx = Number(player.facing_x);
        const fy = Number(player.facing_y);
        if (Math.hypot(fx, fy) > 0.1) {
          const fAngle = Math.atan2(fy, fx);
          ctx.beginPath();
          ctx.moveTo(playerPoint.x, playerPoint.y);
          ctx.lineTo(
            playerPoint.x + Math.cos(fAngle) * (radius + 6),
            playerPoint.y + Math.sin(fAngle) * (radius + 6),
          );
          ctx.strokeStyle = "rgba(255, 255, 255, 0.85)";
          ctx.lineWidth = 2.0;
          ctx.stroke();
        }
      }
      if (player.hasBall) {
        // 持球人聚光脉冲能量环（一眼识别核心战术点）
        const nowSec = performance.now() / 280;
        const pulseR = radius + 5 + Math.sin(nowSec) * 2.2;
        const auraGrad = ctx.createRadialGradient(
          playerPoint.x,
          playerPoint.y,
          radius,
          playerPoint.x,
          playerPoint.y,
          pulseR + 5,
        );
        auraGrad.addColorStop(0, "rgba(245, 158, 11, 0.4)");
        auraGrad.addColorStop(1, "rgba(245, 158, 11, 0)");
        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, pulseR + 5, 0, Math.PI * 2);
        ctx.fillStyle = auraGrad;
        ctx.fill();

        ctx.beginPath();
        ctx.arc(playerPoint.x, playerPoint.y, pulseR, 0, Math.PI * 2);
        ctx.strokeStyle = "#f59e0b";
        ctx.lineWidth = 2.2;
        ctx.stroke();
      }
      ctx.beginPath();
      ctx.arc(playerPoint.x, playerPoint.y, radius, 0, Math.PI * 2);
      ctx.fillStyle = dark;
      ctx.fill();
      ctx.strokeStyle = color;
      ctx.lineWidth = 2.5;
      ctx.stroke();
      // 球衣号码（高对比度深底+清晰白色粗体）
      ctx.fillStyle = "#ffffff";
      ctx.font = "700 11px IBM Plex Mono, monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(player.jersey || "?", playerPoint.x, playerPoint.y + 0.5);
      // 球员角色战术标签（增加暗色胶囊半透明背景，彻底杜绝在木地板上字迹模糊辨识困难）
      const labelText = player.slot || player.action || "";
      if (labelText) {
        ctx.font = "600 9px IBM Plex Mono, -apple-system, sans-serif";
        const textWidth = ctx.measureText(labelText).width;
        const pillW = textWidth + 8;
        const pillH = 14;
        const pillX = playerPoint.x - pillW / 2;
        const pillY = playerPoint.y + radius + 5;
        ctx.fillStyle = "rgba(10, 18, 24, 0.85)";
        ctx.beginPath();
        ctx.roundRect(pillX, pillY, pillW, pillH, 3);
        ctx.fill();
        ctx.strokeStyle = "rgba(190, 215, 224, 0.35)";
        ctx.lineWidth = 0.8;
        ctx.stroke();
        ctx.fillStyle = player.team === "home" ? "#4ade80" : "#fbbf24";
        ctx.fillText(labelText, playerPoint.x, pillY + pillH / 2 + 0.5);
      }
      const stamina = clamp(
        finite(player.stm) / Math.max(1, finite(player.stmMax, 100)),
        0,
        1,
      );
      ctx.beginPath();
      ctx.arc(
        playerPoint.x,
        playerPoint.y,
        radius + 3,
        -Math.PI / 2,
        -Math.PI / 2 + stamina * Math.PI * 2,
      );
      ctx.strokeStyle = stamina > 0.45 ? "rgba(255,255,255,.72)" : "#ff6f7e";
      ctx.lineWidth = 2;
      ctx.stroke();
    }
    if (tick.ball) {
      const ballPoint = point(
        finite(tick.ball.x) * rules.courtWidth,
        finite(tick.ball.y) * rules.courtHeight,
      );
      const z = Math.max(0, finite(tick.ball.z));
      // 动态逼真地面阴影：高度低（触地）时阴影聚拢深黑，高度高时发散淡化
      const shadowAlpha = Math.max(0.12, 0.48 - z * 0.035);
      const shadowRx = Math.max(3.5, 6.0 + z * 0.35);
      const shadowRy = Math.max(1.8, 2.6 + z * 0.16);
      ctx.beginPath();
      ctx.ellipse(
        ballPoint.x,
        ballPoint.y,
        shadowRx,
        shadowRy,
        0,
        0,
        Math.PI * 2,
      );
      ctx.fillStyle = `rgba(15, 12, 8, ${shadowAlpha.toFixed(3)})`;
      ctx.fill();

      // 球体本体渲染（根据 3D 高度立体上浮）
      const ballRadius = Math.max(4.5, 5.4 + z * 0.08);
      const ballCenterY = ballPoint.y - z * 2.8;
      ctx.beginPath();
      ctx.arc(ballPoint.x, ballCenterY, ballRadius, 0, Math.PI * 2);
      const grad = ctx.createRadialGradient(
        ballPoint.x - ballRadius * 0.35,
        ballCenterY - ballRadius * 0.35,
        ballRadius * 0.1,
        ballPoint.x,
        ballCenterY,
        ballRadius,
      );
      grad.addColorStop(0, "#f58c42");
      grad.addColorStop(0.7, "#d45d1b");
      grad.addColorStop(1, "#8e3407");
      ctx.fillStyle = grad;
      ctx.fill();
      ctx.strokeStyle = "#4a1902";
      ctx.lineWidth = 0.9;
      ctx.stroke();

      // 篮球黑色接缝线（立体十字圆弧）
      ctx.beginPath();
      ctx.ellipse(
        ballPoint.x,
        ballCenterY,
        ballRadius * 0.85,
        ballRadius * 0.35,
        Math.PI / 4,
        0,
        Math.PI * 2,
      );
      ctx.strokeStyle = "rgba(45, 15, 2, 0.65)";
      ctx.lineWidth = 0.8;
      ctx.stroke();

      const hasScoreEvent =
        (tick.event || "").includes("MADE") ||
        (tick.event || "").includes("3PT") ||
        (tick.event || "").includes("2PT");
      if (hasScoreEvent) {
        const hoopX = tick.ball && tick.ball.x > 47 ? 890 : 70;
        ctx.beginPath();
        ctx.arc(hoopX, 260, 24, 0, Math.PI * 2);
        ctx.strokeStyle = "rgba(44, 229, 155, 0.75)";
        ctx.lineWidth = 3;
        ctx.stroke();
      }
    }
    const holderTeam = (tick.players || []).find(
      (player) => player.hasBall,
    )?.team;
    const attacksRight =
      (holderTeam ||
        state.possessionTeams.get(tick.possession_id) ||
        "home") === "home";
    const attackHoop = point(attacksRight ? rightHoopX : leftHoopX, hoopY);
    ctx.beginPath();
    ctx.arc(
      attackHoop.x,
      attackHoop.y,
      11 + Math.sin(finite(tick.t) * 4) * 2,
      0,
      Math.PI * 2,
    );
    ctx.strokeStyle = attacksRight
      ? "rgba(44,229,155,.72)"
      : "rgba(245,189,69,.72)";
    ctx.lineWidth = 2;
    ctx.stroke();
  }
  function drawHoop(ctx, center, right) {
    ctx.strokeStyle = "rgba(247,239,214,.9)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.arc(center.x, center.y, 7.5, 0, Math.PI * 2);
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(center.x + (right ? 12 : -12), center.y - 20);
    ctx.lineTo(center.x + (right ? 12 : -12), center.y + 20);
    ctx.stroke();
  }
  function drawKey(ctx, right, rules) {
    const hoopX = right
      ? rules.rightHoopX * 10 + 10
      : rules.leftHoopX * 10 + 10;
    const y = rules.hoopY * 10 + 10;
    const keyWidth = 190;
    const keyHeight = 190;
    ctx.strokeStyle = "rgba(247,239,214,.8)";
    ctx.lineWidth = 1.5;
    ctx.strokeRect(right ? 760 : 10, y - keyHeight / 2, keyWidth, keyHeight);
    ctx.beginPath();
    ctx.arc(
      hoopX,
      y,
      60,
      right ? Math.PI / 2 : -Math.PI / 2,
      right ? (3 * Math.PI) / 2 : Math.PI / 2,
    );
    ctx.stroke();
  }
  function drawThreePointLine(ctx, right, rules) {
    const hoopX = (right ? rules.rightHoopX : rules.leftHoopX) * 10 + 10;
    const hoopY = rules.hoopY * 10 + 10;
    const radius = rules.threePointDistance * 10;
    ctx.strokeStyle = "rgba(247,239,214,.72)";
    ctx.lineWidth = 1.2;
    ctx.beginPath();
    ctx.arc(
      hoopX,
      hoopY,
      radius,
      right ? Math.PI / 2 : -Math.PI / 2,
      right ? (3 * Math.PI) / 2 : Math.PI / 2,
    );
    ctx.stroke();
    ctx.beginPath();
    if (right) {
      ctx.moveTo(950, 40);
      ctx.lineTo(950, 480);
    } else {
      ctx.moveTo(10, 40);
      ctx.lineTo(10, 480);
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
