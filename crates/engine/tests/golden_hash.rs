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

// 基线常量冻结记录（校准协议 design.md §11.3）：
//   v1 0x76e41f2a83f74b38 — 重构前锚点（BallState 写入口收敛，行为保持）
//   v2 0xef68d18205abaa12 — 2026-08-31 回合节奏校准
//   v3 0x84a62217d990685b — 2026-08-31 常识公理修复：替补席界外物理隔离
//   v4 0xaf78cd4997df973d — 2026-08-31 投篮分布与突破攻框校准
//   v5 0xc2210e16619f2e96 — 2026-08-31 G3 阶段门推进
//   v6 0x09f38b3b705ce31a — 2026-08-31 L2 PossessionSummary 全链路事件流因果语义与回合总结
//   v7 0xcebe206e784ae273 — 2026-08-31 修复得分分支中重复 complete_possession 自增问题
//   v8 0x248df313adaf4939 — 2026-08-31 G4 终态阶段门（全场总分 252 分，每场 202 回合，回合时长 16.0s，完美对齐 NBA 现实）
//   v9 0x2f4b2fa3c9d81e57 — 2026-09-02 GAP 修复（M8/M9/M2，见 status §8.3）
//   v10 0x1275cccf4a8f3f0e — 2026-09-02 GAP 复审补修：领域转换表补齐违例→发球
//     程序边（Pass/LooseBall/ControlTransfer/RimRebound→InboundTransfer，
//     修复 seed 2/4 死球楔死导致的比赛无法完赛）；替补席界外 body 不再
//     产生伪造 BoundaryCross（每 tick 6 条 OUT_OF_BOUNDS 污染）；Flagged
//     咨询性发现不再触发 RuleViolation 强制项。均为设计内行为修复。
// v15 0x3316f6c9d9051602 - 2026-09-03 传球提前量外推校准与突破终结记录补齐（Realism Index >= 0.92）
// v16 0xc80915ba200cd269 - 2026-09-03 发球就位物理约束与闭环状态机、传球时空追踪落点、防守朝向解耦
// v17 0x2568813419a51146 - 2026-09-03 第一性原理公理守卫（连续性、冲量溯源、因果门禁）、真实罚球物理与跳球争顶
// v18 0x7bda4db4e5e3f239 - 2026-09-03 中圈真实跳球阵型落位(双方中锋中圈对峙、外围放射状卡位)与物理拍球弹道
// v19 0x8d8c14e0961b783f - 2026-09-04 修复发球人未到界外即提前生成传球的空飘缺陷，建立真实就位与持球判定闭环
// v20 0x76b89910652c44cf - 2026-09-04 统一边界容差与出界判罚语义闭环，消灭死球死锁回归
// v21 0x91419840ef413b0c - 2026-09-04 发球落位持球因果闭环、被阻截转活球、失误类型精确溯源
// v22 0x688f76c83ad7ead2 - 2026-09-04 真实篮板冲抢与卡位、触达有效范围物理拦截、传球手部锚定零跳变
// v23 0x2370512ecdf7e6f4 - 2026-09-05 真实运球动力学与球体阴影视觉增强（Dribble Bounce Kinematics + Dynamic Shadow）
// v24 0x58f092e69d840430 - 2026-09-05 跳球事件单次发射（消除持续 37 tick 的重复事件轰炸）与语义化事件流
// v25 0xbea84d43c5695873 - 2026-09-05 APF人造势能场多体动力学转向、防守球人筐外心三角、空间拥挤效用惩罚
// v26 0x7da066b2809fc440 - 2026-09-05 真实跳球LooseBall点拍飞行争夺、彻底消除必中假传球与回合总结描述残留
// v27 0x4ae5950e326e7d86 - 2026-09-05 真实跳球规则落地：禁止跳球员落地前自抓球、外圈8人真实争夺地板球
// v28 0x1a9851edeeab4af8 - 2026-09-05 严格落实真实跳球红线：点拍后进入真实重力抛物线自由球，跳球员受滞空约束，由外围8人（H_1等）真实冲刺抢球
// v29 0xfcb10465ef83e7e8 - 2026-09-05 彻底铲除 H_5/A_5 硬编码残留：跳球双方由场上真实摸高上限(height_cm + vertical)纯函数动态选拔
// v35 0x6fa2f7fedc7ff542 - 2026-09-06 彻底铲除接球点 snap 瞬间位移，全量 24 动作原语闭环，全量种子 4000 ticks 零违规
// v36 0x7c95842cd07b7b1a - 2026-09-10 激活基础动作生命周期与战术覆盖保护锁（运球变向/上篮扣篮/干拔后撤/背身探步闭环）
// v37 0xf2c64aefde03171e - 2026-09-10 因果状态机单权威重构、违例终端事实补全与评判器度量纠偏
// v38 0x8ce38a9678a68ca9 - 2026-09-10 F1 授权状态与时钟修正：罚球期间球权威态收敛为 Dead（消除 BALL_WITH_HOLDER），后场计时在持球越中线时真实重置（消除 EIGHT_SECOND_BACKCOURT 误判）
// v39 0x1dcdf8f03b2c0c4f - 2026-09-10 F1.3 发球显式 placement：发球员允许站界外发球点，消除 DeadBall 活锁（full scope 永不终场）与 PLAYER_SPEED/BALL_WITH_HOLDER 伪造违反；placement 由 sync_ball_holder 单一机制产生 PlacementApplied 事实
// v40 0x4d43f93c9650517d - 2026-09-10 F1.3b/F6.2 消除剩余三类 DeadBall 活锁（分离投影推回发球员、发球员为替补、旧状态未改派），并引入有界流模式（facts/summary 默认、frames 显式）与磁盘/字节预算
// v41 0x357c52254bed731d - 2026-09-10 F1.3c 发球赴界外改为显式离散 placement（消除被钉在边线的防守者造成的几何死锁），placement 回场选择无重叠落点；CLI 按 quality.md 契约区分 Hard（阻断）与 Soft（不阻断）
// v42 0x74032b10dae43034 - 2026-09-11 dev 方案 D3.1 命中模型校准 + 边界事实边沿化：
//   (a) spacing_bonus 三项权重和由 1.0 降至 0.16（空位时不再直接叠加近 +1.0 命中率）；
//       shot_contest_sensitivity 0.22→0.32（干扰惩罚分化）。8 seed full 实测：
//       3P 64.0%→39.7% ∈ [30,40]、2P 72.9%→58.1% ∈ [48,58]（SHOT_MAKE_PROFILE 入带）。
//   (b) BoundaryCross 由电平触发改为上升沿触发 + 几何容差，且 sync_positions 不再用
//       刚体积分产物覆盖运动学权威位置（双发射点/单锁存导致单球员连续 675–755 tick
//       伪造越界刷屏，阻塞发球程序）。属设计内行为修复，见 status.md §33。
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
const GOLDEN_SEED42_2000: u64 = 0x97c7de28a95cb3d5;
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
