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
const GOLDEN_SEED42_2000: u64 = 0x3316f6c9d9051602;
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
                engine.last_tick_violations.is_empty(),
                "seed {} produced invariant violations: {:?}",
                seed,
                engine.last_tick_violations
            );
        }
    }
}
