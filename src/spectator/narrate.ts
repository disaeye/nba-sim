/**
 * Event → broadcast line (CN + EN). Presentation only; no kernel side effects.
 *
 * Narration reads payload subtypes (turnover_type, foul_type, shooting, zone,
 * screen coverage) so two events of the same type no longer collapse to the
 * identical sentence. The sentence bank is indexed by a stable hash of the
 * event seq so the same event always narrates the same way (deterministic),
 * while successive events of the same kind vary.
 */
import type { TimelineEvent } from '../sim-utils.js';
import type { EventType } from '../state/types.js';

export interface Narration {
  readonly cn: string;
  readonly en: string;
  readonly intensity: 'low' | 'mid' | 'high';
}

function clockLabel(e: TimelineEvent): string {
  const g = e.clocks.game;
  const m = Math.floor(g / 60);
  const s = Math.floor(g % 60);
  return `Q${e.period} ${m}:${String(s).padStart(2, '0')}`;
}

function str(payload: Record<string, unknown>, key: string): string {
  const v = payload[key];
  return typeof v === 'string' || typeof v === 'number' ? String(v) : '?';
}

function bool(payload: Record<string, unknown>, key: string): boolean | null {
  const v = payload[key];
  return typeof v === 'boolean' ? v : null;
}

function num(payload: Record<string, unknown>, key: string): number | null {
  const v = payload[key];
  return typeof v === 'number' ? v : null;
}

/** Deterministic pick from a sentence bank, keyed by event seq. */
function pick<T>(bank: readonly T[], e: TimelineEvent): T {
  return bank[e.seq % bank.length]!;
}

// ── zone display names (human, not enum) ────────────────────────────────────
const ZONE_CN: Record<string, string> = {
  rim: '篮下',
  paint: '油漆区',
  dunker_L: '扣篮位',
  dunker_R: '扣篮位',
  elbow_L: '罚球线侧',
  elbow_R: '罚球线侧',
  slot_L: '罚球线延长线',
  slot_R: '罚球线延长线',
  wing_L: '侧翼',
  wing_R: '侧翼',
  corner_L: '底角',
  corner_R: '底角',
  frontcourt_center: '弧顶',
  backcourt: '后场',
};

function zoneCn(zone: string): string {
  return ZONE_CN[zone] ?? zone;
}

// ── drive / screen-use sentence banks ───────────────────────────────────────
const DRIVE_CN = [
  '持球突破，攻击篮筐',
  '加速突破防线，杀向篮下',
  '变向过掉防守人，直杀禁区',
  '借空间杀入三秒区',
  '欧洲步切入，寻找上篮角度',
];
const DRIVE_EN = [
  'drives hard to the rim',
  'blows past the defender into the paint',
  'crosses over and attacks the basket',
  'turns the corner and goes downhill',
  'euro-steps into the lane',
];

const SCREEN_USE_CN = [
  '借掩护突破，挡拆形成进攻优势',
  '绕掩护甩开防守，获得出手空间',
  '利用掩护换防错位，抓住大打小',
  '挡拆外弹，拉开突破路线',
  '借掩护拒绝追防，杀入禁区',
];
const SCREEN_USE_EN = [
  'uses the screen and turns the corner',
  'comes off the screen and creates separation',
  'hunts the switch off the screen',
  'rejects the screen and attacks',
  'uses the ball-screen to get downhill',
];

const PASS_CN = [
  '转移球，找到',
  '横传策应，交给',
  '突破分球，准确找到',
  '塞球给空切的',
  '外线倒球，转移到',
];

const HANDLERS: Partial<Record<EventType, (e: TimelineEvent) => Narration>> = {
  GAME_START: () => ({
    cn: '比赛开始',
    en: 'Tip-off sequence',
    intensity: 'mid',
  }),
  JUMP_CIRCLE_ALIGN: () => ({
    cn: '跳球站位 · 中圈两名跳球手 · 圈外八人',
    en: 'Jump-ball circle · two jumpers · eight outside',
    intensity: 'mid',
  }),
  ALIGNMENT: (e) => {
    const ctx = str(e.payload, 'context');
    const map: Record<string, { cn: string; en: string }> = {
      tip_receive: { cn: '获球后场接应 · 攻防落位', en: 'Tip receive · backcourt outlet' },
      halfcourt: { cn: '半场进攻落位 · 五攻五防', en: 'Halfcourt set · five-on-five' },
      transition: { cn: '转换推进 · 攻防奔跑', en: 'Transition · both ways running' },
      jump_ball: { cn: '跳球阵型', en: 'Jump formation' },
    };
    const m = map[ctx] ?? { cn: `阵型更新 · ${ctx}`, en: `Alignment · ${ctx}` };
    return { cn: m.cn, en: m.en, intensity: 'low' };
  },
  JUMP_BALL_TAP: (e) => {
    const team = str(e.payload, 'tapping_team');
    return {
      cn: `跳球拨出 · ${team === 'home' ? '主队' : '客队'}控制`,
      en: `Tip · ${team} controls`,
      intensity: 'mid',
    };
  },
  POSSESSION_GAINED: (e) => ({
    cn: `球权 · #${str(e.payload, 'player_id')} (${str(e.payload, 'team')})`,
    en: `Possession — #${str(e.payload, 'player_id')} (${str(e.payload, 'team')})`,
    intensity: 'low',
  }),
  ADVANCE_BACKCOURT: (e) => ({
    cn: `#${str(e.payload, 'ballHandlerId')} 持球推进，队友展开转换阵型`,
    en: `#${str(e.payload, 'ballHandlerId')} brings the ball up`,
    intensity: 'low',
  }),
  CROSS_HALF: (e) => ({
    cn: `#${str(e.payload, 'ballHandlerId')} 带球过半场，进入前场组织`,
    en: `#${str(e.payload, 'ballHandlerId')} crosses half court into the frontcourt`,
    intensity: 'mid',
  }),
  ALIGN_HALFCOURT: () => ({
    cn: '半场落位：持球人组织，队友拉开空间',
    en: 'Halfcourt set',
    intensity: 'low',
  }),
  PASS: (e) => ({
    cn: `#${str(e.payload, 'passer_id')} ${pick(PASS_CN, e)} #${str(e.payload, 'receiver_id')}`,
    en: `#${str(e.payload, 'passer_id')} passes to #${str(e.payload, 'receiver_id')}`,
    intensity: 'low',
  }),
  HANDOFF: (e) => ({
    cn: `#${str(e.payload, 'giver_id')} 手递手交球给 #${str(e.payload, 'receiver_id')}`,
    en: `#${str(e.payload, 'giver_id')} handoff to #${str(e.payload, 'receiver_id')}`,
    intensity: 'mid',
  }),
  SCREEN_SET: (e) => {
    const coverage = str(e.payload, 'coverage');
    let suffix = '';
    let suffixEn = '';
    if (coverage === 'SWITCH') { suffix = ' · 防守换防'; suffixEn = ' · defense switches'; }
    else if (coverage === 'DROP') { suffix = ' · 防守沉退护筐'; suffixEn = ' · big drops'; }
    else if (coverage === 'HEDGE') { suffix = ' · 防守上前延误'; suffixEn = ' · hedge'; }
    return {
      cn: `#${str(e.payload, 'screener_id')} 为 #${str(e.payload, 'ballHandlerId')} 设置掩护，挡住持球防守人${suffix}`,
      en: `#${str(e.payload, 'screener_id')} sets screen for #${str(e.payload, 'ballHandlerId')}${suffixEn}`,
      intensity: 'low',
    };
  },
  SCREEN_USE: (e) => ({
    cn: `#${str(e.payload, 'ballHandlerId')} ${pick(SCREEN_USE_CN, e)}`,
    en: `#${str(e.payload, 'ballHandlerId')} ${pick(SCREEN_USE_EN, e)}`,
    intensity: 'mid',
  }),
  DRIVE: (e) => ({
    cn: `#${str(e.payload, 'ballHandlerId')} ${pick(DRIVE_CN, e)}`,
    en: `#${str(e.payload, 'ballHandlerId')} ${pick(DRIVE_EN, e)}`,
    intensity: 'mid',
  }),
  SHOT_RELEASE: (e) => {
    const v = num(e.payload, 'shot_value') ?? 2;
    const zone = zoneCn(str(e.payload, 'zone'));
    const shooter = str(e.payload, 'shooter_id');
    if (v === 3) {
      const threeCn = ['出手三分', '拔起三分', '外线发炮', '三分线外急停跳投'];
      const threeEn = ['pulls up for three', 'launches a three', 'fires from deep', 'steps back for three'];
      const i = e.seq % threeCn.length;
      return {
        cn: `#${shooter} ${threeCn[i]} · ${zone}`,
        en: `#${shooter} ${threeEn[i]} · ${zone}`,
        intensity: 'high',
      };
    }
    const shootCn = zone === '篮下' || zone === '油漆区'
      ? ['攻筐上篮', '强攻篮下', '抛射出手', '低位强打']
      : zone === '罚球线侧'
        ? ['中距离跳投', '急停中投', '罚球线附近后仰']
        : ['出手 2分'];
    const shootEn = zone === '篮下' || zone === '油漆区'
      ? ['goes up for the layup', 'attacks the rim', 'floats it up', 'powers inside']
      : zone === '罚球线侧'
        ? ['pulls up for the mid-range', 'stops and pops', 'fades at the elbow']
        : ['shoots 2'];
    const i = e.seq % shootCn.length;
    return {
      cn: `#${shooter} ${shootCn[i]} · ${zone}`,
      en: `#${shooter} ${shootEn[i]} · ${zone}`,
      intensity: 'high',
    };
  },
  SHOT_RESULT: (e) => {
    const made = bool(e.payload, 'made');
    const v = num(e.payload, 'shot_value') ?? 2;
    const shooter = str(e.payload, 'shooter_id');
    // Event score snapshot is pre-apply in the kernel; bump for display when made.
    let home = e.score.home;
    let away = e.score.away;
    if (made) {
      const n = Number(shooter);
      if (!Number.isNaN(n) && n > 10) away += v;
      else home += v;
    }
    if (made) {
      return {
        cn: `进球！#${shooter} ${v}分命中 · ${home}-${away}`,
        en: `BUCKET — #${shooter} ${v} · ${home}-${away}`,
        intensity: 'high',
      };
    }
    return {
      cn: `#${shooter} 投失`,
      en: `#${shooter} misses`,
      intensity: 'mid',
    };
  },
  REBOUND: (e) => {
    const off = bool(e.payload, 'offensive');
    return {
      cn: `#${str(e.payload, 'rebounder_id')} ${off ? '进攻' : '防守'}篮板`,
      en: `#${str(e.payload, 'rebounder_id')} ${off ? 'offensive' : 'defensive'} rebound`,
      intensity: 'mid',
    };
  },
  TURNOVER: (e) => {
    const player = str(e.payload, 'player_id');
    const ttype = str(e.payload, 'turnover_type');
    const stealer = e.payload['stealer_id'];
    const hasSteal = typeof stealer === 'string' && stealer !== '';
    if (hasSteal) {
      return {
        cn: `#${str(e.payload, 'stealer_id')} 抢断 · #${player} 控球失误`,
        en: `#${str(e.payload, 'stealer_id')} strips #${player} — steal`,
        intensity: 'high',
      };
    }
    // subtype-driven turnover narration
    const map: Record<string, { cn: string; en: string }> = {
      bad_pass: { cn: `#${player} 传球失误，球被破坏出界`, en: `#${player} bad pass — turnover` },
      handoff: { cn: `#${player} 手递手配合失误`, en: `#${player} fumbles the handoff — turnover` },
      drive: { cn: `#${player} 突破掉球，失去球权`, en: `#${player} loses the handle on the drive — turnover` },
    };
    const m = ttype !== '?' ? (map[ttype] ?? null) : null;
    if (m) return { cn: m.cn, en: m.en, intensity: 'high' };
    return {
      cn: `#${player} 失误`,
      en: `#${player} turnover`,
      intensity: 'high',
    };
  },
  STEAL: (e) => ({
    cn: `#${str(e.payload, 'stealer_id')} 抢断 #${str(e.payload, 'victim_id')}`,
    en: `#${str(e.payload, 'stealer_id')} steals from #${str(e.payload, 'victim_id')}`,
    intensity: 'high',
  }),
  FOUL: (e) => {
    const offender = str(e.payload, 'offender_id');
    const shooting = bool(e.payload, 'shooting');
    const ftype = str(e.payload, 'foul_type');
    const fts = num(e.payload, 'free_throws_awarded') ?? 0;
    if (shooting) {
      const andOne = fts === 1;
      if (andOne) {
        return {
          cn: `#${offender} 打手犯规 · 加罚一球`,
          en: `#${offender} shooting foul — and-one`,
          intensity: 'high',
        };
      }
      return {
        cn: `#${offender} 投篮犯规 · 罚球 ${fts} 次`,
        en: `#${offender} shooting foul — ${fts} FTs`,
        intensity: 'high',
      };
    }
    if (ftype === 'reach_in') {
      return {
        cn: `#${offender} 伸手犯规`,
        en: `#${offender} reach-in foul`,
        intensity: 'mid',
      };
    }
    return {
      cn: `#${offender} 犯规`,
      en: `#${offender} foul`,
      intensity: 'mid',
    };
  },
  SHOT_CLOCK_VIOLATION: () => ({
    cn: '24秒进攻违例',
    en: 'Shot-clock violation',
    intensity: 'high',
  }),
  OOB: (e) => ({
    cn: `传球出界 · #${str(e.payload, 'player_id')} 失误`,
    en: `Pass out of bounds · #${str(e.payload, 'player_id')} turnover`,
    intensity: 'high',
  }),
  INBOUND_START: () => ({
    cn: '边线/底线发球',
    en: 'Inbound',
    intensity: 'low',
  }),
  INBOUND_TOUCH: (e) => ({
    cn: `#${str(e.payload, 'receiver_id')} 接发球`,
    en: `#${str(e.payload, 'receiver_id')} receives inbound`,
    intensity: 'low',
  }),
  MADE_BASKET_DEAD: () => ({
    cn: '进球后死球',
    en: 'Dead ball after make',
    intensity: 'low',
  }),
  PERIOD_END: (e) => ({
    cn: `第 ${num(e.payload, 'period') ?? e.period} 节结束 · ${e.score.home}-${e.score.away}`,
    en: `End of period ${num(e.payload, 'period') ?? e.period} · ${e.score.home}-${e.score.away}`,
    intensity: 'high',
  }),
  PERIOD_START: (e) => ({
    cn: `第 ${num(e.payload, 'period') ?? e.period} 节开始`,
    en: `Period ${num(e.payload, 'period') ?? e.period} tip`,
    intensity: 'mid',
  }),
  GAME_END: (e) => ({
    cn: `终场 ${e.score.home}-${e.score.away}`,
    en: `Final ${e.score.home}-${e.score.away}`,
    intensity: 'high',
  }),
  FT_ATTEMPT: (e) => ({
    cn: `#${str(e.payload, 'shooter_id')} 罚球出手`,
    en: `#${str(e.payload, 'shooter_id')} free throw`,
    intensity: 'mid',
  }),
  FT_RESULT: (e) => {
    const made = bool(e.payload, 'made');
    return {
      cn: made ? '罚球命中' : '罚球不中',
      en: made ? 'FT good' : 'FT miss',
      intensity: 'mid',
    };
  },
  CLOCK_EXPIRY_ADJUDICATION: () => ({
    cn: '时钟耗尽判定',
    en: 'Clock expiry',
    intensity: 'mid',
  }),
  HALFTIME: () => ({
    cn: '中场休息',
    en: 'Halftime',
    intensity: 'low',
  }),
  SUB: (e) => ({
    cn: `#${str(e.payload, 'player_in_id')} 换下 #${str(e.payload, 'player_out_id')}`,
    en: `#${str(e.payload, 'player_in_id')} in for #${str(e.payload, 'player_out_id')}`,
    intensity: 'low',
  }),
  LOOSE_BALL_RECOVER: (e) => ({
    cn: `#${str(e.payload, 'recoverer_id')} 抢到球权`,
    en: `#${str(e.payload, 'recoverer_id')} recovers the loose ball`,
    intensity: 'low',
  }),
  FT_START: (e) => ({
    cn: `#${str(e.payload, 'shooter_id')} 执行罚球 · ${num(e.payload, 'attempts') ?? 1} 次`,
    en: `#${str(e.payload, 'shooter_id')} to the line — ${num(e.payload, 'attempts') ?? 1} FTs`,
    intensity: 'mid',
  }),
  FT_SEQUENCE_END: () => ({
    cn: '罚球结束，重新开球',
    en: 'Free throws complete — resuming play',
    intensity: 'low',
  }),
};

export function narrateEvent(e: TimelineEvent): Narration {
  const h = HANDLERS[e.type];
  if (h) return h(e);
  return {
    cn: e.type,
    en: e.type,
    intensity: 'low',
  };
}

export function formatBroadcastLine(e: TimelineEvent, lang: 'cn' | 'en' = 'cn'): string {
  const n = narrateEvent(e);
  const text = lang === 'cn' ? n.cn : n.en;
  return `[${clockLabel(e)} | ${e.score.home}-${e.score.away}] ${text}`;
}
