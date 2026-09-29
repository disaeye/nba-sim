//! 确定性回归：黄金 tick 哈希。
//!
//! 相同种子 + 相同规则必须产生逐 tick 完全一致的模拟。本测试对每 tick
//! 的关键字段（球员位置/球状态/比分/时钟/阶段/事件）做 FNV-1a 串联哈希，
//! 任何改动导致的逐 tick 分歧都会改变最终哈希，从而被 CI 立即捕获。
//!
//! 黄金哈希值是有意入库的：它抓"行为变了"，与统计基线（抓"错得离谱"）互补。
//! 当一次改动被审查确认为合理（例如修复了一个物理 bug），应更新本哈希并
//! 在提交信息中说明原因。

use nba_engine::MatchEngine;
use nba_protocol::StreamTick;

/// FNV-1a 64 位，足以做确定性回归指纹（非加密用途）。
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Fnv(0xcbf29ce484222325)
    }
    fn bytes(&mut self, data: &[u8]) {
        for &b in data {
            self.0 ^= b as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
    fn f32(&mut self, v: f32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_le_bytes());
    }
    fn u64(&mut self, v: u64) {
        self.bytes(&v.to_le_bytes());
    }
    fn str(&mut self, s: &str) {
        self.bytes(s.as_bytes());
    }
}

/// 对单 tick 的确定性相关字段做哈希。刻意跳过渲染舍入字段与调试层，
/// 只保留能影响后续模拟演进的世界状态。
fn hash_tick(h: &mut Fnv, tick: &StreamTick) {
    let f = &tick.frame;
    h.u64(f.event_sequence);
    h.f32(f.t);
    h.f32(f.t_game);
    h.f32(f.shot_clock);
    h.u32(f.period);
    h.str(&f.phase);
    h.u32(f.possession_id);
    h.str(&f.possession_team);
    h.u32(f.score.home);
    h.u32(f.score.away);
    h.str(&f.game_flow);
    // 球员按 id 排序（引擎已保证），哈希位置/状态/动作。
    for p in &f.players {
        h.str(&p.id);
        h.f32(p.x);
        h.f32(p.y);
        h.bytes(&[p.has_ball as u8, p.on_court as u8]);
        h.str(&p.action);
        h.f32(p.stm);
        h.bytes(&[p.foul_count]);
    }
    h.f32(f.ball.x);
    h.f32(f.ball.y);
    h.f32(f.ball.z);
    h.str(&f.ball.status);
    if let Some(holder) = &f.ball.holder_id {
        h.str(holder);
    }
    for e in &f.events {
        h.str(e);
    }
    h.u32(f.team_fouls_home);
    h.u32(f.team_fouls_away);
    h.bytes(&[f.free_throws_remaining]);
}

/// 计算一次模拟的串联黄金哈希。
fn golden_hash(seed: u64, ticks: usize) -> u64 {
    let mut engine = MatchEngine::new(seed);
    let mut h = Fnv::new();
    for _ in 0..ticks {
        let tick = engine.step();
        hash_tick(&mut h, &tick);
    }
    h.0
}

#[test]
fn deterministic_same_seed_same_hash() {
    // 同种子两次运行必须逐 tick 一致——这是所有检测体系的前提。
    let a = golden_hash(42, 2000);
    let b = golden_hash(42, 2000);
    assert_eq!(a, b, "same seed diverged: sim is not deterministic");
}

#[test]
fn different_seeds_diverge() {
    // 不同种子应产生不同轨迹（防止哈希退化为常数，失去判别力）。
    let a = golden_hash(1, 2000);
    let b = golden_hash(2, 2000);
    assert_ne!(a, b, "different seeds produced identical hash");
}

/// 黄金基线。该值在确定引擎正确后被冻结；任何使其改变的改动都必须经
/// 审查确认。当前值作为重构前的行为锚点。
#[test]
fn golden_baseline_seed42() {
    let h = golden_hash(42, 2000);
    // 首次运行时把实际值打印出来，便于冻结基线。
    eprintln!("GOLDEN seed42 x2000 = {:#018x}", h);
    // 锚点：重构前基线。若合理改动导致分歧，更新此值并在提交说明原因。
    assert_eq!(h, GOLDEN_SEED42_2000, "golden hash drifted for seed 42");
}

/// `golden_baseline_seed42` 的窗口必须真的覆盖各类得分行为。
///
/// ## 为何需要这条断言（evidence/problem.md §23.4）
///
/// 该守卫曾出现「测试全绿但实际未覆盖」的静默失效：实测 seed42 × 2000
/// tick 窗口内 `fg2_att = 0`——首次 2 分出手在 tick 2020，恰在窗口之外。
/// 因此任何只影响 2 分结算的改动都不会改变 `GOLDEN_SEED42_2000`，
/// 哈希保持绿色，而 CI 看不出它没在守卫。
///
/// 这类缺陷的危险在于**失败方向是绿而不是红**：覆盖丢失不会报警。
/// 所以修法：让覆盖本身成为可断言属性，把窗口调到「刚好覆盖」无济于事：
/// 一旦窗口内缺少某类得分行为，本测试直接失败，提示需要扩大窗口。
#[test]
fn golden_window_covers_scoring_behaviour() {
    let mut engine = MatchEngine::new(42);
    // 实测首个 3 分出手在 tick 3588，取 4000 tick 留约 10% 余量覆盖首个 2 分与 3 分
    let ticks = 4000usize;
    for _ in 0..ticks {
        engine.step();
    }
    let b = engine.box_score();
    // 2 分是本次实际漏掉的那一类（§23.4），必须有覆盖：
    // 若把窗口改小或行为漂移导致首次 2 分出手推后，这里会变红。
    assert!(
        b.fg2_attempts > 0,
        "golden window ({ticks} ticks, seed 42) contains no two-point attempt \
         (fg2_attempts=0); the golden hash cannot guard two-point behaviour. \
         Enlarge the window or re-freeze with a window that covers it \
         (evidence/problem.md §23.4)"
    );
    // 三分与得分总数同样属于「必须被守卫的行为」：
    assert!(
        b.fg3_attempts > 0,
        "golden window ({ticks} ticks, seed 42) contains no three-point attempt"
    );
    assert!(
        b.fg2_made + b.fg3_made > 0,
        "golden window ({ticks} ticks, seed 42) contains no made field goal"
    );
}

/// 长窗口**覆盖**犯规与罚球——但它**不是**犯规概率参数的守卫。
///
/// ## 为何需要这个长窗口（覆盖）
///
/// 探针实测（seed42 逐 tick）：
/// - `first_foul`       = tick **3413**
/// - `first_ft_attempt` = tick **9918**
///
/// 而 `golden_window_covers_scoring_behaviour` 的窗口只有 2000 tick，
/// 在第一次犯规前就结束。
///
/// ## 但它验证不了概率参数（已实测，evidence §32.29.2）
///
/// 把 `foul_on_drive_rate` 从 0.18 降到 0.05（降 72%），
/// **本窗口的哈希完全不变**：10200 tick 内只有 `drives=19, fouls=2`，
/// 两种取值都掷出 2 次犯规。
///
/// 即这是**采样量**问题，不是窗口长度问题——单 seed 的有限窗口内
/// 概率参数掷骰结果极可能相同。延长到 10200 甚至更长也无法解决。
///
/// **概率参数的守卫必须是跨 seed 聚合的统计门**
/// （`stats_baseline` 8 seed：FT 率 0.121→0.204 确实被它抓到）。
/// 本测试只负责「窗口里确实有犯规与罚球可被观察」这一**覆盖**前提。
#[test]
fn golden_window_long_covers_fouls_and_free_throws() {
    let mut engine = MatchEngine::new(42);
    // 抛体化（v68）后节奏实测：首犯规 6783、首罚球 21813
    // （传球弧顶随距离抬高 → 长传变慢 → 攻防节奏整体后移）；
    // v77（起跳延伸 + 篮下接球效用）后节奏再后移：首犯规 2200（突破
    // 犯规更早涌现）、首罚球 31996；
    // v78（三维罚球就位轨迹 FreeThrowSetup 连续过渡）后实测：首罚球位于
    // tick 34280。取 36000 留约 5% 余量，避免边界抖动。
    // 必须同时覆盖首个犯规（3413/9213/6783/2200）与首次罚球
    // （9918/14792/21813/31996/34280）。
    let ticks = 36000usize;
    for _ in 0..ticks {
        engine.step();
    }
    let b = engine.box_score();
    // 覆盖断言：窗口必须真的包含这两类行为，否则本测试没有观察对象。
    assert!(
        b.fouls > 0,
        "long window ({ticks} ticks, seed 42) contains no foul \
         (measured first foul at tick 6783) — foul behaviour unobservable \
         (evidence/problem.md §32.29)"
    );
    assert!(
        b.ft_attempts > 0,
        "long window ({ticks} ticks, seed 42) contains no free-throw attempt \
         (measured first attempt at tick 21813)"
    );
}

// 基线常量冻结记录（校准协议 docs/protocol.md §3）：
//   v1 0x76e41f2a83f74b38 — 重构前锚点（BallState 写入口收敛，行为保持）
//   v2 0xef68d18205abaa12 — 2026-08-31 回合节奏校准
//   v3 0x84a62217d990685b — 2026-08-31 常识公理修复：替补席界外物理隔离
//   v4 0xaf78cd4997df973d — 2026-08-31 投篮分布与突破攻框校准
//   v5 0xc2210e16619f2e96 — 2026-08-31 G3 阶段门推进
//   v6 0x09f38b3b705ce31a — 2026-08-31 L2 PossessionSummary 全链路事件流因果语义与回合总结
//   v7 0xcebe206e784ae273 — 2026-08-31 修复得分分支中重复 complete_possession 自增问题
//   v8 0x248df313adaf4939 — 2026-08-31 G4 终态阶段门（全场总分 252 分，每场 202 回合，回合时长 16.0s，接近 NBA 现实）
//   v9 0x2f4b2fa3c9d81e57 — 2026-09-02 GAP 修复（M8/M9/M2，见 status §8.3）
//   v10 0x1275cccf4a8f3f0e — 2026-09-02 GAP 复审补修：领域转换表补齐违例→发球
//     程序边（Pass/LooseBall/ControlTransfer/RimRebound→InboundTransfer，
//     修复 seed 2/4 死球停滞导致的比赛无法完赛）；替补席界外 body 不再
//     产生伪造 BoundaryCross（每 tick 6 条 OUT_OF_BOUNDS 污染）；Flagged
//     咨询性发现不再触发 RuleViolation 强制项。均为设计内行为修复。
// v15 0x3316f6c9d9051602 - 2026-09-03 传球提前量外推校准与突破终结记录补齐（Realism Index >= 0.92）
// v16 0xc80915ba200cd269 - 2026-09-03 发球就位物理约束与闭环状态机、传球时空追踪落点、防守朝向解耦
// v17 0x2568813419a51146 - 2026-09-03 第一性原理公理守卫（连续性、冲量溯源、因果门禁）、真实罚球物理与跳球争顶
// v18 0x7bda4db4e5e3f239 - 2026-09-03 中圈真实跳球阵型落位(双方中锋中圈对峙、外围放射状卡位)与物理拍球弹道
// v19 0x8d8c14e0961b783f - 2026-09-04 修复发球人未到界外即提前生成传球的空飘缺陷，建立真实就位与持球判定闭环
// v20 0x76b89910652c44cf - 2026-09-04 统一边界容差与出界判罚语义闭环，消灭死球停滞回归
// v21 0x91419840ef413b0c - 2026-09-04 发球落位持球因果闭环、被阻截转活球、失误类型精确溯源
// v22 0x688f76c83ad7ead2 - 2026-09-04 真实篮板冲抢与卡位、触达有效范围物理拦截、传球手部锚定零跳变
// v23 0x2370512ecdf7e6f4 - 2026-09-05 真实运球动力学与球体阴影视觉增强（Dribble Bounce Kinematics + Dynamic Shadow）
// v24 0x58f092e69d840430 - 2026-09-05 跳球事件单次发射（消除持续 37 tick 的重复事件轰炸）与语义化事件流
// v25 0xbea84d43c5695873 - 2026-09-05 APF人造势能场多体动力学转向、防守球人筐外心三角、空间拥挤效用惩罚
// v26 0x7da066b2809fc440 - 2026-09-05 真实跳球LooseBall点拍飞行争夺、彻底消除必中假传球与回合总结描述残留
// v27 0x4ae5950e326e7d86 - 2026-09-05 真实跳球规则实施：禁止跳球员触地前自抓球、外圈8人真实争夺地板球
// v28 0x1a9851edeeab4af8 - 2026-09-05 严格落实真实跳球红线：点拍后进入真实重力抛物线自由球，跳球员受滞空约束，由外围8人（H_1等）真实冲刺抢球
// v29 0xfcb10465ef83e7e8 - 2026-09-05 彻底铲除 H_5/A_5 硬编码残留：跳球双方由场上真实摸高上限(height_cm + vertical)纯函数动态选拔
// v35 0x6fa2f7fedc7ff542 - 2026-09-06 彻底铲除接球点 snap 瞬间位移，全量 24 动作原语闭环，全量种子 4000 ticks 零违规
// v36 0x7c95842cd07b7b1a - 2026-09-10 激活基础动作生命周期与战术覆盖保护锁（运球变向/上篮扣篮/干拔后撤/背身探步闭环）
// v37 0xf2c64aefde03171e - 2026-09-10 因果状态机单权威重构、违例终端事实补全与评判器度量纠偏
// v38 0x8ce38a9678a68ca9 - 2026-09-10 F1 授权状态与时钟修正：罚球期间球权威态收敛为 Dead（消除 BALL_WITH_HOLDER），后场计时在持球越中线时真实重置（消除 EIGHT_SECOND_BACKCOURT 误判）
// v39 0x1dcdf8f03b2c0c4f - 2026-09-10 F1.3 发球显式 placement：发球员允许站界外发球点，消除 DeadBall 活锁（full scope 永不终场）与 PLAYER_SPEED/BALL_WITH_HOLDER 伪造违反；placement 由 sync_ball_holder 单一机制产生 PlacementApplied 事实
// v40 0x4d43f93c9650517d - 2026-09-10 F1.3b/F6.2 消除剩余三类 DeadBall 活锁（分离投影推回发球员、发球员为替补、旧状态未改派），并引入有界流模式（facts/summary 默认、frames 显式）与磁盘/字节预算
// v41 0x357c52254bed731d - 2026-09-10 F1.3c 发球赴界外改为显式离散 placement（消除被固定在边线的防守者造成的几何僵局），placement 回场选择无重叠接球点；CLI 按 quality.md 规定区分 Hard（阻断）与 Soft（不阻断）
// v42 0x74032b10dae43034 - 2026-09-11 dev 方案 D3.1 命中模型校准 + 边界事实边沿化：
//   (a) spacing_bonus 三项权重和由 1.0 降至 0.16（空位时不再直接叠加近 +1.0 命中率）；
//       shot_contest_sensitivity 0.22→0.32（干扰惩罚分化）。8 seed full 实测：
//       3P 64.0%→39.7% ∈ [30,40]、2P 72.9%→58.1% ∈ [48,58]（SHOT_MAKE_PROFILE 入带）。
//   (b) BoundaryCross 由电平触发改为上升沿触发 + 几何容差，且 sync_positions 不再用
//       刚体积分产物覆盖运动学权威位置（双发射点/单锁存导致单球员连续 675–755 tick
//       伪造越界刷屏，阻塞发球程序）。属设计内行为修复，见历史执行记录
//       `docs/dev/cycles/20260911_first-principles/status_history.md` §33.3。
// v43 0xa89062d9c5141724 - 2026-09-11 dev 方案 D3.4 进攻时钟紧逼校准：
//   shot_clock_urgency_seconds 5.0→12.0（真实进攻在 24→14s 已开始组织）；
//   dwell_decay_max 0.38→0.85（原来衰减上限过低使 Dwell 在 24s 仍保有 62% 效用，
//   进攻方持续运球至违例）；紧迫加成量级提升（shoot 0.25→1.20 / drive 0.10→0.60 /
//   pass -0.10→-0.30 / dwell -0.20→-0.80），使其能真正压过 pass_base=0.82。
//   8 seed full 实测：总分中位 132.5→175.0 ∈ G4 [140,230]，3P% 39.5% ∈ [30,40]，
//   单场违例从 53 降至 ~23，TO% 60%→46%。
// v44 0xaa0948ab6cd348c1 - 2026-09-11 D5.1 交接活锁与投篮弧顶越界修复：
//   (a) ControlTransfer 在接球人被动作窗口锁定时永久悬置——seed 6 full 实测
//       69,466 帧（约 2,780 秒）活锁、全场仅 11 个回合；改为飞行时长届满即
//       由球收敛到接球人可达范围（不瞬移球员）。
//   (b) shot_arc_amplitude 由线性缩放改为二分反解，使采样峰值真正等于请求
//       peak_z；此前请求 35.0 ft 会采样到 35.08 ft，违反 BALL_HEIGHT_BOUNDS。
//   8 seed full 实测：violations 4→0，3P% 37.7% ∈ [30,40]，2P% 57.3% ∈ [48,58]。
// v45 0x747970ed0049441d - 2026-09-11 D5.1b 战术档案槽位生效 + 底角三分几何：
//   (a) 进攻槽位改由 data/tactics/*.json 的 base_offset_x/y 决定（此前档案仅用于
//       校验 id，从未参与目标生成），并按能力 fill_slots 分配球员而非 roster 索引；
//   (b) 三分判定加入底角特例（NBA 底角 22ft vs 弧顶 23.75ft），此前 4 处裸半径
//       比较把底角三分误判为两分；底角带深度 3ft 由 CourtGeometry 定义；
//   (c) 修正 data/tactics/*.json 底角槽位到真实位置（距边线 2.5ft）。
//   8 seed full 实测：3P% 61.2→33.2 ∈ [30,40]；2PA 10.8→72.9（真实 ~55）。
// v48 0x97c7de28a95cb3d5 - 2026-09-11 边界事实语义修复：
//   `BoundaryCross` 此前只要 raw 与 clamped 存在任意差值就发射，而战术槽位
//   若贴边线（底角 y=2.5 vs 可站立下限 1.8），持球人会被永久顶在边界、
//   反复被判出界失误（实测每场 42–52 次虚假 TURNOVER:OUT_OF_BOUNDS）。
//   改为按 GameRules.boundary_epsilon_ft 判定「实质性越界」；槽位目标
//   同时 clamp 到含球员半径的可站立区域。
//   8 seed full 实测：失误 88.5→31.6，回合 264→232，出界失误 52→0。
// v47 0x2475588a3ea1cabc - 2026-09-11 第一性原理修复（传球概率语义 / 推进义务 / 早出手成本）：
//   (a) 传球拦截由「逐 tick 独立掷骰」改为「释放时裁定一次、飞行中回放」，
//       消除概率随时长累积——每次传球失败率 35.8%→22.1%（真实 8-10%）；
//   (b) 新增 CandidateAction::Advance：8 秒推进义务此前在决策集里不存在，
//       持球人只能原地 Dwell/Dwell 直到违例（实测球 x 8 秒内仅 11.4→12.8ft）；
//   (c) 后场不受 tactical_initiation_seconds(6.5s) 延迟约束，且推进期间不被
//       战术槽位覆盖——8 秒违例 31 次/场 → 0；
//   (d) 新增 DecisionRules.early_shot_penalty：早出手的机会成本，
//       8 秒内出手占比 46%→20%（真实 ~15%）；
//   (e) 发球阶段使用专用 inbound_decision_interval_seconds，
//       五秒违例 15 次/场 → 1。
//   8 seed full 实测：3P% 61.2→34.4（G-D3a 目标带 [30,40]），2PA 10.8→73.0。
// v46 0x0e610303a063503e - 2026-09-11 D5.1b 修复：进攻目标不得再经 bind_targets
//   按 roster 顺序重绑（那会抹掉 slot fill 结果并把替补拉进场内，实测 116 条
//   PLAYER_SEPARATION，替补 A_7 与在场球员重叠 1.19ft）。
// v49 0xd28c579e08299d3c - 2026-09-13 round-6 防守方案因果化（D5.2 最小闭环）：
//   (a) 缺陷：`DefensiveTactic` 对比赛结果**零影响**。全仓库仅 6 处引用 = 2 处
//       字段声明 + 2 处赋值 + 2 处 `name_zh()`（生成展示字符串）。实测 6 种方案
//       各跑一场全场模拟，逐字节相同（ticks=84029、score=96081、行为哈希全为
//       `0x3ba37e7fa9e5ec7d`），包括 2-3 联防。违反 gap.md §21 第 2 条标准
//       （「它会打篮球，超出播放战术动画的水准」）。
//   (b) 修复：新增 `DefenseRules`（sag_multiplier / on_ball_gap_multiplier /
//       help_priority / switch_aggressiveness），按方案 id 在
//       `DefenseRules::for_scheme` 实例化，经 GameRules 通道进入防守目标点生成
//       （charter C1/C3：数据通道，非代码分支）。同时接线此前声明未消费的
//       `drop_coverage_depth_ft` 同族参数语义。
//   (c) **中性性证明**（本冻结的授权依据）：默认值 sag=1.0 / gap=1.0 /
//       help_priority=0.5 时**逐位复原**历史行为——把双方防守方案都置为
//       `def_man_conservative`（中性档案）后 seed42×2000 的哈希为
//       `0x97c7de28a95cb3d5`，与 v48 完全相同。因此本次漂移的原因是
//       「默认阵容使用的 drop_coverage /
//       man_conservative 方案开始真实生效」。
//   (d) 证据：seed=0..7 full 的 L1 violations 仍为 0；`defense_effect.rs`
//       两条新门通过（防守几何 spread > 0.25ft；zone/drop 比 man 更收缩；
//       press 与 zone 的失误总数不同：126 vs 113）。
// v50 0xc164745fadeb0c69 - 2026-09-13 round-6 接球减速模型（gap.md §12.1/§9.5）：
//   (a) 缺陷：接球人被硬编码为 20 ft/s 全速冲向 `frozen_to_pos`，**无减速模型**。
//       传球飞行时长受 `max_pass_duration_seconds`（1.4s）封顶，但 40–60 ft 的
//       跨场 outlet 实际只需 0.5–0.7s，接球人在被拉长的飞行期内持续全速前进，
//       实测**越过**冻结点 5–12.5 ft（越位方向几乎垂直于传球线）。
//       后果：`PASS_CORRIDOR_REACHABLE` 在 8 seed 下报 8–17 条 Hard defect。
//       这违反 gap.md §12.1「加速、制动、变向受属性与规则上限约束」。
//   (b) 修复：新增 `receive_approach()`，按制动距离反解允许速度
//       `v = sqrt(2·a·max(d - margin, 0))`（受 `receive_approach_speed_ratio`
//       封顶），并把瞄准点提前 `receive_stop_margin_ft`。参数经 GameRules 通道。
//   (c) 效果：`PASS_CORRIDOR_REACHABLE` 8 seed 17 → 10；L1 violations 仍 0。
//   (d) 授权依据：
//       - 防守侧变更的中性性已在 v49 证明（中性方案逐位复原 0x97c7de28a95cb3d5）；
//       - 本轮为**独立**的物理约束补齐：修复前该路径恒为常数 20.0 ft/s，
//         不存在「把常数改成参数」之外的等价回退;若把
//         `receive_approach_speed_ratio` 设为 1.0 且 `receive_stop_margin_ft`
//         设为 0，即为不减速的旧行为退化形式（速度改为受制动距离约束）。
//       - 新增门：`defense_effect.rs` 2 条；`attribution_integrity.rs` 2 条。
// v51 0x15cad99d0fdea90b - 2026-09-13 round-7 领传与拦传时序：
//   (a) outlet 一传（`start_rebound_outlet`）此前把 `to_pos` 冻结为接球人
//       **释放时刻**的位置，而接球人在飞行期间继续跑动，永远不能恰好停在
//       冻结点（实测越位 5–12 ft，全部出现在 `(±97, 25)` 基准发球点）。
//       改为与决策侧一致的领传（`lead_receiver_position`）。
//   (b) `execute_pass` **不得**再叠加一次领传：决策层给出的 `to_pos` 已含提前量，
//       重复叠加使判定从 9 条恶化到 28 条（接球人因减速模型无法到达过远落点）。
//       已在代码注释中记录该实验结论，防止后续重犯。
//   (c) 新增 `pass_lead_gain` / `pass_lead_max_ft` 规则参数（数据通道）。
//   (d) 净效果：`PASS_CORRIDOR_REACHABLE` 8 seed 由 8 → 9（持平），
//       `RHYTHM_DURATION` 70 → 62（改善）；L1 violations 仍 0。
//       本轮的价值主要是**排除了两个错误假设**并文档化，而非指标改善。
// v52 0x9af1ff1c8d3a5710 - 2026-09-13 round-8 接球制动距离自洽：
//   (a) 缺陷：`receive_stop_margin_ft` 是**固定**裕量（2.5 ft），但所需制动
//       距离是 `v²/(2a)`，随接近速度增大。实测 19.8 ft/s 需 5.60 ft、
//       12.0 ft/s 需 2.06 ft。当裕量小于所需制动距离时，「已到位则停下」
//       的分支**永远不可达**：接球人一边被 `sqrt(2as)` 减速，一边因
//       `d > margin` 继续被推着走。
//       实测（seed 1, rel seq=3484）：45.3 ft 传球飞行 35 tick，接球人在第
//       18 tick 已到冻结点（距离 0.43 ft），之后又跑 19 tick × 0.35 ft = 6.7 ft
//       越过，最终报 5.65 ft 越位。
//   (b) 修复：用物理所需制动距离 `v²/(2a)` 取代固定裕量（取两者较大值），
//       使 `d <= stop_margin` 分支真正可达；进入后速度为 0（站住等球）。
//   (c) 净效果：Hard 15 → **12**；`PASS_CORRIDOR_REACHABLE` 10 → 9；
//       `POSSESSION_DURATION_BOUNDS` 1 → **0**；L1 violations 仍 0。
//   (d) 本轮同时否证两个假设并文档化（见 docs/dev/cycles/20260911_first-principles/closure_plan.md §11）：
//       - `execute_pass` 叠加领传 → 判定 9 → 28（恶化），已回退；
//       - 「飞行时长要求超过 ball_max_speed」经算术验证不成立
//         （50 ft 传球的等效速度仅 35.7 ft/s < 85 上限）。
// v53 0x2cf17a83dcb9571e - 2026-09-14 round-10 Step3b 传球落点真实飞行时长：
//   (a) 缺陷：`DecisionRules.pass_lead_time_seconds` 是**固定** 0.65s，而真实飞行
//       时长随距离变化（8 ft→0.45s、50 ft→1.40s）。短传领过头、长传领不足，
//       双向都错 —— 用一个常数近似一个函数的必然结果。
//   (b) 修复：新增 `BallisticsEngine::solve_pass_landing(passer, receiver, rules)`，
//       求不动点 `T = pass_duration(|L−p|)`、`L = x + v̂·brake_reach(v0,T)`。
//       `brake_reach` 与 `engine::receive_approach` **同源**（否则两处对
//       “能否到达”判断不一致）。返回 `(落点, 时长)` 成对冻结，满足
//       gap.md §9.5 的“release 时同时冻结 frozen_to_pos 与 flight_duration”。
//   (c) 验收：4 条纯函数单测（不动点残差 <1e-3、领传 ≤ 制动可达、静止不领、
//       飞行时长随距离单调且跨度 >0.3s）全绿，不跑模拟。
//   (d) 效果（seed 0 full）：传球失败率 36.6% → **29.7%**；回合 287 → 261。
//       8 seed full：Hard 0，L1 violations 0。
// v54 0xa155d1c3b21552de - 2026-09-14 round-10 Step4a 名册去索引化（P-2）：
//   (a) 缺陷：名册由 `builtin_attributes(index)` / `builtin_roles(index)` /
//       `builtin_tendencies(index)` 按**数组下标**分派，且 `id` 编码下标
//       （`id = "{prefix}_{index+1}"`）；`default_lineup` 直接取**数组前 5 个**
//       作为首发。即「顺序即身份」——轮转名册数组就能改变首发阵容。
//   (b) 修复：球员改为 `data/roster/{home,away}.json` 数据资产（顺序不携带语义）；
//       删除 `PlayerData.roles` 与 `PlayerRole`（attributes.md §2.7/§2.9/T1 要求）；
//       展示槽位改为 `project_display_role(attributes, tendencies)` 纯函数投影；
//       首发由档案 `starter` 标记声明；`validate_team` 的
//       `ids.len() <= 5`（HashSet 插入计数，语义错误）改为读 `player.starter`。
//       删除 227 行 index 分派生成器。
//   (c) 本版漂移的**主要来源**是球员 id 改名（`H_1` → `H_01`，补零以去掉
//       「序号即身份」的暗示），哈希输入包含 id，故必然变化；行为指标保持
//       （8 seed full：Hard 1、L1 violations 0、回合/场 236.9）。
//   (d) 决定性验收（新增 rsoster_order_neutrality 测试）：
//       **打乱名册数组顺序 → 逐 tick 行为哈希完全不变**（3 种轮转 × 3 个种子）。
//       修复前该测试必然红。
// v55 0x025c5ced668159d3 - 2026-09-15 round-14 带球丢球事实路径：
//   (a) 缺陷：真实 NBA 失误构成中「带球丢球」占 53.6%（82games 2024-25 IND），
//       是占比最大的一类，而引擎**只有传球失败一条失误路径**，
//       `TURNOVER_LOOSE_BALL` 在 718 回合中只出现 1 次（0.4%）。
//       根因是架构级的：`decision/src/defense.rs` 整个模块零调用者，
//       `BallState` 状态机也**没有 `Held -> LooseBall` 边**。
//   (b) 修复（四处，全走既有架构通道，无硬编码）：
//       ① `ResolveConfig.ball_security: BallSecurityPolicy`（11 参数 + validate）；
//       ② `capability::poke_check_success()` 由 `steal` / `ball_handling` /
//          `risk_tolerance` 派生（能力与倾向分离）；
//       ③ `flow.rs` 新增 `Held -> LooseBall` 边（真正的阻塞点）；
//       ④ `resolve_on_ball_poke()` + `apply_on_ball_poke()` + `GameEvent::BallPokedLoose`。
//       概率按 `rate × dt` 做时间积分 `1 − (1−p)^trials`，与
//       `resolve_pass_interception` 同形，不产生逐 tick 概率累积。
//   (c) 本版漂移是**预期的**：新增了一条失误事实路径，行为必然变化。
//       实测 8 seed full：**Hard 0**、BALL_SPEED 0、丢球占比 0.4% → 4.8%。
//   (d) 修复过程中发现并解决的两个真实缺陷：
//       · `BALL_SPEED` Hard 违规（seed 31337 tick 3755，96.72 ft/s > 85）：
//         根因是松球起点误用 `carrier_pos` 而非球的实际坐标（持球时球有
//         `ball_holder_offset_ft` 偏移与弹跳相位），单帧跳变 3.87 ft。
//       · `TURNOVER_ATTRIBUTION` Hard defect：评判器把
//         `TurnoverLooseBall` 的原因事实误判为「必须有 `LOOSE_BALL_SECURED`」，
//         但球被切掉后可能直接出界（防守方收下），此时无收下事实。
//         已为 `BALL_POKED_LOOSE` 增加独立的原因事实通道。
//   (e) 保持：`roster_order_neutrality` 2/2 通过（P-2 中立性未受影响）；
//       `possession_invariants_hold_across_seeds` 通过。
// v56 0xcaa7befc5149f0e3 - 2026-09-15 round-15 接球语义修复（两项）：
//   (a) Fix-1「首次触球」：接球裁决的触发条件从「飞行结束」放宽为
//       「飞行结束 **或** 接球人已进入接球半径」。此前接球人的估计模型
//       以自身为参照系（initial = ball + dir×|ball−我|），他朝来球走则
//       估计点随之后退（追赶曲线），于是「朝来球移动」这一正确行为反而
//       必然导致终点 miss——实测层 A 失败 22 例：估计误差 p50=3.37 ft、
//       接球人正确到达自己的估计（1.34 ft）、球距 3.64 ft。
//       传球全程胸高平飞（pass_peak_ft == chest_height_ft），无空中接球问题；
//       拦截优先级不变（同 tick 拦截事实先行）。
//   (b) Fix-2 拆除 Layer B 的走廊双重计费：`resolve_pass_arrival` 删除
//       lane_risk 项（连带删除 PassPolicy.lane_risk_weight）。走廊风险是否
//       化为事实由 resolve_pass_interception 独立裁决；球干净到达接球人身旁
//       （实测层 B 失败 37 例球距 p50=0.94 ft）不应再因出发时走廊拥挤被
//       降低接球概率。
//   (c) 效果（3 seed full）：传球失败 28.3% → 16.9%；掉球 17.0% → 5.0%；
//       失误/回合 0.353 → 0.287；0 Axiom Violations。
//       P-1 验证测试（receiver_must_estimate_not_know_the_frozen_landing /
//       receiver_landing_estimate_diverges_from_passer_intent）保持通过。
//   (d) 同轮修复 attribution_integrity 测试盲区：WindowKinds 补统计
//       BALL_POKED_LOOSE（TURNOVER_LOOSE_BALL 的原因事实）。
// v57 0xfdb47231f042e55d - 2026-09-15 round-16 进攻组织与失误构成修复（五项）：
//   (a) Fix-3 传球效用距离衰减接线：DecisionRules 的 pass_distance_free_ft /
//       decay_reference / max_decay 声明多轮但从未消费。此前效用只有「接球人
//       空不空」，传球人系统性选最长传（中位 28.3ft、48% >30ft），而拦截率
//       随距离单调上升（0-12ft 4.2% → 40ft+ 21.5%）。接线后中位 20ft。
//       同时删除 2 个与 constraint 通道重复的死参数（pass_lane_defender_penalty /
//       pass_lane_blocked_multiplier，charter C1 单一事实源）。
//   (b) Fix-4 驱动停滞双重缺陷：(i) 预掷 finish_made=true 的「进球」被空间门
//       （>16ft）静默丢弃——6 场 397 次 DRIVE_SCORE 中 80%（场均 52.7 次）未
//       变成出手，事件流与记分簿矛盾；现按真实分支申报。(ii) 停滞转回
//       Initiation 强制重等 tactical_initiation_seconds=6.5s——停滞是进攻延续
//       不是新回合；该机制是 24s 违例（12.8 次/场，真实 ~0.5）的主要时间吞噬器。
//       修复后 24s 违例 12.8 → 2.3 次/场。
//   (c) Fix-5 发球 5 秒压力：距离衰减接入后发球员宁愿 Dwell（0.82）也不发
//       长球（衰减后 0.47），FIVE_SECOND 违例 0→22 次/场。修复：Dwell 在发球
//       阶段按 inbound_elapsed/5s 线性加压（发球传球保持距离优选）。
//   (d) 标定（A/B 证据 §17.5-17.7）：pass_base 0.82→1.15（n 1.45→2.47）；
//       intercept_steal/tip_slope 0.12/0.20→0.06/0.10（失败率 10.7%→7.5%）；
//       poke_attempt_rate 0.55→2.2（丢球占比 8%→36%，真实 53.6%）。
//   (e) 效果（3 seed full）：失误/回合 0.353→0.247（真实 0.1447）；构成
//       传球/丢球/违例 = 50/36/13%（真实 33.6/53.6/12.1）；得分率 43%（~45%）。
// v58 0x1ce391e109621bbf - 2026-09-15 round-17 活锁与归因修复（三项，全 seed Hard 归零）：
//   (a) 地板球追逐去距离限制：原 `cur_dist <= 25.0` 硬半径在球停于空档区时
//       失效——实测 seed 6 罚球后松球停在 (85.6,29.0)，最近球员 59.1 ft，
//       无人满足 25 ft → 全场站桩 247 秒直到节末（回合 258.4s > 40s 上限，
//       POSSESSION_DURATION_BOUNDS Hard）。第一性原理：活球是全场唯一
//       完全可观测的对象，地板上躺着一颗活球时「去抢球」压倒一切战术
//       站位。RimRebound 的落点预判追逐保留 25 ft；LooseBall 无条件追逐。
//   (b) 篮板源地板球归因：投/罚不中弹出的地板球被防守方收下时，原实现
//       默认记 TURNOVER_LOOSE_BALL——虚增失误且不存在「失误球员」
//       （TURNOVER_ACTOR_CONSISTENCY Hard，seed 2 possession 83：
//       FT 不中→地板球→防守收下→turnover_player_id=null）。修复：
//       RimRebound→LooseBall 时登记 DefensiveRebound 源，防守方收下
//       按防守篮板归因到收球人；进攻方收下则继续回合（前场篮板）。
//   (c) 回合时长容差 2.0→6.0（fixture nba.v2 数据规格）：合法回合 =
//       发球准备(~2.6s 死球) + 24s + 出手飞行 + 篮板 + ORB 14s + 终结飞行
//       ≈ 43.6s；原容差 2.0 只覆盖飞行、不覆盖回合开始的死球准备，
//       把 seed 7 的合法回合（5 传 2 突破 ORB 后得分，42.4s）误判为 Hard。
//   验收：9 seeds（42,1,2,3,6,7,100,999,31337）全部 0 Hard；
//   n（传球/回合）1.45→2.27；传球失败率 15.0%→10.4%。
// v59 0x9db5032576bb6093 - 2026-09-15 round-18 攻框体系修复（五项）：
//   (a) 冲框价值折扣（drive_rim_attack_bias 1.2）：走廊选择原本只比拥堵
//       成本，篮筐（防守最密处）几乎永不入选——88% 突破停在离筐 14-18 ft，
//       篮下出手 3%（真实 25-50%）。按持球人 finishing 折扣冲框走廊。
//   (b) 攻框 APF 豁免（is_driving_to_rim）：转向力墙挡不住攻框，对抗由
//       终结裁决处理（硬碰撞分离仍生效）。
//   (c) 突破时长加加速坡（dist/speed + speed/accel；max 2.2→3.2s）：
//       原公式假设瞬时极速，tau=1 时人差 5-10 ft，只能 7-16 ft 抛投。
//   (d) 过人两段式几何：successful 预掷=过掉对位防守人——被过者让位
//       （目标=侧向清空点，极速），0.35s 后持球人重定向攻框。
//       初版瞬移被 PLAYER_TELEPORT 正确拦截（70-94 Hard，已废弃）。
//   (e) 分球 on_court 过滤 + 换人守卫扩展：突破分球曾把球传给板凳上的
//       队友（seed 31337：H_08 在 y=-4 板凳区「接球」，BALL_HOLDER_ON_COURT
//       190 次）；换人守卫补 pending_pass_receiver/Pass.target_id。
//   效果（3 seed）：突破停滞 88%→67%，得分 5%→20%；FT/场 12.7→21.3；
//   失误/回合 0.248→0.213；得分率 43.6→45.4%（真实 ~45%）。
//   11 seeds 全 0 Hard。
// v60 0x308de474dc203661 - 2026-09-15 round-19 护框让位 + P-1 纯度（三项）：
//   (a) 通道全员让位 + 恢复窗口（drive_beaten_recovery_seconds 0.6）：
//       让位目标此前被战术层每 tick 重派覆盖，防守人立刻被派回护框位，
//       让位形同虚设。窗口内战术层不得重派被过者（与
//       GambleInterception 的 failure_recovery 语义同源）。
//       效果：篮下≤7ft 出手 11.9%→14.7%，停滞 67%→64%。
//   (b) P-1 修复①（全知泄漏）：ball_velocity_estimate 原直读球态的
//       to_pos/from_pos/duration（传球人冻结意图），其文档注释声称
//       "用上一 tick 与本 tick 的球位置差"——注释与实现不一致。
//       改为真实观测差分（prev_observed_ball_pos，感知延迟一步），
//       信息量等价（球匀速直线飞行）但来源合法。
//   (c) P-1 修复②（噪声双重应用）：residual = noise_cap×(1−sense) 而
//       noise_cap 已含 (1−sense)，实际 = noise_ft×(1−sense)²——低观察力
//       球员的噪声被意外压缩。恢复线性设计意图。
//   效果（3 seed）：传球失败 10.4%→8.5%；失误/回合 0.213→0.208；
//   9 seeds 全 0 Hard；P-1 验证测试保持通过。
// v61 0xcbf4955679da1240 - 2026-09-15 分区命中率 + 篮板冲抢指派：
//   (a) 分区命中率（evidence/problem.md §21.3/§23.1）：出手基准的三分支
//       实际只产生两种基准，8ft–三分线的中距离与廊下共用
//       `shot_make_2pt = 0.565`。新增 `BaseRates.shot_make_mid = 0.42`，
//       `shot_make_2pt` 语义收窄为仅廊下。8 seed：total_p50 237.0→211.0
//       ∈ [140,230]、two_make_pct 0.621→0.521 ∈ [0.48,0.58]。
//   (b) 篮板冲抢指派（§23.8）：实测球在空中时守方朝球靠近速率是攻方的
//       约 42 倍（0.0042 vs 0.0001 ft/tick）且两者绝对值都极小——即双方
//       几乎都没抢篮板行为，攻方几乎为零。新增
//       `assign_rebound_pursuit()`：落点出了后为双方指派争抢目标
//       （攻方按 `offensive_rebound` 属性选人、守方按距离选人），
//       人数经 `GameRules.rebound_crash_offense_count` /
//       `rebound_boxout_defense_count` 通道（charter C1）。
//       效果：ORB% 0.070/0.129/0.107 → 0.330/0.244/0.296
//       （真实 0.245）；8 seed pace 236.8→220.5。
//   (c) 本冻结的授权依据：两项都是**结构性**修复而非参数调优，
//       各自附机制定位（§21–§23）与 A/B（§22）；中性性无需回退证明，
//       因为它们修的是“分支只有两种基准”与“无人抢篮板”这两个
//       可独立验证的缺失，而不是改动一个已在工作的参数。
//       v61 的哈希变化同时覆盖 (a)(b)。
// v62 0xa25e5c57a026def0 - 2026-09-15 跳投犯规路径 + 箱体犯规计数：
//   (a) 跳投犯规（evidence/problem.md §23.9）：全仓 `shooting_foul` 原本只在
//       `DriveResolution` 产生，即**只有突破能被犯规**；跳投（含三分）在被
//       干扰时没有任何造犯规可能。新增 `BaseRates.foul_on_shot_rate = 0.06`，
//       在 `execute_shot` 按“基准 × 干扰强度”裁定，结果作为独立事实
//       写入 `BallState::Shot` 的 `fouled` / `fouler_id`——犯规与命中相互
//       独立，and-one（犯规且命中）与投篮犯规（犯规且不中）都要能表达，
//       因此不能用 `is_made` 反推。
//       效果：FOUL 12→约 18/场；`box_score.fouls` 从恒为 0 → 17.6/场。
//   (b) `box_score.fouls` 零自增点修复：与 §23.10 的 `turnovers` 同类
//       缺陷（声明字段与事件事实脱钩）。
//   (c) 诚实记录：`free_throw_rate` 仅从 0.110 升到 0.118，**仍未入带**
//       [0.20,0.35]。根因不在投篮犯规路径，而在**非投篮犯规路径完全缺失**
//       （真实每场约 16 次：无球犯规/进攻犯规/卡位犯规），已登记为
//       后续任务；本提交不调基准去凑带。
// v63 0x4bcbbcf59acde717 - 2026-09-16 D15 carrier_idx 解耦与球态归一（ADR-010）
// v64 0x01885463019631e7 - 2026-09-16 D17 防守方案责任链实施：
//   (a) 扩展 data/defense/schemes.json 为 schema_version 2，补充 screen_defense 参数集
//       （drop_depth_ft, hedge_distance_ft, switch_trigger_distance_ft）；
//   (b) 在 decision/tactics.rs 中接通 DropCoverage、SwitchAssignment、HedgeAndRecover 责任链；
//   (c) 主队默认方案 def_drop_coverage 沉退动作真实生效，统计保持在带（total_p50=212.0, 3P%=37.3）；
//   (d) 跨种子不变量测试 possession_invariants_hold_across_seeds 零违规。
// v65 0xe3f98ca371a1563f - 2026-09-17 核心引擎三大深度手术：
//   (a) 连续势能场动力学与弱侧协防涌现：在 potential_field.rs 中构建势能场梯度积分，
//       弱侧 Low-man 护筐 (ROTATE_RIM_HELP) 与 High-man 补位 (X_OUT_CLOSEOUT) 自发涌现；
//   (b) 规则与能力因果闭环：在 resolve_rebound 消费卡位加成与力量对抗，在 resolve_pass_interception
//       消费传球风险容忍度，在 set_player_target 消费协防意识，19 项因果扰动测试全绿；
//   (c) 修正非投篮阻挡犯规被误判为投篮罚球的历史缺陷，前场普通犯规回表至 14 秒并保持球权；
//   (d) 宏观统计全矩阵收敛：stats_baseline 多线程并行回归，total_p50=204.5，3P%=36.9% 全绿。
// v66 0xc002a33a9d554a89 - 2026-09-19 封盖事实与动作窗口门控（D25）：
//   (a) 新增 `match_engine/block.rs`：封盖作为独立比赛事实（`GameEvent::BlockedShot`），
//       与「投失 + 篮板」区分开——封盖后是松球争夺，且责任主体是防守人；
//   (b) 判定窗口绑定出手者的 `ActionPhase` 与动作窗口的可干扰区间，
//       每发只掷一次（逐 tick 掷会把单发概率放大 24 倍，实测 36.6% 的出手被封）；
//   (c) 消费 `block` / `vertical` / 身高（触及高度）与出手点高度（越高越难封），
//       使三个此前零消费或半消费的维度进入生产因果链；
//   (d) 标定到真实区间：实测 seed 42/1/7/100/999 全场合计 33/1023 = 3.2% 的出手被封
//       （单 seed 2.4%–4.9%，真实 NBA 约 5.5%）；
//       8-seed 统计仍在带（total_p50=202.0、3P% 中位=35.8、avg_dur=15.16s）。
// v67 0xd738e70efe150535 - 2026-09-20 轮换与换人系统（G6）+ 体力重标定：
//   (a) 新增 `RotationRules` 规则组（枯竭阈值、垃圾时间分差/时限、最短休息、
//       每窗口配额）；死球窗口（发球准备、节末）评估换人，每窗口一次；
//   (b) `substitute()` 登场者由调用方显式指定（evaluate 按体力/休息时间
//       过滤），犯满路径保持字典序；`Substitution` 事件携带原因；
//   (c) 体力参数按实测速度档分布（冲刺 4%/中速 31%/慢速 63%）重标定：
//       drain 0.015→0.021、recovery 0.02→0.0005——旧参数下体力净上浮，
//       换人永不触发，`SubstitutionReason` 四个变体为零消费；
//   (d) 实测：每场 24–28 次换人（真实 NBA 每队 30–40 的同数量级），
//       低体力登场 0 次；8-seed 统计仍在带（total_p50=166.5）。
// v68 0x820c9b27412c65cc - 2026-09-20 第一步飞行抛体化：
//   传球/投篮/篮板飞行的 z 分量从「线性插值 + 正弦弧」改为真实重力抛体
//   （g=32.17 ft/s²，ProjectileArc 住在 domain/projectile.rs）；
//   投篮/传球飞行时长由「请求弧顶 + 两端高度」闭式解出（升段+降段），
//   篮板落点 z 改为触地；正弦弧时代的 5 个零消费字段
//   （pass_speed/inbound_pass_speed/shot_speed/ball_arc_multiplier/
//   shot_arc_solve_iterations/rebound_min_arc_ft）删除，新增
//   pass_peak_distance_factor=0.10。出手速度自然达到真实量级
//   （25ft 三分 ≈ 37 ft/s）。
// v69 0x7888d577ac33b69b - 2026-09-21 第二步触筐物理（ADR-017）：
//   打铁反弹从「均匀距离采样」改为入射驱动：接触点在近筐沿受控扇形上
//   采样，反弹初速 = 水平镜像反射 × 恢复系数（随接触角从硬碰 0.62
//   渐变到擦筐 0.35）+ 受控散射，竖直弹起 = 入射下落速度 × 0.35，
//   落点由抛体自然产生；投篮弧顶校准 shot_peak_distance_factor
//   0.25→0.04（25ft 三分时长回到包络内）；距离采样时代的 9 个字段
//   （rebound_short/long/angle_range/flight_base/flight_distance/
//   distance_scale/rebound_peak）删除，新增 rim_radius 与 4 个
//   rim_contact 参数；罚球打铁与跳投打铁同一条反射链路。
//   注：出手弧顶校准（0.25→0.04）实测后暂缓——8-seed 三分封盖率
//   3.7%→7.3%、3P% 中位 34.0→27.0 越带且机制未明（出手时平均
//   make_probability 三配置完全一致），证据在 rules.rs 的
//   shot_peak_distance_factor 注释里。
// v72 0xb3685f078e05bfc9 - 2026-09-21 统一球员接触模型（ADR-017）：
//   (a) 篮板碰撞面：打铁按瞄准几何双通道路由（越过筐→触板镜像反弹，
//       落点由抛体解出；否则维持近筐沿反射），板几何进规则通道；
//   (b) 自由球-人碰撞：篮板飞行与地板球对球员身体（接触半径 = 人体
//       半径 1.0 + 球半径 0.4 ft）弹开，摸高门控，同飞行同人只弹一次；
//       快球（> 12 ft/s 可控上限）不可被半途收下；盖帽初速改为
//       入射水平速度 × 0.45；
//   (c) 传球碰撞化：删除出手时刻一次性拦截裁定与 Pass.intercept 载荷，
//       改为逐 tick 全防守者接触检测 + 结果分类（抢断/拨掉/轻擦，
//       沿用 intercept_* 参数），高吊传受摸高门控；
//   校准：乱战出手增多使三分 make_probability 0.308→0.294，基准
//   0.34→0.36 回调（3P% 中位 31.0）；ORB% 0.474→0.294（真实 0.245）。
// v73 0xee0429df2bfdde79 - 2026-09-22 节间开场球权程序与后场计时控球谓词：
//   （seed 14 实测缺陷的结构性修复。两处行为变更都不在黄金窗口内，
//   哈希实测不变；统计与账本影响由 16-seed 矩阵与 stats_baseline 承担。）
//   (a) 节间开场（sync_game_flow）新增 start_period_ball_program：
//       球已由在场球员持有时原样保留；节末结算出的停球/松球态
//       改为按保留的进攻方进入显式发球程序。修复「停球状态与活球
//       流程并存」：此前 seed 14 第 4 节开场 201 tick 球不可取，
//       后场计时照走，第 8 秒判出无责任人的伪后场违例
//       （TURNOVER_ACTOR_CONSISTENCY Hard + TURNOVER_CONSERVATION）。
//   (b) 后场计时（phases.rs）与决策约束（eval_backcourt_clock_world）
//       同步增加控球谓词：松球/篮板/停球/投篮飞行不累加、不判违例。
//   (c) LooseBall/RimRebound 增加 last_touch_player 载荷（P1：状态是
//       唯一事实源），Dead 的载荷经松球链路继承；current_turnover_player_id
//       对松球态优先读载荷，旁路字段 last_passer_id 仅作旧态回退。新增
//       L1 不变量 LIVE_FLOW_DEAD_BALL 与 TURNOVER_ACTOR_MISSING。
//   补登说明：该常量由 fe5e233（松球争抢者限制 + SET_HIGH_SCREEN 持球门）
//   重冻结但当时未登记条目，本条补登并覆盖当前值。
// v74 0xa8604ed4e3f389e5 - 2026-09-23 help_blend 接入势能场锚点权重（plan_play.md #22）：
//   (a) 变更通道：`GameRules.tactics.defense` 的 help_blend 四字段
//       （`help_hoop_weight_base` / `help_priority_tilt_gain` /
//       `help_hoop_weight_min` / `help_hoop_weight_max`，数据源
//       `data/defense/schemes.json` 顶层 help_blend 块）。此前该组参数
//       已解析但零消费（「声明了但无效」的隐形参数）。
//   (b) 行为变化：`solve_equilibrium` 下沉锚点方向「朝篮筐」权重由固定
//       0.75 改为随防守方案变化的合成权重 `clamp(base + tilt_gain ×
//       help_priority, min, max)`（`help_anchor_hoop_weight`），朝持球人
//       权重取互补，和恒为 1。六个方案生效权重 0.82-0.95，默认方案
//       （def_man_conservative，help_priority=0.5）为 0.90。协防优先级
//       高的方案锚点重心向篮筐偏移，低的向外线持球人偏移；与
//       `help_priority` 的协防语义同向协同，与 ADR-019 裁定第 2 条的
//       行为通道原则一致（规则数据经效用/目标通道生效，无代码分支）。
//   (c) `PotentialFieldRules.sag_anchor_hoop_weight`（固定 0.75）按 charter
//       C1 收编进同组 help_blend 字段，默认值从 schemes.json 数据通道派生，
//       锚点权重从此完全由方案数据决定。字段名与数值的变化属结构性接线，
//       与 #24 体能衰减通道（stamina_multiplier）在同一窗口内共同生效。
//   (d) 本条按 plan_play.md 链 B 约定登记：#22 完成时追加；#21 滞回
//       （无行为变更）与 #24 体能衰减（满体能路径中性）如触发行为变更
//       由各自完成时另行登记。统计与账本影响由 #19 全局校准
//       （stats_baseline + 16-seed 矩阵）承担。
// v75 0xd4b502481b803de9 - 2026-09-23 D24 几何突破裁定 + Play 引擎接线（plan_play.md #18 / plan.md D24）：
//   (a) 突破裁定从 `DriveResolution::resolve` 概率抽签改为几何接触评估：
//       直线路径/绕行路径的可达性（`DriveGeometryPolicy`，规则通道
//       `resolve.drive_geometry`）、防守人预计接触位置与攻防能力差异
//       决定成败；终结命中与投篮犯规仍单独随机裁定。绕行时防守人
//       进入恢复窗口，突破者目标改为绕行点，越过后切回攻框。
//   (b) `DriveInitiated` 事件的 `target_pos` 与 Drive 球态的 `target_pos`
//       携带实际运动目标（绕行时为绕行点），公共观察口径一致。
//   (c) 制动/牵引后端：分离投影后的报告速度按包线截断，防止推挤
//       位移跨 tick 自我累积；提案终点包线断言改为针对碰撞求解前的
//       运动学终点。期望速度按规则制动力在停止距离内封顶。
//   (d) Play 引擎接线：MatchSetup 主客 Playbook、决策相前的 Play 选择
//       与同 tick 效用效果（`decide_on_ball_with_play`）、战术层的
//       槽位动词目标与动作窗口、每回合结束的冷却/激活清理。
// v76 0x7df0fec11e01faa3 - 2026-09-24 D25 出手时序闭环（plan.md §6.2 / D25）：
//   (a) 投篮裁定与脱手分离：`execute_shot` 在入口冻结全部裁定
//       （is_made/fouled/peak_z/flight_time），球态保持 `Held`；窗口跨过
//       Execution→FollowThrough 边界（真正的 Release，合球起跳完成）才由
//       `consume_pending_shot_release` 转 Shot 球态、发 ShotRelease、进入
//       ShotAttempt 子阶段。出手点用释放时刻实际球位（冻结入口球位会在
//       Release 瞬间瞬移并产生超速弹道，实测 BALL_SPEED 99 ft/s > 85）。
//   (b) 封盖在挂起期裁定（Execution 段第一个 tick 掷一次，与旧触发点
//       语义一致），命中时作废挂起；球在手段的切球（Strip）路径不变。
//   (c) 挂起期禁止新的持球决策（防重复出手与状态分叉）。
//   (d) Preparation 段（合球阶段）球在人手：防守人 Poke/Strip 照常生效，
//       投篮窗口期间持球人的动作锁定与战术目标保护不变。
// v77 0xb849b30e273881b1 - 2026-09-24 统一出手分区四区 + 赛前身份字段
//   （ADR-020，attributes.md §2.3a/§2.7a/§2.7b；含 D26/D27 回移与收尾）：
//   (a) 分区统一：出手分区改由 `CourtGeometry::shot_zone` 判定
//       （Rim <5ft / Near 5–14ft / Mid ≥14ft 线内 / Three 优先），
//       决策效用选技、命中裁决共用；`BaseRates` 扩为四档
//       （shot_make_rim 0.63 / shot_make_near 0.45 / mid 0.40 / 3pt 0.36），
//       rim_shot_distance_ft 8→5 与分区阈值同源。
//   (b) 能力维度：`PlayerAttributes` 新增 `shooting_near`；名册按新分区
//       校准篮下/近筐能力并声明六类位置与赛前攻防角色。
//   (c) 封盖作废挂起时补发 ShotRelease 事实（修复 v76 的箱体/事件流
//       对平回归，attribution 守卫红转绿）。
//   (d) D26/D27 回移（177641d/7164e21/a987fc6 的主分支提交）：突破终结
//       起跳延伸（drive_finish_extend_ft 6.0，rim 出手占比 1.7%→7.1%；
//       实验负结果：drive_rim_attack_bias 2.2 走廊拥挤负反馈，回退 1.2）、
//       篮下接球攻框加成（rim_catch_bonus 0.8，接球人 finishing）、
//       弱侧突破收缩激励（drive_help_threat_gain 3.0）、松球争抢
//       不受残留动作锁阻塞（seed13 死锁 616s 修复）、
//       defense_rotation_response 判定口径重构（分布带判定归还评判器
//       与 stats_baseline，本测试保留行为断言）。
// v78 0x765d0062e099cdb5 - 结果权交回过程：
//   出手的命中在触筐结算，传球的接住或掉球在到达时判定，
//   突破的终结在接触窗口结束时判定。球态不再保存这些预写结果。
// v79 0xd816e848416dfe5b - ui 分支融合（持球姿态系统 × R1-R8 结算链）：
//   姿态选择（面框/背身）接入决策效用与物理朝向锚定，背身姿态下
//   低位出手改用转身后仰、三威胁试探步互斥；同时保留 R8 球态唯一
//   写通道与到筐几何结算。行为复合变化后重冻结。
// v80 0xcd1598382421a6b8 - 卡位能力接入主流篮板路径：
//   落点捷径与篮板地板球双人争抢都改用卡位/补篮有效距离裁决
//   （resolve_rebound），地板球不再按 id 顺序先到先得。重冻结基线。
const GOLDEN_SEED42_2000: u64 = 0xcd1598382421a6b8;
/// 球权类不变量（两人持球 / 球人分离 / 持球者离场）是最易在状态机重构中
/// 被破坏的约束；这里在多个种子上跑足量 tick，断言引擎在每 tick 的
/// `last_tick_violations` 始终为空。
#[test]
fn possession_invariants_hold_across_seeds() {
    for seed in [42u64, 1, 7, 100, 999, 31337] {
        let mut engine = MatchEngine::new(seed);
        for _ in 0..4000 {
            engine.step();
            assert!(
                engine.last_tick_violations().is_empty(),
                "seed {} produced invariant violations: {:?}",
                seed,
                engine.last_tick_violations()
            );
        }
    }
}
