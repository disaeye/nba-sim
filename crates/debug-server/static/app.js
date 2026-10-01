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
    teamPerspective: "home",
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
    BALL_ORIENTATION: "球位调整",
    PASS: "传球",
    PASS_RECEIVED: "接球就绪",
    PASS_LANDING_CORRECTED: "传球落点修正",
    PASS_DROPPED: "传球脱手",
    PASS_INTERCEPT_OPPORTUNITY: "断球机会",
    BALL_POKED_LOOSE: "防守拍掉球权",
    LOOSE_BALL_SECURED: "控制活球",
    SHOT_RELEASE: "投篮出手",
    SCORE: "进球得分",
    DRIVE_SCORE: "突破上篮得分",
    DRIVE_MISS: "突破终结未中",
    DRIVE_INITIATED: "持球突破",
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
    PLAY_ACTIVATED: "战术启动",
    PLACEMENT_APPLIED: "落位调整",
    LOOSE_BALL: "地板球争夺",
    ADVANCE: "持球推进",
    HELD: "持球控制",
    DRIVE: "突破推进",
    SHOT: "投篮飞行",
    DEAD: "死球",
    INBOUND_TRANSFER: "发球传递",
    INBOUND_READY: "发球就位",
    CONTROL_TRANSFER: "球权争夺",
    TRIPLE_THREAT_JAB: "三威胁试探步",
    POST_UP: "低位背身要位",
    TACTICAL_EXECUTION: "战术执行",
    INBOUND_SETUP: "发球落位",
    BLOCKED_SHOT: "盖帽封盖",
    DRIVE_FOUL: "突破道犯规",
    DRIVE_KICKOUT: "突破分球",
    DRIVE_PULLUP: "突破急停跳投",
    OUTLET_PASS: "一传发动",
    SHOT_MISSED: "投篮未中",
    GAME_END: "比赛结束",
    OVERTIME_START: "加时开始",
    PERIOD_START: "节间开始",
    PERIOD_END: "节间结束",
    PASS_TIPPED: "传球被拨",
    JUMP_BALL_TRIGGERED: "争球判罚",
    ENFORCEMENT_APPLIED: "规则修正执行",
    FREE_THROW: "执行罚球",
    SUBSTITUTION: "换人",
  };
  function getEventNameZh(name) {
    return EVENT_NAME_ZH[name] || name;
  }

  // 决策 kind 实际为复合串：基名(槽位) 或 基名→球员ID。
  // 解析为「中文动作 · 球员名」可读形式。
  function describeDecisionKind(kind) {
    if (!kind) return "—";
    const arrowMatch = kind.match(/^([A-Z_]+)\s*→\s*(\S+)$/);
    if (arrowMatch) {
      const target = playerNameFromId(arrowMatch[2]);
      return `${getDecisionKindZh(arrowMatch[1])} → ${target}`;
    }
    const parenMatch = kind.match(/^([A-Z_]+)\s*\(([^)]+)\)$/);
    if (parenMatch) {
      const target = playerNameFromId(parenMatch[2]);
      return target ? `${getDecisionKindZh(parenMatch[1])} · ${target}` : `${getDecisionKindZh(parenMatch[1])} · ${parenMatch[2]}`;
    }
    return getDecisionKindZh(kind);
  }

  // 球员内部 ID（H_01/A_03）转「号码 · 中文名」。studio 名单是名字的
  // 权威来源；帧 players 不携带 name，仅能提供 jersey 兜底。
  function playerNameFromId(id) {
    if (!id) return "";
    const roster = studio?.default_setup;
    if (roster) {
      const all = [...(roster.home_team?.players || []), ...(roster.away_team?.players || [])];
      const found = all.find((p) => p.id === id);
      if (found) return `#${found.jersey} ${getPlayerNameZh(found.name)}`;
    }
    for (const tick of [state.ticks[state.idx], state.ticks[0]]) {
      const player = (tick?.players || []).find((p) => p.id === id);
      if (player) return `#${player.jersey} ${player.name || id}`;
    }
    return id;
  }

  const ACTION_ZH = {
    SPOT_UP_3PT: "定点拉开",
    HighScreenRoll: "挡拆顺下",
    DRIVE_OFF_SCREEN: "借掩护突破",
    PERIMETER_CUT: "外线跑位",
    SET_HIGH_SCREEN: "高位掩护",
    ROLL_TO_RIM: "顺下攻筐",
    ROTATE_RIM_HELP: "轮转护筐",
    X_OUT_CLOSEOUT: "轮转扑防",
    HELP_SIDE_SHELL: "弱侧协防",
    ON_BALL_CONTEST: "贴身紧逼",
    DROP_CONTAIN: "沉退遏制",
    HEDGE_AND_RECOVER: "延误返位",
    SWITCH_ASSIGNMENT: "对位换防",
    LOOSE_BALL_RECOVERY: "拼抢活球",
    REBOUND_CRASH: "冲抢篮板",
    BOXOUT: "卡位保护",
    DRIBBLE_TOP: "弧顶组织",
    BACKDOOR_CUT: "后门空切",
    DIP_TO_RIM: "直插篮下",
    LIFT: "弱侧上提",
    SCREEN_POP: "外弹三分",
    PLAY_ScreenRoll: "挡拆顺下",
    PLAY_ScreenPop: "掩护外弹",
    PLAY_CutBackdoor: "后门空切",
    PLAY_Lift: "弱侧上提",
    Initiate: "战术发起",
    Crossover: "变向突破",
    DriveToBasket: "突破攻筐",
    SetPosition: "战术站位",
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
    BACK_SCREEN: "背掩护",
    SPAIN_POP: "外弹三分",
  };
  function getActionZh(action) {
    return ACTION_ZH[action] || action || "—";
  }

  const SLOT_ZH = {
    top: "弧顶持球位",
    screener: "高位掩护位",
    stack_screener: "背掩护外弹位",
    weak_wing: "弱侧侧翼",
    strong_corner: "强侧底角",
    weak_corner: "弱侧底角",
    strong_wing: "强侧侧翼",
    left_wing: "左侧 45° 侧翼",
    right_wing: "右侧 45° 侧翼",
    left_corner: "左底角射手位",
    right_corner: "右底角射手位",
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
    HighScreenRoll: "牛角高位挡拆体系",
    HighPickAndRoll: "牛角高位挡拆体系",
    SpainPickAndRoll: "西班牙双掩护体系",
    DriveKick: "突分与追身掩护体系",
    DriveAndKick: "突分与追身掩护体系",
    FiveOutMotion: "五外动态进攻体系",
    HornsSet: "牛角高位挡拆体系",
    TriangleOffense: "三角进攻体系",
    IsolationDrive: "弧顶发牌高位单打",
    PostUp: "低位背身策应体系",
    FastBreakTransition: "快攻闪击转换体系",
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
    north_city_hawks: "北城老鹰",
    south_bay_mariners: "南湾水手",
    hawks: "北城老鹰",
    mariners: "南湾水手",
    Hawks: "北城老鹰",
    Mariners: "南湾水手",
    "North City Hawks": "北城老鹰",
    "South Bay Mariners": "南湾水手",
    south_coast_celtics: "波士顿凯尔特人",
    celtics: "波士顿凯尔特人",
    Celtics: "波士顿凯尔特人",
    lakers: "南湾水手",
    LAL: "南湾水手",
    BOS: "北城老鹰",
    home: "北城老鹰",
    away: "南湾水手",
    HOME: "北城老鹰",
    AWAY: "南湾水手",
  };
  function getTeamNameZh(team) {
    if (!team) return "—";
    const name = typeof team === "string" ? team : (team.short_name || team.name || "");
    return TEAM_ZH[name] || TEAM_ZH[team.id] || name || "—";
  }

  const PLAYER_NAME_ZH = {
    // 客队 (南湾水手)
    "Luka Maren": "卢卡·马伦",
    "Kellan Price": "凯兰·普莱斯",
    "Scott Rowan": "斯科特·罗万",
    "Drake Ellis": "德雷克·埃利斯",
    "Anton Vale": "安东·韦尔",
    "Tyler Quinn": "泰勒·奎因",
    "Mika Stone": "米卡·斯通",
    "Niko Voss": "尼科·沃斯",
    // 主队 (北城老鹰)
    "Darius Vale": "达柳斯·韦尔",
    "Malik Rowan": "马利克·罗万",
    "Andre Mercer": "安德烈·默瑟",
    "Caleb North": "凯莱布·诺斯",
    "Jonas Reed": "乔纳斯·里德",
    "Jordan Pike": "乔丹·派克",
    "Aaron Wells": "阿隆·韦尔斯",
    "Rudy Moss": "鲁迪·莫斯",
  };
  function getPlayerNameZh(name) {
    if (!name) return "球员";
    return PLAYER_NAME_ZH[name] || name;
  }

  const TACTIC_SYSTEM_DESC_ZH = {
    off_horns_pnr: "双高位牛角挡拆 · 掩护顺下攻筐",
    off_spain_pnr: "西班牙双掩护 · 背挡顺下与外弹空位",
    off_motion_spacing: "五外动态空间 · 无球传切与反跑后门",
    off_transition_push: "快攻闪击转换 · 8秒奔袭冲筐与追身三分",
    off_delay_attack: "弧顶高位发牌 · 手递手接球突分与反切",
    off_post_split: "低位背身强打 · 强侧交叉反切与底角分球",
    off_drag_screen: "转换追身掩护 · 突分突破与拖尾跳投",
    def_man_conservative: "常规半场人盯人 · 保持对位与防守平衡",
    def_man_pressure: "全场紧逼领防 · 持续消耗体能逼迫失误",
    def_switch_heavy: "无限轮转换防 · 扑灭三分需警惕身材错位",
    def_drop_coverage: "中锋沉退护筐 · 封锁油漆区迫使中距离",
    def_hedge_recover: "大延误快速返位 · 阻绝后卫急停出手空间",
    def_zone_23: "经典二三联防 · 保护篮下禁区与后场篮板",
  };
  function getTacticDescZh(id) {
    return TACTIC_SYSTEM_DESC_ZH[id] || "战术执行策略";
  }

  function cleanTacticNameZh(name) {
    if (!name) return "";
    return name.replace(/\s*\([^)]*\)/g, "").trim();
  }

  const PLAY_VERB_ZH = {
    ScreenRoll: "挡拆顺下",
    CutBackdoor: "空切后门",
    Lift: "弱侧上提",
    SpotUp: "定点待命",
    Drive: "持球突破",
    Pass: "传球转移",
    DipToRim: "直切篮下",
    BackdoorCut: "空切偷门",
    DribbleTop: "弧顶组织",
    PerimeterRelocate: "外线跑位",
    HighScreenRoll: "高位挡拆",
  };
  function getPlayVerbZh(verb) {
    return PLAY_VERB_ZH[verb] || verb || "执行战术";
  }

  const DECISION_KIND_ZH = {
    Shoot: "投篮出手",
    Pass: "传球",
    Drive: "持球突破",
    Dwell: "持球观察",
    Reset: "重置战术",
    Cut: "空切跑位",
    Screen: "设立掩护",
    JAB: "试探步",
    ADVANCE: "向前推进",
  };
  function getDecisionKindZh(kind) {
    return DECISION_KIND_ZH[kind] || kind || "—";
  }

  // 引擎约束/规则标识符的中文释义（决策面板的约束列表、阻截列表用）
  const CONSTRAINT_ZH = {
    backcourt_clock: "八秒未过半场计时",
    out_of_bounds: "边线界外限制",
    out_of_bounds_event: "出界事件后续",
    action_eligibility: "动作可用性",
    dead_ball_action: "死球动作限制",
    contact_fact: "接触事实登记",
    risky_pass: "高风险传球惩罚",
    contested_shot: "强干扰出手惩罚",
    crowded_receiver: "接球人被贴防惩罚",
    shot_clock_urgency: "进攻时间紧迫惩罚",
    shot_clock: "二十四秒进攻计时",
    inbound_clock: "发球五秒计时",
    inbound_constraint: "发球限制",
    shooting_ft: "罚球流程",
  };
  function getConstraintZh(name) {
    return CONSTRAINT_ZH[name] || name;
  }

  const ANOMALY_KIND_ZH = {
    SpeedViolation: "移动超速违例",
    InvariantViolation: "引擎不变量违规",
    CollisionViolation: "物理碰撞穿透违例",
    BallHolderLeash: "持球人脱缰位移违规",
    BoundaryViolation: "非法出界脱轨违例",
    ShotClockViolation: "24秒进攻违例",
    BackcourtViolation: "回场违例",
    EightSecondViolation: "8秒未过半场违例",
  };
  function getAnomalyKindZh(kind) {
    return ANOMALY_KIND_ZH[kind] || kind || "不变量监测记录";
  }

  function formatCalloutZh(text) {
    if (!text) return "";
    let res = String(text);
    for (const [enName, zhName] of Object.entries(PLAYER_NAME_ZH)) {
      if (res.includes(enName)) {
        res = res.replaceAll(enName, zhName);
      }
    }
    for (const [enTeam, zhTeam] of Object.entries(TEAM_ZH)) {
      if (res.includes(enTeam)) {
        res = res.replaceAll(enTeam, zhTeam);
      }
    }
    return res;
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
      window.__studio = studio;
    } catch {
      studio = null;
    }
    selectedPlayerId = studio?.default_setup?.home_team?.players?.[0]?.id || null;
    updatePerspectiveNav();
    renderStudio();
  }

  function label(group, key) {
    return studio?.labels?.[group]?.[key] || key;
  }

  function setTeamPerspective(side) {
    state.teamPerspective = side;
    const isHome = side === "home";
    const team = isHome ? studio?.default_setup?.home_team : studio?.default_setup?.away_team;
    selectedPlayerId = team?.players?.[0]?.id || null;
    updatePerspectiveNav();
    renderStudio();
  }

  function updatePerspectiveNav() {
    if (!studio) return;
    syncRosterTeamButtons();
  }

  // 阵容 pane 内主客队按钮的名称与激活态同步
  function syncRosterTeamButtons() {
    if (!studio) return;
    const homeName = getTeamNameZh(studio.default_setup.home_team.id) || studio.default_setup.home_team.name;
    const awayName = getTeamNameZh(studio.default_setup.away_team.id) || studio.default_setup.away_team.name;
    const homeText = $("rosterHomeTeamName");
    const awayText = $("rosterAwayTeamName");
    if (homeText) homeText.textContent = `${homeName} (主队)`;
    if (awayText) awayText.textContent = `${awayName} (客队)`;
    const isHome = state.teamPerspective === "home";
    const homeBtn = $("rosterTeamHomeBtn");
    const awayBtn = $("rosterTeamAwayBtn");
    if (homeBtn) homeBtn.classList.toggle("active", isHome);
    if (awayBtn) awayBtn.classList.toggle("active", !isHome);
  }

  function renderStudio() {
    if (!studio) return;
    renderBoard();
    renderRoster();
    syncScoreboardTactics();
  }

  // 记分牌两侧的默认战术标签（推演开始前无 tick 数据时的静态展示）
  function syncScoreboardTactics() {
    if (!studio) return;
    const setup = studio.default_setup;
    const homeOff = studio.offense.find((item) => item.id === setup.home_lineup.offense_tactic);
    const awayOff = studio.offense.find((item) => item.id === setup.away_lineup.offense_tactic);
    const homeTag = $("homeTacticTag");
    const awayTag = $("awayTacticTag");
    const homeDef = $("homeDefTag");
    const awayDef = $("awayDefTag");
    const homeDefTactic = studio.defense.find((d) => d.id === setup.home_lineup?.defense_tactic);
    const awayDefTactic = studio.defense.find((d) => d.id === setup.away_lineup?.defense_tactic);
    if (homeTag) homeTag.textContent = `攻 · ${cleanTacticNameZh(homeOff?.name_zh) || "—"}`;
    if (awayTag) awayTag.textContent = `攻 · ${cleanTacticNameZh(awayOff?.name_zh) || "—"}`;
    if (homeDef) homeDef.textContent = `守 · ${cleanTacticNameZh(homeDefTactic?.name_zh) || "—"}`;
    if (awayDef) awayDef.textContent = `守 · ${cleanTacticNameZh(awayDefTactic?.name_zh) || "—"}`;
  }

  function renderBoard() {
    const root = $("tacticBoard");
    if (!root || !studio) return;
    const setup = studio.default_setup;
    const isHome = state.teamPerspective === "home";
    const side = isHome ? "home" : "away";
    const team = isHome ? setup.home_team : setup.away_team;
    const lineup = isHome ? setup.home_lineup : setup.away_lineup;
    const playbook = isHome ? setup.home_playbook : setup.away_playbook;

    root.replaceChildren(
      singleTeamTacticalBoard(side, team, lineup, playbook)
    );
  }

  // ========================================================
  // 职业战术百科全书与战术博弈情报库 (Tactical Playbook Encyclopedia)
  // ========================================================
  const TACTICAL_PLAYBOOK_ENCYCLOPEDIA = {
    off_horns_pnr: {
      name: "牛角高位挡拆战术体系",
      style: "双高位牛角站位构型 (Horns Spacing)",
      philosophy: "现代 NBA 最经典的挡拆起手式。中锋与大前锋分居罚球线左右两肘区，双射手底角拉开，迫使防守大个子远离禁区。控卫借掩护突破，顺下、外弹或分底角形成立体打击。",
      progression: [
        { rank: "第一选择", title: "持球突破 / 抛投急停", desc: "控卫借挡拆压低重心突破，若掩护防守人沉退，直接在罚球线干拔中投或抛投终结。" },
        { rank: "第二配合", title: "中锋顺下空接攻筐", desc: "掩护中锋迅速转身向篮筐顺下，控卫送出高吊球或击地传球，直接完成篮下空接扣篮。" },
        { rank: "第三策应", title: "弱侧肘区外弹三分", desc: "另一名大个子不顺下而选择向三分线外弹，接横传球命中大空位三分。" },
        { rank: "弱侧兜底", title: "强弱侧大对角转移", desc: "若防守强侧过度协防收缩，控卫大跳传至弱侧底角射手，完成无干扰底角三分。" },
      ],
      advantage: "精准瓦解中锋沉退防守与传统人盯人；双掩护人令防守无法预判突破方向。",
      caution: "若对手采取无限换防，应停止快速传切，转入内线错位背身单打。",
      routes: [
        { type: "dribble", from: [50, 62], to: [40, 46], label: "借掩护突破" },
        { type: "roll", from: [38, 40], to: [48, 20], label: "顺下冲击" },
        { type: "pop", from: [62, 40], to: [75, 52], label: "外弹远投" },
        { type: "spot", from: [5, 9], to: [5, 9], label: "底角定点" },
        { type: "spot", from: [95, 9], to: [95, 9], label: "底角定点" },
      ],
    },
    off_spain_pnr: {
      name: "西班牙双掩护战术体系",
      style: "双掩护叠影构型 (Stack & Pop Spacing)",
      philosophy: "针对沉退防守最高效的现代战术杀器。中锋高位给控卫挡拆顺下的同时，射手在罚球线为顺下中锋的防守人架设背掩护，迫使防守人同时面对顺下空接与外弹远投，陷入无解两难。",
      progression: [
        { rank: "第一选择", title: "顺下中锋空接终结", desc: "中锋借背掩护完全摆脱防守追赶，接控卫高吊传球直接完成空中接力暴扣。" },
        { rank: "第二配合", title: "背掩护射手反弹三分", desc: "背掩护人员完成掩护后以极快速度外弹至弧顶三分线，接控卫回传命中绝对空位三分。" },
        { rank: "第三策应", title: "持球控卫急停抛投", desc: "若防守内线被背掩护完全卡住，持球人面对真空禁区直接上篮或中投得分。" },
        { rank: "弱侧兜底", title: "底角定点牵制投射", desc: "底角两名射手吸附弱侧底线防守人，一旦对方协防收缩，立刻形成致命底角三分。" },
      ],
      advantage: "完全破除大中锋沉退护筐策略；防守人若缺乏默契换防，必然出现顺下扣篮或弧顶空位三分。",
      caution: "对第三人背掩护的设立质量要求极高；若对手采取提前高位包夹控卫，需持球人快速出球。",
      routes: [
        { type: "dribble", from: [50, 62], to: [38, 48], label: "侧向突破" },
        { type: "roll", from: [50, 42], to: [50, 18], label: "切入空接" },
        { type: "pop", from: [50, 26], to: [64, 58], label: "外弹三分" },
        { type: "spot", from: [5, 9], to: [5, 9], label: "底角拉开" },
        { type: "spot", from: [95, 9], to: [95, 9], label: "底角拉开" },
      ],
    },
    off_motion_spacing: {
      name: "五外动态无球传切体系",
      style: "五外环形全拉开构型 (Perimeter Five-Out)",
      philosophy: "极致动态空间篮球。五名球员均置身于三分线外，禁区彻底腾空。依靠高频次的传球转移、后门空切、手递手与弱侧无球反向掩护，创造无死角的进攻火力点。",
      progression: [
        { rank: "第一选择", title: "反跑后门空切攻筐", desc: "防守人外扑紧逼时，侧翼球员利用反向垫步反切篮下，接传球直取篮筐。" },
        { rank: "第二配合", title: "弱侧连续上提三分", desc: "弱侧球员借连续无球掩护横穿底线并上提至侧翼，迎着防守空档干拔三分。" },
        { rank: "第三策应", title: "手递手掩护突分", desc: "外线两人高位手递手快速借掩护突破，吸附协防后回敲外线射手群。" },
        { rank: "弱侧兜底", title: "底角切入二次分球", desc: "底角人员沿底线纵深切入吸引防守，传球给空切跟进的侧翼终结者。" },
      ],
      advantage: "对机动性弱、沉退护筐的大中锋形成沉重打击；全员具备三分与切入能力，防不胜防。",
      caution: "极其依赖全队的战术默契与传球视野；遇到对抗极强的肉搏盯人需保持耐心运转。",
      routes: [
        { type: "pass", from: [50, 62], to: [30, 52], label: "传球转移", style: "dashed" },
        { type: "screen", from: [22, 51], to: [28, 55], label: "外线组织" },
        { type: "back_cut", from: [78, 51], to: [52, 20], label: "后门空切" },
        { type: "lift", from: [5, 9], to: [16, 32], label: "弱侧上提" },
        { type: "dip", from: [95, 9], to: [80, 16], label: "下沉禁区" },
      ],
    },
    off_delay_attack: {
      name: "弧顶发牌策应战术体系",
      style: "高位发牌五外构型 (Delay Hub Spacing)",
      philosophy: "大个子站在弧顶三分线外作为核心分球中枢，后卫与侧翼球员通过交叉跑位、手递手配合与后门切入交织发起进攻。",
      progression: [
        { rank: "第一选择", title: "手递手急停跳投 / 突破", desc: "后卫高速绕过中枢接手递手，借中枢身躯阻挡防守人，直接急停跳投或突入内线。" },
        { rank: "第二配合", title: "中枢击地妙传空切后门", desc: "防守人员提前预判手递手抢过时，后卫突然假动作变向反切篮下，接高位击地传球轻松上篮。" },
        { rank: "第三策应", title: "中枢面框直接单打 / 远投", desc: "若防守人放一步防突破，大个子直接在弧顶干拔三分，或持球强力突破。" },
        { rank: "弱侧兜底", title: "弱侧对角大空位投射", desc: "强侧双人手递手牵制全队防守，中枢大跨度横传弱侧底角射手投进空位三分。" },
      ],
      advantage: "彻底废黜对方内线防守护筐价值；发牌中枢视野宽广，进攻不易陷入失误停滞。",
      caution: "要求发牌核心具备顶级传球智商与远投威胁；后卫切入时机必须与传球节奏精准同步。",
      routes: [
        { type: "dho", from: [50, 60], to: [46, 56], label: "手递手交接" },
        { type: "dribble", from: [28, 47], to: [42, 32], label: "借掩护攻筐" },
        { type: "back_cut", from: [72, 47], to: [52, 20], label: "反跑空切" },
        { type: "spot", from: [5, 9], to: [5, 9], label: "定点待命" },
        { type: "spot", from: [95, 9], to: [95, 9], label: "拉开空间" },
      ],
    },
    off_post_split: {
      name: "低位背身策应战术体系",
      style: "四外一内低位站位构型 (Post-Up Spacing)",
      philosophy: "传统低位单打与现代动态切分的融合。球直接喂给低位背打核心，防守重心收缩包夹的瞬间，强侧两名外线射手立刻展开双人交叉反切，打乱防守阵型。",
      progression: [
        { rank: "第一选择", title: "低位核心直接背打终结", desc: "若对方不包夹，低位核心利用脚步、勾手或后仰跳投直接在禁区单打得分。" },
        { rank: "第二配合", title: "强侧双人交叉反切空接", desc: "外线两名球员在肘区互相掩护交叉反跑，其中一人直切篮下接低位分球完成上篮。" },
        { rank: "第三策应", title: "反弹外线急停三分", desc: "另一名反切人员在掩护后反弹外线，接低位回传命中正面大空位三分。" },
        { rank: "弱侧兜底", title: "大对角分球弱侧底角", desc: "弱侧防守收缩协防时，低位核心背身单手大甩球至弱侧底角，命中底角三分。" },
      ],
      advantage: "杀伤力极高，能迅速令对方主力内线背上犯规困扰；战术节奏稳健，压迫感强。",
      caution: "极其考验低位人员的出球视野与抗包夹能力；外线射手命中率过低时易遭铁桶合围。",
      routes: [
        { type: "post", from: [30, 26], to: [34, 18], label: "低位单打" },
        { type: "pass", from: [50, 60], to: [42, 52], label: "强侧拉开" },
        { type: "pop", from: [28, 47], to: [22, 55], label: "反弹三分" },
        { type: "cut", from: [72, 47], to: [46, 18], label: "内切攻筐" },
        { type: "spot", from: [95, 9], to: [95, 9], label: "底角埋伏" },
      ],
    },
    off_drag_screen: {
      name: "突分与追身掩护战术体系",
      style: "快节奏拖尾突分构型 (Pistol Drag Spacing)",
      philosophy: "现代高节奏跑轰与魔球打法的主力引擎。由守转攻落位未稳之际，跟进的大个子直接在弧顶挂上追身掩护，控卫依仗冲势撕裂防守，突分结合外线拖尾投射。",
      progression: [
        { rank: "第一选择", title: "控卫借追身掩护冲筐", desc: "防守退防立足未稳，控卫借追身掩护加速过人，直取篮下完成上篮。" },
        { rank: "第二配合", title: "拖尾大个子外弹追身三分", desc: "设立追身掩护后大个子留在弧顶三分线，接控卫突分回传命中追身三分。" },
        { rank: "第三策应", title: "突破吸引协防分底角", desc: "持球人突入腹地吸引底线防守收缩，突分甩传两侧底角空位射手投篮。" },
        { rank: "弱侧兜底", title: "次级持球人二次突破", desc: "球回给弱侧弧顶跟进人员，立刻发动二次突破冲击防守失衡的半场。" },
      ],
      advantage: "利用攻防转换立足未稳打时间差，防守极难设立包夹；进攻节拍极快，压迫力强。",
      caution: "要求控卫拥有极强的终结和传球决断力；急躁失误易被对手反打快攻反击。",
      routes: [
        { type: "dribble", from: [50, 68], to: [40, 36], label: "高速突击" },
        { type: "pop", from: [56, 60], to: [64, 62], label: "拖尾跳投" },
        { type: "spot", from: [24, 53], to: [18, 48], label: "侧翼跟进" },
        { type: "spot", from: [76, 53], to: [82, 48], label: "拉开防守" },
        { type: "roll", from: [90, 21], to: [60, 16], label: "冲筐冲板" },
      ],
    },
    off_transition_push: {
      name: "快攻闪击全场转换体系",
      style: "全场两翼极速拉开构型 (Fastbreak Wide Spacing)",
      philosophy: "极致追求速度的快打旋风体系。抢下后场篮板或抢断瞬间，两翼飞奔球员以最快速度全速下快攻，控卫快速推进或长传，追求在防守落位前 8 秒内完成进攻。",
      progression: [
        { rank: "第一选择", title: "后场长传直冲篮下上篮", desc: "推进引擎在后场直接送出精确长传，快下球员迎球直接飞身冲筐或扣篮。" },
        { rank: "第二配合", title: "前场以多打少击地分球", desc: "前场形成多打少，持球人吸引最后一名防守人后击地妙传队友空篮得分。" },
        { rank: "第三策应", title: "追身急停三分破网", desc: "两翼射手快下底角拉开防线，持球人急停吸引防守后回敲外线命中追身三分。" },
        { rank: "弱侧兜底", title: "拖尾跟进冲抢进攻篮板", desc: "内线大个子作为拖尾人员全力冲抢前场篮板，直接完成二次进攻补篮得分。" },
      ],
      advantage: "绕开阵地战复杂博弈，以极高效率获取轻松得分机会；极大消耗对手主力体能。",
      caution: "退防失误率显著高于阵地战；遇到全场退防迅速的强队容易陷入进攻滞涩。",
      routes: [
        { type: "dribble", from: [50, 74], to: [50, 42], label: "快推发牌" },
        { type: "sprint", from: [16, 43], to: [20, 16], label: "左翼飞奔" },
        { type: "sprint", from: [84, 43], to: [80, 16], label: "右翼顺下" },
        { type: "pop", from: [30, 64], to: [26, 55], label: "追身三分" },
        { type: "roll", from: [70, 60], to: [52, 18], label: "冲抢篮板" },
      ],
    },
  };

  const DEFENSIVE_INTEL_ENCYCLOPEDIA = {
    def_drop_coverage: {
      advantage: "克制冲击型控卫与空切内线；死守油漆区保护后场篮板。",
      caution: "被西班牙背掩护射手与顶级急停跳投手严重惩罚。",
    },
    def_hedge_recover: {
      advantage: "大延误阻绝控卫直接干拔跳投；强力压迫持球挡拆发起人。",
      caution: "掩护中锋顺下空接威胁大，对弱侧底线轮转补位要求极高。",
    },
    def_switch_heavy: {
      advantage: "彻底扑灭对手空位三分出手；所有传球路线均被贴身切断。",
      caution: "频繁出现小防大与大防小错位，容易被对手错位单打强吃。",
    },
    def_man_conservative: {
      advantage: "保持五对五基础防守平衡，防守失位概率最低。",
      caution: "缺乏强力施压手段，面对顶级持球大核容易被单点打穿。",
    },
    def_man_pressure: {
      advantage: "全场紧逼逼迫后卫失误；极大破坏对方战术执行节拍。",
      caution: "全场防守体能消耗剧烈，一旦被突破容易失位形成多打少。",
    },
    def_zone_23: {
      advantage: "铁桶合围禁区油漆区；强力克制突破攻筐与低位单打球队。",
      caution: "弧顶与两翼 45 度三分线空档较大，易被连续外线三分射穿。",
    },
  };

  function singleTeamTacticalBoard(side, team, lineup, playbook) {
    const offense = studio.offense.find((item) => item.id === lineup.offense_tactic);
    const defense = studio.defense.find((item) => item.id === lineup.defense_tactic);
    const isHome = side === "home";
    const teamNameZh = getTeamNameZh(team.id) || team.name;

    const wrap = el("div", "studio-board");

    // 顶部执教信息横幅
    const banner = el("div", "studio-team-banner");
    const metaBox = el("div", "banner-team-meta");
    metaBox.append(
      el("span", `perspective-dot ${isHome ? "home-dot" : "away-dot"}`),
      el("strong", null, `${teamNameZh} · 战术研讨指挥中枢`),
      el("span", `banner-team-tag ${isHome ? "home" : "away"}`, isHome ? "主场作战" : "客场作战"),
      el("span", "banner-team-tag", `进攻体系: ${cleanTacticNameZh(offense?.name_zh) || "未配置"}`),
      el("span", "banner-team-tag", `防守策略: ${cleanTacticNameZh(defense?.name_zh) || "未配置"}`),
    );
    const privacyHint = el(
      "div",
      "banner-privacy-badge",
      el("span", null, "更衣室机密档案 (对手不可见)")
    );
    banner.append(metaBox, privacyHint);
    wrap.append(banner);

    // 三列并列专业战术网格
    const grid = el("div", "tactic-three-grid");

    // 第 1 栏：进攻体系与战术偏好指令
    const offenseCard = el("section", "tactic-panel-card");
    const offTitle = el("div", "tactic-panel-title");
    offTitle.append(el("strong", null, "核心进攻体系"), el("span", null, "7大职业战术体系"));
    offenseCard.append(
      offTitle,
      choiceRow(studio.offense, lineup.offense_tactic, async (id) => {
        lineup.offense_tactic = id;
        const next = studio.offense.find((item) => item.id === id);
        const compatible = playsForTactic(next?.spec);
        if (side === "home") studio.default_setup.home_playbook = compatible;
        else studio.default_setup.away_playbook = compatible;
        renderStudio();
        setRunStatus(`已切换至【${cleanTacticNameZh(next?.name_zh)}】，正在重新模拟比赛…`);
        await runSimulation(null, true);
      }),
      renderTacticalDirectives(team),
    );

    // 第 2 栏：防守博弈策略与克制情报
    const defenseCard = el("section", "tactic-panel-card");
    const defTitle = el("div", "tactic-panel-title");
    defTitle.append(el("strong", null, "防守博弈策略"), el("span", null, "阵型与掩护应对"));
    defenseCard.append(
      defTitle,
      defenseGroupedRow(studio.defense, lineup.defense_tactic, async (id) => {
        lineup.defense_tactic = id;
        const defItem = studio.defense.find((d) => d.id === id);
        renderStudio();
        setRunStatus(`已切换防守策略为【${cleanTacticNameZh(defItem?.name_zh || "防守")}】，正在重新模拟比赛…`);
        await runSimulation(null, true);
      }),
      renderDefensiveIntel(lineup.defense_tactic),
    );

    // 第 3 栏：专业战术沙盘、破防决策树与战术手册
    const playbookCard = el("section", "tactic-panel-card");
    const playTitle = el("div", "tactic-panel-title");
    playTitle.append(el("strong", null, "战术沙盘与决策配合"), el("span", null, `装配 ${playbook.length} 套执行动作`));
    playbookCard.append(
      playTitle,
      renderTacticalChalkboard(offense?.spec, lineup.offense_tactic),
      renderTacticalBreakdown(lineup.offense_tactic),
      playList(playbook),
    );

    grid.append(offenseCard, defenseCard, playbookCard);
    wrap.append(grid);
    return wrap;
  }

  function renderTacticalDirectives(team) {
    const wrap = el("div", "tactic-directives-panel");
    const traits = team?.team_traits || {};

    // 终结重心偏好
    const row1 = el("div", "directive-row");
    row1.append(el("span", "directive-label", "终结重心"));
    const chips1 = el("div", "directive-chips");
    const rimBtn = el("button", `directive-chip${traits.rim_pressure >= 0.55 ? " active" : ""}`, "攻筐为主");
    const threeBtn = el("button", `directive-chip${traits.three_point_emphasis >= 0.55 ? " active" : ""}`, "三分投射");
    rimBtn.type = "button";
    threeBtn.type = "button";
    rimBtn.addEventListener("click", async () => {
      traits.rim_pressure = 0.75;
      traits.three_point_emphasis = 0.45;
      renderStudio();
      setRunStatus("已调整战术倾向为【攻筐为主】，正在重新模拟比赛…");
      await runSimulation(null, true);
    });
    threeBtn.addEventListener("click", async () => {
      traits.rim_pressure = 0.45;
      traits.three_point_emphasis = 0.75;
      renderStudio();
      setRunStatus("已调整战术倾向为【三分投射】，正在重新模拟比赛…");
      await runSimulation(null, true);
    });
    chips1.append(rimBtn, threeBtn);
    row1.append(chips1);

    // 推进节奏偏好
    const row2 = el("div", "directive-row");
    row2.append(el("span", "directive-label", "推进节奏"));
    const chips2 = el("div", "directive-chips");
    const fastBtn = el("button", `directive-chip${traits.pace >= 0.55 ? " active" : ""}`, "快打风暴");
    const controlBtn = el("button", `directive-chip${traits.pace < 0.55 ? " active" : ""}`, "阵地耐心");
    fastBtn.type = "button";
    controlBtn.type = "button";
    fastBtn.addEventListener("click", async () => {
      traits.pace = 0.75;
      renderStudio();
      setRunStatus("已调整比赛节奏为【快打风暴】，正在重新模拟比赛…");
      await runSimulation(null, true);
    });
    controlBtn.addEventListener("click", async () => {
      traits.pace = 0.42;
      renderStudio();
      setRunStatus("已调整比赛节奏为【阵地耐心】，正在重新模拟比赛…");
      await runSimulation(null, true);
    });
    chips2.append(fastBtn, controlBtn);
    row2.append(chips2);

    wrap.append(row1, row2);
    return wrap;
  }

  function renderDefensiveIntel(defenseId) {
    const intel = DEFENSIVE_INTEL_ENCYCLOPEDIA[defenseId];
    if (!intel) return el("div");
    const box = el("div", "tactic-intel-box");
    box.append(
      el("div", "intel-row", el("span", "intel-tag advantage", "战术优势"), el("span", "intel-text", intel.advantage)),
      el("div", "intel-row", el("span", "intel-tag caution", "防守软肋"), el("span", "intel-text", intel.caution)),
    );
    return box;
  }

  function renderTacticalBreakdown(tacticId) {
    const data = TACTICAL_PLAYBOOK_ENCYCLOPEDIA[tacticId];
    if (!data) return el("div");

    const wrap = el("div", "progression-list");
    for (const item of data.progression) {
      const row = el("div", "progression-item");
      const head = el("div", "progression-head");
      head.append(el("span", "progression-rank", item.rank), el("strong", "progression-title", item.title));
      row.append(head, el("span", "progression-desc", item.desc));
      wrap.append(row);
    }

    const intelBox = el("div", "tactic-intel-box");
    intelBox.append(
      el("div", "intel-row", el("span", "intel-tag advantage", "专克阵型"), el("span", "intel-text", data.advantage)),
      el("div", "intel-row", el("span", "intel-tag caution", "破解之道"), el("span", "intel-text", data.caution)),
    );
    wrap.append(intelBox);
    return wrap;
  }

  function renderTacticalChalkboard(spec, tacticId) {
    const wrap = el("div", "tactic-chalkboard-wrap");
    const box = el("div", "chalkboard-canvas-box");
    const canvas = document.createElement("canvas");
    canvas.width = 360;
    canvas.height = 200;
    box.append(canvas);

    const data = TACTICAL_PLAYBOOK_ENCYCLOPEDIA[tacticId];
    drawChalkboardRoutes(canvas, spec, data?.routes || []);

    const toolbar = el("div", "chalkboard-toolbar");
    const legend = el("div", "chalkboard-legend");
    legend.append(
      el("span", null, "──> 跑位"),
      el("span", null, "~~~> 突破"),
      el("span", null, "- - > 传球"),
    );
    const demoBtn = el("button", "btn-run-chalkboard", "▶ 跑位演练");
    demoBtn.type = "button";
    demoBtn.addEventListener("click", () => {
      animateChalkboard(canvas, spec, data?.routes || []);
    });
    toolbar.append(legend, demoBtn);

    wrap.append(box, toolbar);
    return wrap;
  }

  function drawChalkboardRoutes(canvas, spec, routes, progress = 0) {
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const w = canvas.width;
    const h = canvas.height;

    ctx.clearRect(0, 0, w, h);

    // 绘制半场战术板墨黑底色
    ctx.fillStyle = "#0c1322";
    ctx.fillRect(0, 0, w, h);

    // 半场标准几何比例映射 (长 47ft, 宽 50ft)
    const courtLeft = 14;
    const courtTop = 14;
    const courtW = w - 28;
    const courtH = h - 28;

    // 绘制半场白色球场线
    ctx.strokeStyle = "rgba(148, 163, 184, 0.28)";
    ctx.lineWidth = 1.2;

    // 半场边界
    ctx.strokeRect(courtLeft, courtTop, courtW, courtH);

    // 篮筐与禁区 (顶端为前场底线，篮筐距底线 5.25ft)
    const hoopX = courtLeft + courtW / 2;
    const hoopY = courtTop + (5.25 / 47) * courtH;
    ctx.beginPath();
    ctx.arc(hoopX, hoopY, 6, 0, Math.PI * 2);
    ctx.strokeStyle = "rgba(245, 158, 11, 0.85)";
    ctx.stroke();

    // 禁区油漆区 (宽 16ft, 长 19ft)
    const paintW = (16 / 50) * courtW;
    const paintH = (19 / 47) * courtH;
    ctx.strokeStyle = "rgba(148, 163, 184, 0.32)";
    ctx.strokeRect(hoopX - paintW / 2, courtTop, paintW, paintH);

    // 罚球线半圆
    ctx.beginPath();
    ctx.arc(hoopX, courtTop + paintH, (6 / 50) * courtW, 0, Math.PI);
    ctx.stroke();

    // 三分线 (距边线 3ft 直线段，切入 23.75ft 圆弧)
    const cornerXDist = (3 / 50) * courtW;
    const leftCornerX = courtLeft + cornerXDist;
    const rightCornerX = courtLeft + courtW - cornerXDist;
    const cornerH = (14 / 47) * courtH;
    const arcR = (23.75 / 47) * courtH;
    ctx.beginPath();
    ctx.moveTo(leftCornerX, courtTop);
    ctx.lineTo(leftCornerX, courtTop + cornerH);
    const chordHalf = hoopX - leftCornerX;
    const angle = Math.acos(Math.min(1.0, chordHalf / arcR));
    ctx.arc(hoopX, hoopY, arcR, Math.PI - angle, angle, false);
    ctx.lineTo(rightCornerX, courtTop + cornerH);
    ctx.lineTo(rightCornerX, courtTop);
    ctx.stroke();

    // 绘制战术跑位路线
    for (const r of routes) {
      if (r.type === "spot") continue;
      const sx = courtLeft + (r.from[0] / 100) * courtW;
      const sy = courtTop + (r.from[1] / 100) * courtH;
      const ex = courtLeft + (r.to[0] / 100) * courtW;
      const ey = courtTop + (r.to[1] / 100) * courtH;

      ctx.save();
      if (r.style === "dashed" || r.type === "pass") {
        ctx.setLineDash([4, 4]);
        ctx.strokeStyle = "#fbbf24";
        ctx.lineWidth = 1.5;
      } else if (r.type === "screen" || r.type === "back_screen") {
        ctx.strokeStyle = "#ef4444";
        ctx.lineWidth = 2.0;
      } else if (r.type === "dribble") {
        ctx.strokeStyle = "#38bdf8";
        ctx.lineWidth = 2.2;
      } else {
        ctx.strokeStyle = "#10b981";
        ctx.lineWidth = 2.0;
      }

      ctx.beginPath();
      ctx.moveTo(sx, sy);
      ctx.lineTo(ex, ey);
      ctx.stroke();

      // 箭头
      const angleArrow = Math.atan2(ey - sy, ex - sx);
      ctx.beginPath();
      ctx.moveTo(ex, ey);
      ctx.lineTo(ex - 7 * Math.cos(angleArrow - Math.PI / 6), ey - 7 * Math.sin(angleArrow - Math.PI / 6));
      ctx.lineTo(ex - 7 * Math.cos(angleArrow + Math.PI / 6), ey - 7 * Math.sin(angleArrow + Math.PI / 6));
      ctx.fillStyle = ctx.strokeStyle;
      ctx.fill();

      // 掩护 T 形挡板
      if (r.type === "screen" || r.type === "back_screen") {
        ctx.beginPath();
        const perp = angleArrow + Math.PI / 2;
        ctx.moveTo(ex - 6 * Math.cos(perp), ey - 6 * Math.sin(perp));
        ctx.lineTo(ex + 6 * Math.cos(perp), ey + 6 * Math.sin(perp));
        ctx.stroke();
      }

      // 标注
      if (r.label) {
        ctx.font = "9px sans-serif";
        ctx.fillStyle = "rgba(226, 232, 240, 0.85)";
        ctx.fillText(r.label, (sx + ex) / 2 + 4, (sy + ey) / 2);
      }
      ctx.restore();
    }

    // 绘制 5 球员槽位点 (严格按档案几何坐标投影: 0=底线, 47=中圈)
    if (spec?.slots) {
      for (let i = 0; i < spec.slots.length; i++) {
        const slot = spec.slots[i];
        let px = courtLeft + (slot.base_offset_y / 50) * courtW;
        let py = courtTop + (slot.base_offset_x / 47) * courtH;

        // 动画插值移动
        if (progress > 0 && routes[i] && routes[i].type !== "spot") {
          const r = routes[i];
          const ex = courtLeft + (r.to[0] / 100) * courtW;
          const ey = courtTop + (r.to[1] / 100) * courtH;
          px = px + (ex - px) * progress;
          py = py + (ey - py) * progress;
        }

        const isHandler = slot.id === "top" || slot.behaviour === "DribbleTop";

        ctx.save();
        ctx.beginPath();
        ctx.arc(px, py, 11, 0, Math.PI * 2);
        ctx.fillStyle = isHandler ? "#f59e0b" : "#0284c7";
        ctx.fill();
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 1.5;
        ctx.stroke();

        ctx.fillStyle = "#ffffff";
        ctx.font = "bold 9px sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "middle";
        ctx.fillText(String(i + 1), px, py);

        ctx.font = "8.5px sans-serif";
        ctx.fillStyle = "#cbd5e1";
        ctx.fillText(slot.name_zh, px, py + 15);
        ctx.restore();
      }
    }
  }

  function animateChalkboard(canvas, spec, routes) {
    let start = null;
    const duration = 2200;
    function step(timestamp) {
      if (!start) start = timestamp;
      const elapsed = timestamp - start;
      const progress = Math.min(1, elapsed / duration);
      // 正弦缓动
      const ease = 0.5 - 0.5 * Math.cos(progress * Math.PI);
      drawChalkboardRoutes(canvas, spec, routes, ease);
      if (progress < 1) {
        requestAnimationFrame(step);
      } else {
        setTimeout(() => drawChalkboardRoutes(canvas, spec, routes, 0), 1200);
      }
    }
    requestAnimationFrame(step);
  }

  function defenseGroupedRow(items, selected, onPick) {
    const wrap = el("div", "defense-grouped-wrap");

    // 组 1：全队基础防守形态 (Base Scheme)
    const baseIds = ["def_man_conservative", "def_man_pressure", "def_zone_23"];
    const baseItems = items.filter((item) => baseIds.includes(item.id));
    const baseGroup = el("div", "defense-group");
    baseGroup.append(el("div", "tactic-sub-header", "全队基础防守阵型"));
    const baseRow = el("div", "choice-row");
    for (const item of baseItems) {
      const button = el(
        "button",
        `choice-card${item.id === selected ? " active" : ""}`,
        el("strong", null, cleanTacticNameZh(item.name_zh)),
        el("span", "choice-meta", getTacticDescZh(item.id)),
      );
      button.type = "button";
      button.addEventListener("click", () => onPick(item.id));
      baseRow.append(button);
    }
    baseGroup.append(baseRow);

    // 组 2：持球挡拆掩护应对 (Pick-and-Roll Coverage)
    const pnrIds = ["def_drop_coverage", "def_hedge_recover", "def_switch_heavy"];
    const pnrItems = items.filter((item) => pnrIds.includes(item.id));
    const pnrGroup = el("div", "defense-group");
    pnrGroup.append(el("div", "tactic-sub-header", "持球挡拆掩护应对策略"));
    const pnrRow = el("div", "choice-row");
    for (const item of pnrItems) {
      const button = el(
        "button",
        `choice-card${item.id === selected ? " active" : ""}`,
        el("strong", null, cleanTacticNameZh(item.name_zh)),
        el("span", "choice-meta", getTacticDescZh(item.id)),
      );
      button.type = "button";
      button.addEventListener("click", () => onPick(item.id));
      pnrRow.append(button);
    }
    pnrGroup.append(pnrRow);

    wrap.append(baseGroup, pnrGroup);
    return wrap;
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
      button.append(
        el("strong", null, cleanTacticNameZh(item.name_zh)),
        el("span", "choice-meta", item.available === false ? "当前没有落位档案" : getTacticDescZh(item.id)),
      );
      button.addEventListener("click", () => onPick(item.id));
      row.append(button);
    }
    return row;
  }

  function playList(playbook) {
    const list = el("div", "play-list");
    if (!playbook.length) {
      list.append(el("div", "play-note", "当前进攻体系没有可配合的战术。"));
      return list;
    }
    for (const play of playbook) {
      const card = el("article", "play-card active");
      const verbs = (play.rules || [])
        .map((rule) => {
          const actionText = `${getPlayVerbZh(rule.then.verb)} · ${getSlotZh(rule.then.slot)}`;
          const bonus = (rule.carrier_preferences || [])
            .map((p) => `${p.action_family === "Pass" ? "传球意图" : "终结意图"} +${Math.round(p.bonus * 100)}%`)
            .join(" · ");
          return bonus ? `${actionText} (${bonus})` : actionText;
        })
        .join(" / ");
      const triggerNote = (play.triggers || []).length
        ? "时机: 掩护确立或对位移动时触发"
        : "时机: 阵地持球就位自动触发";
      card.append(
        el("strong", null, cleanTacticNameZh(play.name_zh)),
        el("span", "play-note", verbs || "战术配合"),
        el("span", "play-trigger-tag", triggerNote),
      );
      list.append(card);
    }
    return list;
  }

  function playsForTactic(spec) {
    if (!spec) return [];
    const selectedIds = new Set(spec.play_ids || []);
    return (studio.plays || []).filter(
      (play) => selectedIds.has(play.id) && playFits(play, spec),
    );
  }

  function playFits(play, spec) {
    if (!spec) return false;
    const slots = new Set(spec.slots.map((slot) => slot.id));
    return (play.rules || []).every((rule) => slots.has(rule.then.slot));
  }

  function renderRoster() {
    const root = $("rosterStudio");
    if (!root || !studio) return;
    const isHome = state.teamPerspective === "home";
    const team = isHome ? studio.default_setup.home_team : studio.default_setup.away_team;
    const teamPlayers = team.players || [];

    // 若当前选中的球员不属于该队伍，默认切换为该队伍首位球员
    let selected = teamPlayers.find((p) => p.id === selectedPlayerId);
    if (!selected) {
      selected = teamPlayers[0];
      selectedPlayerId = selected?.id || null;
    }

    const list = el("div", "player-list");

    // 首发阵容
    const starters = teamPlayers.filter((p) => p.starter);
    if (starters.length) {
      list.append(el("div", "roster-group-label", "首发阵容"));
      for (const player of starters) {
        list.append(createPlayerButton(player, selected));
      }
    }

    // 轮换替补
    const bench = teamPlayers.filter((p) => !p.starter);
    if (bench.length) {
      list.append(el("div", "roster-group-label", "轮换替补"));
      for (const player of bench) {
        list.append(createPlayerButton(player, selected));
      }
    }

    root.replaceChildren(
      el("div", "roster-layout", list, playerSheet(team, selected))
    );
  }

  function createPlayerButton(player, selected) {
    const button = el(
      "button",
      player.id === selected.id ? "active" : ""
    );
    button.type = "button";
    button.append(
      el("strong", null, `#${player.jersey} ${getPlayerNameZh(player.name)}`),
      el("span", "choice-meta", `${getPositionZh(player.position)} · ${getOffensiveRoleZh(player.offensive_role)}`),
    );
    button.addEventListener("click", () => {
      selectedPlayerId = player.id;
      renderRoster();
    });
    return button;
  }

  function playerSheet(team, player) {
    const sheet = el("article", "player-sheet");
    const identity = el("div", "player-identity");
    const pNameZh = getPlayerNameZh(player.name);
    const tNameZh = getTeamNameZh(team.id) || team.name;
    identity.append(
      el("div", null, el("strong", null, pNameZh), el("div", "choice-meta", tNameZh + " · #" + player.jersey + " · " + player.height_cm + " 厘米 · " + player.weight_kg + " 公斤")),
      el("div", "role-pills",
        el("span", null, getPositionZh(player.position)),
        el("span", null, getOffensiveRoleZh(player.offensive_role)),
        el("span", null, getDefensiveRoleZh(player.defensive_role)),
        el("span", null, player.starter ? "首发主力" : "轮换替补"),
      ),
    );
    sheet.append(
      identity,
      el("div", "choice-meta", "球员关键属性"),
      meters(player.attributes, "attributes"),
      el("div", "choice-meta", "战术决策倾向"),
      meters(player.tendencies, "tendencies"),
      el("div", "choice-meta", "球队作战风格"),
      traitGrid(team.team_traits),
    );
    return sheet;
  }

  function meters(values, group) {
    const list = el("div", "meter-list");
    for (const [key, value] of Object.entries(values)) {
      const ratio = Math.max(0, Math.min(1, Number(value)));
      const bar = el("i", null, el("b"));
      bar.firstChild.style.width = `${Math.round(ratio * 100)}%`;
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
    if (updateEditor) {
      const editor = $("rulesEditor");
      if (editor) editor.value = JSON.stringify(rules, null, 2);
    }
    return rules;
  }

  async function runSimulation(withRules = null, autoPlay = true, startAtLive = true) {
    stopPlayback();
    // 移动端体验：模拟开始时视口保持在球场核心区域，杜绝下滚遮挡
    window.scrollTo({ top: 0, behavior: "smooth" });
    const parsedSeed = Number.parseInt($("seedInput").value, 10);
    const seed = Number.isFinite(parsedSeed) ? parsedSeed : 42;
    const scope = $("scopeInput").value;
    setRunStatus("重新模拟推演中…");
    setControlsBusy(true);
    try {
      if (!withRules) await fetchDefaultRules(false);
      if (!studio) await loadStudio();
      const setup = studio?.default_setup || null;
      let responseText = null;

      // 优先请求后端真实的 Rust MatchEngine 模拟 API，完整消费战术体系、防守模型与剧本
      try {
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
      } catch (apiError) {
        console.warn("后端 API 离线，尝试降级至本地 WASM 运行", apiError);
        const wasm = await ensureWasm();
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
          throw apiError;
        }
      }

      if (withRules) state.rules = { ...state.rules, ...withRules };
      loadStream(responseText, `seed ${seed} · ${scope}`);
      setRunStatus(`${state.ticks.length.toLocaleString()} 帧 · 模拟推演完成`);
      if (startAtLive && state.ticks.length) {
        const liveIdx = state.ticks.findIndex((t) => t.game_flow === "LiveBall" && t.possession_id >= 1);
        if (liveIdx > 0) seek(liveIdx);
      }
      if (autoPlay) {
        startPlayback();
      }
      return true;
    } catch (error) {
      setRunStatus(`运行失败：${error.message}`, true);
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
    let carriedTacticalSet = "";
    let carriedRules = parsed[0]?.rules || {};
    for (const tick of parsed) {
      if (tick.tactical_set) {
        carriedTacticalSet = tick.tactical_set;
      } else if (carriedTacticalSet) {
        tick.tactical_set = carriedTacticalSet;
      }
      if (!tick.rules && carriedRules) {
        tick.rules = carriedRules;
      }
    }
    state.engineViolations = engineViolations;
    state.ticks = parsed;
    state.idx = 0;
    state.accumulator = 0;
    state.previousRaf = 0;
    state.lastFrameJson = -1;
    state.rules = { ...state.rules, ...parsed[0].rules };
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
    renderMatchOverview();
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

    for (let index = 0; index < ticks.length; index += 1) {
      const tick = ticks[index];
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
      for (const name of names) {
        events.push({ index, name, tick });
      }
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
          eventDetails: [],
          retainedRebound: false,
        };
        possessionMap.set(id, possession);
      }
      possession.end = index;
      possession.events.push(...names);
      // 回合内事件时间轴素材：本 tick 新增的领域事件（含参与者 payload）
      if (Array.isArray(tick.event_log)) {
        for (const entry of tick.event_log) {
          possession.eventDetails.push({
            index,
            sequence: entry.sequence ?? 0,
            time: finite(entry.time),
            kind: entry.kind,
            data: entry.data || null,
          });
        }
        possession.eventDetails.sort((a, b) => a.sequence - b.sequence);
      }
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
              ? (pendingShots.length > 0 ? 0 : -1)
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
    }

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
      // 回合叙事字段：进攻方、发起点、结果、得分归属与净胜分变化
      const startTick = ticks[possession.start];
      const endTick = ticks[possession.end];
      possession.offenseTeam =
        possessionTeams.get(possession.id) || startTick.possession_team || "home";
      possession.period = startTick.period || 1;
      const startNames = eventNames(startTick);
      const allNames = possession.events;
      if (startNames.includes("TIPOFF")) possession.origin = "tipoff";
      else if (allNames.includes("STEAL")) possession.origin = "steal";
      else if (allNames.includes("BLOCK")) possession.origin = "block";
      else if (
        allNames.includes("REBOUND") &&
        !possession.retainedRebound
      )
        possession.origin = "defensive_rebound";
      else if (possession.retainedRebound) possession.origin = "offensive_rebound";
      else if (allNames.includes("OUT_OF_BOUNDS")) possession.origin = "inbound";
      else possession.origin = "inbound";
      const scoreBefore = possession.start > 0
        ? ticks[possession.start - 1].score
        : { home: 0, away: 0 };
      possession.scoreBefore = {
        home: finite(scoreBefore?.home),
        away: finite(scoreBefore?.away),
      };
      possession.scoreAfter = {
        home: finite(endTick.score?.home),
        away: finite(endTick.score?.away),
      };
      const homeDelta = possession.scoreAfter.home - possession.scoreBefore.home;
      const awayDelta = possession.scoreAfter.away - possession.scoreBefore.away;
      possession.pointsScored =
        possession.offenseTeam === "home" ? homeDelta : awayDelta;
      const turnoverish = allNames.some((name) =>
        /TURNOVER|STEAL|VIOLATION|OUT_OF_BOUNDS/.test(name) &&
        name !== "OUT_OF_BOUNDS" ||
        (name === "OUT_OF_BOUNDS" &&
          ticks[possession.end].callout?.includes("出界")),
      );
      const shotAttempted = possession.shots > 0 || possession.driveScores > 0;
      if (possession.pointsScored > 0) {
        possession.result = "scored";
      } else if (turnoverish) {
        possession.result = "turnover";
      } else if (shotAttempted) {
        possession.result = "missed";
      } else if (allNames.includes("FOUL")) {
        possession.result = "foul";
      } else {
        possession.result = "other";
      }
      possession.turnovers = allNames.filter((name) =>
        /TURNOVER|STEAL/.test(name),
      ).length;
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
      kind: `${violation.rule}${violation.severity ? ` · ${violation.severity}` : ""}`,
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
    headerRow.style.fontSize = "11px";
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
      for (const name of names) {
        if (!defaultHidden.has(name)) state.filters.add(name);
      }
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
        button.textContent = `#${item.index} · ${getAnomalyKindZh(item.kind)} · ${item.detail}`;
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
  }
  function setRunStatus(text, error = false) {
    $("runStatus").textContent = text;
    $("runStatus").style.color = error ? "var(--color-urgent)" : "";
  }

  function renderCurrent(forceJson = false) {
    const tick = state.ticks[state.idx];
    if (!tick) return;
    updateHud(tick);
    drawCourt(tick);
    updateTimelineCursor();
    updateMatchFlowCursor();
    $("progressInput").value = String(state.idx);
    $("jumpInput").value = String(state.idx);
    $("tickReadout").textContent =
      `${state.idx.toLocaleString()} / ${state.ticks.length.toLocaleString()} 帧`;
    $("progressTime").textContent = timeWithTenths(tick.t);
    if (forceJson || !state.playing || state.idx % 3 === 0)
      renderFrameJson(tick);
    renderDecision(tick);
  }
  function updateHud(tick) {
    const homeTeam = tick.home_team || {};
    const awayTeam = tick.away_team || {};
    $("homeTeamName").textContent = getTeamNameZh(homeTeam);
    $("awayTeamName").textContent = getTeamNameZh(awayTeam);
    const isHomePossession = tick.possession_team === "home" || tick.possession_id % 2 === 1;
    const currentOffenseTactic = getTacticsZh(tick.tactical_set) || "半场战术体系";
    // 进攻标签展示持球方战术，防守标签展示对位方的防守策略
    const homeTacticEl = $("homeTacticTag");
    const awayTacticEl = $("awayTacticTag");
    const homeDefEl = $("homeDefTag");
    const awayDefEl = $("awayDefTag");
    const defensiveTactic = cleanTacticNameZh(tick.defensive_tactic) || "沉退防守";
    if (homeTacticEl && awayTacticEl) {
      if (isHomePossession) {
        homeTacticEl.textContent = `攻 · ${currentOffenseTactic}`;
        awayTacticEl.textContent = `攻 · —`;
      } else {
        awayTacticEl.textContent = `攻 · ${currentOffenseTactic}`;
        homeTacticEl.textContent = `攻 · —`;
      }
    }
    if (homeDefEl && awayDefEl) {
      // defensive_tactic 是当前防守方（对方）的战术名，归位到对侧
      if (isHomePossession) {
        awayDefEl.textContent = `守 · ${defensiveTactic}`;
        homeDefEl.textContent = `守 · —`;
      } else {
        homeDefEl.textContent = `守 · ${defensiveTactic}`;
        awayDefEl.textContent = `守 · —`;
      }
    }
    const allNames = eventNames(tick);
    const highlightNames = allNames.filter((name) =>
      HIGHLIGHT_EVENTS.has(name),
    );
    const overlay = $("eventOverlayChip");
    if (highlightNames.length) {
      const txt = highlightNames.map(getEventNameZh).join(" · ");
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
    // 犯规数各归各队块
    $("homeFouls").textContent = String(tick.team_fouls_home ?? 0);
    $("awayFouls").textContent = String(tick.team_fouls_away ?? 0);
    // 罚球提示仅在有剩余罚球时出现
    const ftChip = $("freeThrowChip");
    const ftRemaining = Number(tick.free_throws_remaining ?? 0);
    if (ftChip) {
      ftChip.hidden = !(ftRemaining > 0);
      ftChip.textContent = `罚球 ${ftRemaining}`;
    }
    const callout = formatCalloutZh(tick.callout) || "—";
    if ($("calloutText").textContent !== callout) {
      $("calloutText").textContent = callout;
      if (callout !== "—") {
        $("calloutText").classList.remove("callout-flash");
        void $("calloutText").offsetWidth;
        $("calloutText").classList.add("callout-flash");
      }
    }
    updateFlowState(tick);
  }

  const GAME_FLOW_ZH = {
    Pregame: "赛前",
    TipOff: "跳球",
    LiveBall: "活球",
    DeadBall: "死球",
    Timeout: "暂停",
    FreeThrow: "罚球中",
    QuarterEnd: "节末",
    Halftime: "半场休息",
    Overtime: "加时",
    GameEnd: "全场结束",
  };
  const SUB_PHASE_ZH = {
    Initiation: "回合发起",
    ActionExecution: "阵地战术",
    ShotAttempt: "出手飞行",
    FlightAndRebound: "篮板争抢",
    DeadBallReset: "死球重置",
  };
  // 字幕条状态词：game_flow 非 LiveBall 时显示流程状态，活球时显示回合内子阶段
  function updateFlowState(tick) {
    const el = $("flowStateLabel");
    if (!el) return;
    const flow = String(tick.game_flow || "LiveBall").replace(/^.*\("?/, "").replace(/"?\).*$/, "");
    const isDead = flow !== "LiveBall";
    if (isDead) {
      el.textContent = GAME_FLOW_ZH[flow] || flow;
    } else {
      el.textContent = SUB_PHASE_ZH[tick.phase] || getPhaseZh(tick.phase);
    }
    el.classList.toggle("dead", isDead);
    el.classList.toggle("idle", !state.ticks.length);
  }

  function renderFrameJson(tick) {
    if (state.lastFrameJson === state.idx) return;
    $("frameJson").textContent = JSON.stringify(tick, null, 2);
    state.lastFrameJson = state.idx;
  }

  // ========================================================
  // 比赛总览：回合流水线 + 校验与判罚汇总
  // ========================================================
  const ORIGIN_ZH = {
    tipoff: "跳球",
    inbound: "发球",
    steal: "抢断",
    block: "盖帽",
    defensive_rebound: "防守篮板",
    offensive_rebound: "补篮二次进攻",
  };
  const RESULT_ZH = {
    scored: "得分",
    missed: "出手未中",
    turnover: "球权转换",
    foul: "犯规终止",
    other: "结束",
  };
  // 回合内事件时间轴：按 event_log payload 生成可读描述
  function describeFlowEvent(entry) {
    const payload = entry.data ? entry.data[Object.keys(entry.data)[0]] : null;
    const name = (id) => playerNameFromId(id);
    switch (entry.kind) {
      case "TIPOFF": return "中圈跳球";
      case "TIPOFF_SECURED": return "跳球获得球权";
      case "PLAY_ACTIVATED": {
        const playId = payload?.play_id || "";
        const play = studio?.plays?.find((p) => p.id === playId);
        return `启动战术 · ${cleanTacticNameZh(play?.name_zh) || playId}`;
      }
      case "PASS": return `${name(payload?.passer_id)} 传给 ${name(payload?.receiver_id)}`;
      case "PASS_RECEIVED": return `${name(payload?.receiver_id)} 接球`;
      case "PASS_DROPPED": return `${name(payload?.receiver_id)} 没接住 ${name(payload?.passer_id)} 的传球`;
      case "PASS_LANDING_CORRECTED": return `传球落点修正 · 偏差 ${one(finite(payload?.divergence_ft))} 英尺`;
      case "SHOT_RELEASE": {
        const three = payload?.is_three ? "三分出手" : "出手";
        return `${name(payload?.shooter_id)} ${three} · 命中率 ${pct(finite(payload?.make_probability))}`;
      }
      case "SCORE": {
        const three = payload?.is_three ? "三分命中" : "命中";
        return `${name(payload?.shooter_id)} ${three}`;
      }
      case "SHOT_MISS": return `${name(payload?.shooter_id)} 出手未中`;
      case "REBOUND": return `${name(payload?.rebounder_id)} 抢下${payload?.is_offensive ? "进攻" : "防守"}篮板`;
      case "DRIVE_INITIATED": return `${name(payload?.driver_id)} 持球突破`;
      case "DRIVE_SCORE": return `${name(payload?.driver_id)} 突破得分`;
      case "DRIVE_STOPPED": return `${name(payload?.driver_id)} 突破被阻截`;
      case "FOUL": return `${name(payload?.fouler_id)} 犯规 · ${name(payload?.fouled_player_id)} 被犯${payload?.is_shooting ? "（投篮动作）" : ""}`;
      case "BALL_POKED_LOOSE": return `${name(payload?.defender_id)} 拍掉 ${name(payload?.handler_id)} 的球`;
      case "LOOSE_BALL_SECURED": return `${name(payload?.player_id)} 控制活球`;
      case "OUT_OF_BOUNDS": return `球出界 · ${name(payload?.responsible_player_id)} 责任`;
      case "SCREEN_CONTACT": {
        const legal = payload?.legal_position ? "合法掩护" : "掩护接触";
        return `${name(payload?.player_a)} 与 ${name(payload?.player_b)} ${legal}`;
      }
      case "CONTACT_BUMP": return `${name(payload?.player_a)} 与 ${name(payload?.player_b)} 身体对抗`;
      case "BALL_ORIENTATION": return `${name(payload?.player_id)} 调整背身朝向`;
      case "ACTION_WINDOW_SHIFT": return `${name(payload?.player_id)} ${getActionZh(payload?.action_type)}`;
      case "PHASE_TRANSITION": return `阶段流转：${FLOW_PHASE_ZH[payload?.from] || payload?.from} → ${FLOW_PHASE_ZH[payload?.to] || payload?.to}`;
      case "PLACEMENT_APPLIED": return `${name(payload?.player_id)} 落位调整（${FLOW_PHASE_ZH[payload?.phase] || payload?.phase}）`;
      case "POSSESSION_SUMMARY": {
        const parts = [`回合结束 · ${one(finite(payload?.duration_seconds))} 秒`];
        if (payload?.shooter_id) parts.push(`最后出手 ${name(payload.shooter_id)}`);
        if (payload?.turnover_player_id) parts.push(`失误 ${name(payload.turnover_player_id)}`);
        return parts.join(" · ");
      }
      default: return getEventNameZh(entry.kind) || entry.kind;
    }
  }
  const FLOW_PHASE_ZH = {
    SetPlay: "阵地战术",
    Transition: "转换进攻",
    DeadBall: "死球",
    Inbound: "发球",
    FreeThrow: "罚球",
    SetPlayExecution: "战术执行",
    DeadBallReset: "死球重置",
    Initiation: "回合发起",
    ActionExecution: "阵地战术",
    ShotAttempt: "出手飞行",
    FlightAndRebound: "篮板争抢",
  };
  // 回合内事件重要性分级：核心事件始终展示，底层物理事件折叠
  const FLOW_EVENT_MAJOR = new Set([
    "TIPOFF", "TIPOFF_SECURED", "PLAY_ACTIVATED", "PASS", "PASS_DROPPED",
    "SHOT_RELEASE", "SCORE", "SHOT_MISS", "REBOUND", "DRIVE_INITIATED",
    "DRIVE_SCORE", "DRIVE_STOPPED", "FOUL", "BALL_POKED_LOOSE",
    "LOOSE_BALL_SECURED", "OUT_OF_BOUNDS", "POSSESSION_SUMMARY",
  ]);
  const expandedPossessions = new Set();
  function togglePossessionExpand(possessionId) {
    if (expandedPossessions.has(possessionId)) expandedPossessions.delete(possessionId);
    else expandedPossessions.add(possessionId);
    renderMatchOverview();
  }
  function renderMatchOverview() {
    const list = $("matchFlowList");
    if (!list) return;
    const possessions = state.possessions || [];
    if (!possessions.length) {
      list.replaceChildren(el("div", "empty-state", "等待推演开始…"));
      renderValidationSummary();
      return;
    }
    const fragment = document.createDocumentFragment();
    for (const possession of possessions) {
      const row = el("button", `match-flow-row ${possession.result === "scored" ? "flow-scored" : possession.result === "turnover" ? "flow-turnover" : ""} ${expandedPossessions.has(possession.id) ? "flow-expanded" : ""}`);
      row.type = "button";
      row.dataset.possessionId = String(possession.id);
      row.dataset.index = String(possession.start);
      row.addEventListener("click", () => togglePossessionExpand(possession.id));

      const offenseTeam = possession.offenseTeam === "home"
        ? $("homeTeamName")?.textContent || "主队"
        : $("awayTeamName")?.textContent || "客队";
      const offenseSide = el("span", `flow-offense ${possession.offenseTeam === "home" ? "home-dot-text" : "away-dot-text"}`, offenseTeam);

      const idEl = el("span", "flow-id", `#${possession.id}`);
      const originEl = el("span", "flow-origin", ORIGIN_ZH[possession.origin] || possession.origin);
      const statEl = el("span", "flow-stat", `${possession.passes} 传 · ${possession.shots} 投 · ${one(possession.duration)} 秒`);
      const resultEl = el("span", `flow-result result-${possession.result}`,
        possession.pointsScored > 0 ? `${RESULT_ZH.scored} +${possession.pointsScored}` : RESULT_ZH[possession.result] || "结束");
      const scoreEl = el("span", "flow-score",
        `${possession.scoreAfter.away}:${possession.scoreAfter.home}`);

      row.append(idEl, offenseSide, originEl, statEl, resultEl, scoreEl);
      fragment.append(row);

      // 展开态：回合内事件时间轴
      if (expandedPossessions.has(possession.id)) {
        fragment.append(buildPossessionTimeline(possession));
      }
    }
    list.replaceChildren(fragment);
    updateMatchFlowCursor();
    renderValidationSummary();
  }
  function buildPossessionTimeline(possession) {
    const wrap = el("div", "flow-timeline");
    const details = possession.eventDetails || [];
    if (!details.length) {
      wrap.append(el("div", "flow-timeline-empty muted-label", "该回合没有记录到领域事件"));
      return wrap;
    }
    const majors = details.filter((entry) => FLOW_EVENT_MAJOR.has(entry.kind));
    const minors = details.filter((entry) => !FLOW_EVENT_MAJOR.has(entry.kind));
    const visible = expandedPossessions.has(`details:${possession.id}`) ? details : majors;

    const line = el("div", "flow-timeline-list");
    let lastTime = null;
    for (const entry of visible) {
      const item = el("button", `flow-event kind-${entry.kind.toLowerCase().replaceAll("_", "-")}`);
      item.type = "button";
      item.dataset.index = String(entry.index);
      item.title = "点击跳到该事件所在帧";
      // 时刻列：与上一事件同秒则留空避免重复
      const clock = String(Math.floor(entry.time));
      const timeEl = el("span", "flow-event-time", clock === String(lastTime) ? "" : `${clock}s`);
      lastTime = Math.floor(entry.time);
      item.append(
        timeEl,
        el("span", `flow-event-dot major-${FLOW_EVENT_MAJOR.has(entry.kind)}`),
        el("span", "flow-event-text", describeFlowEvent(entry)),
      );
      item.addEventListener("click", (event) => {
        event.stopPropagation();
        seek(entry.index);
      });
      line.append(item);
    }
    wrap.append(line);

    if (minors.length) {
      const detailOpen = expandedPossessions.has(`details:${possession.id}`);
      const toggle = el("button", "flow-timeline-toggle", detailOpen
        ? `收起 ${minors.length} 条底层物理事件`
        : `展开 ${minors.length} 条底层物理事件（接触/球位/落位调整）`);
      toggle.type = "button";
      toggle.addEventListener("click", (event) => {
        event.stopPropagation();
        if (detailOpen) expandedPossessions.delete(`details:${possession.id}`);
        else expandedPossessions.add(`details:${possession.id}`);
        renderMatchOverview();
      });
      wrap.append(toggle);
    }
    return wrap;
  }
  function updateMatchFlowCursor() {
    const rows = $("matchFlowList")?.querySelectorAll(".match-flow-row") || [];
    let current = null;
    for (const row of rows) {
      const active = Number(row.dataset.index) <= state.idx;
      row.classList.toggle("flow-past", active);
      row.classList.remove("flow-current");
      if (active) current = row;
    }
    if (current) current.classList.add("flow-current");
    // 展开的时间轴内：当前帧所在事件及之前的事件标为已发生，当前帧命中行高亮
    const events = $("matchFlowList")?.querySelectorAll(".flow-event") || [];
    for (const item of events) {
      const entryIdx = Number(item.dataset.index);
      item.classList.toggle("flow-event-past", entryIdx <= state.idx);
      item.classList.remove("flow-event-current");
    }
    let currentEvent = null;
    for (const item of events) {
      if (Number(item.dataset.index) <= state.idx) currentEvent = item;
    }
    if (currentEvent) currentEvent.classList.add("flow-event-current");
  }
  function renderValidationSummary() {
    const box = $("validationSummary");
    if (!box) return;
    const stats = state.analytics?.stats;
    const anomalies = state.anomalies || [];
    const possessions = state.possessions || [];
    if (!stats || !possessions.length) {
      box.replaceChildren(el("span", "muted-label", "等待推演开始…"));
      return;
    }
    const turnoverCount = possessions.filter((p) => p.result === "turnover").length;
    const scoredCount = possessions.filter((p) => p.result === "scored").length;
    const cards = [
      { label: "引擎不变量违规", value: anomalies.length, tone: anomalies.length ? "bad" : "good", hint: anomalies.length ? "点击违规监测查看详情" : "本轮推演全部校验通过" },
      { label: "犯规", value: stats.fouls, tone: stats.fouls > 0 ? "warn" : "good", hint: `每回合 ${two(stats.foulRate)} 次` },
      { label: "失误与违例", value: stats.turnovers ?? turnoverCount, tone: "neutral", hint: `占回合 ${pct(stats.turnoverRate)}` },
      { label: "回合成功率", value: possessions.length ? pct(scoredCount / possessions.length) : "—", tone: "good", hint: `${scoredCount}/${possessions.length} 回合得分` },
    ];
    const grid = el("div", "validation-grid");
    for (const card of cards) {
      const item = el("div", `validation-card tone-${card.tone}`);
      item.append(
        el("span", "validation-label", card.label),
        el("strong", "validation-value", String(card.value)),
        el("span", "validation-hint", card.hint),
      );
      grid.append(item);
    }
    box.replaceChildren(grid);
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
        const sx =
          tick.ball || !shooter
            ? ballFtX
            : finite(shooter.x) * rules.courtWidth;
        const sy =
          tick.ball || !shooter
            ? ballFtY
            : finite(shooter.y) * rules.courtHeight;
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

    // 1. 赛场外围环带 (Arena Apron / Perimeter - 清新素雅运动质感)
    ctx.fillStyle = "#edf1f7";
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
    ctx.fillStyle = "#d97706";
    ctx.fillText(awayTeamName, 0, 0);
    ctx.restore();

    // 右侧底线外主队字样
    ctx.save();
    ctx.translate(985, 280);
    ctx.rotate(Math.PI / 2);
    ctx.fillStyle = "#059669";
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
            ? "rgba(16, 185, 129, 0.75)"
            : "rgba(245, 158, 11, 0.75)";
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
      // 5. 真实战术板球员动作动力学渲染 (Pro Tactical Player Kinetics)
      // 脚踏实地自然跑位，动作姿态清晰鲜明，无悬浮飞行
      // ========================================================
      const isHome = player.team === "home";
      const px = playerPoint.x;
      const py = playerPoint.y;
      let radius = 14.5;

      // 提取动作意图与动作窗口阶段
      const actionRaw = String(player.action || "").toUpperCase();
      const actionPhase = String(player.action_phase || "Execution");

      const isShooting = actionRaw.includes("SHOT") || actionRaw.includes("3PT") || actionRaw.includes("LAYUP");
      const isCrossover = actionRaw.includes("CROSSOVER") || actionRaw.includes("BETWEENTHELEGS");
      const isDribbleDrive = isCrossover || actionRaw.includes("DRIVE") || actionRaw.includes("ADVANCE") || actionRaw.includes("INITIATE") || actionRaw.includes("DRIBBLE");
      const isScreen = actionRaw.includes("SCREEN") || actionRaw.includes("SET_HIGH_SCREEN");
      const isContest = actionRaw.includes("CONTEST") || actionRaw.includes("DROP_CONTAIN") || actionRaw.includes("HELP_SIDE") || actionRaw.includes("CLOSEOUT");
      // 步频周期震荡（支撑高速跑动与运球动感节奏）
      const gaitCycle = curSpeed > 0.4 ? (state.idx * 0.45 + (Number(player.number) || 1) * 1.5) : 0;
      const gaitPulse = Math.sin(gaitCycle);

      // 1. 身体朝向角 (Facing Angle) 与动态锁定
      const isBackToBasket = player.hasBall && player.orientation === "back_to_basket";
      let facingAngle = 0;
      if (isShooting) {
        // 投篮动作严格朝向进攻目标篮筐
        const targetHoopX = isHome ? rules.rightHoopX : rules.leftHoopX;
        facingAngle = Math.atan2(
          rules.hoopY - finite(player.y) * rules.courtHeight,
          targetHoopX - finite(player.x) * rules.courtWidth,
        );
      } else if (isContest) {
        // 防守压迫时优先朝向对方持球人
        const carrier = (tick.players || []).find((p) => p.hasBall && p.onCourt !== false);
        if (carrier && carrier.id !== player.id) {
          const carrierPt = point(
            finite(carrier.x) * rules.courtWidth,
            finite(carrier.y) * rules.courtHeight,
          );
          facingAngle = Math.atan2(carrierPt.y - py, carrierPt.x - px);
        } else if (
          player.facing_x !== undefined &&
          player.facing_y !== undefined &&
          Math.hypot(player.facing_x, player.facing_y) > 0.05
        ) {
          facingAngle = Math.atan2(player.facing_y, player.facing_x);
        } else {
          const hoopX = isHome ? rules.rightHoopX : rules.leftHoopX;
          facingAngle = Math.atan2(
            rules.hoopY - finite(player.y) * rules.courtHeight,
            hoopX - finite(player.x) * rules.courtWidth,
          );
        }
      } else if (isBackToBasket && player.facing_x !== undefined && player.facing_y !== undefined && Math.hypot(player.facing_x, player.facing_y) > 0.05) {
        // 背身持球人的朝向被姿态锚定（背对篮筐面向传球侧），
        // 引擎下发的 facing 向量就是权威姿态朝向。
        facingAngle = Math.atan2(player.facing_y, player.facing_x);
      } else if (
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

      // ========================================================
      // 真实三维起跳腾空动力学系统 (Aerial Elevation & Jump Kinetics)
      // 覆盖投篮、上篮、扣篮、抢篮板、盖帽、抢断突扑等核心发力动作
      // ========================================================
      const isDunk = actionRaw.includes("DUNK");
      const isLayup = actionRaw.includes("LAYUP") || actionRaw.includes("FLOATER") || actionRaw.includes("HOOK");
      const isJumpShot = actionRaw.includes("JUMP") || actionRaw.includes("3PT") || actionRaw.includes("SHOT") || actionRaw.includes("PULLUP");
      const isBlock = actionRaw.includes("BLOCK") || (actionRaw.includes("HELP") && actionPhase === "Execution");
      const isReboundJump = actionRaw.includes("REBOUND") || actionRaw.includes("CRASH");
      const isStealLunge = actionRaw.includes("STEAL") || actionRaw.includes("POKE") || actionRaw.includes("LOOSE");
      const isJumpAction = isDunk || isLayup || isJumpShot || isBlock || isReboundJump || isStealLunge;

      let jumpHeight = 0; // 0.0 ~ 1.0 相对腾空高度
      if (isJumpAction) {
        let maxLift = 0.85;
        if (isDunk) maxLift = 1.0;
        else if (isBlock) maxLift = 0.95;
        else if (isReboundJump) maxLift = 0.9;
        else if (isJumpShot) maxLift = 0.85;
        else if (isLayup) maxLift = 0.8;
        else if (isStealLunge) maxLift = 0.65;

        if (actionPhase === "Preparation") {
          // 起跳准备：屈膝蓄力，身体下沉
          radius -= 1.4;
          jumpHeight = 0;
        } else if (actionPhase === "Execution") {
          // 腾空发力：达到起跳峰值高度
          jumpHeight = maxLift;
        } else if (actionPhase === "FollowThrough") {
          // 落地缓冲：下落着地过程
          jumpHeight = maxLift * 0.35;
        } else {
          jumpHeight = maxLift * 0.68;
        }
      }

      // 透视垂直位移与体量尺寸透视放大：位移量级必须达到「一个身位」
      // （全场视角下 11px 无感知，30px ≈ 1.8 倍圆盘半径才显著）
      const verticalLift = jumpHeight * 30;
      const renderPx = px;
      const renderPy = py - verticalLift; // 腾空后的圆盘中心
      if (jumpHeight > 0) {
        radius = radius * (1.0 + jumpHeight * 0.45); // 透视放大（扈篮是 1.45x）
      }

      // 2. 地面自然接触阴影 (起跳时阴影留在地面原点，并随高度扩散淡化)
      ctx.beginPath();
      const shadowSpread = 1.0 + jumpHeight * 1.1;
      const shadowStretch = Math.min(1.25, 1.0 + curSpeed * 0.04);
      ctx.ellipse(
        px,
        py + 2,
        radius * 0.95 * shadowStretch * shadowSpread,
        radius * 0.46 * shadowSpread,
        curSpeed > 0.4 ? Math.atan2(curVy, curVx) : 0,
        0,
        Math.PI * 2,
      );
      const shadowAlpha = Math.max(0.08, 0.24 * (1.0 - jumpHeight * 0.55));
      ctx.fillStyle = `rgba(0, 0, 0, ${shadowAlpha.toFixed(2)})`;
      ctx.fill();

      // 2.1 地面起跳落点锚环与垂直落差轴线 (Ground Anchor Ring & Elevation Drop Line)
      if (jumpHeight > 0.25) {
        ctx.save();
        // 地面垂直落差导引线 (从地面原点连向空中球员中心)
        ctx.setLineDash([2, 3]);
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.45)" : "rgba(245, 158, 11, 0.45)";
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.moveTo(px, py + 2);
        ctx.lineTo(renderPx, renderPy);
        ctx.stroke();

        // 地面起跳落点锚环 (清晰指示起跳位置与身体落点)
        ctx.setLineDash([]);
        ctx.beginPath();
        ctx.ellipse(px, py + 2, 7.5 * shadowSpread, 3.8 * shadowSpread, 0, 0, Math.PI * 2);
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.65)" : "rgba(245, 158, 11, 0.65)";
        ctx.lineWidth = 1.3;
        ctx.stroke();
        ctx.restore();
      }

      // 3. 掩护挡拆基座动作 (Screen Base Stance)
      if (isScreen) {
        ctx.save();
        ctx.translate(renderPx, renderPy);
        ctx.rotate(facingAngle);
        ctx.fillStyle = isHome ? "#042c16" : "#421801";
        ctx.strokeStyle = isHome ? "#10b981" : "#f59e0b";
        ctx.lineWidth = 1.8;
        ctx.beginPath();
        ctx.roundRect(-4, -radius - 4, 8, (radius + 4) * 2, 3);
        ctx.fill();
        ctx.stroke();
        ctx.restore();
      }

      // 4. 防守压迫干扰罩动作 (Defensive Contest Envelope)
      if (isContest) {
        ctx.save();
        ctx.translate(renderPx, renderPy);
        ctx.rotate(facingAngle);
        const contestR = actionPhase === "Execution" ? radius + 11 : radius + 7;
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.85)" : "rgba(245, 158, 11, 0.85)";
        ctx.lineWidth = actionPhase === "Execution" ? 3.0 : 2.0;
        ctx.beginPath();
        ctx.arc(0, 0, contestR, -Math.PI / 3, Math.PI / 3);
        ctx.stroke();

        // 强力扑防干扰触点 (封盖伸臂位)
        if (actionPhase === "Execution") {
          const capAng1 = -Math.PI / 3;
          const capAng2 = Math.PI / 3;
          ctx.fillStyle = isHome ? "#10b981" : "#f59e0b";
          ctx.beginPath();
          ctx.arc(Math.cos(capAng1) * contestR, Math.sin(capAng1) * contestR, 2.0, 0, Math.PI * 2);
          ctx.arc(Math.cos(capAng2) * contestR, Math.sin(capAng2) * contestR, 2.0, 0, Math.PI * 2);
          ctx.fill();
        }
        ctx.restore();
      }

      // 5. 篮板卡位阻隔弧动作 (Box-Out Wall Barrier)
      if (actionRaw.includes("BOXOUT")) {
        ctx.save();
        ctx.translate(renderPx, renderPy);
        ctx.rotate(facingAngle + Math.PI); // 朝向后方阻隔
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.75)" : "rgba(245, 158, 11, 0.75)";
        ctx.lineWidth = 2.0;
        ctx.beginPath();
        ctx.arc(0, 0, radius + 4.5, -Math.PI * 0.38, Math.PI * 0.38);
        ctx.stroke();
        ctx.restore();
      }

      // 6. 投篮瞄准与跟随压腕动作 (Shooting Aim & Follow-Through)
      if (isShooting) {
        ctx.save();
        ctx.translate(renderPx, renderPy);
        ctx.rotate(facingAngle);

        if (actionPhase === "Execution") {
          // 出手瞬间瞄准导引线（加长到 22px，全场可辨）
          ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.9)" : "rgba(245, 158, 11, 0.9)";
          ctx.lineWidth = 2.4;
          ctx.beginPath();
          ctx.moveTo(radius, 0);
          ctx.lineTo(radius + 22, 0);
          ctx.stroke();
          // 出手准星导向标
          ctx.fillStyle = isHome ? "#10b981" : "#f59e0b";
          ctx.beginPath();
          ctx.moveTo(radius + 27, 0);
          ctx.lineTo(radius + 19, -4);
          ctx.lineTo(radius + 19, 4);
          ctx.closePath();
          ctx.fill();
        } else if (actionPhase === "FollowThrough") {
          // 压腕保持跟随指针
          ctx.fillStyle = isHome ? "#10b981" : "#f59e0b";
          ctx.beginPath();
          ctx.moveTo(radius + 7, 0);
          ctx.lineTo(radius + 1, -2);
          ctx.lineTo(radius + 1, 2);
          ctx.closePath();
          ctx.fill();
        }
        ctx.restore();
      }

      // 6.5 攻击与防守动作专属轨迹标记（尺寸 ≥ 半径级，全场视角可辨认）
      // 变向突破：地面 V 形折线残影（旧运动方向 → 新朝向的切向证据）
      if (isCrossover && actionPhase === "Execution" && curSpeed > 0.4) {
        const moveAng = Math.atan2(curVy, curVx);
        ctx.save();
        ctx.translate(px, py);
        ctx.strokeStyle = isHome ? "rgba(5, 150, 105, 0.8)" : "rgba(217, 119, 6, 0.8)";
        ctx.lineWidth = 2.6;
        ctx.beginPath();
        ctx.moveTo(0, 0);
        ctx.lineTo(Math.cos(moveAng) * (radius + 9), Math.sin(moveAng) * (radius + 9));
        ctx.stroke();
        const headX = Math.cos(facingAngle) * (radius + 12);
        const headY = Math.sin(facingAngle) * (radius + 12);
        ctx.beginPath();
        ctx.moveTo(Math.cos(facingAngle) * (radius + 2), Math.sin(facingAngle) * (radius + 2));
        ctx.lineTo(headX, headY);
        ctx.stroke();
        ctx.fillStyle = isHome ? "#059669" : "#d97706";
        ctx.beginPath();
        ctx.moveTo(headX + Math.cos(facingAngle) * 6, headY + Math.sin(facingAngle) * 6);
        ctx.lineTo(headX + Math.cos(facingAngle + 2.5) * 6, headY + Math.sin(facingAngle + 2.5) * 6);
        ctx.lineTo(headX + Math.cos(facingAngle - 2.5) * 6, headY + Math.sin(facingAngle - 2.5) * 6);
        ctx.closePath();
        ctx.fill();
        ctx.restore();
      }

      // 三威胁：脚下三向箭头扇形（可投 / 可传 / 可突三个选项同时在线）
      if (actionRaw.includes("TRIPLETHREAT")) {
        ctx.save();
        ctx.translate(px, py);
        ctx.rotate(facingAngle);
        const threatColor = isHome ? "rgba(5, 150, 105, 0.8)" : "rgba(217, 119, 6, 0.8)";
        for (const off of [-0.6, 0, 0.6]) {
          const tipX = Math.cos(off) * (radius + 12);
          const tipY = Math.sin(off) * (radius + 12);
          ctx.strokeStyle = threatColor;
          ctx.lineWidth = 2.2;
          ctx.beginPath();
          ctx.moveTo(Math.cos(off) * (radius + 3), Math.sin(off) * (radius + 3));
          ctx.lineTo(tipX, tipY);
          ctx.stroke();
          ctx.fillStyle = threatColor;
          ctx.beginPath();
          ctx.moveTo(tipX + Math.cos(off) * 5, tipY + Math.sin(off) * 5);
          ctx.lineTo(tipX + Math.cos(off + 2.4) * 5, tipY + Math.sin(off + 2.4) * 5);
          ctx.lineTo(tipX + Math.cos(off - 2.4) * 5, tipY + Math.sin(off - 2.4) * 5);
          ctx.closePath();
          ctx.fill();
        }
        ctx.restore();
      }

      // 上篮：朝篮筐的低平抛物虚线弧（腾空期间持续显示）
      if (isLayup && jumpHeight > 0.2) {
        const hoopX = isHome ? rules.rightHoopX : rules.leftHoopX;
        ctx.save();
        ctx.setLineDash([4, 4]);
        ctx.strokeStyle = "rgba(249, 115, 22, 0.85)";
        ctx.lineWidth = 2.2;
        ctx.beginPath();
        ctx.moveTo(px, py);
        ctx.quadraticCurveTo((px + hoopX) / 2, py - 12, hoopX, rules.hoopY);
        ctx.stroke();
        ctx.restore();
      }

      // 扣篮：起跳点到篮筐的直线冲刺轨迹（力度感与上篮的抛物线对立）
      if (isDunk && jumpHeight > 0.2) {
        const hoopX = isHome ? rules.rightHoopX : rules.leftHoopX;
        ctx.save();
        ctx.setLineDash([7, 4]);
        ctx.strokeStyle = "rgba(234, 88, 12, 0.9)";
        ctx.lineWidth = 3.0;
        ctx.beginPath();
        ctx.moveTo(px, py);
        ctx.lineTo(hoopX, rules.hoopY);
        ctx.stroke();
        ctx.restore();
      }

      // 盖帽：腾空伸臂扑球（身体伸向朝向方向的粗臂条 + 掌端圆）
      if (isBlock && jumpHeight > 0.2) {
        ctx.save();
        ctx.translate(renderPx, renderPy);
        ctx.rotate(facingAngle);
        ctx.strokeStyle = isHome ? "rgba(5, 150, 105, 0.9)" : "rgba(217, 119, 6, 0.9)";
        ctx.lineWidth = 4.0;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.moveTo(radius * 0.5, 0);
        ctx.lineTo(radius * 1.75, 0);
        ctx.stroke();
        ctx.fillStyle = isHome ? "#059669" : "#d97706";
        ctx.beginPath();
        ctx.arc(radius * 1.75, 0, 3.2, 0, Math.PI * 2);
        ctx.fill();
        ctx.restore();
      }

      // 篮板争抢：身体上方双臂上举标记
      if (isReboundJump && jumpHeight > 0.25) {
        ctx.save();
        ctx.strokeStyle = isHome ? "rgba(5, 150, 105, 0.85)" : "rgba(217, 119, 6, 0.85)";
        ctx.lineWidth = 2.8;
        ctx.lineCap = "round";
        ctx.beginPath();
        ctx.moveTo(renderPx - 4, renderPy - radius - 1);
        ctx.lineTo(renderPx - 4, renderPy - radius - 9);
        ctx.moveTo(renderPx + 4, renderPy - radius - 1);
        ctx.lineTo(renderPx + 4, renderPy - radius - 9);
        ctx.stroke();
        ctx.restore();
      }

      // 7. 运球触地节奏脉冲 (Dribble Bounce Cadence)
      if (player.hasBall && isDribbleDrive && curSpeed > 0.5) {
        const handSide = gaitPulse >= 0 ? 1 : -1;
        const dribbleAngle = facingAngle + handSide * 0.42;
        const dribbleDist = radius + 6.0;
        const dx = px + Math.cos(dribbleAngle) * dribbleDist;
        const dy = py + Math.sin(dribbleAngle) * dribbleDist;
        const bounceR = 2.5 + Math.abs(gaitPulse) * 3.0;

        ctx.save();
        ctx.beginPath();
        ctx.arc(dx, dy, bounceR, 0, Math.PI * 2);
        ctx.strokeStyle = `rgba(255, 120, 40, ${(0.55 - Math.abs(gaitPulse) * 0.25).toFixed(2)})`;
        ctx.lineWidth = 1.3;
        ctx.stroke();
        ctx.restore();
      }

      // 8. 奔跑动感切向微拉伸 (Locomotion Stretch)
      ctx.save();
      ctx.translate(renderPx, renderPy);
      if (curSpeed > 0.6) {
        const moveAng = Math.atan2(curVy, curVx);
        ctx.rotate(moveAng);
        ctx.scale(1.0 + Math.min(0.12, curSpeed * 0.015), 1.0 - Math.min(0.06, curSpeed * 0.008));
        ctx.rotate(-moveAng);
      }

      // 稳重专业的朝向指示微标 (等腰三角，长 7px，全场视角可辨认)
      ctx.save();
      ctx.rotate(facingAngle);
      ctx.beginPath();
      ctx.moveTo(radius + 7.0, 0);
      ctx.lineTo(radius + 0.5, -4.5);
      ctx.lineTo(radius + 0.5, 4.5);
      ctx.closePath();
      ctx.fillStyle = isHome ? "#059669" : "#d97706";
      ctx.fill();
      ctx.restore();

      // 9. 战术圆盘徽章主体 (Pro Tactical Disc - 清新活力运动风格)
      // 主队：鲜活薄荷翡翠绿；客队：明媚阳光琥珀金
      ctx.beginPath();
      ctx.arc(0, 0, radius, 0, Math.PI * 2);
      ctx.fillStyle = isHome ? "#059669" : "#d97706";
      ctx.fill();

      // 队色边框 (持球人加粗至 3.2px 醒目标识)
      ctx.strokeStyle = isHome ? "#10b981" : "#f59e0b";
      ctx.lineWidth = player.hasBall ? 3.2 : 2.0;
      ctx.stroke();

      // 持球人外围极细微自然提示环
      if (player.hasBall) {
        ctx.beginPath();
        ctx.arc(0, 0, radius + 3.5, 0, Math.PI * 2);
        ctx.strokeStyle = isHome ? "rgba(16, 185, 129, 0.6)" : "rgba(245, 158, 11, 0.6)";
        ctx.lineWidth = 1.2;
        ctx.stroke();
      }

      // 背身姿态视觉：圆盘两侧张肘卡位短弧 + 盘下「背身」姿态标签
      // （与面框三威胁的朝向三角形成直接的视觉对立语言）
      if (isBackToBasket) {
        const elbowColor = isHome ? "rgba(6, 95, 70, 0.85)" : "rgba(146, 64, 14, 0.85)";
        ctx.strokeStyle = elbowColor;
        ctx.lineWidth = 3.2;
        ctx.beginPath();
        ctx.arc(0, 0, radius + 5.0, Math.PI * 0.68, Math.PI * 1.0);
        ctx.stroke();
        ctx.beginPath();
        ctx.arc(0, 0, radius + 5.0, 0.0, Math.PI * 0.32);
        ctx.stroke();
        ctx.font = "800 10.5px system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        ctx.fillStyle = elbowColor;
        ctx.fillText("背身", 0, radius + 6.5);
      } else if (player.hasBall && !isShooting) {
        // 面框三威胁：盘下细微姿态标签，与背身标签对称
        ctx.font = "600 9.5px system-ui, sans-serif";
        ctx.textAlign = "center";
        ctx.textBaseline = "top";
        ctx.fillStyle = isHome ? "rgba(6, 95, 70, 0.55)" : "rgba(146, 64, 14, 0.55)";
        ctx.fillText("面框", 0, radius + 6.5);
      }

      // 纯白清晰数字背号 (居中，高对比度)
      ctx.font = "700 11.5px monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const num =
        player.number === undefined
          ? String(player.id || "")
          : String(player.number);

      ctx.fillStyle = "rgba(0, 0, 0, 0.4)";
      ctx.fillText(num, 0.5, 1.0);
      ctx.fillStyle = "#ffffff";
      ctx.fillText(num, 0, 0.5);

      ctx.restore(); // 还原 translate(renderPx, renderPy)

      // 6. 脚下纯中文战术动作与位置角色微标 (动态呈现当前场上实时战意与配合)
      const actionZh = getActionZh(player.action);
      let jumpActionZh = "";
      if (jumpHeight > 0.25) {
        if (isDunk) jumpActionZh = "腾空暴扣";
        else if (isBlock) jumpActionZh = "跃起封盖";
        else if (isReboundJump) jumpActionZh = "起跳争板";
        else if (isJumpShot) jumpActionZh = "干拔跳投";
        else if (isLayup) jumpActionZh = "飞身上篮";
        else if (isStealLunge) jumpActionZh = "飞身抢断";
      }
      const isDynamicAction =
        Boolean(jumpActionZh) ||
        (player.action &&
        ![
          "SetPosition",
          "Bench",
          "Normal",
          "Idle",
          "",
          "—",
        ].includes(player.action));
      const tagText = jumpActionZh || (isDynamicAction ? actionZh : getPositionZh(player.position));
      ctx.save();
      // 标签跟随腾空身体（否则人跳起来了标签留在地面，动作与文字分离）
      const posTagY = renderPy + radius + 9;
      ctx.font = isDynamicAction
        ? "800 11px system-ui, -apple-system, sans-serif"
        : "700 10.5px system-ui, -apple-system, sans-serif";
      const posTagW = ctx.measureText(tagText).width + 10;
      const homeFill = isHome ? "rgba(6, 95, 70, 0.95)" : "rgba(146, 64, 14, 0.95)";
      let tagFill = "rgba(255, 255, 255, 0.92)";
      let tagStroke = "rgba(203, 213, 225, 0.8)";
      let tagWidth = 1.0;
      let textColor = isHome ? "#065f46" : "#92400e";
      if (isDynamicAction) {
        tagFill = "rgba(255, 255, 255, 0.98)";
        tagStroke = isHome ? "rgba(5, 150, 105, 0.6)" : "rgba(217, 119, 6, 0.6)";
        tagWidth = 1.4;
        textColor = isHome ? "#047857" : "#b45309";
      }
      if (jumpActionZh) {
        tagFill = homeFill;
        tagStroke = "#ffffff";
        tagWidth = 1.8;
        textColor = "#ffffff";
      }
      ctx.fillStyle = tagFill;
      ctx.strokeStyle = tagStroke;
      ctx.lineWidth = tagWidth;
      ctx.beginPath();
      ctx.roundRect(px - posTagW / 2, posTagY - 7, posTagW, 14, 4);
      ctx.fill();
      ctx.stroke();
      ctx.fillStyle = textColor;
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      ctx.fillText(tagText, px, posTagY);
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
          ctx.strokeStyle = `rgba(249, 115, 22, ${trailAlpha.toFixed(3)})`;
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

    // 全场空间连续势能曲面 (高分辨率 1.5ft 步长)
    const stepFt = 1.5;
    const nx = Math.ceil(rules.courtWidth / stepFt);
    const ny = Math.ceil(rules.courtHeight / stepFt);
    const sigma2 = 2 * 7.5 * 7.5; // 控制辐射半衰期

    const homePlayers = [];
    const awayPlayers = [];
    for (const p of players) {
      const px = finite(p.x) * rules.courtWidth;
      const py = finite(p.y) * rules.courtHeight;
      const weight = p.hasBall ? 1.5 : 1.0;
      if (p.team === "home") homePlayers.push({ x: px, y: py, weight });
      else if (p.team === "away") awayPlayers.push({ x: px, y: py, weight });
    }

    const solverSamples = samples.map((s) => ({
      x: finite(s.x) * rules.courtWidth,
      y: finite(s.y) * rules.courtHeight,
      targetX: finite(s.target_x) * rules.courtWidth,
      targetY: finite(s.target_y) * rules.courtHeight,
      pressure: clamp(finite(s.pressure), 0, 1),
      team: s.team,
    }));

    let homeControlPoints = 0;
    let awayControlPoints = 0;

    for (let ix = 0; ix < nx; ix++) {
      const gx = ix * stepFt;
      for (let iy = 0; iy < ny; iy++) {
        const gy = iy * stepFt;

        let uHome = 0;
        for (const hp of homePlayers) {
          const d2 = (gx - hp.x) * (gx - hp.x) + (gy - hp.y) * (gy - hp.y);
          uHome += Math.exp(-d2 / sigma2) * hp.weight;
        }

        let uAway = 0;
        for (const ap of awayPlayers) {
          const d2 = (gx - ap.x) * (gx - ap.x) + (gy - ap.y) * (gy - ap.y);
          uAway += Math.exp(-d2 / sigma2) * ap.weight;
        }

        for (const ss of solverSamples) {
          const d2 = (gx - ss.x) * (gx - ss.x) + (gy - ss.y) * (gy - ss.y);
          const weight = Math.exp(-d2 / (2 * 10 * 10)) * ss.pressure * 1.6;
          if (ss.team === "home") uHome += weight;
          else if (ss.team === "away") uAway += weight;
        }

        const total = uHome + uAway + 0.05;
        const dominance = (uHome - uAway) / total; // [-1, 1]
        const intensity = Math.min(1.0, (uHome + uAway) * 0.7);

        // 统计全场两队空间占有率
        if (dominance > 0.08) homeControlPoints++;
        else if (dominance < -0.08) awayControlPoints++;

        if (Math.abs(dominance) > 0.06) {
          const cellPt = point(gx, gy);
          const cellNext = point(gx + stepFt, gy + stepFt);
          const w = cellNext.x - cellPt.x;
          const h = cellNext.y - cellPt.y;

          // 高辨识度色彩映射：主队翡翠绿 (#10b981)，客队琥珀金 (#f59e0b)
          const alpha = Math.min(
            0.40,
            0.15 + Math.abs(dominance) * 0.25 * (0.35 + intensity * 0.65),
          );
          if (dominance > 0) {
            ctx.fillStyle = `rgba(16, 185, 129, ${alpha.toFixed(3)})`;
          } else {
            ctx.fillStyle = `rgba(245, 158, 11, ${alpha.toFixed(3)})`;
          }
          ctx.fillRect(cellPt.x, cellPt.y, w + 0.5, h + 0.5);
        } else if (intensity > 0.28) {
          // 均势交锋争夺分界区域 (白色微透光带)
          const cellPt = point(gx, gy);
          const cellNext = point(gx + stepFt, gy + stepFt);
          const w = cellNext.x - cellPt.x;
          const h = cellNext.y - cellPt.y;
          ctx.fillStyle = `rgba(255, 255, 255, ${(0.08 + intensity * 0.1).toFixed(3)})`;
          ctx.fillRect(cellPt.x, cellPt.y, w + 0.5, h + 0.5);
        }
      }
    }
    ctx.restore();

    // 弱侧防守平衡驱动向量与目标锚点 (主队绿 / 客队金)
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
        const color = sample.team === "home" ? "#10b981" : "#f59e0b";

        // 驱动虚线与主体箭头
        ctx.strokeStyle = color;
        ctx.globalAlpha = 0.65 + sample.pressure * 0.35;
        ctx.lineWidth = 2.0;
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
          end.x - Math.cos(angle - 0.45) * 7,
          end.y - Math.sin(angle - 0.45) * 7,
        );
        ctx.lineTo(
          end.x - Math.cos(angle + 0.45) * 7,
          end.y - Math.sin(angle + 0.45) * 7,
        );
        ctx.closePath();
        ctx.fill();

        // 平衡目标锚点 (实心内点 + 轮廓环)
        const targetPt = point(sample.targetX, sample.targetY);
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(targetPt.x, targetPt.y, 4.0, 0, Math.PI * 2);
        ctx.fill();
        ctx.strokeStyle = color;
        ctx.lineWidth = 1.2;
        ctx.beginPath();
        ctx.arc(targetPt.x, targetPt.y, 7.5, 0, Math.PI * 2);
        ctx.stroke();
      }
      ctx.restore();
    }

    // 全场空间控制率统计面板 (Tactical Territory HUD)
    const totalControl = homeControlPoints + awayControlPoints || 1;
    const homePct = Math.round((homeControlPoints / totalControl) * 100);
    const awayPct = 100 - homePct;

    const awayTeamName = getTeamNameZh(tick.away_team) || "客队";
    const homeTeamName = getTeamNameZh(tick.home_team) || "主队";

    ctx.save();
    // 居中放置在球场顶部中央 (清新现代微浮动卡片)
    const hudW = 210;
    const hudH = 26;
    const hudX = 500 - hudW / 2;
    const hudY = 12;

    ctx.fillStyle = "rgba(255, 255, 255, 0.94)";
    ctx.strokeStyle = "rgba(203, 213, 225, 0.9)";
    ctx.lineWidth = 1.0;
    ctx.beginPath();
    ctx.roundRect(hudX, hudY, hudW, hudH, 6);
    ctx.fill();
    ctx.stroke();

    ctx.font = "700 11px system-ui, -apple-system, sans-serif";
    ctx.textBaseline = "middle";

    // 客队控制率 (左侧，明朗琥珀金)
    ctx.fillStyle = "#d97706";
    ctx.textAlign = "left";
    ctx.fillText(`${awayTeamName.slice(0, 4)} ${awayPct}%`, hudX + 10, hudY + 9);

    // 中间标签
    ctx.fillStyle = "#64748b";
    ctx.textAlign = "center";
    ctx.font = "600 9.5px system-ui, -apple-system, sans-serif";
    ctx.fillText("空间控制", 500, hudY + 9);

    // 主队控制率 (右侧，薄荷翡翠绿)
    ctx.fillStyle = "#059669";
    ctx.textAlign = "right";
    ctx.font = "700 11px system-ui, -apple-system, sans-serif";
    ctx.fillText(`${homePct}% ${homeTeamName.slice(0, 4)}`, hudX + hudW - 10, hudY + 9);

    // 底部双色空间对比进度条
    const barW = hudW - 20;
    const barH = 3.0;
    const barX = hudX + 10;
    const barY = hudY + 19;
    const awayBarW = (barW * awayPct) / 100;

    // 底槽浅灰
    ctx.fillStyle = "#e2e8f0";
    ctx.fillRect(barX, barY, barW, barH);

    ctx.fillStyle = "#d97706";
    ctx.fillRect(barX, barY, awayBarW, barH);
    ctx.fillStyle = "#059669";
    ctx.fillRect(barX + awayBarW, barY, barW - awayBarW, barH);

    ctx.restore();
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
    ctx.strokeStyle = "#94a3b8";
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
          "跳到一个决策帧，或点击事件流中的传球 / 投篮出手",
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
          el("span", "score-bar-label", describeDecisionKind(item.kind)),
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
          el("span", null, getConstraintZh(flag.constraint)),
          el("span", null, flag.reason),
          el("b", null, two(finite(flag.penalty))),
        );
        box.appendChild(row);
      }
      return box;
    };
    // 阻截串形如 "ADVANCE(H_05) ✗ action_eligibility"，解析为中文
    const describeBlocked = (entry) => {
      const match = String(entry).match(/^(.*?)\s*✗\s*(\S+)$/);
      if (!match) return String(entry);
      return `${describeDecisionKind(match[1])} · 被${getConstraintZh(match[2])}阻拦`;
    };
    const chips = (items, className, emptyText) => {
      const box = el("div", "chips");
      if (items?.length) {
        for (const item of items)
          box.append(
            el("span", `chip ${className}`, className === "bad" ? describeBlocked(item) : getConstraintZh(item)),
          );
      } else {
        box.append(el("span", "muted-label", emptyText));
      }
      return box;
    };
    const head = el("div", "decision-head");
    head.append(
      el("strong", null, describeDecisionKind(chosen)),
      el("span", null, `${playerNameFromId(debug.player)} · 第 ${decisionIndex} 帧`),
    );
    panel.append(
      head,
      section(
        "候选动作效用评分",
        scoreRows(utilities, (item) => item.utility, 100 / maxUtility),
      ),
      section(
        "候选动作最终概率",
        scoreRows(probabilities, (item) => item.prob, 100),
      ),
      section(
        "被规则直接拦下的动作",
        chips(debug.blocked, "bad", "没有被硬约束剔除的候选"),
      ),
      section("约束惩罚标记", flagSection(debug.flags || [])),
      section("本帧生效中的规则", chips(debug.active_constraints, "", "—")),
      section(
        "执行阻碍反馈",
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

    // 高质感清新素雅科技底色
    ctx.fillStyle = "#f8fafc";
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // 绘制清晰球场外框与半场线
    ctx.strokeStyle = "rgba(148, 163, 184, 0.4)";
    ctx.lineWidth = 1.2;
    ctx.strokeRect(10, 10, canvas.width - 20, canvas.height - 20);

    ctx.beginPath();
    ctx.moveTo(canvas.width / 2, 10);
    ctx.lineTo(canvas.width / 2, canvas.height - 10);
    ctx.stroke();

    for (const hoopX of [leftHoopX, rightHoopX]) {
      ctx.beginPath();
      ctx.arc(hoopX * scaleX, hoopY * scaleY, 7 * scaleX, 0, Math.PI * 2);
      ctx.strokeStyle = "rgba(234, 88, 12, 0.45)";
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
      [`篮下近筐 (< ${rim} 英尺)`, (shot) => shot.distance < rim],
      [
        `近中距离 (${rim}–16 英尺)`,
        (shot) => shot.distance >= rim && shot.distance < 16,
      ],
      [`远中距离 (16–${three} 英尺)`, (shot) => shot.distance >= 16 && !shot.three],
      [`三分外线 (${three}+ 英尺)`, (shot) => shot.three],
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
    for (const button of document.querySelectorAll(".speed-group button")) {
      button.disabled = busy;
    }
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
  function startPlayback() {
    if (!state.ticks.length) return;
    state.playing = true;
    state.previousRaf = 0;
    updatePlayButton();
    if (!state.rafId)
      state.rafId = requestAnimationFrame(playbackFrame);
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
    const el = $("rulesStatus");
    if (!el) return;
    el.textContent = text;
    el.className = `rules-status ${error ? "error" : "success"}`;
  }
  function applyPreset(name) {
    const editor = $("rulesEditor") || $("sheetRulesEditor");
    if (!editor) return;
    let rules;
    try {
      rules = JSON.parse(editor.value || "{}");
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
    editor.value = JSON.stringify(rules, null, 2);
    setRulesStatus(`已载入 ${name.toUpperCase()} 预设，点击应用并重跑`);
  }

  // ========================================================
  // 全维战术推演与比赛设置中心 (SettingsCenterManager)
  // ========================================================
  function initSettingsCenter() {
    const sheet = $("settingsSheet");
    if (!sheet) return;

    // 侧边栏导航切换
    for (const btn of document.querySelectorAll(".settings-nav-item")) {
      btn.addEventListener("click", () => {
        for (const b of document.querySelectorAll(".settings-nav-item")) {
          b.classList.remove("active");
        }
        btn.classList.add("active");
        const paneId = `pane-${btn.dataset.pane}`;
        for (const p of document.querySelectorAll(".settings-pane")) {
          p.classList.toggle("active", p.id === paneId);
        }
      });
    }

    // 打开设置中心并同步
    function openSettings() {
      sheet.hidden = false;
      syncSettingsCenter();
    }
    function closeSettings() {
      sheet.hidden = true;
    }

    const openBtn = $("openSettingsBtn");
    if (openBtn) openBtn.addEventListener("click", openSettings);
    const closeBtn = $("closeSettingsBtn");
    if (closeBtn) closeBtn.addEventListener("click", closeSettings);
    const cancelBtn = $("cancelSettingsBtn");
    if (cancelBtn) cancelBtn.addEventListener("click", closeSettings);
    sheet.addEventListener("click", (e) => {
      if (e.target === sheet) closeSettings();
    });

    // 快捷键支持 (S / O 键打开设置中心，Escape 关闭)
    window.addEventListener("keydown", (e) => {
      const activeEl = document.activeElement;
      const isInput = activeEl && (activeEl.tagName === "INPUT" || activeEl.tagName === "TEXTAREA" || activeEl.isContentEditable);
      if (e.key === "Escape" && !sheet.hidden) {
        closeSettings();
        return;
      }
      if ((e.key === "s" || e.key === "S" || e.key === "o" || e.key === "O") && !isInput && !e.ctrlKey && !e.metaKey) {
        if (sheet.hidden) openSettings();
        else closeSettings();
      }
    });

    // 种子输入双向同步
    const seedInput = $("seedInput");
    const sheetSeedInput = $("sheetSeedInput");
    if (sheetSeedInput) {
      sheetSeedInput.addEventListener("input", () => {
        if (seedInput) seedInput.value = sheetSeedInput.value;
        updateSettingsSummaryHint();
      });
    }

    // 随机种子按钮
    const sheetRandomBtn = $("sheetRandomSeedBtn");
    if (sheetRandomBtn) {
      sheetRandomBtn.addEventListener("click", () => {
        const val = String(Math.floor(Math.random() * 90000) + 1000);
        if (seedInput) seedInput.value = val;
        if (sheetSeedInput) sheetSeedInput.value = val;
        updateSettingsSummaryHint();
      });
    }

    // 范围选择双向同步
    for (const chip of document.querySelectorAll(".scope-chip")) {
      chip.addEventListener("click", () => {
        for (const c of document.querySelectorAll(".scope-chip")) {
          c.classList.remove("active");
        }
        chip.classList.add("active");
        $("scopeInput").value = chip.dataset.scope;
        updateSettingsSummaryHint();
      });
    }

    // 快速预设模式卡片
    for (const card of document.querySelectorAll(".preset-mode-card")) {
      card.addEventListener("click", () => {
        const mode = card.dataset.presetMode;
        if (mode === "default") {
          applyPreset("default");
          $("scopeInput").value = "5p";
          if (sheetSeedInput) sheetSeedInput.value = "42";
          if (seedInput) seedInput.value = "42";
        } else if (mode === "fast") {
          applyPreset("fast");
          $("scopeInput").value = "10p";
          if (sheetSeedInput) sheetSeedInput.value = String(Math.floor(Math.random() * 90000) + 1000);
          if (seedInput) seedInput.value = sheetSeedInput.value;
        } else if (mode === "contest") {
          applyPreset("contest");
          $("scopeInput").value = "5p";
        }
        syncSettingsCenter();
      });
    }

    // 本地文件导入
    const sheetUploadBtn = $("sheetUploadTriggerBtn");
    if (sheetUploadBtn) {
      sheetUploadBtn.addEventListener("click", () => $("fileInput").click());
    }

    // 主客队视角切换
    const homeBtn = $("sheetTeamHomeBtn");
    const awayBtn = $("sheetTeamAwayBtn");
    if (homeBtn) {
      homeBtn.addEventListener("click", () => {
        setTeamPerspective("home");
        syncSettingsCenter();
      });
    }
    if (awayBtn) {
      awayBtn.addEventListener("click", () => {
        setTeamPerspective("away");
        syncSettingsCenter();
      });
    }

    // 规则预设按钮
    for (const btn of document.querySelectorAll(".sheet-preset-btn")) {
      btn.addEventListener("click", () => {
        applyPreset(btn.dataset.rulesPreset);
        updateSettingsSummaryHint();
      });
    }

    // 应用并立即推演
    const applyBtn = $("applyAndRunBtn");
    if (applyBtn) {
      applyBtn.addEventListener("click", async () => {
        closeSettings();
        if (seedInput && sheetSeedInput) seedInput.value = sheetSeedInput.value;
        let rules = null;
        const editor = $("sheetRulesEditor");
        try {
          if (editor && editor.value.trim()) {
            rules = JSON.parse(editor.value);
          }
        } catch (e) {
          console.warn("规则解析异常", e);
        }
        await runSimulation(rules);
      });
    }
  }

  function syncSettingsCenter() {
    if (!studio) return;
    const isHome = state.teamPerspective === "home";
    const setup = studio.default_setup;
    const currentTeam = isHome ? setup.home_team : setup.away_team;

    // 同步种子与范围
    if ($("sheetSeedInput") && $("seedInput")) $("sheetSeedInput").value = $("seedInput").value;
    const currentScope = $("scopeInput")?.value || "5p";
    for (const chip of document.querySelectorAll(".scope-chip")) {
      chip.classList.toggle("active", chip.dataset.scope === currentScope);
    }

    // 同步规则编辑器内容
    if ($("sheetRulesEditor")) {
      $("sheetRulesEditor").value = state.rules ? JSON.stringify(state.rules, null, 2) : "";
    }

    // 同步队伍按钮（战术 pane）
    const homeBtn = $("sheetTeamHomeBtn");
    const awayBtn = $("sheetTeamAwayBtn");
    if (homeBtn) homeBtn.classList.toggle("active", isHome);
    if (awayBtn) awayBtn.classList.toggle("active", !isHome);
    const homeName = getTeamNameZh(setup.home_team.id) || setup.home_team.name;
    const awayName = getTeamNameZh(setup.away_team.id) || setup.away_team.name;
    if ($("sheetHomeTeamName")) $("sheetHomeTeamName").textContent = `${homeName} (主队)`;
    if ($("sheetAwayTeamName")) $("sheetAwayTeamName").textContent = `${awayName} (客队)`;

    // 渲染战术板与阵容列表
    renderBoard();
    renderRoster();
    syncRosterTeamButtons();

    // 更新底部简报
    updateSettingsSummaryHint();
  }

  function updateSettingsSummaryHint() {
    const hint = $("settingsSummaryHint");
    if (!hint) return;
    const seed = $("sheetSeedInput")?.value || $("seedInput")?.value || "42";
    const scope = $("scopeInput")?.value || "5p";
    const isHome = state.teamPerspective === "home";
    const teamLabel = isHome ? "北城老鹰 (主)" : "南湾水手 (客)";
    hint.textContent = `就绪 · 当前执教：${teamLabel} · 种子：${seed} · 范围：${scope} · 点击立即生效并推演`;
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
        setRunStatus(`文件读取失败：${error.message}`, true);
      }
    });

    // 右栏分析面板选项卡 (Deck Tabs)
    for (const button of document.querySelectorAll(".deck-tab-button")) {
      button.addEventListener("click", () => {
        const tab = button.dataset.deckTab;
        for (const item of document.querySelectorAll(".deck-tab-button")) {
          item.classList.toggle("active", item === button);
        }
        for (const pane of document.querySelectorAll(".deck-pane")) {
          const active = pane.id === `deck-${tab}`;
          pane.classList.toggle("active", active);
        }
        if (tab === "decisions" && state.ticks[state.idx]) {
          renderDecision(state.ticks[state.idx]);
        }
        if (tab === "anomalies") {
          renderAnomalies();
        }
        if (tab === "shots") {
          renderShotMap();
        }
        if (tab === "stats" && state.analytics) {
          renderStats(state.analytics.stats);
        }
        if (tab === "frame" && state.ticks[state.idx]) {
          renderFrameJson(state.ticks[state.idx]);
        }
      });
    }

    $("anomalyBadge").addEventListener("click", () => {
      const anomaliesBtn = document.querySelector('.deck-tab-button[data-deck-tab="anomalies"]');
      if (anomaliesBtn) anomaliesBtn.click();
    });

    // 设置中心内阵容 pane 的球队切换按钮
    const rosterHomeBtn = $("rosterTeamHomeBtn");
    const rosterAwayBtn = $("rosterTeamAwayBtn");
    if (rosterHomeBtn) {
      rosterHomeBtn.addEventListener("click", () => {
        setTeamPerspective("home");
        syncRosterTeamButtons();
      });
    }
    if (rosterAwayBtn) {
      rosterAwayBtn.addEventListener("click", () => {
        setTeamPerspective("away");
        syncRosterTeamButtons();
      });
    }
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

    for (const button of document.querySelectorAll(".speed-group button")) {
      button.addEventListener("click", () => {
        state.speed = Number(button.dataset.speed);
        for (const item of document.querySelectorAll(".speed-group button")) {
          item.classList.toggle("active", item === button);
        }
      });
    }
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

    initSettingsCenter();

    const randomBtn = $("randomSeedBtn");
    if (randomBtn) {
      randomBtn.addEventListener("click", () => {
        const val = String(Math.floor(Math.random() * 90000) + 1000);
        $("seedInput").value = val;
        if ($("sheetSeedInput")) $("sheetSeedInput").value = val;
      });
    }

    const uploadTrigger = $("uploadTriggerBtn");
    if (uploadTrigger) {
      uploadTrigger.addEventListener("click", () => $("fileInput").click());
    }
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
    async selectOffense(side, id) {
      const lineup = studio.default_setup[`${side}_lineup`];
      lineup.offense_tactic = id;
      const next = studio.offense.find((item) => item.id === id);
      studio.default_setup[`${side}_playbook`] = playsForTactic(next?.spec);
      renderStudio();
      await runSimulation(null, true);
    },
    async selectDefense(side, id) {
      const lineup = studio.default_setup[`${side}_lineup`];
      lineup.defense_tactic = id;
      renderStudio();
      await runSimulation(null, true);
    },
  };
  boot().catch((error) => {
    setRunStatus(`启动失败：${error.message}`, true);
    console.error(error);
  });
})();
