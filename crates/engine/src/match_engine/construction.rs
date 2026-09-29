//! 比赛构造：从 `MatchSetup` 与种子建立一场比赛的初始状态。
//!
//! 依据 ADR-005 与 `current/plan.md` D5：初始处理球人由**能力**派生，
//! 不取 `starters[0]`——数组位置不是身份。属性到物理量的映射统一经
//! `nba_domain::effective_*` 能力层（曲线下限走 `GameRules` 通道），
//! 因此构造期不存在绕过规则通道的常数。

use glam::Vec2;
use nba_decision::modulation::{CoachStrategy, PlayerModulationState};
use nba_decision::pipeline::DecisionSystem;
use nba_decision::tactics::{DefensiveTactic, TacticalSet};
use nba_domain::{GameFlowState, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::{LocomotionState, PhysicsWorld, PlayerPhysicsState};
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;

use crate::setup::{LineupConfig, MatchSetup};

use super::MatchEngine;

impl MatchEngine {
    pub fn with_setup(setup: MatchSetup, seed: u64) -> Self {
        if let Err(error) = setup.validate() {
            panic!("invalid match setup: {error}");
        }
        let mut rules = setup.rules.clone();
        let mut physics = PhysicsWorld::with_backend(&rules, setup.physics_backend);
        // 初始处理球人由**能力**派生（round-11 Step4b），而不是 `starters[0]`。
        //
        // `starters[0]` 是数组位置，用它当身份即"顺序即身份"（P-2）。
        // 选择口径与 `new_possession_pg` 一致：按**档案声明的持球槽位需求**打分，
        // 确定性排序。
        let initial_handler = |team: &nba_domain::TeamData, lineup: &LineupConfig| -> String {
            let spec = nba_domain::TacticalSetSpec::builtin(&lineup.offense_tactic)
                .expect("lineup offense tactic was validated by MatchSetup::validate");
            let mut ids: Vec<(f32, String)> = team
                .players
                .iter()
                .filter(|p| lineup.starters.contains(&p.id))
                .map(|p| {
                    let score =
                        nba_decision::tactics::TacticalPlanner::handler_score(&spec, &p.attributes);
                    (score, p.id.clone())
                })
                .collect();
            ids.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
            ids.into_iter().next().map(|(_, id)| id).unwrap_or_default()
        };
        let home_initial_handler = initial_handler(&setup.home_team, &setup.home_lineup);

        for (team, lineup, side) in [
            (&setup.home_team, &setup.home_lineup, "home"),
            (&setup.away_team, &setup.away_lineup, "away"),
        ] {
            let starter_ids = lineup
                .starters
                .iter()
                .collect::<std::collections::HashSet<_>>();
            let active_ids = lineup
                .starters
                .iter()
                .chain(lineup.bench.iter())
                .collect::<std::collections::HashSet<_>>();
            for player in &team.players {
                if !active_ids.contains(&&player.id) {
                    continue;
                }
                let (x, y) = player.initial_position_ft;
                let is_home = side == "home";
                physics.register_player(PlayerPhysicsState {
                    id: player.id.clone(),
                    jersey: player.jersey.clone(),
                    team: side.to_string(),
                    pos_ft: Vec2::new(x, y),
                    vel_ft: Vec2::ZERO,
                    accel_ft: Vec2::ZERO,
                    target_pos_ft: Vec2::new(x, y),
                    target_speed_ftps: 0.0,
                    // M9/attributes T2：属性→物理量经 capability 映射层（曲线下限走规则通道）。
                    max_speed_ftps: nba_domain::effective_max_speed(&rules, &player.attributes),
                    max_accel_ftps2: nba_domain::effective_max_accel(&rules, &player.attributes),
                    on_court: starter_ids.contains(&&player.id),
                    action: if is_home && player.id == home_initial_handler {
                        "Initiate".to_string()
                    } else if starter_ids.contains(&&player.id) {
                        "SetPosition".to_string()
                    } else {
                        "Bench".to_string()
                    },
                    // `roles` 字段已按文档移除（attributes.md §2.7/§2.9/T1）。
                    // 展示用槽位改为**能力与倾向的纯函数投影**：
                    // 不存字段、不按名册下标分派，因此名册数组顺序不携带语义。
                    slot: nba_domain::project_display_role(&player.attributes, &player.tendencies),
                    morale: "Normal".to_string(),
                    stamina: 100.0 * player.attributes.stamina.max(0.1),
                    max_stamina: 100.0 * player.attributes.stamina.max(0.1),
                    foul_count: 0,

                    locomotion: LocomotionState::Idle,
                    facing_dir: if is_home { Vec2::X } else { -Vec2::X },
                    ball_orientation: nba_domain::action_window::BallOrientation::FaceUp,
                    turn_decel_timer: 0.0,
                    is_locked_kinematics: false,
                    out_of_bounds_placement: false,
                    is_receiving_pass: false,
                    is_driving_to_rim: false,
                    boundary_cross_latched: false,
                    attributes: player.attributes.clone(),
                    tendencies: player.tendencies.clone(),
                });
            }
        }
        let initial_carrier_id = home_initial_handler.clone();
        let initial_pos = physics
            .get_player(&initial_carrier_id)
            .map(|p| p.pos_ft)
            .unwrap_or(Vec2::new(rules.court.width_ft * 0.3, rules.court.hoop_y_ft));
        let mut player_ids: Vec<String> = physics.get_players().keys().cloned().collect();
        player_ids.sort();
        let modulation = player_ids
            .into_iter()
            .filter_map(|id| {
                physics.get_player(&id).map(|player| {
                    (
                        player.id.clone(),
                        PlayerModulationState {
                            stamina: (player.stamina / player.max_stamina.max(f32::EPSILON))
                                .clamp(0.0, 1.0),
                            ..PlayerModulationState::default()
                        },
                    )
                })
            })
            .collect();
        let home_tactic = TacticalSet::from_id(&setup.home_lineup.offense_tactic)
            .expect("validated home offense tactic");
        let away_tactic = TacticalSet::from_id(&setup.away_lineup.offense_tactic)
            .expect("validated away offense tactic");
        let home_spec = nba_domain::TacticalSetSpec::builtin(&setup.home_lineup.offense_tactic)
            .expect("validated home offense spec");
        let away_spec = nba_domain::TacticalSetSpec::builtin(&setup.away_lineup.offense_tactic)
            .expect("validated away offense spec");
        let home_defense = DefensiveTactic::from_id(&setup.home_lineup.defense_tactic)
            .expect("validated home defense tactic");
        let away_defense = DefensiveTactic::from_id(&setup.away_lineup.defense_tactic)
            .expect("validated away defense tactic");
        if let Some(d) = nba_domain::DefenseRules::for_scheme(away_defense.id()) {
            let tuned = rules.tactics.defense.potential_field;
            rules.tactics.defense = nba_domain::DefenseRules {
                potential_field: tuned,
                ..d
            };
        }
        let team_traits = [
            ("home".to_string(), setup.home_team.team_traits.clone()),
            ("away".to_string(), setup.away_team.team_traits.clone()),
        ]
        .into_iter()
        .collect();
        Self {
            clock: super::state::MatchClock::new(
                rules.period_duration(1),
                rules.league.shot_clock_seconds,
                SubPhase::Initiation,
                -rules.decision_interval_seconds,
            ),
            flow: super::state::GameFlow::new(
                if rules.tip_off_duration_seconds > 0.0 {
                    GameFlowState::TipOff
                } else {
                    GameFlowState::LiveBall
                },
                Vec2::new(0.0, rules.court.hoop_y_ft),
            ),
            ball: super::state::BallRuntime::new(
                if rules.tip_off_duration_seconds > 0.0 {
                    (
                        Vec2::new(rules.court.width_ft * 0.5, rules.court.height_ft * 0.5),
                        4.0,
                    )
                } else {
                    (initial_pos, rules.ball_bounce_base_ft)
                },
                if rules.tip_off_duration_seconds > 0.0 {
                    BallTrajectoryKind::LooseBall {
                        pos: Vec2::new(rules.court.width_ft * 0.5, rules.court.height_ft * 0.5),
                        vel: Vec2::ZERO,
                        z: 4.0,
                        vel_z: 0.0,
                        last_touch_team: Possession::Home,
                        // 跳球尚未发生：还没有任何触球人。
                        last_touch_player: None,
                    }
                } else {
                    BallTrajectoryKind::Held {
                        carrier_id: initial_carrier_id.clone(),
                    }
                },
            ),
            systems: super::state::Systems {
                physics,
                decision: DecisionSystem::with_weights(rules.decision.clone()),
                coach: CoachStrategy::default(),
                rng: ChaCha8Rng::seed_from_u64(seed),
            },
            observations: super::state::RuntimeObservations::new(HashMap::new(), modulation),
            journal: {
                let mut journal = super::state::EventJournal::new();
                if rules.tip_off_duration_seconds > 0.0 {
                    journal.current_event = Some("TIPOFF".to_string());
                    journal.current_callout =
                        Some("裁判中圈垂直抛球，双方中锋起跳争顶！".to_string());
                } else {
                    journal.current_callout =
                        Some("比赛开始，跳球后主队获得第一攻球权。".to_string());
                }
                journal.current_intensity = Some("BuildUp".to_string());
                journal
            },
            config: super::state::TeamConfig {
                rules: rules.clone(),
                tactical_set: home_tactic,
                home_team: setup.home_team,
                away_team: setup.away_team,
                team_traits,
                home_roster_order: setup.home_lineup.starters.to_vec(),
                away_roster_order: setup.away_lineup.starters.to_vec(),
                home_offense_tactic: home_tactic,
                away_offense_tactic: away_tactic,
                home_offense_spec: home_spec,
                away_offense_spec: away_spec,
                home_playbook: setup.home_playbook,
                away_playbook: setup.away_playbook,
                home_defensive_tactic: home_defense,
                away_defensive_tactic: away_defense,
            },
            ledger: super::state::ScoreLedger::new(),
            possession_ctx: {
                let mut ctx =
                    super::state::PossessionContext::new(rules.league.period_duration_seconds);
                // 初始回合的失误责任人预先指向开局持球人（与改动前一致）。
                ctx.current_possession_turnover_player = Some(initial_carrier_id);
                ctx
            },

            audit: super::state::AuditTrail::new(),
        }
    }
}
