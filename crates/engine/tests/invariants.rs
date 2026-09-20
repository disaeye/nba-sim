//! 不变量检查：每 tick 的物理与生命周期底线，以及不变量检查器的零违反。
//!
//! 守卫对象：
//! - 引擎每 tick 自检的 `last_tick_violations` 必须为空（跨多种子长跑）；
//! - 连续的帧投影必须满足速度与间距底线（显式 placement 除外）；
//! - 自定义几何下越界不得发生；
//! - 子阶段迁移必须是 `nba.v2` 合法性表允许的边；
//! - 曾触发停滞的种子必须完赛（不完赛的模拟连被检测的资格都没有）。
//!
//! 对应 `docs/quality.md` §1.1（L1 检测网）与 `docs/dev/gap.md` §8.2。

mod support;

use std::collections::HashMap;

use nba_domain::GameRules;
use nba_engine::MatchEngine;

// ============================================================================
// L1：引擎每 tick 自检
// ============================================================================

/// 跨多种子长跑：每一 tick 的引擎自检必须零违反。
#[test]
fn axiom_fuzzing_multi_seed_long_run() {
    let seeds = [42, 7, 100, 999, 1201, 31337, 2024, 8888];
    for seed in seeds {
        let mut engine = MatchEngine::new(seed);
        for tick_idx in 0..2500 {
            let _tick = engine.step();
            assert!(
                engine.last_tick_violations().is_empty(),
                "Seed {} at tick {} violated axioms: {:?}",
                seed,
                tick_idx,
                engine.last_tick_violations()
            );
            if engine.is_finished() {
                break;
            }
        }
    }
}

/// 全场跑完后必须完赛且回合数处于合理范围。
///
/// 根因（2026-09-02 GAP 复审）：违例判罚在球处于 `Pass`/`LooseBall`/
/// `ControlTransfer` 等非持球状态时触发 `start_inbound_transition`，而领域
/// 转换表缺少这些状态到 `InboundTransfer` 的合法边；写入口拒绝后引擎停滞在
/// 「DeadBall + Held」无出口状态——比赛永远无法完赛（seed 2 的 CLI 流曾因此
/// 膨胀至 6.8GB 直到磁盘耗尽）。
#[test]
fn full_game_completes_on_formerly_wedged_seeds() {
    for seed in [2u64, 4] {
        let mut engine = MatchEngine::new(seed);
        engine.set_scope("full").expect("full scope is valid");
        let mut ticks = 0usize;
        // 全场正常 ~81k tick；上限防无限循环，超限即停滞回归。
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

// ============================================================================
// 连续运动的物理底线（在帧投影上独立复算）
// ============================================================================

/// 球员速度与间距的底线必须成立。
///
/// 判定在帧投影上独立复算（不依赖引擎自检），因此能抓「引擎自检漏了但帧里
/// 看得见」的差异。
#[test]
fn test_physics_zero_anomalies_full_match() {
    let mut engine = MatchEngine::new(42);
    let total_ticks = 1500;
    let mut prev_positions: HashMap<String, (f32, f32)> = HashMap::new();
    let mut speed_violations = 0;
    let mut spacing_violations = 0;
    let rules = engine.rules().clone();
    let dt = rules.tick_seconds;
    for tick_idx in 0..total_ticks {
        let tick = engine.step();
        // 显式 placement tick（gap.md §4.3）：离散位置重置不是连续运动，
        // 不参与逐 tick 速度连续性判定。
        let placement_tick = tick.frame.events.iter().any(|e| e == "PLACEMENT_APPLIED");
        for p in &tick.frame.players {
            if placement_tick {
                prev_positions.insert(p.id.clone(), (p.x, p.y));
                continue;
            }
            if let Some(&(prev_x, prev_y)) = prev_positions.get(&p.id) {
                let dx_ft = (p.x - prev_x) * rules.court.width_ft;
                let dy_ft = (p.y - prev_y) * rules.court.height_ft;
                let speed_ftps = (dx_ft * dx_ft + dy_ft * dy_ft).sqrt() / dt;
                if speed_ftps > rules.max_player_speed_ftps + 1.5 {
                    speed_violations += 1;
                    if speed_violations <= 5 {
                        eprintln!(
                            "Tick {}: Player {} exceeded max speed: {:.2} ft/s",
                            tick_idx, p.id, speed_ftps
                        );
                    }
                }
            }
            prev_positions.insert(p.id.clone(), (p.x, p.y));
        }
        // 间距不变量只对**在场球员**成立（与 L1 `PLAYER_SEPARATION` 同口径）。
        //
        // 替补席球员按设计集中在场外替补席区域（彼此本就相邻），把他们计入
        // 会得到虚假违反。实测：H_01（在场，发球员站位）与 H_08（替补）相距
        // 1.45 ft，被误判 66 次。
        for i in 0..tick.frame.players.len() {
            for j in (i + 1)..tick.frame.players.len() {
                let p1 = &tick.frame.players[i];
                let p2 = &tick.frame.players[j];
                if !p1.on_court || !p2.on_court {
                    continue;
                }
                let dx_ft = (p1.x - p2.x) * rules.court.width_ft;
                let dy_ft = (p1.y - p2.y) * rules.court.height_ft;
                if (dx_ft * dx_ft + dy_ft * dy_ft).sqrt() < rules.player_radius_ft * 0.9 {
                    spacing_violations += 1;
                }
            }
        }
    }
    assert_eq!(speed_violations, 0);
    assert_eq!(spacing_violations, 0);
}

/// 自定义几何下的界内底线。
#[test]
fn test_custom_geometry_boundaries_invariants() {
    let mut rules = GameRules::default();
    rules.court.width_ft = 80.0;
    rules.court.height_ft = 40.0;
    rules.court.hoop_left_x_ft = 4.0;
    rules.court.hoop_right_x_ft = 76.0;
    rules.court.hoop_y_ft = 20.0;
    let mut engine = MatchEngine::with_rules(108, rules.clone());
    for _ in 0..1500 {
        let tick = engine.step();
        // 发球程序中的发球员是显式 placement 角色，允许站界外
        // （gap.md §4.3/§8.5），该 tick 不参与界内判定。
        let placement_tick = tick.frame.events.iter().any(|e| e == "PLACEMENT_APPLIED")
            || matches!(
                engine.ball_state(),
                nba_physics::BallTrajectoryKind::InboundTransfer { .. }
                    | nba_physics::BallTrajectoryKind::InboundReady { .. }
            );
        for p in &tick.frame.players {
            // 边界不变量只约束在场球员（与 L1 `PLAYER_IN_BOUNDS` 同口径）：
            // 替补严格位于场外替补席（data.rs），其归一化坐标允许越出 [0,1]
            // ——物理层不为替补伪造 BoundaryCross，也不钳回场内。
            if !p.on_court {
                continue;
            }
            if placement_tick {
                continue;
            }
            assert!(
                (0.0..=1.0).contains(&p.x),
                "Player {} x out of bounds: {}",
                p.id,
                p.x
            );
            assert!(
                (0.0..=1.0).contains(&p.y),
                "Player {} y out of bounds: {}",
                p.id,
                p.y
            );
        }
    }
}

// ============================================================================
// 子阶段迁移合法性
// ============================================================================

/// `nba.v2` 的合法性表。键为来源阶段，值为允许的目标阶段。
///
/// ## 为何必须放进测试套件
///
/// 这张表原先只由 G-STATS 16-seed 矩阵在 CLI 批处理里检查，而那个矩阵
/// **不属于** `run-tests.sh`：日常回归看不到它。实测因此漏过一条 Hard 缺陷
/// （seed 12：非投篮犯规在球飞行中把子阶段重置为 `Initiation`，球触地后再
/// 执行 `Initiation -> FlightAndRebound`）。
///
/// 判定用 fixture 里的原始表（`include_str!` 读同一份文件），不重抄一份，
/// 避免两处漂移。
fn allowed_transitions() -> HashMap<String, Vec<String>> {
    let raw: serde_json::Value =
        serde_json::from_str(include_str!("../../evaluator/fixtures/nba.v2.json"))
            .expect("nba.v2 fixture must parse");
    let table = raw
        .get("phase_transitions")
        .expect("nba.v2 must declare phase_transitions")
        .as_object()
        .expect("phase_transitions must be an object");
    table
        .iter()
        .map(|(from, targets)| {
            let list = targets
                .as_array()
                .expect("each transition entry must be an array")
                .iter()
                .map(|t| t.as_str().expect("target must be a string").to_string())
                .collect();
            (from.clone(), list)
        })
        .collect()
}

#[test]
fn phase_transitions_are_legal_across_seeds() {
    let allowed = allowed_transitions();
    // 全场约 87000 tick；用多种子覆盖不同比赛分支。
    let seeds: [u64; 4] = [42, 12, 7, 31337];
    for seed in seeds {
        let mut engine = MatchEngine::new(seed);
        let mut prev: Option<String> = None;
        for tick in 0..90000u32 {
            let frame = engine.step().frame;
            let phase = frame.phase.clone();
            if let Some(from) = &prev {
                if from != &phase {
                    // 表里没有的来源阶段视为允许（与评判器同一口径）。
                    if let Some(targets) = allowed.get(from) {
                        assert!(
                            targets.contains(&phase),
                            "seed {seed} tick {tick}: illegal phase transition \
                             {from} -> {phase}; allowed from {from}: {targets:?} \
                             (contract: crates/evaluator/fixtures/nba.v2.json)"
                        );
                    }
                }
            }
            prev = Some(phase);
        }
    }
}

/// 空规则下的迁移同样必须合法：合法性表不依赖具体规则档案。
#[test]
fn phase_transitions_are_legal_under_default_rules() {
    let allowed = allowed_transitions();
    let mut engine = MatchEngine::with_rules(12, GameRules::default());
    let mut prev: Option<String> = None;
    for tick in 0..90000u32 {
        let phase = engine.step().frame.phase.clone();
        if let Some(from) = &prev {
            if from != &phase {
                if let Some(targets) = allowed.get(from) {
                    assert!(
                        targets.contains(&phase),
                        "tick {tick}: illegal phase transition {from} -> {phase}"
                    );
                }
            }
        }
        prev = Some(phase);
    }
}
