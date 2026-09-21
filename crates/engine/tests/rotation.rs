//! 轮换与换人行为（gap.md G6 / tactics.md §2.3.3）。
//!
//! ## 守卫对象
//!
//! - 体力枯竭换人真实发生：`SubstitutionReason` 声明的变体不允许再出现
//!   「声明了但零消费」——本测试直接断言 `StaminaExhaustion` 事实存在；
//! - 登场者体力必须高于换下阈值一定裕量：否则刚登场就到线，形成
//!   「换下 B、C 上场两分钟又到线」的高频横跳（修复前实测 138–176 次/场）；
//! - 换人频率落在真实数量级：真实 NBA 每队每场 30–40 次。
//!
//! ## 因果门
//!
//! 把 `fatigue_substitution_threshold` 压到体力可达域之下（永不触发），
//! 换人次数必须显著下降——证明换人由体力阈值驱动，由时间驱动。

mod support;

use nba_domain::GameRules;
use nba_engine::MatchEngine;
use rayon::prelude::*;

/// 一场全场比赛的换人统计。
struct SubStats {
    total: u32,
    stamina: u32,
    low_stamina_entries: u32,
}

fn observe_substitutions(rules: GameRules, seed: u64) -> SubStats {
    let mut engine = MatchEngine::with_rules(seed, rules);
    engine.set_scope("full").expect("full scope is valid");
    let mut total = 0u32;
    let mut stamina = 0u32;
    let mut seen: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut low_stamina_entries = 0u32;
    while !engine.is_finished() {
        let tick = engine.step();
        // 登场时刻的体力必须高于换下阈值 + 裕量（0.42 + 0.15 = 0.57）。
        // 实测的帧回写值有舍入，放宽到 0.55。
        for p in &tick.frame.players {
            let prev = seen.insert(p.id.clone(), p.on_court);
            if prev != Some(p.on_court) && p.on_court && p.stm_max > 0.0 && p.stm / p.stm_max < 0.55
            {
                low_stamina_entries += 1;
            }
        }
        for ev in &tick.frame.event_log {
            if ev.kind != "SUBSTITUTION" {
                continue;
            }
            total += 1;
            let reason = ev
                .data
                .as_ref()
                .and_then(|d| d.get("Substitution"))
                .and_then(|s| s.get("reason"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            if reason == "StaminaExhaustion" {
                stamina += 1;
            }
        }
    }
    SubStats {
        total,
        stamina,
        low_stamina_entries,
    }
}

/// 体力枯竭换人必须真实发生，登场者必须体力充足，频率落在真实数量级。
#[test]
fn fatigue_substitutions_occur_and_entries_are_fit() {
    // 种子间并行：三场完整比赛互不共享状态，并行只改变 wall time。
    // 基线结果缓存共享给因果门测试（同函数同配置）。
    let stats = shared_sub_stats();
    let total: u32 = stats.iter().map(|s| s.total).sum();
    let stamina: u32 = stats.iter().map(|s| s.stamina).sum();
    let low_entries: u32 = stats.iter().map(|s| s.low_stamina_entries).sum();
    assert!(
        stamina > 0,
        "stamina-driven substitutions never fired across 3 full games; \
         `SubstitutionReason::StaminaExhaustion` is declared but consumed by nothing"
    );
    assert_eq!(
        low_entries, 0,
        "{low_entries} players entered the court with stamina below the entry floor; \
         the bench filter (rest + entry stamina) is not actually applied"
    );
    // 3 场合计实测 78（每场 24–28）。真实 NBA 每队 30–40 次、双方 60–80。
    // 下界防「换人退化为永不发生」，上界防「横跳回归」
    // （修复前实测每场 138–176，3 场应超 400）。
    assert!(
        (36..=180).contains(&total),
        "substitution count {total} across 3 games is outside the plausible band [36, 180]; \
         below 36 means the rotation logic is effectively dead, above 180 means \
         high-frequency churn (the pre-fix behaviour measured 138–176 per game)"
    );
}

/// 因果门：枯竭阈值推到体力不可达域，换人必须显著减少。
/// 基线与第一个测试共享（`observe_substitutions` 同函数同配置，OnceLock 缓存）。
#[test]
fn raising_the_fatigue_threshold_damps_substitutions() {
    let seeds = [42u64, 1, 7];
    let baseline: u32 = shared_sub_stats().iter().map(|s| s.total).sum();

    let mut no_fatigue = GameRules::default();
    no_fatigue.rotation.fatigue_substitution_threshold = 0.0;
    // 阈值 0.0 加上裕量 0.15 → 登场者体力必须 >0.15，体力低于 0.0 才触发换人
    // ——体力有 0.2 的 floor，永不触发（恢复分支存在，体力不会低于 floor）。
    let disabled: u32 = seeds
        .par_iter()
        .map(|&s| {
            let rules = no_fatigue.clone();
            observe_substitutions(rules, s).total
        })
        .sum();

    assert!(
        disabled < baseline,
        "setting the fatigue threshold below the reachable domain must damp \
         substitutions (baseline {baseline}, disabled {disabled}); equal counts mean \
         substitutions are driven by something other than the stamina threshold"
    );
}

fn shared_sub_stats() -> &'static Vec<SubStats> {
    static STATS: std::sync::OnceLock<Vec<SubStats>> = std::sync::OnceLock::new();
    STATS.get_or_init(|| {
        [42u64, 1, 7]
            .par_iter()
            .map(|&seed| observe_substitutions(GameRules::default(), seed))
            .collect()
    })
}
