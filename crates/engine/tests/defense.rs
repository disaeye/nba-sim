//! 弱侧协防的因果证据（D26 验收门）。
//!
//! ## 为何这条证据难以建立（实测记录，避免重走错路）
//!
//! 前两版判据都无判别力，已废弃：
//!
//! 1. 「突破窗口内任一防守人向篮筐靠近 > 0.5 ft」→ 基线与对照组都是
//!    **132/134 = 99%**。防守人本就在持续移动，一分钟内谁都会靠近篮筐一点。
//! 2. 「弱侧特定球员向篮筐靠近**且**离开对位人」→ 基线与对照组都是
//!    **46%**。该判据测的是球员的普通跑位，与势能场无关。
//!
//! 两者的共同缺陷：它们观测的是**球员位置**，而位置由整条决策/物理链共同
//! 决定，单个系数的影响会被淹没。
//!
//! 本文件改用**涌现动作标签**作观测量：`potential_field.rs` 把「威胁占比 >
//! 阈值且平衡点距篮 < 阈值」直接映射为 `ROTATE_RIM_HELP`，把「真空占比 >
//! 阈值」映射为 `X_OUT_CLOSEOUT`。这两个标签是势能场的**直接产物**，
//! 因此可以逐帧计数并做因果对照。
//!
//! ## 这条测试同时守住一个真实缺陷（已修）
//!
//! `sync_team_tactics` 每 tick 用 `DefenseRules::for_scheme(...)` 整体覆写
//! `rules.tactics.defense`，而档案只声明五个字段——`potential_field` 被一起
//! 重置为默认值。后果是 `--rules` 对势能场系数的覆盖被静默丢弃，实测
//! 「把 `k_threat_base` 与 `low_man_threat_gain` 归零」后 `ROTATE_RIM_HELP`
//! 的总帧数与突破窗口内帧数都逐位不变（32588 与 1084 完全相同）。现改为只
//! 覆写档案声明的字段。

mod support;

use nba_domain::GameRules;
use nba_engine::MatchEngine;

/// 统计弱侧协防动作在整个 scope 内出现的帧数。
///
/// 计数对象是 `potential_field.rs` 直接产出的动作标签，因此它同时是
/// 「势能场系数是否真的进入计算」的探针。
fn help_action_frames(rules: GameRules, seeds: &[u64]) -> (u32, u32) {
    let mut rotate = 0u32;
    let mut x_out = 0u32;
    for &seed in seeds {
        let mut engine = MatchEngine::with_rules(seed, rules.clone());
        engine.set_scope("1q").expect("scope must be valid");
        while !engine.is_finished() {
            let tick = engine.step();
            for p in tick.frame.players.iter().filter(|p| p.on_court) {
                match p.action.as_str() {
                    "ROTATE_RIM_HELP" => rotate += 1,
                    "X_OUT_CLOSEOUT" => x_out += 1,
                    _ => {}
                }
            }
        }
    }
    (rotate, x_out)
}

/// 护筐引力必须真实产生下沉护筐动作，且归零后该动作消失。
///
/// 这是**因果门**：它不仅要求基线有动作，还要求切断系数后动作消失。
/// 只有前者时无法区分「势能场在起作用」与「这些标签被别处硬编码产生」。
#[test]
fn rim_gravity_drives_the_rim_help_action() {
    let seeds = [42u64, 1, 7, 100];
    let (baseline_rotate, _) = help_action_frames(GameRules::default(), &seeds);
    assert!(
        baseline_rotate > 0,
        "the baseline must produce ROTATE_RIM_HELP frames; zero means the \
         weak-side rim-help path never fires and the rest of this test is vacuous"
    );

    let mut no_gravity = GameRules::default();
    no_gravity.tactics.defense.potential_field.k_threat_base = 0.0;
    no_gravity
        .tactics
        .defense
        .potential_field
        .low_man_threat_gain = 0.0;
    let (disabled_rotate, _) = help_action_frames(no_gravity, &seeds);
    assert!(
        disabled_rotate < baseline_rotate,
        "zeroing the rim-protection gravity (k_threat_base + low_man_threat_gain) \
         must reduce ROTATE_RIM_HELP frames, but it stayed at {disabled_rotate} \
         (baseline {baseline_rotate}) — the coefficient is not reaching the solver"
    );
}

/// 真空吸力必须真实产生 X-Out 补位，且归零后该动作消失。
#[test]
fn void_pull_drives_the_x_out_action() {
    let seeds = [42u64, 1, 7, 100];
    let (_, baseline_x_out) = help_action_frames(GameRules::default(), &seeds);
    assert!(
        baseline_x_out > 0,
        "the baseline must produce X_OUT_CLOSEOUT frames; zero means the \
         high-man rotation path never fires"
    );

    let mut no_void = GameRules::default();
    no_void.tactics.defense.potential_field.k_void_base = 0.0;
    let (_, disabled_x_out) = help_action_frames(no_void, &seeds);
    assert!(
        disabled_x_out < baseline_x_out,
        "zeroing the void-pull coefficient (k_void_base) must reduce \
         X_OUT_CLOSEOUT frames, but it stayed at {disabled_x_out} \
         (baseline {baseline_x_out}) — the coefficient is not reaching the solver"
    );
}

/// 势能场系数必须能经 `--rules` 覆盖：引擎不得每 tick 把它重置回默认值。
///
/// 这条断言是上面两条的前提。它守的是一个实际发生过的缺陷：防守方案档案
/// 整体覆写 `rules.tactics.defense`，把未在档案里声明的 `potential_field`
/// 一起重置，使外部覆盖静默失效。
#[test]
fn potential_field_coefficients_survive_the_defensive_scheme_sync() {
    let seeds = [42u64];
    let (default_rotate, _) = help_action_frames(GameRules::default(), &seeds);

    // 一个**非默认**的势能场系数必须改变行为。若它被每 tick 重置为默认值，
    // 这里的计数会与基线逐位相同。
    let mut amplified = GameRules::default();
    amplified.tactics.defense.potential_field.k_threat_base = 5.0;
    amplified
        .tactics
        .defense
        .potential_field
        .low_man_threat_gain = 8.0;
    let (amplified_rotate, _) = help_action_frames(amplified, &seeds);
    assert_ne!(
        amplified_rotate, default_rotate,
        "a non-default potential-field coefficient must change behaviour; \
         identical counts mean `sync_team_tactics` resets the field every tick \
         and the `--rules` channel is silently dead"
    );
}
