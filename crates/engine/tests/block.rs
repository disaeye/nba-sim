//! 封盖行为：封盖作为独立比赛事实，且只在合法的动作阶段窗口内成立。
//!
//! 守卫对象：
//! - 封盖与「投失 + 篮板」是不同事实（前者之后是松球争夺，责任主体是防守人）；
//! - 封盖只在出手者的 `ActionPhase::Execution` 内发生，合球阶段归切球（Strip）；
//! - 封盖事实必须携带出手阶段与命中裁定，使评判能判断「封盖是否改变了结果」。
//!
//! 对应 `docs/architecture.md` §6、`docs/quality.md` §2.1（回合因果）与
//! `attributes.md` §2.4（`block` / `vertical` 的消费链）。

mod support;

use nba_engine::MatchEngine;
use rayon::prelude::*;

/// 封盖事实必须真实出现在比赛里，且比率处于真实区间。
///
/// 实测（seed 42/1/7/100/999，全场）：合计 33/1023 = 3.2% 的出手被封，
/// 单 seed 2.4%–4.9%；真实 NBA 约 5.5%。这里只断言一个宽松区间——精确标定属于 `stats_baseline`。
#[test]
fn blocked_shots_occur_at_a_plausible_rate() {
    // 种子间并行：两场完整比赛互不共享状态，并行只改变 wall time。
    let (blocks, shots) = [42u64, 100]
        .par_iter()
        .copied()
        .map(|seed| {
            let mut engine = MatchEngine::new(seed);
            engine.set_scope("full").expect("full scope is valid");
            let mut blocks = 0u32;
            let mut shots = 0u32;
            while !engine.is_finished() {
                let tick = engine.step();
                for ev in &tick.frame.event_log {
                    match ev.kind.as_str() {
                        "BLOCKED_SHOT" => blocks += 1,
                        "SHOT_RELEASE" => shots += 1,
                        _ => {}
                    }
                }
            }
            (blocks, shots)
        })
        .reduce(|| (0u32, 0u32), |a, b| (a.0 + b.0, a.1 + b.1));
    assert!(shots > 0, "a full game must contain shot attempts");
    let rate = f64::from(blocks) / f64::from(shots);
    assert!(
        (0.005..=0.12).contains(&rate),
        "blocked-shot rate {:.3} ({blocks}/{shots}) is outside the plausible band; \
         a rate near 0 means the block path never fires, a rate above 0.12 means it \
         fires more than once per attempt or on every tick",
        rate
    );
}

/// 封盖事实必须与「投失」区分：`BLOCKED_SHOT` 出现时，后续是松球而非篮板。
#[test]
fn blocked_shot_is_distinct_from_a_miss() {
    let mut engine = MatchEngine::new(100);
    engine.set_scope("full").expect("full scope is valid");
    let mut saw_block = false;
    while !engine.is_finished() && !saw_block {
        let tick = engine.step();
        for ev in &tick.frame.event_log {
            if ev.kind != "BLOCKED_SHOT" {
                continue;
            }
            saw_block = true;
            let payload = ev
                .data
                .as_ref()
                .and_then(|d| d.get("BlockedShot"))
                .expect("BLOCKED_SHOT must carry its own payload");
            // 责任主体必须是防守人（与投失不同：投失没有防守责任方）。
            assert!(
                payload.get("blocker_id").and_then(|v| v.as_str()).is_some(),
                "a block must name the defender who stopped the shot"
            );
            // 出手阶段必须是可封盖窗口，使评判能验证门控真的生效。
            assert_eq!(
                payload.get("phase").and_then(|v| v.as_str()),
                Some("Execution"),
                "blocks may only be recorded during the Execution phase \
                 (Preparation belongs to strip attempts, FollowThrough means the ball is gone)"
            );
        }
    }
    assert!(
        saw_block,
        "a full game must contain at least one blocked shot (seed 100 measures ~11)"
    );
}

/// 封盖率必须响应 `block` 能力：把全队封盖能力推到极端高，被封数必须上升。
#[test]
fn block_rate_responds_to_the_block_attribute() {
    use nba_domain::GameRules;
    use nba_engine::MatchSetup;

    let run = |block_value: f32| -> u32 {
        [42u64, 100]
            .par_iter()
            .map(|&seed| {
                let mut setup = MatchSetup::builtin(GameRules::default());
                for team in [&mut setup.home_team, &mut setup.away_team] {
                    for p in team.players.iter_mut() {
                        p.attributes.block = block_value;
                        p.attributes.vertical = block_value;
                    }
                }
                let mut engine = MatchEngine::with_setup(setup, seed);
                engine.set_scope("full").expect("full scope is valid");
                let mut blocks = 0u32;
                while !engine.is_finished() {
                    let tick = engine.step();
                    for ev in &tick.frame.event_log {
                        if ev.kind == "BLOCKED_SHOT" {
                            blocks += 1;
                        }
                    }
                }
                blocks
            })
            .sum()
    };

    let low = run(0.05);
    let high = run(0.95);
    assert!(
        high > low,
        "raising `block` + `vertical` on every player must raise the blocked-shot count \
         (low={low}, high={high}); equal counts mean the capability is not consumed"
    );
}
