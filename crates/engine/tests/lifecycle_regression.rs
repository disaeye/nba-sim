//! 回合转换健壮性回归（2026-09-02 GAP 复审实证的根因防护）。
//!
//! 根因：违例判罚在球处于 Pass/LooseBall/ControlTransfer 等非持球状态时触发
//! `start_inbound_transition`，而领域转换表缺少这些状态 → InboundTransfer 的
//! 合法边，写入口拒绝后引擎楔死在"DeadBall + Held"无出口状态（决策路径要求
//! InboundReady，时钟对死球停走）——比赛永远无法完赛（seed 2/4 实证，seed 2
//! 的 CLI 流曾因此膨胀至 6.8GB 直到磁盘耗尽）。
//!
//! 本测试用曾触发楔死的种子断言：全场模拟必须完赛（lifecycle 无死锁是
//! M1 检测网之下的更底层契约——不完赛的模拟连被检测的资格都没有）。

use nba_engine::MatchEngine;

#[test]
fn full_game_completes_on_formerly_wedged_seeds() {
    for seed in [2u64, 4] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").expect("full scope is valid");
        let mut ticks = 0usize;
        // 全场正常 ~81k tick；上限防死循环，超限即楔死回归。
        while !engine.is_finished() && ticks < 300_000 {
            let _ = engine.step();
            ticks += 1;
        }
        assert!(
            engine.is_finished(),
            "seed {seed} wedged: game did not finish after {ticks} ticks \
             (dead-ball lifelock regression; see 2026-09-02 GAP review)"
        );
        assert!(
            engine.completed_possessions() >= 150,
            "seed {seed} finished with only {} possessions — implausible full game",
            engine.completed_possessions()
        );
    }
}
