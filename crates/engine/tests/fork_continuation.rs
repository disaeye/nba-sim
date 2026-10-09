//! 检查点分叉的位精确续跑验收（validation.md §7.1 引擎侧前提）。
//!
//! `MatchEngine::experimental_fork` 复制整场比赛状态；本测试验证：
//! 1. fork 不加扰动继续跑，与母局的逐 tick 哈希完全一致（多个检查点位置
//!    × 多个 seed 覆盖，检查点落在回合中段/死球/罚球等不同子相位）；
//! 2. fork 是真复制：母局继续推进不影响副本（副本先行跑 N tick 后，
//!    母局续跑哈希仍与直接跑完整场一致）。
//!
//! 哈希函数与 golden_hash.rs 的 `hash_tick` 同一逻辑（确定性相关字段子集）。

use nba_engine::MatchEngine;
use nba_protocol::StreamTick;

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

/// 母局跑 `checkpoint_ticks` 个 tick 后 fork，双方各自续跑到 `total_ticks`，
/// 两段续跑的逐 tick 哈希必须一致。
fn fork_continuation_matches(seed: u64, checkpoint_ticks: u64, total_ticks: u64) {
    let mut mother = MatchEngine::new(seed);
    let mut mother_hashes: Vec<u64> = Vec::with_capacity((total_ticks - checkpoint_ticks) as usize);
    let mut fork_hashes: Vec<u64> = Vec::with_capacity((total_ticks - checkpoint_ticks) as usize);

    let mut fork: Option<MatchEngine> = None;
    for i in 0..total_ticks {
        let tick = mother.step();
        match &mut fork {
            None => {
                if i + 1 == checkpoint_ticks {
                    // 母局此 tick 结束后复制；从此母局与副本双线推进。
                    fork = Some(mother.experimental_fork());
                }
            }
            Some(engine) => {
                let mut hm = Fnv::new();
                hash_tick(&mut hm, &tick);
                mother_hashes.push(hm.0);
                let fork_tick = engine.step();
                let mut hf = Fnv::new();
                hash_tick(&mut hf, &fork_tick);
                fork_hashes.push(hf.0);
            }
        }
    }
    assert_eq!(
        mother_hashes.len(),
        fork_hashes.len(),
        "续跑段长度不一致 (seed {seed}, checkpoint {checkpoint_ticks})"
    );
    for (i, (m, f)) in mother_hashes.iter().zip(&fork_hashes).enumerate() {
        assert_eq!(
            m, f,
            "fork 第 {i} 续跑 tick 哈希分歧 (seed {seed}, checkpoint {checkpoint_ticks})"
        );
    }
}

#[test]
fn fork_continuation_is_bit_exact_across_checkpoints_and_seeds() {
    // 检查点覆盖回合中段（早）、死球/罚球概率窗口（中）、节末附近（晚）。
    for seed in [42u64, 7, 123] {
        for checkpoint in [300u64, 1500, 9000] {
            fork_continuation_matches(seed, checkpoint, 9_200);
        }
    }
}

#[test]
fn fork_is_independent_copy_mother_unaffected() {
    // 副本先行跑 500 tick，母局再续跑：母局哈希与从未 fork 的参照局一致。
    let seed = 99;
    let checkpoint_ticks = 2_000u64;
    let total_ticks = 4_000u64;

    // 参照局：一路跑完。
    let mut reference = MatchEngine::new(seed);
    let mut reference_hashes: Vec<u64> = Vec::new();
    for i in 0..total_ticks {
        let tick = reference.step();
        if i >= checkpoint_ticks {
            let mut h = Fnv::new();
            hash_tick(&mut h, &tick);
            reference_hashes.push(h.0);
        }
    }

    // fork 局：checkpoint 处复制，副本先跑 500 tick，然后母局续跑。
    let mut mother = MatchEngine::new(seed);
    let mut mother_hashes: Vec<u64> = Vec::new();
    let mut fork: Option<MatchEngine> = None;
    for i in 0..total_ticks {
        let tick = mother.step();
        match &mut fork {
            None => {
                if i + 1 == checkpoint_ticks {
                    let mut copy = mother.experimental_fork();
                    for _ in 0..500 {
                        copy.step(); // 副本分岔跑，用完即弃
                    }
                    fork = Some(copy);
                }
            }
            Some(_) => {
                let mut h = Fnv::new();
                hash_tick(&mut h, &tick);
                mother_hashes.push(h.0);
            }
        }
    }
    assert_eq!(mother_hashes, reference_hashes, "副本分岔跑污染了母局状态");
}
