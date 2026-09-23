//! Play 选板器测试（plan_play.md #16 / tactics.md §2.2.4 选板触发语义）。
//!
//! 覆盖：单候选激活与窗口折算；多候选打分（谓词多的优先、同分 rng
//! 抽取且相同种子可复现）；冷却期内不评估、冷却结束恢复；回合终结立即
//! 结束激活并进入冷却；场输出谓词消费注入的滞回稳定值；相同输入加相同
//! 种子输出相同（确定性）；无候选返回 `None`；八个谓词变体逐个覆盖。

use std::collections::BTreeMap;

use nba_decision::evaluate_active_play;
use nba_decision::play_selector::{
    eval_predicate, seconds_to_ticks, select, PlayActivationBook, PlaySelectionContext,
};
use nba_domain::{
    DecisionActionFamily, PlayAction, PlayCarrierPreference, PlayInhibition, PlayInhibitionMode,
    PlayKind, PlayPredicate, PlayRule, PlaySpec, PlayTrigger, PlayVerb,
};
use rand::rngs::StdRng;
use rand::SeedableRng;

/// 测试用 tick 时长：与 `GameRules::tick_seconds` 默认值一致（0.04 秒）。
const TICK_SECONDS: f32 = 0.04;

/// 全假上下文（个别测试按需翻转字段）。
fn base_ctx() -> PlaySelectionContext {
    PlaySelectionContext {
        possession_ticks: 10,
        shot_clock_seconds: 14.0,
        tick_seconds: TICK_SECONDS,
        carrier_possed: true,
        halfcourt: true,
        carrier_dist_to_hoop_ft: 25.0,
        screen_established: false,
        corner_left_occupied: false,
        corner_right_occupied: false,
        help_shading_off_stable: false,
        weak_side_vacated_stable: false,
    }
}

/// 冷却时长查表（play_id → 冷却 tick），从在场库的触发声明折算。
fn cooldowns(playbook: &[PlaySpec]) -> BTreeMap<String, u32> {
    let mut map = BTreeMap::new();
    for spec in playbook {
        let trigger = &spec.triggers[0];
        map.insert(
            spec.id.clone(),
            seconds_to_ticks(trigger.cooldown_seconds, TICK_SECONDS),
        );
    }
    map
}

/// 单触发档案的构造辅助。
fn spec_with_trigger(
    id: &str,
    when: Vec<PlayPredicate>,
    window_s: f32,
    cooldown_s: f32,
) -> PlaySpec {
    PlaySpec {
        schema_version: 1,
        id: id.to_string(),
        name_zh: format!("测试档案 {id}"),
        kind: PlayKind::Play,
        carrier_preferences: Vec::new(),
        inhibitions: Vec::new(),
        rules: Vec::new(),
        triggers: vec![PlayTrigger {
            when,
            window_seconds: window_s,
            cooldown_seconds: cooldown_s,
        }],
    }
}

/// 单触发档案，窗口 6 秒、冷却 10 秒（与规格示例一致）。
fn spec_single(id: &str, when: Vec<PlayPredicate>) -> PlaySpec {
    spec_with_trigger(id, when, 6.0, 10.0)
}

#[test]
fn single_candidate_activates_with_converted_window() {
    let playbook = vec![spec_single(
        "halfcourt_catch_v1",
        vec![PlayPredicate::HalfcourtPossession],
    )];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(42);

    let activation = select(&playbook, &base_ctx(), &mut book, &mut rng)
        .expect("唯一候选全部谓词为真，必须激活");

    assert_eq!(activation.play_id, "halfcourt_catch_v1");
    // 6.0 秒 ÷ 0.04 秒/tick = 150 tick。
    assert_eq!(activation.window_ticks, 150);
    assert_eq!(activation.cooldown_ticks, 250);
    assert_eq!(activation.spec.id, "halfcourt_catch_v1");

    // 窗口耗尽自动进入冷却：150 tick 全部递减完，簿记应转为冷却相位。
    let table = cooldowns(&playbook);
    for _ in 0..150 {
        book.tick(&table);
    }
    let entries = book.entries();
    assert_eq!(entries.len(), 1, "窗口耗尽后应恰有一条冷却条目");
    assert_eq!(entries[0].play_id, "halfcourt_catch_v1");
    assert_eq!(entries[0].remaining_ticks, 250, "10.0 秒冷却 = 250 tick");

    // 冷却期间不得重复激活。
    let mut rng = StdRng::seed_from_u64(42);
    assert!(
        select(&playbook, &base_ctx(), &mut book, &mut rng).is_none(),
        "冷却中的 Play 不评估"
    );

    // 冷却归零移出簿记，恢复可激活。
    for _ in 0..250 {
        book.tick(&table);
    }
    assert!(book.entries().is_empty(), "冷却归零后簿记应为空");
    let mut rng = StdRng::seed_from_u64(42);
    assert!(
        select(&playbook, &base_ctx(), &mut book, &mut rng).is_some(),
        "冷却结束恢复可激活"
    );
}

#[test]
fn more_specific_trigger_wins_over_broader() {
    // 两份档案同时触发：三谓词的更特异，必须胜过单谓词候选。
    let playbook = vec![
        spec_single("broad_v1", vec![PlayPredicate::HalfcourtPossession]),
        spec_single(
            "specific_v1",
            vec![
                PlayPredicate::HalfcourtPossession,
                PlayPredicate::CarrierPossed,
                PlayPredicate::BeyondCircleFt { r: 20.0 },
            ],
        ),
    ];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(7);

    let activation =
        select(&playbook, &base_ctx(), &mut book, &mut rng).expect("两个候选都触发，必须有激活");
    assert_eq!(activation.play_id, "specific_v1");
}

#[test]
fn rule_conditions_are_rechecked_each_tick_and_preferences_accumulate() {
    let spec = PlaySpec {
        carrier_preferences: vec![PlayCarrierPreference {
            action_family: DecisionActionFamily::Pass,
            bonus: 0.3,
        }],
        rules: vec![
            PlayRule {
                id: "screen_help_rule".to_string(),
                when: vec![
                    PlayPredicate::CarrierPossed,
                    PlayPredicate::ScreenEstablished,
                ],
                then: PlayAction {
                    verb: PlayVerb::ScreenRoll,
                    slot: "screener".to_string(),
                },
                carrier_preferences: vec![PlayCarrierPreference {
                    action_family: DecisionActionFamily::Pass,
                    bonus: 0.2,
                }],
            },
            PlayRule {
                id: "weak_side_lift_rule".to_string(),
                when: vec![PlayPredicate::CarrierPossed],
                then: PlayAction {
                    verb: PlayVerb::Lift,
                    slot: "left_wing".to_string(),
                },
                carrier_preferences: vec![PlayCarrierPreference {
                    action_family: DecisionActionFamily::Pass,
                    bonus: 0.1,
                }],
            },
        ],
        ..spec_single("preference_sum_v1", vec![PlayPredicate::CarrierPossed])
    };

    let matched = evaluate_active_play(
        &spec,
        &PlaySelectionContext {
            screen_established: true,
            ..base_ctx()
        },
    );
    assert_eq!(matched.family_effect(DecisionActionFamily::Pass).bonus, 0.6);
    assert_eq!(matched.matched_rule_actions().len(), 2);

    let unmatched = evaluate_active_play(&spec, &base_ctx());
    assert_eq!(
        unmatched.family_effect(DecisionActionFamily::Pass).bonus,
        0.4
    );
    assert_eq!(unmatched.matched_rule_actions().len(), 1);
}

#[test]
#[should_panic(expected = "rule `empty_rule` has an empty `when` list")]
fn executor_rejects_invalid_play_without_rule_when() {
    let spec = PlaySpec {
        rules: vec![PlayRule {
            id: "empty_rule".to_string(),
            when: Vec::new(),
            then: PlayAction {
                verb: PlayVerb::Lift,
                slot: "left_wing".to_string(),
            },
            carrier_preferences: Vec::new(),
        }],
        ..spec_single("empty_rule_when_v1", vec![PlayPredicate::CarrierPossed])
    };

    let _ = evaluate_active_play(&spec, &base_ctx());
}

#[test]
fn highest_scoring_matching_trigger_sets_window_and_cooldown() {
    let mut spec = spec_with_trigger(
        "multi_trigger_v1",
        vec![PlayPredicate::HalfcourtPossession],
        1.0,
        2.0,
    );
    spec.triggers.push(PlayTrigger {
        when: vec![
            PlayPredicate::HalfcourtPossession,
            PlayPredicate::CarrierPossed,
        ],
        window_seconds: 3.5,
        cooldown_seconds: 7.5,
    });
    let playbook = vec![spec];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(41);

    let activation = select(&playbook, &base_ctx(), &mut book, &mut rng)
        .expect("同一 Play 的两条触发都命中时应激活");

    assert_eq!(activation.window_ticks, 88);
    assert_eq!(activation.cooldown_ticks, 188);
    assert_eq!(book.entries()[0].remaining_ticks, 88);
}

#[test]
fn equal_scoring_triggers_keep_first_declared_durations() {
    let mut spec = spec_with_trigger(
        "tied_triggers_v1",
        vec![PlayPredicate::HalfcourtPossession],
        0.5,
        0.1,
    );
    spec.triggers.push(PlayTrigger {
        when: vec![PlayPredicate::CarrierPossed],
        window_seconds: 3.0,
        cooldown_seconds: 4.0,
    });
    let playbook = vec![spec];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(42);

    let activation =
        select(&playbook, &base_ctx(), &mut book, &mut rng).expect("同分触发项都命中时应激活");

    assert_eq!(activation.window_ticks, 13);
    assert_eq!(activation.cooldown_ticks, 3);
    assert_eq!(book.entries()[0].remaining_ticks, 13);
}

#[test]
fn tied_score_same_seed_is_reproducible() {
    // 两份单谓词档案同分：同一种子两次运行应选同一份。
    let playbook = vec![
        spec_single("alpha_v1", vec![PlayPredicate::HalfcourtPossession]),
        spec_single("beta_v1", vec![PlayPredicate::CarrierPossed]),
    ];
    let outcomes: Vec<String> = (0..2)
        .map(|_| {
            let mut book = PlayActivationBook::new();
            let mut rng = StdRng::seed_from_u64(99);
            select(&playbook, &base_ctx(), &mut book, &mut rng)
                .expect("同分候选仍应产出一个激活")
                .play_id
        })
        .collect();
    assert_eq!(outcomes[0], outcomes[1], "相同种子必须可复现");
}

#[test]
fn tied_score_rng_changes_winner_across_seeds() {
    // 大量种子下同分抽取应命中两份档案（rng 生效证据），且每种种子都
    // 严格二选一。
    let playbook = vec![
        spec_single("alpha_v1", vec![PlayPredicate::HalfcourtPossession]),
        spec_single("beta_v1", vec![PlayPredicate::CarrierPossed]),
    ];
    let mut winners: Vec<String> = Vec::new();
    for seed in 0..64u64 {
        let mut book = PlayActivationBook::new();
        let mut rng = StdRng::seed_from_u64(seed);
        let activation =
            select(&playbook, &base_ctx(), &mut book, &mut rng).expect("每次都应有激活");
        assert!(
            activation.play_id == "alpha_v1" || activation.play_id == "beta_v1",
            "胜者必须是两份候选之一"
        );
        winners.push(activation.play_id);
    }
    assert!(
        winners.iter().any(|id| id == "alpha_v1") && winners.iter().any(|id| id == "beta_v1"),
        "64 个种子下两个同分候选都应被抽到过"
    );
}

#[test]
fn cooldown_blocks_reactivation_and_recovers() {
    let playbook = vec![spec_single("once_v1", vec![PlayPredicate::CarrierPossed])];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(1);
    let table = cooldowns(&playbook);

    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_some());
    // 激活期内重复选板不得再次激活同一 Play。
    let mut rng = StdRng::seed_from_u64(1);
    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_none());

    // 窗口 150 tick + 冷却 250 tick，期间不得激活；冷却归零移出簿记
    // 发生在第 400 次 tick 内部，故前 399 次 tick 后都不可激活。
    for _ in 0..(150 + 250 - 1) {
        book.tick(&table);
        let mut rng = StdRng::seed_from_u64(1);
        assert!(
            select(&playbook, &base_ctx(), &mut book, &mut rng).is_none(),
            "激活与冷却期内不得重复激活"
        );
    }
    // 最后一次 tick 把冷却剩余 1 递减归零并移出簿记，当 tick 恢复可激活。
    book.tick(&table);
    let mut rng = StdRng::seed_from_u64(1);
    assert!(
        select(&playbook, &base_ctx(), &mut book, &mut rng).is_some(),
        "冷却归零后恢复可激活"
    );
}

#[test]
fn possession_end_stops_activation_and_starts_cooldown() {
    let playbook = vec![spec_single("live_v1", vec![PlayPredicate::CarrierPossed])];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(3);

    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_some());
    // 激活窗口刚写入时剩余 150 tick（6.0 秒 ÷ 0.04 秒/tick）。
    assert_eq!(book.entries()[0].remaining_ticks, 150);

    // 回合终结：激活立即结束，剩余窗口不计，直接进入冷却。
    book.note_possession_end(&cooldowns(&playbook));
    let entries = book.entries();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].phase,
        nba_decision::play_selector::PlayBookPhase::Cooldown
    );
    assert_eq!(entries[0].remaining_ticks, 250);

    // 冷却期未过不得激活；冷却走完恢复。
    let table = cooldowns(&playbook);
    let mut rng = StdRng::seed_from_u64(3);
    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_none());
    for _ in 0..250 {
        book.tick(&table);
    }
    let mut rng = StdRng::seed_from_u64(3);
    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_some());
}

#[test]
fn field_predicates_consume_injected_stable_values() {
    // 场输出谓词只读上下文里的稳定值：翻转注入值即翻转谓词结果，
    // 且两条方向都必须可行（true/false 各自被消费为真/假）。
    let playbook = vec![spec_single(
        "help_off_v1",
        vec![PlayPredicate::HelpShadingOff],
    )];

    let mut ctx = base_ctx();
    ctx.help_shading_off_stable = true;
    ctx.weak_side_vacated_stable = false;
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(5);
    let activation =
        select(&playbook, &ctx, &mut book, &mut rng).expect("稳定值为 true 时场输出谓词必须放行");
    assert_eq!(activation.play_id, "help_off_v1");

    // 同一档案，稳定值为 false：不触发，无激活。
    let playbook_fresh = vec![spec_single(
        "help_off_v1",
        vec![PlayPredicate::HelpShadingOff],
    )];
    let mut ctx = base_ctx();
    ctx.help_shading_off_stable = false;
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(5);
    assert!(
        select(&playbook_fresh, &ctx, &mut book, &mut rng).is_none(),
        "稳定值为 false 时不触发"
    );

    // 两个方向的原始量（threat/void 抖动）不在上下文里，选板器无从消费：
    // 注入的稳定值与谓词结果逐字段一致。
    assert!(eval_predicate(
        &PlayPredicate::HelpShadingOff,
        &PlaySelectionContext {
            help_shading_off_stable: true,
            ..base_ctx()
        }
    ));
    assert!(!eval_predicate(
        &PlayPredicate::HelpShadingOff,
        &PlaySelectionContext {
            help_shading_off_stable: false,
            ..base_ctx()
        }
    ));
    assert!(eval_predicate(
        &PlayPredicate::WeakSideVacated,
        &PlaySelectionContext {
            weak_side_vacated_stable: true,
            ..base_ctx()
        }
    ));
    assert!(!eval_predicate(
        &PlayPredicate::WeakSideVacated,
        &PlaySelectionContext {
            weak_side_vacated_stable: false,
            ..base_ctx()
        }
    ));
}

#[test]
fn identical_inputs_and_seed_give_identical_output() {
    // 两份档案 + 同分候选 + 相同种子：完整输出（含窗口与档案克隆）一致。
    let playbook = vec![
        spec_single("tau_v1", vec![PlayPredicate::HalfcourtPossession]),
        spec_single(
            "kappa_v1",
            vec![
                PlayPredicate::HalfcourtPossession,
                PlayPredicate::CornerOccupied {
                    side: nba_domain::PlayCourtSide::Left,
                },
            ],
        ),
        spec_single("omega_v1", vec![PlayPredicate::CarrierPossed]),
    ];
    let run = || {
        let mut book = PlayActivationBook::new();
        let mut rng = StdRng::seed_from_u64(2024);
        select(&playbook, &base_ctx(), &mut book, &mut rng).expect("应有激活")
    };
    let a = run();
    let b = run();
    assert_eq!(a.play_id, b.play_id);
    assert_eq!(a.window_ticks, b.window_ticks);
    assert_eq!(a.cooldown_ticks, b.cooldown_ticks);
    assert_eq!(a.spec, b.spec);
}

#[test]
#[should_panic(expected = "has an empty `when` list")]
fn selector_rejects_invalid_play_without_when() {
    let playbook = vec![PlaySpec {
        triggers: vec![PlayTrigger {
            when: Vec::new(),
            window_seconds: 1.0,
            cooldown_seconds: 0.0,
        }],
        ..spec_single("empty_trigger_v1", Vec::new())
    }];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(5);
    let _ = select(&playbook, &base_ctx(), &mut book, &mut rng);
}

#[test]
fn no_candidates_returns_none() {
    // 无候选路径一：空库。
    let playbook: Vec<PlaySpec> = Vec::new();
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(11);
    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_none());

    // 无候选路径二：库非空但谓词全假（carrier_possed 为 false）。
    let playbook = vec![spec_single("never_v1", vec![PlayPredicate::CarrierPossed])];
    let ctx = PlaySelectionContext {
        carrier_possed: false,
        ..base_ctx()
    };
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(11);
    assert!(select(&playbook, &ctx, &mut book, &mut rng).is_none());

    // 无候选路径三：多谓词仅部分为真（全真才触发）。
    let playbook = vec![spec_single(
        "partial_v1",
        vec![
            PlayPredicate::HalfcourtPossession,
            PlayPredicate::ScreenEstablished,
        ],
    )];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(11);
    assert!(select(&playbook, &base_ctx(), &mut book, &mut rng).is_none());
}

// ---- 八个谓词变体的独立覆盖 ----

#[test]
fn predicate_carrier_possed() {
    let ctx = base_ctx();
    assert!(eval_predicate(&PlayPredicate::CarrierPossed, &ctx));
    let ctx = PlaySelectionContext {
        carrier_possed: false,
        ..base_ctx()
    };
    assert!(!eval_predicate(&PlayPredicate::CarrierPossed, &ctx));
}

#[test]
fn predicate_halfcourt_possession() {
    let ctx = base_ctx();
    assert!(eval_predicate(&PlayPredicate::HalfcourtPossession, &ctx));
    let ctx = PlaySelectionContext {
        halfcourt: false,
        ..base_ctx()
    };
    assert!(!eval_predicate(&PlayPredicate::HalfcourtPossession, &ctx));
}

#[test]
fn predicate_shot_clock_urgent() {
    let ctx = base_ctx(); // shot_clock_seconds = 14.0
    assert!(eval_predicate(
        &PlayPredicate::ShotClockUrgent { threshold_s: 16.0 },
        &ctx
    ));
    assert!(!eval_predicate(
        &PlayPredicate::ShotClockUrgent { threshold_s: 14.0 },
        &ctx
    ));
}

#[test]
fn predicate_beyond_circle_ft() {
    let ctx = base_ctx(); // carrier_dist_to_hoop_ft = 25.0
    assert!(eval_predicate(
        &PlayPredicate::BeyondCircleFt { r: 20.0 },
        &ctx
    ));
    assert!(!eval_predicate(
        &PlayPredicate::BeyondCircleFt { r: 30.0 },
        &ctx
    ));
}

#[test]
fn predicate_screen_established() {
    let ctx = PlaySelectionContext {
        screen_established: true,
        ..base_ctx()
    };
    assert!(eval_predicate(&PlayPredicate::ScreenEstablished, &ctx));
    let ctx = PlaySelectionContext {
        screen_established: false,
        ..base_ctx()
    };
    assert!(!eval_predicate(&PlayPredicate::ScreenEstablished, &ctx));
}

#[test]
fn predicate_corner_occupied_left() {
    let ctx = base_ctx(); // corner_left_occupied = false
    assert!(!eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Left
        },
        &ctx
    ));
    let ctx = PlaySelectionContext {
        corner_left_occupied: true,
        ..base_ctx()
    };
    assert!(eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Left
        },
        &ctx
    ));
}

#[test]
fn predicate_corner_occupied_right() {
    let ctx = base_ctx(); // corner_right_occupied = false
    assert!(!eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Right
        },
        &ctx
    ));
    let ctx = PlaySelectionContext {
        corner_right_occupied: true,
        ..base_ctx()
    };
    assert!(eval_predicate(
        &PlayPredicate::CornerOccupied {
            side: nba_domain::PlayCourtSide::Right
        },
        &ctx
    ));
}

// ---- 抑制面出口与窗口折算 ----

#[test]
fn activation_with_hard_inhibition_keeps_viable_exit() {
    // hard 抑制合法（保留出路）的档案可正常激活；覆盖全部族的档案在
    // domain 校验层被拒绝，此处构造绕过校验的结构以验证出口断言
    // （直接构造结构体，不经 from_json 校验）。
    let mut spec = spec_single("hard_ok_v1", vec![PlayPredicate::HalfcourtPossession]);
    spec.inhibitions = vec![PlayInhibition {
        action_family: DecisionActionFamily::Shoot,
        mode: PlayInhibitionMode::Hard,
    }];
    let playbook = vec![spec];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(17);
    let activation = select(&playbook, &base_ctx(), &mut book, &mut rng)
        .expect("保留出路的 hard 抑制档案必须可激活");
    assert_eq!(activation.play_id, "hard_ok_v1");
}

#[test]
#[should_panic(expected = "hard-inhibits every action family")]
fn hard_inhibition_covers_all_families_panics() {
    // 覆盖全部族的 hard 抑制集合出现在激活出口时就地崩溃（fast-fail；
    // 档案层校验器本应拒绝它，这里是选板器出口的防御断言）。
    let mut spec = spec_single("hard_bad_v1", vec![PlayPredicate::HalfcourtPossession]);
    spec.inhibitions = DecisionActionFamily::ALL
        .iter()
        .map(|family| PlayInhibition {
            action_family: *family,
            mode: PlayInhibitionMode::Hard,
        })
        .collect();
    let playbook = vec![spec];
    let mut book = PlayActivationBook::new();
    let mut rng = StdRng::seed_from_u64(17);
    let _ = select(&playbook, &base_ctx(), &mut book, &mut rng);
}

#[test]
fn window_conversion_rounds_up_to_tick() {
    // 非整秒窗口向上取整（0.04 秒/tick）：0.5 秒 = 13 tick，
    // 0.1 秒 = 3 tick，0.033 秒 = 1 tick。
    for (window_s, expected) in [(0.5_f32, 13_u32), (0.1, 3), (0.033, 1)] {
        let playbook = vec![spec_with_trigger(
            "tick_math_v1",
            vec![PlayPredicate::HalfcourtPossession],
            window_s,
            0.0,
        )];
        let mut book = PlayActivationBook::new();
        let mut rng = StdRng::seed_from_u64(23);
        let activation =
            select(&playbook, &base_ctx(), &mut book, &mut rng).expect("窗口折算测试档案必须激活");
        assert_eq!(
            activation.window_ticks, expected,
            "window_seconds = {window_s}"
        );
    }
}
