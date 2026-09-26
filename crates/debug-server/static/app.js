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
    rim_shot_distance_ft: 5,
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
    courtMode: "full",
    potentialFieldVisible: true,
    lastFrameJson: -1,
    courtFX: null,
    fxRafId: null,
  };

  window.__nbaDebug = state;
  window.__nbaDebugReady = true;
  function el(tag, className, ...parts) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    for (const part of parts) {
      if (part == null || part === false) continue;
      node.append(part.nodeType ? part : document.createTextNode(String(part)));
    }
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
    PASS_DROPPED: "传球脱手",
    BALL_POKED_LOOSE: "防守破坏球权",
    LOOSE_BALL_SECURED: "控制活球",
    SHOT_RELEASE: "投篮出手",
    SCORE: "进球得分",
    DRIVE_SCORE: "突破上篮得分",
    DRIVE_MISS: "突破终结未中",
    DRIVE_STOPPED: "突破被阻截",
    SHOT_MADE: "投篮命中",
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
    FREE_THROW_MADE: "罚球命中",
    FREE_THROW_MISSED: "罚球未中",
    INBOUND_PASS: "发球入界",
    CONTROL_TRANSFER: "球权争夺",
  };
  function getEventNameZh(name) {
    return EVENT_NAME_ZH[name] || name;
  }

  const ACTION_ZH = {
    SPOT_UP_3PT: "定点三分",
    HighScreenRoll: "高位挡拆顺下",
    DRIVE_OFF_SCREEN: "借掩护突破",
    PERIMETER_CUT: "外线空切",
    SET_HIGH_SCREEN: "设立高位掩护",
    ROLL_TO_RIM: "顺下攻筐",
    ROTATE_RIM_HELP: "轮转护筐",
    X_OUT_CLOSEOUT: "交叉扑防",
    HELP_SIDE_SHELL: "弱侧协防",
    ON_BALL_CONTEST: "持球紧逼",
    DROP_CONTAIN: "沉退遏制",
    HEDGE_AND_RECOVER: "延误返位",
    SWITCH_ASSIGNMENT: "对位换防",
    LOOSE_BALL_RECOVERY: "拼抢活球",
    REBOUND_CRASH: "冲抢篮板",
    BOXOUT: "卡位保护",
    SetPosition: "站位就绪",
    TripleThreat: "三威胁准备",
    PostUp: "低位背打",
    JumpShot: "跳投出手",
    HookShot: "勾手投篮",
    Layup: "突破上篮",
    Dunk: "大力扣篮",
    Floater: "骑马射箭抛投",
    Bench: "替补席待命",
    CatchAndShoot: "接球就投",
    DriveAndKick: "突破分球",
    Inbounder: "发球球员",
  };
  function getActionZh(action) {
    return ACTION_ZH[action] || action || "—";
  }

  const SLOT_ZH = {
    top: "弧顶持球位",
    screener: "高位掩护位",
    weak_wing: "弱侧侧翼",
    strong_corner: "强侧底角",
    weak_corner: "弱侧底角",
    strong_wing: "强侧侧翼",
    paint: "禁区油漆区",
    rim: "篮下护筐位",
    on_ball: "领防对位",
    drop: "沉退防守位",
    weak_low: "弱侧低位",
    weak_high: "弱侧高位",
  };
  function getSlotZh(slot) {
    return SLOT_ZH[slot] || slot || "自由战术位";
  }

  const POSITION_ZH = {
    Point: "控卫",
    Combo: "双能卫",
    Wing: "侧翼",
    Forward: "锋线",
    Big: "内线",
    Center: "中锋",
  };
  function getPositionZh(position) {
    return POSITION_ZH[position] || position || "—";
  }

  const OFFENSIVE_ROLE_ZH = {
    PrimaryHandler: "主控",
    SecondaryHandler: "副控",
    ShotCreator: "持球得分手",
    Slasher: "突破手",
    AthleticFinisher: "空切终结者",
    OffScreenShooter: "绕掩护射手",
    StationaryShooter: "定点射手",
    VersatileBig: "多面手内线",
    PostScorer: "背身得分手",
    StretchBig: "空间型内线",
    RollCutBig: "顺下内线",
  };
  function getOffensiveRoleZh(role) {
    return OFFENSIVE_ROLE_ZH[role] || role || "—";
  }

  const DEFENSIVE_ROLE_ZH = {
    PointOfAttack: "领防人",
    Chaser: "追射手",
    Helper: "协防者",
    WingStopper: "侧翼锁编",
    MobileBig: "机动内线",
    AnchorBig: "护框中枢",
    LowActivity: "低活动量",
  };
  function getDefensiveRoleZh(role) {
    return DEFENSIVE_ROLE_ZH[role] || role || "—";
  }

  const PHASE_ZH = {
    OpeningTip: "开场跳球",
    HalfCourtOffense: "半场阵地战",
    TransitionOffense: "快速转换进攻",
    DeadBall: "停表阶段",
    FreeThrow: "执行罚球",
    Inbound: "边底线发球",
    PeriodBreak: "节间休息",
    GameOver: "比赛结束",
  };
  function getPhaseZh(phase) {
    return PHASE_ZH[phase] || String(phase || "—").replaceAll("_", " ");
  }

  const TACTICS_ZH = {
    HighScreenRoll: "高位挡拆体系",
    DriveKick: "突破分球体系",
    FiveOutMotion: "五外动态进攻体系",
    HornsSet: "牛角战术体系",
    TriangleOffense: "三角进攻体系",
    DropCoverage: "沉退护筐防守体系",
    SwitchAll: "无限换防体系",
    HedgeAndRecover: "延误返位防守体系",
  };
  function getTacticsZh(tactics) {
    return TACTICS_ZH[tactics] || tactics || "半场战术体系";
  }

  const MORALE_ZH = {
    Neutral: "平稳",
    Confident: "高昂",
    Frustrated: "受挫",
    Hot: "手感火热",
    Cold: "手感低迷",
  };
  function getMoraleZh(morale) {
    return MORALE_ZH[morale] || morale || "平稳";
  }

  const TEAM_ZH = {
    north_city_hawks: "老鹰",
    south_coast_celtics: "凯尔特人",
    hawks: "老鹰",
    celtics: "凯尔特人",
    Hawks: "老鹰",
    Celtics: "凯尔特人",
    lakers: "湖人",
    LAL: "湖人",
    BOS: "凯尔特人",
    home: "主队",
    away: "客队",
    HOME: "主队",
    AWAY: "客队",
  };
  function getTeamNameZh(team) {
    if (!team) return "—";
    const name = typeof team === "string" ? team : (team.short_name || team.name || "");
    return TEAM_ZH[name] || TEAM_ZH[team.id] || name || "—";
  }

  let studio = null;
  let selectedPlayerId = null;

  async function loadStudio() {
    let text;
    try {
      text = await fetchText("/api/studio");
    } catch {
      text = await fetchText("/studio.json");
    }
    try {
      studio = JSON.parse(text);
    } catch {
      studio = null;
    }
    selectedPlayerId = studio?.default_setup?.home_team?.players?.[0]?.id || null;
    renderStudio();
  }

  function label(group, key) {
    return studio?.labels?.[group]?.[key] || key;
  }

  function renderStudio() {
    if (!studio) return;
    renderBoard();
    renderRoster();
    syncScoreboardTactics();
  }

  function syncScoreboardTactics() {
    const setup = studio.default_setup;
    const away = studio.defense.find((item) => item.id === setup.away_lineup.defense_tactic);
    const home = studio.defense.find((item) => item.id === setup.home_lineup.defense_tactic);
    const awayTag = document.querySelector(".away-side .team-tactic-tag");
    if (awayTag) awayTag.textContent = away?.name_zh || "客队防守";
    const homeTag = document.querySelector(".home-side .team-tactic-tag");
    if (homeTag) homeTag.textContent = home?.name_zh || "主队防守";
  }

  function renderBoard() {
    const root = $("tacticBoard");
    root.replaceChildren(
      boardSide("away", studio.default_setup.away_team, studio.default_setup.away_lineup, studio.default_setup.away_playbook),
      boardSide("home", studio.default_setup.home_team, studio.default_setup.home_lineup, studio.default_setup.home_playbook),
    );
  }

  function boardSide(side, team, lineup, playbook) {
    const offense = studio.offense.find((item) => item.id === lineup.offense_tactic);
    const card = el("section", `studio-side ${side}`);
    card.append(
      el("div", "studio-head", el("strong", null, team.name), el("span", null, side === "home" ? "主队" : "客队")),
      el("div", "choice-meta", "进攻体系"),
      choiceRow(studio.offense, lineup.offense_tactic, (id) => {
        lineup.offense_tactic = id;
        const next = studio.offense.find((item) => item.id === id);
        const compatible = (studio.plays || []).filter((play) => playFits(play, next?.spec));
        if (side === "home") studio.default_setup.home_playbook = compatible;
        else studio.default_setup.away_playbook = compatible;
        renderStudio();
      }),
      courtMini(offense?.spec),
      el("div", "choice-meta", "防守体系"),
      choiceRow(studio.defense, lineup.defense_tactic, (id) => {
        lineup.defense_tactic = id;
        renderStudio();
      }),
      el("div", "choice-meta", "战术板"),
      playList(playbook),
    );
    return card;
  }

  function choiceRow(items, selected, onPick) {
    const row = el("div", "choice-row");
    for (const item of items) {
      const button = el(
        "button",
        `choice-card${item.id === selected ? " active" : ""}${item.available === false ? " disabled" : ""}`,
      );
      button.type = "button";
      button.disabled = item.available === false;
      button.append(el("strong", null, item.name_zh), el("span", "choice-meta", item.available === false ? "当前没有落位档案" : item.id));
      button.addEventListener("click", () => onPick(item.id));
      row.append(button);
    }
    return row;
  }

  function courtMini(spec) {
    const court = el("div", "court-mini");
    if (!spec) {
      court.append(el("span", "play-note", "这个体系目前只有名称，没有落位点。"));
      return court;
    }
    for (const slot of spec.slots) {
      const pin = el("div", "slot-pin", slot.name_zh);
      const x = Math.max(8, Math.min(92, (slot.base_offset_y / 50) * 100));
      const y = Math.max(12, Math.min(88, 100 - (slot.base_offset_x / 47) * 100));
      pin.style.left = x + "%";
      pin.style.top = y + "%";
      pin.title = slot.behaviour;
      court.append(pin);
    }
    return court;
  }

  function playList(playbook) {
    const list = el("div", "play-list");
    if (!playbook.length) {
      list.append(el("div", "play-note", "当前进攻体系没有可配合的战术。"));
      return list;
    }
    for (const play of playbook) {
      const card = el("article", "play-card active");
      const verbs = (play.rules || []).map((rule) => rule.then.verb + " · " + rule.then.slot).join(" / ");
      card.append(el("strong", null, play.name_zh), el("span", "play-note", verbs || play.id));
      list.append(card);
    }
    return list;
  }

  function playFits(play, spec) {
    if (!spec) return false;
    const slots = new Set(spec.slots.map((slot) => slot.id));
    return (play.rules || []).every((rule) => slots.has(rule.then.slot));
  }

  function renderRoster() {
    const root = $("rosterStudio");
    const teams = [studio.default_setup.home_team, studio.default_setup.away_team];
    const players = teams.flatMap((team) => team.players.map((player) => ({ team, player })));
    const selected = players.find((item) => item.player.id === selectedPlayerId) || players[0];
    const list = el("div", "player-list");
    for (const item of players) {
      const button = el("button", item.player.id === selected.player.id ? "active" : "");
      button.type = "button";
      button.append(
        el("strong", null, item.player.jersey + " " + item.player.name),
        el("span", "choice-meta", item.team.short_name + " · " + getPositionZh(item.player.position)),
      );
      button.addEventListener("click", () => {
        selectedPlayerId = item.player.id;
        renderRoster();
      });
      list.append(button);
    }
    root.replaceChildren(el("div", "roster-layout", list, playerSheet(selected.team, selected.player)));
  }

  function playerSheet(team, player) {
    const sheet = el("article", "player-sheet");
    const identity = el("div", "player-identity");
    identity.append(
      el("div", null, el("strong", null, player.name), el("div", "choice-meta", team.name + " · #" + player.jersey + " · " + player.height_cm + " cm · " + player.weight_kg + " kg")),
      el("div", "role-pills",
        el("span", null, getPositionZh(player.position)),
        el("span", null, getOffensiveRoleZh(player.offensive_role)),
        el("span", null, getDefensiveRoleZh(player.defensive_role)),
        el("span", null, player.starter ? "首发" : "替补"),
      ),
    );
    sheet.append(
      identity,
      el("div", "choice-meta", "能力"),
      meters(player.attributes, "attributes"),
      el("div", "choice-meta", "倾向"),
      meters(player.tendencies, "tendencies"),
      el("div", "choice-meta", "球队风格"),
      traitGrid(team.team_traits),
    );
    return sheet;
  }

  function meters(values, group) {
    const list = el("div", "meter-list");
    for (const [key, value] of Object.entries(values)) {
      const ratio = Math.max(0, Math.min(1, Number(value)));
      const bar = el("i", null, el("b"));
      bar.firstChild.style.width = Math.round(ratio * 100) + "%";
      list.append(el("div", "meter", el("span", null, label(group, key)), bar, el("span", null, ratio.toFixed(2))));
    }
    return list;
  }

  function traitGrid(traits) {
    const grid = el("div", "trait-grid");
    for (const [key, value] of Object.entries(traits)) {
      const item = el("div");
      item.append(el("span", "choice-meta", label("traits", key)), el("strong", null, Number(value).toFixed(2)));
      grid.append(item);
    }
    return grid;
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
      if (!studio) await loadStudio();
      const wasm = await ensureWasm();
      let responseText;
      const setup = studio?.default_setup || null;
      if (wasm && wasm.simulateToNdjson) {
        let rulesJson = null;
        if (withRules) {
          rulesJson = JSON.stringify(withRules);
        } else if (state.rules) {
          rulesJson = JSON.stringify(state.rules);
        }
        responseText = wasm.simulateToNdjson(
          BigInt(seed),
          scope,
          rulesJson,
          setup ? JSON.stringify(setup) : null,
        );
      } else {
        responseText = await fetchText("/api/simulate", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            seed,
            scope,
            rules: withRules || state.rules || null,
            setup,
          }),
        });
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
        return `阶段流转 → ${getPhaseZh(tick.phase)}`;
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
        return tick.callout || getPhaseZh(tick.phase) || "";
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
    if (badge) {
      const textSpan = badge.querySelector(".anomaly-text");
      const label = count ? `● ${count} 违规` : "● 0 违规";
      if (textSpan) textSpan.textContent = label;
      else badge.textContent = label;
      badge.className = `anomaly-pill-badge ${count > 10 ? "anomaly-danger" : count ? "anomaly-warn" : "anomaly-ok"}`;
    }
    // DOM API 构建（textContent 赋值，无 HTML 拼接）。
    const list = $("anomalyList");
    if (!list) return;
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
  }
  function updateReadouts(source) {
    $("eventReadout").textContent = `共 ${state.events.length} 条事件`;
    const normalizedSource = String(source || "")
      .replace("seed", "种子")
      .replace("5p", "5 回合")
      .replace("1p", "1 回合")
      .replace("10p", "10 回合")
      .replace("1q", "1 单节")
      .replace("full", "全场 48 分钟");
    $("streamSummary").textContent =
      `${normalizedSource} · ${state.possessions.length} 回合 · ${state.shots.length} 次投篮`;
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
    if ($("frameLabel")) $("frameLabel").textContent = `第 ${state.idx} 帧`;
    $("homeTeamName").textContent = getTeamNameZh(homeTeam);
    $("awayTeamName").textContent = getTeamNameZh(awayTeam);
    $("tacticalSet").textContent = getTacticsZh(tick.tactical_set);
    $("phaseLabel").textContent = getPhaseZh(tick.phase);
    const allNames = eventNames(tick);
    const highlightNames = allNames.filter((name) =>
      HIGHLIGHT_EVENTS.has(name),
    );
    const chip = $("eventChip");
    const overlay = $("eventOverlayChip");
    if (highlightNames.length) {
      const txt = highlightNames.map(getEventNameZh).join(" · ");
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
    const flowText = String(tick.game_flow || "LIVE");
    $("flowLabel").textContent = /Dead|Free|Quarter|Half|GameEnd/.test(flowText)
      ? "鸣哨停表"
      : "活球推进";
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

  // ========================================================
  // 球场视觉动效与动态物理渲染系统 (CourtVisualFXManager)
  // ========================================================
  class CourtVisualFXManager {
    constructor() {
      this.effects = [];
      this.processedKeys = new Set();
      this.lastProcessedIdx = -1;
      this.ballHistory = []; // { x, y, z, ftX, ftY, t, speed }
      this.playerHistories = new Map(); // id -> [{ x, y, vx, vy, speed }]
      this.rimStates = {
        left: { shake: 0, netOffset: 0, netVel: 0 },
        right: { shake: 0, netOffset: 0, netVel: 0 },
      };
      this.activeHalfCourt = "right";
      this.ballRotationAngle = 0;
      this.lastTickTime = performance.now();
    }

    reset() {
      this.effects = [];
      this.processedKeys.clear();
      this.lastProcessedIdx = -1;
      this.ballHistory = [];
      this.playerHistories.clear();
      this.rimStates.left = { shake: 0, netOffset: 0, netVel: 0, backboardLight: 0 };
      this.rimStates.right = { shake: 0, netOffset: 0, netVel: 0, backboardLight: 0 };
      this.ballRotationAngle = 0;
    }

    update(tick, frameIdx, rules) {
      if (!tick) return;
      const now = performance.now();
      const dt = Math.min(0.1, Math.max(0.01, (now - this.lastTickTime) / 1000));
      this.lastTickTime = now;

      // 跨帧跳变时清理过期特效
      if (Math.abs(frameIdx - this.lastProcessedIdx) > 6) {
        this.processedKeys.clear();
        this.effects = [];
        this.ballHistory = [];
        this.rimStates.left = { shake: 0, netOffset: 0, netVel: 0, backboardLight: 0 };
        this.rimStates.right = { shake: 0, netOffset: 0, netVel: 0, backboardLight: 0 };
      }
      this.lastProcessedIdx = frameIdx;

      // 1. 动态半场追踪防抖更新
      const ballNormX = finite(tick.ball?.x, 0.5);
      if (ballNormX > 0.55) {
        this.activeHalfCourt = "right";
      } else if (ballNormX < 0.45) {
        this.activeHalfCourt = "left";
      }

      // 2. 篮球轨迹与运动学统计
      if (tick.ball) {
        const bx = finite(tick.ball.x) * rules.courtWidth;
        const by = finite(tick.ball.y) * rules.courtHeight;
        const bz = finite(tick.ball.z, 0);
        const prevBall = this.ballHistory[this.ballHistory.length - 1];
        let speed = 0;
        if (prevBall) {
          const dx = bx - prevBall.x;
          const dy = by - prevBall.y;
          speed = Math.hypot(dx, dy) / Math.max(0.01, rules.tickSeconds);
          this.ballRotationAngle += Math.hypot(dx, dy) * 0.4;
        }
        this.ballHistory.push({
          x: bx,
          y: by,
          z: bz,
          speed,
          t: now,
          frame: frameIdx,
        });
        if (this.ballHistory.length > 16) this.ballHistory.shift();

        // 运球触地扩散波纹检测
        if (
          prevBall &&
          prevBall.z >= 1.2 &&
          bz < 1.0 &&
          tick.ball.status !== "Flying"
        ) {
          this.effects.push({
            type: "ground_ripple",
            x: bx,
            y: by,
            color: "#f59e0b",
            startR: 4,
            endR: 16,
            duration: 450,
            startTime: now,
            startFrame: frameIdx,
          });
        }
      }

      // 3. 球员轨迹与运动学统计
      for (const player of tick.players || []) {
        if (player.onCourt === false) continue;
        const px = finite(player.x) * rules.courtWidth;
        const py = finite(player.y) * rules.courtHeight;
        let hist = this.playerHistories.get(player.id);
        if (!hist) {
          hist = [];
          this.playerHistories.set(player.id, hist);
        }
        let vx = 0;
        let vy = 0;
        let speed = 0;
        const prev = hist[hist.length - 1];
        if (prev) {
          vx = px - prev.x;
          vy = py - prev.y;
          speed = Math.hypot(vx, vy) / Math.max(0.01, rules.tickSeconds);
        }
        hist.push({ x: px, y: py, vx, vy, speed, frame: frameIdx });
        if (hist.length > 8) hist.shift();
      }

      // 4. 事件扫描与动效激发
      const events = eventNames(tick);
      for (const evtName of events) {
        const key = `${frameIdx}_${evtName}`;
        if (this.processedKeys.has(key)) continue;
        this.processedKeys.add(key);

        this.triggerEventFX(evtName, tick, frameIdx, rules, now);
      }

      // 5. 篮筐物理动力学更新（Net Swish & Rim Shake & Backboard Light）
      for (const side of ["left", "right"]) {
        const rim = this.rimStates[side];
        if (rim.shake > 0.01) {
          rim.shake *= 0.86;
        } else {
          rim.shake = 0;
        }
        if (rim.backboardLight > 0.01) {
          rim.backboardLight *= 0.88;
        } else {
          rim.backboardLight = 0;
        }
        if (rim.netOffset > 0.1 || Math.abs(rim.netVel) > 0.1) {
          const spring = -28.0 * rim.netOffset;
          const damping = -4.8 * rim.netVel;
          rim.netVel += (spring + damping) * dt;
          rim.netOffset += rim.netVel * dt;
          if (rim.netOffset < 0) rim.netOffset = 0;
        } else {
          rim.netOffset = 0;
          rim.netVel = 0;
        }
      }

      // 6. 清理生命周期结束的特效
      this.effects = this.effects.filter((fx) => {
        const age = now - fx.startTime;
        return age < fx.duration;
      });
    }

    triggerEventFX(evtName, tick, frameIdx, rules, now) {
      const ballFtX = tick.ball
        ? finite(tick.ball.x) * rules.courtWidth
        : rules.courtWidth * 0.5;
      const ballFtY = tick.ball
        ? finite(tick.ball.y) * rules.courtHeight
        : rules.courtHeight * 0.5;

      const isAttackingHome =
        tick.possession_team === "home" ||
        (tick.ball ? tick.ball.x > 0.45 : true);
      const targetHoopX = isAttackingHome ? rules.rightHoopX : rules.leftHoopX;
      const targetSide = isAttackingHome ? "right" : "left";

      if (evtName === "SHOT_RELEASE") {
        let shooter = (tick.players || []).find((p) => p.hasBall);
        if (!shooter && tick.ball) {
          let minDist = Infinity;
          for (const p of tick.players || []) {
            const d = Math.hypot(
              finite(p.x) * rules.courtWidth - ballFtX,
              finite(p.y) * rules.courtHeight - ballFtY,
            );
            if (d < minDist) {
              minDist = d;
              shooter = p;
            }
          }
        }
        const sx = tick.ball
          ? ballFtX
          : shooter
            ? finite(shooter.x) * rules.courtWidth
            : ballFtX;
        const sy = tick.ball
          ? ballFtY
          : shooter
            ? finite(shooter.y) * rules.courtHeight
            : ballFtY;
        const distToHoop = Math.hypot(sx - targetHoopX, sy - rules.hoopY);
        const isThree = distToHoop >= rules.threePointDistance;

        // 真实战术分析投篮飞行路线 (严格指向目标篮筐几何圆心)
        this.effects.push({
          type: "shot_arc",
          startX: sx,
          startY: sy,
          targetX: targetHoopX,
          targetY: rules.hoopY,
          isThree,
          duration: 1300,
          startTime: now,
          startFrame: frameIdx,
        });
      } else if (
        evtName === "SHOT_MADE" ||
        evtName === "SCORE" ||
        evtName === "DRIVE_SCORE"
      ) {
        // 进球得分强反馈：白色编织篮网大幅下抽激荡，篮板四周瞬间点亮绿色得分确认灯框
        const rim = this.rimStates[targetSide];
        rim.netOffset = 36;
        rim.netVel = 52;
        rim.backboardLight = 1.0;

        // 投篮弧线瞬间转为鲜明实线绿弧，清晰指引空心穿网
        const lastArc = [...this.effects].reverse().find((e) => e.type === "shot_arc" && !e.status);
        if (lastArc) lastArc.status = "made";

      } else if (evtName === "SHOT_MISS" || evtName === "DRIVE_MISS") {
        // 投篮打铁强反馈：加厚金属篮圈机械高频阻尼震颤，篮网不动，篮板绝不亮灯
        const rim = this.rimStates[targetSide];
        rim.shake = 1.6;
        rim.backboardLight = 0;

        // 投篮弧线转为暗红虚线迅速消散
        const lastArc = [...this.effects].reverse().find((e) => e.type === "shot_arc" && !e.status);
        if (lastArc) lastArc.status = "missed";
      }
    }

    // 绘制地面真实接触物理痕迹 (急停刹车抓地印)
    drawGroundFX(ctx, point, now) {
      for (const fx of this.effects) {
        const progress = Math.min(1.0, (now - fx.startTime) / fx.duration);
        if (progress >= 1.0) continue;
        const alpha = (1.0 - progress) * 0.35;

        if (fx.type === "floor_skid") {
          const pt = point(fx.x, fx.y);
          ctx.save();
          ctx.translate(pt.x, pt.y);
          ctx.rotate(fx.angle);
          ctx.fillStyle = `rgba(30, 25, 20, ${alpha.toFixed(3)})`;
          ctx.fillRect(-6, -2, 12, 1.6);
          ctx.fillRect(-6, 2, 12, 1.6);
          ctx.restore();
        }
      }
    }

    // 绘制空中真实物理轨迹 (极细弱战术抛物线)
    drawAirFX(ctx, point, now) {
      for (const fx of this.effects) {
        const progress = Math.min(1.0, (now - fx.startTime) / fx.duration);
        if (progress >= 1.0) continue;
        const alpha = Math.min(0.4, (1.0 - progress) * 0.6);

        if (fx.type === "shot_arc") {
          const startPt = point(fx.startX, fx.startY);
          const targetPt = point(fx.targetX, fx.targetY);

          ctx.save();
          if (fx.status === "made") {
            // 进球命中：鲜明实线绿导轨，空心穿网路径极具辨识度
            ctx.strokeStyle = `rgba(16, 185, 129, ${Math.min(0.95, alpha * 2.2).toFixed(3)})`;
            ctx.lineWidth = 2.4;
            ctx.setLineDash([]);
          } else if (fx.status === "missed") {
            // 打铁未中：暗红虚线并迅速消散
            ctx.strokeStyle = `rgba(239, 68, 68, ${Math.min(0.55, alpha * 1.2).toFixed(3)})`;
            ctx.lineWidth = 1.0;
            ctx.setLineDash([3, 4]);
          } else {
            // 飞行中：极弱纯白战术瞄准虚线
            ctx.strokeStyle = `rgba(255, 255, 255, ${alpha.toFixed(3)})`;
            ctx.lineWidth = 1.2;
            ctx.setLineDash([4, 4]);
          }
          ctx.beginPath();
          ctx.moveTo(startPt.x, startPt.y);
          ctx.lineTo(targetPt.x, targetPt.y);
          ctx.stroke();
          ctx.restore();
        } else if (fx.type === "disabled_block_shield") {
          ctx.moveTo(pt.x, pt.y - r);
          ctx.lineTo(pt.x, pt.y + r);
          ctx.strokeStyle = "rgba(56, 189, 248, 0.6)";
          ctx.lineWidth = 1.5;
          ctx.stroke();
          ctx.restore();
        } else if (fx.type === "steal_lightning") {
          // 抢断金黄电光闪现
          const pt = point(fx.x, fx.y);
          ctx.save();
          ctx.strokeStyle = "#fbbf24";
          ctx.lineWidth = 2.5;
          ctx.globalAlpha = alpha;
          ctx.beginPath();
          ctx.moveTo(pt.x - 14, pt.y - 14);
          ctx.lineTo(pt.x - 2, pt.y - 1);
          ctx.lineTo(pt.x + 3, pt.y - 10);
          ctx.lineTo(pt.x + 15, pt.y + 12);
          ctx.stroke();

          ctx.beginPath();
          ctx.arc(pt.x, pt.y, 6 + progress * 24, 0, Math.PI * 2);
          ctx.strokeStyle = "rgba(251, 191, 36, 0.7)";
          ctx.lineWidth = 1.8;
          ctx.stroke();
          ctx.restore();
        } else if (fx.type === "particles") {
          // 爆裂礼花粒子更新
          ctx.save();
          for (const p of fx.particles) {
            const age = (now - fx.startTime) / 1000;
            const px = p.x + p.vx * age;
            const py = p.y + p.vy * age + 15 * age * age; // 轻微重力
            const pPt = point(px, py);
            ctx.beginPath();
            ctx.arc(pPt.x, pPt.y, Math.max(0.5, p.size * (1.0 - progress)), 0, Math.PI * 2);
            ctx.fillStyle = p.color;
            ctx.globalAlpha = alpha * 0.9;
            ctx.fill();
          }
          ctx.restore();
        }
      }
    }
  }

  const courtFX = new CourtVisualFXManager();
  state.courtFX = courtFX;

  // ========================================================
  // 核心球场绘制函数 (drawCourt)
  // ========================================================
  function drawCourt(tick) {
    const canvas = $("courtCanvas");
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const rules = runtimeRules(tick);
    const now = performance.now();

    // 更新动效状态机
    courtFX.update(tick, state.idx, rules);

    // 标准 NBA 全场物理世界等比例严密映射
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

    // 屏幕物理像素映射转换（全场恒定标准展示）
    const toScreen = (courtX, courtY) => ({ x: courtX, y: courtY });

    // 1. 赛场外围环带 (Arena Apron / Perimeter)
    ctx.fillStyle = "#0a0d12";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // 底线外侧主客队文字 (球队规范全称)
    ctx.save();
    ctx.font = "800 13px 'Plus Jakarta Sans', system-ui, -apple-system, sans-serif";
    ctx.textAlign = "center";
    ctx.textBaseline = "middle";

    const awayTeamName = getTeamNameZh(tick.away_team) || "客队";
    const homeTeamName = getTeamNameZh(tick.home_team) || "主队";

    // 左侧底线外客队字样
    ctx.save();
    ctx.translate(15, 280);
    ctx.rotate(-Math.PI / 2);
    ctx.fillStyle = "rgba(245, 158, 11, 0.65)";
    ctx.fillText(awayTeamName, 0, 0);
    ctx.restore();

    // 右侧底线外主队字样
    ctx.save();
    ctx.translate(985, 280);
    ctx.rotate(Math.PI / 2);
    ctx.fillStyle = "rgba(16, 185, 129, 0.65)";
    ctx.fillText(homeTeamName, 0, 0);
    ctx.restore();
    ctx.restore();

    // 2. 比赛主场地高级浅色枫木地板 (Playing Surface: 940 x 500)
    const floorGrad = ctx.createLinearGradient(30, 30, 970, 530);
    floorGrad.addColorStop(0, "#dfcca6");
    floorGrad.addColorStop(0.5, "#d2be97");
    floorGrad.addColorStop(1, "#dac59f");
    ctx.fillStyle = floorGrad;
    ctx.fillRect(30, 30, 940, 500);

    // 枫木拼板纵向缝隙微纹
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

    // 中圈 (Center Circle, NBA 标准半径 6 ft = 60px)
    ctx.beginPath();
    ctx.arc(500, 280, 60, 0, Math.PI * 2);
    ctx.stroke();

    // 绘制标准禁区、三分线与篮筐
    drawKey(ctx, false);
    drawKey(ctx, true);
    drawThreePointLine(ctx, false);
    drawThreePointLine(ctx, true);
    drawEnhancedHoop(
      ctx,
      30 + rules.leftHoopX * 10,
      280,
      false,
      courtFX.rimStates.left,
    );
    drawEnhancedHoop(
      ctx,
      30 + rules.rightHoopX * 10,
      280,
      true,
      courtFX.rimStates.right,
    );

    if (state.potentialFieldVisible) drawPotentialField(ctx, tick, point, rules);

    // 动感高级轨迹绘制
    drawTrails(ctx, point);

    // 动效系统：地面层动效渲染
    courtFX.drawGroundFX(ctx, point, now);

    // 5. 球员渲染
    state.hitPlayers.length = 0;
    const playerRadius = 16.5;

    for (const player of tick.players || []) {
      if (player.onCourt === false) continue;
      const playerPoint = point(
        finite(player.x) * rules.courtWidth,
        finite(player.y) * rules.courtHeight,
      );

      // 计算屏幕物理映射坐标用于鼠标检测
      const screenPt = toScreen(playerPoint.x, playerPoint.y);
      state.hitPlayers.push({ player, x: screenPt.x, y: screenPt.y });

      // 提取球员运动学历史
      const hist = courtFX.playerHistories.get(player.id) || [];
      const curSpeed = hist[hist.length - 1]?.speed || 0;
      const curVx = hist[hist.length - 1]?.vx || 0;
      const curVy = hist[hist.length - 1]?.vy || 0;

      // 战术路线 (Play-art route)
      if (
        state.potentialFieldVisible &&
        player.potential_target_x !== undefined &&
        player.potential_target_y !== undefined
      ) {
        const fieldTargetPt = point(
          finite(player.potential_target_x) * rules.courtWidth,
          finite(player.potential_target_y) * rules.courtHeight,
        );
        ctx.save();
        ctx.setLineDash([2, 5]);
        ctx.strokeStyle =
          player.team === "home"
            ? "rgba(0, 210, 255, 0.6)"
            : "rgba(255, 109, 171, 0.62)";
        ctx.lineWidth = 1.3;
        ctx.beginPath();
        ctx.moveTo(playerPoint.x, playerPoint.y);
        ctx.lineTo(fieldTargetPt.x, fieldTargetPt.y);
        ctx.stroke();
        ctx.fillStyle = ctx.strokeStyle;
        ctx.beginPath();
        ctx.arc(fieldTargetPt.x, fieldTargetPt.y, 4, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }

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

      // ========================================================
      // 5. 真实战术板球员渲染 (Pro Tactical Player Badge)
      // 脚踏实地自然跑位，绝不悬浮飞行，无花哨 AI 特效
      // ========================================================
      const isHome = player.team === "home";
      const px = playerPoint.x;
      const py = playerPoint.y;
      const radius = 14.5;

      // 1. 地面自然接触阴影 (自然紧贴脚下，柔和逼真)
      ctx.beginPath();
      ctx.ellipse(px, py + 2, radius * 0.95, radius * 0.45, 0, 0, Math.PI * 2);
      ctx.fillStyle = "rgba(0, 0, 0, 0.22)";
      ctx.fill();

      // 2. 身体朝向角 (Facing Angle)
      let facingAngle = 0;
      if (
        player.facing_x !== undefined &&
        player.facing_y !== undefined &&
        Math.hypot(player.facing_x, player.facing_y) > 0.05
      ) {
        facingAngle = Math.atan2(player.facing_y, player.facing_x);
      } else if (Math.hypot(curVx, curVy) > 0.3) {
        facingAngle = Math.atan2(curVy, curVx);
      } else {
        const hoopX = isHome ? rules.rightHoopX : rules.leftHoopX;
        facingAngle = Math.atan2(
          rules.hoopY - finite(player.y) * rules.courtHeight,
          hoopX - finite(player.x) * rules.courtWidth,
        );
      }

      // 3. 稳重专业的朝向指示微标 (极小等腰三角，长 4px，低调清晰)
      ctx.save();
      ctx.translate(px, py);
      ctx.rotate(facingAngle);
      ctx.beginPath();
      ctx.moveTo(radius + 4, 0);
      ctx.lineTo(radius, -3);
      ctx.lineTo(radius, 3);
      ctx.closePath();
      ctx.fillStyle = isHome ? "#10b981" : "#f59e0b";
      ctx.fill();
      ctx.restore();

      // 4. 战术圆盘徽章 (Pro Tactical Disc)
      // 主队：高级墨绿；客队：沉稳深琥珀；拒绝塑料感
      ctx.save();
      ctx.beginPath();
      ctx.arc(px, py, radius, 0, Math.PI * 2);
      ctx.fillStyle = isHome ? "#084925" : "#632704";
      ctx.fill();

      // 队色边框 (持球人加粗至 3.2px 醒目标识，绝无发光飞碟或白色虚浮框)
      ctx.strokeStyle = isHome ? "#10b981" : "#f59e0b";
      ctx.lineWidth = player.hasBall ? 3.2 : 2.0;
      ctx.stroke();

      // 持球人外围极细微自然提示环
      if (player.hasBall) {
        ctx.beginPath();
        ctx.arc(px, py, radius + 3.5, 0, Math.PI * 2);
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.6)" : "rgba(245, 158, 11, 0.6)";
        ctx.lineWidth = 1.2;
        ctx.stroke();
      }
      ctx.restore();

      // 5. 纯白清晰数字背号 (居中，高对比度)
      ctx.save();
      ctx.font = "700 11.5px monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const num =
        player.number === undefined
          ? String(player.id || "")
          : String(player.number);

      ctx.fillStyle = "rgba(0, 0, 0, 0.8)";
      ctx.fillText(num, px + 0.5, py + 1.0);
      ctx.fillStyle = "#ffffff";
      ctx.fillText(num, px, py + 0.5);
      ctx.restore();

      // 6. 脚下纯中文位置角色微标 (控卫 / 分卫 / 小前 / 大前 / 中锋)
      const posText = getPositionZh(player.position);
      ctx.save();
      const posTagY = py + 16.5;
      ctx.font = "700 8.5px system-ui, -apple-system, sans-serif";
      const posTagW = ctx.measureText(posText).width + 8;
      ctx.fillStyle = "rgba(12, 16, 26, 0.85)";
      ctx.beginPath();
      ctx.roundRect(px - posTagW / 2, posTagY - 4.5, posTagW, 11, 3.5);
      ctx.fill();
      ctx.fillStyle = isHome ? "#6ee7b7" : "#fde047";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(posText, px, posTagY + 0.5);
      ctx.restore();
    }

    // 6. 篮球渲染 (严格对齐真实俯视投影平面与三维透视物理)
    if (tick.ball) {
      const ballFtX = finite(tick.ball.x) * rules.courtWidth;
      const ballFtY = finite(tick.ball.y) * rules.courtHeight;
      const groundPoint = point(ballFtX, ballFtY);

      const ballZ = finite(tick.ball.z, 0);
      const zScale = 1.0 + Math.min(0.42, ballZ * 0.035);
      const ballRadius = 7.0 * zScale;
      const ballCenterX = groundPoint.x;
      const ballCenterY = groundPoint.y;

      // 地面物理投影阴影 (随着高度上升阴影扩散变淡)
      ctx.beginPath();
      const shadowSpread = 1.0 + Math.min(1.3, ballZ * 0.07);
      const shadowAlpha = Math.max(0.1, 0.45 * (1.0 - Math.min(0.75, ballZ * 0.04)));
      ctx.ellipse(
        groundPoint.x,
        groundPoint.y + 1,
        Math.max(3, ballRadius * 1.05 * shadowSpread),
        Math.max(2, ballRadius * 0.52 * shadowSpread),
        0,
        0,
        Math.PI * 2,
      );
      ctx.fillStyle = `rgba(0, 0, 0, ${shadowAlpha.toFixed(3)})`;
      ctx.fill();

      // 高速飞行金色流光彗星尾迹 (Comet Trail: 严格对齐球体同轴轨迹)
      if (courtFX.ballHistory.length >= 3) {
        const tailPts = courtFX.ballHistory.slice(-8);
        for (let i = 0; i < tailPts.length - 1; i++) {
          const p1 = tailPts[i];
          const p2 = tailPts[i + 1];
          const pt1 = point(p1.x, p1.y);
          const pt2 = point(p2.x, p2.y);
          const trailAlpha = ((i + 1) / tailPts.length) * 0.55;

          ctx.beginPath();
          ctx.moveTo(pt1.x, pt1.y);
          ctx.lineTo(pt2.x, pt2.y);
          ctx.strokeStyle = "rgba(249, 115, 22, " + trailAlpha.toFixed(3) + ")";
          ctx.lineWidth = 1.5 + (i / tailPts.length) * 3.0;
          ctx.lineCap = "round";
          ctx.stroke();
        }
      }

      // 篮球球体渐变与立体高光 (圆心分毫不差坐落于物理平面点)
      ctx.save();
      ctx.beginPath();
      ctx.arc(ballCenterX, ballCenterY, ballRadius, 0, Math.PI * 2);
      const bGrad = ctx.createRadialGradient(
        ballCenterX - ballRadius * 0.35,
        ballCenterY - ballRadius * 0.35,
        ballRadius * 0.1,
        ballCenterX,
        ballCenterY,
        ballRadius,
      );
      bGrad.addColorStop(0, "#fb923c");
      bGrad.addColorStop(0.65, "#ea580c");
      bGrad.addColorStop(1, "#7c2d12");
      ctx.fillStyle = bGrad;
      ctx.fill();

      // 篮球经典黑色十字旋转筋线 (Seams)
      ctx.save();
      ctx.translate(ballCenterX, ballCenterY);
      ctx.rotate(courtFX.ballRotationAngle);
      ctx.strokeStyle = "#381006";
      ctx.lineWidth = 1.1;

      // 横向弧线
      ctx.beginPath();
      ctx.ellipse(0, 0, ballRadius * 0.95, ballRadius * 0.45, 0, 0, Math.PI * 2);
      ctx.stroke();

      // 纵向线
      ctx.beginPath();
      ctx.moveTo(0, -ballRadius);
      ctx.lineTo(0, ballRadius);
      ctx.stroke();
      ctx.restore();

      // 球体边缘深色描边
      ctx.strokeStyle = "#431407";
      ctx.lineWidth = 1.2;
      ctx.stroke();
      ctx.restore();
    }

    // 动效系统：空中层纯视觉动效渲染
    courtFX.drawAirFX(ctx, point, now);

    ctx.restore();

    // 暂停状态下，若场上有正在消散的动效粒子与水花，以轻量帧循环平滑完成过渡
    if (!state.playing && courtFX.effects.length > 0) {
      if (!state.fxRafId) {
        state.fxRafId = requestAnimationFrame(() => {
          state.fxRafId = null;
          if (!state.playing && state.ticks && state.ticks[state.idx]) {
            drawCourt(state.ticks[state.idx]);
          }
        });
      }
    }
  }

  function drawPotentialField(ctx, tick, point, rules) {
    const samples = Array.isArray(tick.potential_field) ? tick.potential_field : [];
    const players = (tick.players || []).filter((p) => p.onCourt !== false);
    if (!samples.length && !players.length) return;

    ctx.save();
    ctx.globalCompositeOperation = "source-over";

    // 全场网格连续势能曲面
    const stepFt = 2.0;
    const nx = Math.ceil(rules.courtWidth / stepFt);
    const ny = Math.ceil(rules.courtHeight / stepFt);
    const sigma2 = 2 * 8.5 * 8.5;

    const homePlayers = [];
    const awayPlayers = [];
    for (const p of players) {
      const px = finite(p.x) * rules.courtWidth;
      const py = finite(p.y) * rules.courtHeight;
      if (p.team === "home") homePlayers.push({ x: px, y: py });
      else if (p.team === "away") awayPlayers.push({ x: px, y: py });
    }

    const solverSamples = samples.map((s) => ({
      x: finite(s.x) * rules.courtWidth,
      y: finite(s.y) * rules.courtHeight,
      targetX: finite(s.target_x) * rules.courtWidth,
      targetY: finite(s.target_y) * rules.courtHeight,
      pressure: clamp(finite(s.pressure), 0, 1),
      team: s.team,
    }));

    for (let ix = 0; ix < nx; ix++) {
      const gx = ix * stepFt;
      for (let iy = 0; iy < ny; iy++) {
        const gy = iy * stepFt;

        let uHome = 0;
        for (const hp of homePlayers) {
          const d2 = (gx - hp.x) * (gx - hp.x) + (gy - hp.y) * (gy - hp.y);
          uHome += Math.exp(-d2 / sigma2);
        }

        let uAway = 0;
        for (const ap of awayPlayers) {
          const d2 = (gx - ap.x) * (gx - ap.x) + (gy - ap.y) * (gy - ap.y);
          uAway += Math.exp(-d2 / sigma2);
        }

        for (const ss of solverSamples) {
          const d2 = (gx - ss.x) * (gx - ss.x) + (gy - ss.y) * (gy - ss.y);
          const weight = Math.exp(-d2 / (2 * 11 * 11)) * ss.pressure * 1.5;
          if (ss.team === "home") uHome += weight;
          else if (ss.team === "away") uAway += weight;
        }

        const total = uHome + uAway + 0.08;
        const dominance = (uHome - uAway) / total;
        const totalDensity = Math.min(1.0, uHome + uAway);

        if (Math.abs(dominance) > 0.06) {
          const cellPt = point(gx, gy);
          const cellNext = point(gx + stepFt, gy + stepFt);
          const w = cellNext.x - cellPt.x;
          const h = cellNext.y - cellPt.y;

          const alpha = Math.min(
            0.32,
            Math.abs(dominance) * 0.3 * (0.35 + totalDensity * 0.65),
          );
          if (dominance > 0) {
            ctx.fillStyle = `rgba(0, 210, 255, ${alpha.toFixed(3)})`;
          } else {
            ctx.fillStyle = `rgba(255, 80, 146, ${alpha.toFixed(3)})`;
          }
          ctx.fillRect(cellPt.x, cellPt.y, w + 0.5, h + 0.5);
        }
      }
    }
    ctx.restore();

    // 弱侧防守平衡驱动向量与目标锚点
    if (solverSamples.length) {
      ctx.save();
      ctx.lineCap = "round";
      ctx.setLineDash([]);
      for (const sample of solverSamples) {
        const start = point(sample.x, sample.y);
        const driveX = sample.targetX - sample.x;
        const driveY = sample.targetY - sample.y;
        const magnitude = Math.hypot(driveX, driveY);
        if (magnitude < 1) continue;
        const length = clamp(14 + magnitude * 0.55, 14, 55);
        const end = {
          x: start.x + (driveX / magnitude) * length,
          y: start.y + (driveY / magnitude) * length,
        };
        const color = sample.team === "home" ? "#00d2ff" : "#ff6dab";

        // 驱动箭头主体
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.55 + sample.pressure * 0.45;
        ctx.lineWidth = 1.8;
        ctx.beginPath();
        ctx.moveTo(start.x, start.y);
        ctx.lineTo(end.x, end.y);
        ctx.stroke();

        // 箭头尖端
        const angle = Math.atan2(end.y - start.y, end.x - start.x);
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(end.x, end.y);
        ctx.lineTo(
          end.x - Math.cos(angle - 0.5) * 6,
          end.y - Math.sin(angle - 0.5) * 6,
        );
        ctx.lineTo(
          end.x - Math.cos(angle + 0.5) * 6,
          end.y - Math.sin(angle + 0.5) * 6,
        );
        ctx.closePath();
        ctx.fill();

        // 平衡目标锚点
        const targetPt = point(sample.targetX, sample.targetY);
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(targetPt.x, targetPt.y, 4.5, 0, Math.PI * 2);
        ctx.fill();
      }
      ctx.restore();
    }
  }

  // ========================================================
  // 增强篮筐与动态晃动篮网绘制 (drawEnhancedHoop)
  // ========================================================
  function drawEnhancedHoop(ctx, hoopX, hoopY, right, rimState) {
    const shake = rimState?.shake || 0;
    const netOffset = rimState?.netOffset || 0;

    // 晃动偏置 (打铁与暴扣时震颤)
    const shakeX = shake > 0 ? (Math.sin(performance.now() * 0.08) * shake * 3.5) : 0;
    const shakeY = shake > 0 ? (Math.cos(performance.now() * 0.08) * shake * 2.5) : 0;

    const actualHoopX = hoopX + shakeX;
    const actualHoopY = hoopY + shakeY;

    // 篮板 (Backboard: 宽 60px, 厚 5px)
    const boardX = right ? 970 - 40 : 30 + 40;
    ctx.strokeStyle = "#0f172a";
    ctx.lineWidth = 5.0;
    ctx.beginPath();
    ctx.moveTo(boardX, hoopY - 30);
    ctx.lineTo(boardX, hoopY + 30);
    ctx.stroke();

    ctx.strokeStyle = "#ffffff";
    ctx.lineWidth = 3.0;
    ctx.beginPath();
    ctx.moveTo(boardX, hoopY - 30);
    ctx.lineTo(boardX, hoopY + 30);
    ctx.stroke();

    // 进球命中得分时，篮板四周瞬间点亮高对比度亮绿确认灯框 (Backboard Goal Flash)
    const goalLight = rimState?.backboardLight || 0;
    if (goalLight > 0.05) {
      ctx.save();
      ctx.strokeStyle = `rgba(16, 185, 129, ${(goalLight * 0.95).toFixed(3)})`;
      ctx.lineWidth = 5.0;
      ctx.beginPath();
      ctx.moveTo(boardX, hoopY - 32);
      ctx.lineTo(boardX, hoopY + 32);
      ctx.stroke();

      ctx.strokeStyle = `rgba(16, 185, 129, ${(goalLight * 0.9).toFixed(3)})`;
      ctx.lineWidth = 2.0;
      ctx.strokeRect(boardX - (right ? 1 : -1) * 3, hoopY - 11, right ? -3 : 3, 22);
      ctx.restore();
    }

    // 篮板内侧小方框 (Target Box: 24px x 18px)
    ctx.strokeStyle = "rgba(239, 68, 68, 0.75)";
    ctx.lineWidth = 1.5;
    ctx.strokeRect(boardX - (right ? 1 : -1) * 3, hoopY - 10, right ? -2 : 2, 20);

    // 支架 (Stanchion)
    ctx.strokeStyle = "rgba(255, 255, 255, 0.35)";
    ctx.lineWidth = 1.5;
    ctx.beginPath();
    ctx.moveTo(right ? 970 : 30, hoopY);
    ctx.lineTo(boardX, hoopY);
    ctx.stroke();

    // 篮圈连接杆 (Neck)
    ctx.strokeStyle = "#ea580c";
    ctx.lineWidth = 3.0;
    ctx.beginPath();
    ctx.moveTo(boardX, hoopY);
    ctx.lineTo(actualHoopX + (right ? 7.5 : -7.5), actualHoopY);
    ctx.stroke();

    // 动态摆动白色纤维篮网 (Net)
    ctx.save();
    ctx.strokeStyle = "rgba(255, 255, 255, 0.7)";
    ctx.lineWidth = 1.1;
    const netBottomY = actualHoopY + 18 + netOffset;
    const netWidth = 14;
    const netBottomWidth = Math.max(4, 9 - netOffset * 0.25);

    // 篮网垂直织线
    for (let i = -3; i <= 3; i++) {
      const topX = actualHoopX + (i / 3) * (netWidth / 2);
      const botX = actualHoopX + (i / 3) * (netBottomWidth / 2);
      ctx.beginPath();
      ctx.moveTo(topX, actualHoopY + 3);
      ctx.lineTo(botX, netBottomY);
      ctx.stroke();
    }
    // 篮网横向编织环
    ctx.beginPath();
    ctx.moveTo(actualHoopX - netWidth * 0.45, actualHoopY + 8 + netOffset * 0.35);
    ctx.lineTo(actualHoopX + netWidth * 0.45, actualHoopY + 8 + netOffset * 0.35);
    ctx.moveTo(actualHoopX - netBottomWidth * 0.7, actualHoopY + 14 + netOffset * 0.65);
    ctx.lineTo(actualHoopX + netBottomWidth * 0.7, actualHoopY + 14 + netOffset * 0.65);
    ctx.stroke();
    ctx.restore();

    // 加厚实心橙红篮圈 (Rim)
    ctx.save();
    ctx.beginPath();
    ctx.arc(actualHoopX, actualHoopY, 7.5, 0, Math.PI * 2);
    ctx.strokeStyle = "#ea580c";
    ctx.lineWidth = 2.8;
    ctx.stroke();

    // 篮圈内沿高光
    ctx.beginPath();
    ctx.arc(actualHoopX, actualHoopY, 6.5, 0, Math.PI * 2);
    ctx.strokeStyle = "#fdba74";
    ctx.lineWidth = 1.0;
    ctx.stroke();
    ctx.restore();
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
    const start = Math.max(0, state.idx - 40);
    const count = state.idx - start;
    if (count <= 1) return;

    for (let index = start; index < state.idx; index += 1) {
      const tick = state.ticks[index];
      const nextTick = state.ticks[index + 1];
      if (!tick?.ball || !nextTick?.ball) continue;
      const rules = runtimeRules(tick);

      const p1 = point(
        finite(tick.ball.x) * rules.courtWidth,
        finite(tick.ball.y) * rules.courtHeight,
      );
      const p2 = point(
        finite(nextTick.ball.x) * rules.courtWidth,
        finite(nextTick.ball.y) * rules.courtHeight,
      );

      const progress = (index - start) / count;
      ctx.beginPath();
      ctx.moveTo(p1.x, p1.y);
      ctx.lineTo(p2.x, p2.y);
      ctx.strokeStyle = `rgba(251, 191, 36, ${(progress * 0.45).toFixed(3)})`;
      ctx.lineWidth = 1.0 + progress * 2.2;
      ctx.stroke();
    }
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
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const rules = runtimeRules(state.ticks[0] || {});
    const scaleX = canvas.width / rules.courtWidth;
    const scaleY = canvas.height / rules.courtHeight;
    const leftHoopX = rules.leftHoopX;
    const rightHoopX = rules.rightHoopX;
    const hoopY = rules.hoopY;

    // 高质感深色运动科技底色
    ctx.fillStyle = "#0a0e16";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // 绘制微弱球场外框与半场线
    ctx.strokeStyle = "rgba(255, 255, 255, 0.12)";
    ctx.lineWidth = 1.2;
    ctx.strokeRect(10, 10, canvas.width - 20, canvas.height - 20);

    ctx.beginPath();
    ctx.moveTo(canvas.width / 2, 10);
    ctx.lineTo(canvas.width / 2, canvas.height - 10);
    ctx.stroke();

    for (const hoopX of [leftHoopX, rightHoopX]) {
      ctx.beginPath();
      ctx.arc(hoopX * scaleX, hoopY * scaleY, 7 * scaleX, 0, Math.PI * 2);
      ctx.strokeStyle = "rgba(255, 120, 40, 0.4)";
      ctx.stroke();
    }
    for (const shot of state.shots) {
      const sx = shot.x * scaleX;
      const sy = shot.y * scaleY;
      const r = shot.three ? 5.5 : 4;
      ctx.beginPath();
      ctx.arc(sx, sy, r, 0, Math.PI * 2);
      if (shot.made === true) {
        ctx.fillStyle = "#2ce59b";
        ctx.fill();
        ctx.strokeStyle = "#a7f3d0";
        ctx.lineWidth = 1.0;
        ctx.stroke();
      } else if (shot.made === false) {
        ctx.strokeStyle = "#ff6f7e";
        ctx.lineWidth = 1.6;
        ctx.stroke();
      } else {
        ctx.strokeStyle = "#f5bd45";
        ctx.lineWidth = 1.2;
        ctx.stroke();
      }
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

    // 伴随式实时透视台选项卡 (Deck Tabs)
    document.querySelectorAll(".deck-tab-button").forEach((button) =>
      button.addEventListener("click", () => {
        const tab = button.dataset.deckTab;
        document
          .querySelectorAll(".deck-tab-button")
          .forEach((item) => item.classList.toggle("active", item === button));
        document.querySelectorAll(".deck-pane").forEach((pane) => {
          const active = pane.id === `deck-${tab}`;
          pane.classList.toggle("active", active);
        });
        if (tab === "decisions" && state.ticks[state.idx]) {
          renderDecision(state.ticks[state.idx]);
        }
        if (tab === "anomalies") {
          renderAnomalies();
        }
      }),
    );

    $("anomalyBadge").addEventListener("click", () => {
      const anomaliesBtn = document.querySelector('.deck-tab-button[data-deck-tab="anomalies"]');
      if (anomaliesBtn) anomaliesBtn.click();
      const panel = $("anomalyPanel");
      if (panel) {
        const open = panel.hidden;
        panel.hidden = !open;
        $("anomalyBadge").setAttribute("aria-expanded", String(open));
      }
    });

    // 底部研讨舱选项卡 (Studio Tabs)
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
        if (state.currentTab === "frame" && state.ticks[state.idx]) {
          renderFrameJson(state.ticks[state.idx]);
        }
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
    const jumpInput = $("jumpInput");
    if (jumpInput) {
      jumpInput.addEventListener("keydown", (e) => {
        if (e.key === "Enter") seek(jumpInput.value);
      });
    }

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

    const potentialToggle = $("potentialFieldToggle");
    if (potentialToggle) {
      potentialToggle.addEventListener("click", () => {
        state.potentialFieldVisible = !state.potentialFieldVisible;
        potentialToggle.classList.toggle("active", state.potentialFieldVisible);
        potentialToggle.setAttribute("aria-pressed", String(state.potentialFieldVisible));
        if (state.ticks[state.idx]) drawCourt(state.ticks[state.idx]);
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
        const val = String(Math.floor(Math.random() * 90000) + 1000);
        $("seedInput").value = val;
        if ($("sheetSeedInput")) $("sheetSeedInput").value = val;
      });
    }

    const sheetRandom = $("sheetRandomSeedBtn");
    if (sheetRandom) {
      sheetRandom.addEventListener("click", () => {
        const val = String(Math.floor(Math.random() * 90000) + 1000);
        $("seedInput").value = val;
        if ($("sheetSeedInput")) $("sheetSeedInput").value = val;
      });
    }

    const sheetUpload = $("sheetUploadTriggerBtn");
    if (sheetUpload) {
      sheetUpload.addEventListener("click", () => $("fileInput").click());
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
      const isMobile = window.innerWidth <= 768;
      if (isMobile) {
        tooltip.style.left = "";
        tooltip.style.right = "";
        tooltip.style.top = "";
        tooltip.style.bottom = "";
        tooltip.style.transform = "";
      } else {
        tooltip.style.transform = "none";
        tooltip.style.left = `${Math.min(rect.width - 165, (x / canvas.width) * rect.width + 12)}px`;
        tooltip.style.top = `${Math.max(4, (y / canvas.height) * rect.height - 35)}px`;
      }

      const closeBtn = el("button", "btn-close-tooltip", "✕");
      closeBtn.addEventListener("click", (e) => {
        e.stopPropagation();
        tooltip.hidden = true;
      });

      const headerRow = el(
        "div",
        "tooltip-header-row",
        el("strong", null, `${getTeamNameZh(player.team)} · ${player.number ?? player.jersey ?? "—"}号`),
        closeBtn,
      );

      tooltip.replaceChildren(
        headerRow,
        el(
          "span",
          null,
          `${getPositionZh(player.position)} · 进攻职责：${getOffensiveRoleZh(player.offensiveRole)} · 防守职责：${getDefensiveRoleZh(player.defensiveRole)}`,
        ),
        el("span", null, `战术动作：${getActionZh(player.action)} · 战术落位：${getSlotZh(player.slot)}`),
        el(
          "span",
          null,
          `体能储备 ${one(player.stm)}/${one(player.stmMax)} · 心理士气 ${getMoraleZh(player.morale)}`,
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
    $("playerTooltip").addEventListener("click", () => {
      $("playerTooltip").hidden = true;
    });
    $("courtCanvas").addEventListener("touchend", () => {
      setTimeout(() => {
        $("playerTooltip").hidden = true;
      }, 3500);
    });
    document.addEventListener("keydown", (event) => {
      if (event.target.matches("input, textarea, select")) return;
      if (event.code === "Space") {
        event.preventDefault();
        togglePlayback();
      }
      if (event.key === "ArrowLeft") step(-1);
      if (event.key === "ArrowRight") step(1);
      if (event.key === "[" || event.key === "{" || event.key === "ArrowUp") possessionJump(-1);
      if (event.key === "]" || event.key === "}" || event.key === "ArrowDown") possessionJump(1);
      if (event.code === "KeyR") {
        event.preventDefault();
        runSimulation();
      }
      if (event.code === "KeyP") {
        event.preventDefault();
        $("potentialFieldToggle")?.click();
      }
    });
  }

  async function boot() {
    wire();
    await loadDefaultRules();
    await loadStudio();
    await runSimulation();
  }
  window.__nbaStudio = {
    load(catalog) {
      studio = catalog;
      selectedPlayerId = catalog.default_setup.home_team.players[0].id;
      renderStudio();
    },
    selectOffense(side, id) {
      const lineup = studio.default_setup[side + "_lineup"];
      lineup.offense_tactic = id;
      const next = studio.offense.find((item) => item.id === id);
      studio.default_setup[side + "_playbook"] = (studio.plays || []).filter((play) => playFits(play, next?.spec));
      renderStudio();
    },
  };
  boot().catch((error) => {
    setRunStatus("启动失败", true);
    $("streamSummary").textContent = error.message;
    console.error(error);
  });
})();
