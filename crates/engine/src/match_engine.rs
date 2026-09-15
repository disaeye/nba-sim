//! 比赛模拟编排层：阶段驱动 + 约束过滤 + 效用决策 + 事件化输出。
//!
//! 这是引擎的主循环（架构文档 §11），取代旧的散落 if 决策：
//! - 阶段状态机推进（Inbound → Transition/SetPlay → Resolution → Rebound/DeadBallReset）
//! - 世界级 Runtime 约束检查（24 秒等）
//! - 决策系统按约束管线产出动作意图
//! - 执行系统更新球弹道与物理
//! - 裁决系统处理抢断/得分/篮板与阶段转换

use glam::Vec2;
use rand::Rng;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Write};

use flate2::write::GzEncoder;
use flate2::Compression;

use crate::setup::{LineupConfig, MatchSetup};
use nba_decision::constraint::{
    CandidateAction, ConstraintContext, EnforcementAction, PhaseType, ViolationKind,
};
use nba_decision::modulation::{CoachStrategy, PlayerModulationState};
use nba_decision::pipeline::{DecisionOutput, DecisionSystem};
use nba_decision::tactics::{DefensiveTactic, TacticalPlanner, TacticalSet};
use nba_domain::action_window::ActionTimeWindow;
use nba_domain::court::Court;
use nba_domain::{GameEvent, GameFlowState, GameRules, Possession, SubPhase, TeamData};
use nba_invariants::{InvariantChecker, Violation, ViolationTaxonomy};
use nba_officiating::resolution::{DriveResolution, ResolutionLayer, ResolutionOutcome};
use nba_physics::ballistics::{BallTrajectoryKind, BallisticsEngine};
use nba_physics::movement::{LocomotionState, PhysicsWorld, PlayerPhysicsState};
use nba_protocol::*;
use nba_semantics::{SemanticContact, SemanticEvaluator, SpacingEvaluation};

#[derive(Debug, Clone, Copy)]
enum ScopeBoundary {
    Possessions,
    Period { last_period: u32 },
    Game,
}

/// 流导出模式（gap.md §16.4 资源治理）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StreamMode {
    /// 逐 tick 完整帧：展示/回放需要，体积最大。
    Frames,
    /// gzip 压缩的逐 tick 完整帧：保留 Frames 语义，显著降低落盘体积。
    FramesGzip,
    /// 因果事实流（默认）：事实、事件日志、阶段/生命周期变化、回合总结。
    /// 引擎每 tick 自检 L1，因此无需输出每 tick 球员投影。
    #[default]
    Facts,
    /// 仅回合总结与比赛级元数据：体积极小，用于批量统计。
    Summary,
}

impl StreamMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "frames" | "full" => Ok(Self::Frames),
            "frames-gzip" | "gzip" | "compressed-frames" => Ok(Self::FramesGzip),
            "facts" | "causal" => Ok(Self::Facts),
            "summary" | "summaries" => Ok(Self::Summary),
            other => Err(format!(
                "unknown stream mode `{other}` (expected frames | frames-gzip | facts | summary)"
            )),
        }
    }
}

enum StreamWriter {
    Plain(BufWriter<File>),
    Gzip(GzEncoder<BufWriter<File>>),
}

impl Write for StreamWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self {
            Self::Plain(writer) => writer.write(bytes),
            Self::Gzip(writer) => writer.write(bytes),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Self::Plain(writer) => writer.flush(),
            Self::Gzip(writer) => writer.flush(),
        }
    }
}

impl StreamWriter {
    fn finish(self) -> std::io::Result<()> {
        match self {
            Self::Plain(mut writer) => writer.flush(),
            Self::Gzip(writer) => {
                let mut writer = writer.finish()?;
                writer.flush()
            }
        }
    }
}

/// `GameRules` → `FrameRules` 的唯一投影（C6.4）。
///
/// 此前引擎在 `build_tick` 内联展开 18 个字段，同时 protocol 的
/// `FrameRules::default()` 手抄同一组数字——两处独立漂移已实际发生
/// （`player_radius_ft` 引擎 1.8 vs 协议默认 1.0）。收敛为单一函数后，
/// `constraint_system` 的全字段一致性测试负责让任何一侧漂移必红。
pub fn frame_rules_from_game_rules(rules: &GameRules) -> FrameRules {
    FrameRules {
        tick_seconds: rules.tick_seconds,
        court_width_ft: rules.court.width_ft,
        court_height_ft: rules.court.height_ft,
        hoop_left_x_ft: rules.court.hoop_left_x_ft,
        hoop_right_x_ft: rules.court.hoop_right_x_ft,
        hoop_y_ft: rules.court.hoop_y_ft,
        player_radius_ft: rules.player_radius_ft,
        min_player_separation_ft: rules.min_player_separation_ft,
        separation_safety_margin_ft: rules.separation_safety_margin_ft,
        max_player_speed_ftps: rules.max_player_speed_ftps,
        max_player_accel_ftps2: rules.max_player_accel_ftps2,
        ball_max_speed_ftps: rules.ball_max_speed_ftps,
        three_point_distance_ft: rules.league.three_point_distance_ft,
        shot_clock_seconds: rules.league.shot_clock_seconds,
        holder_leash_ft: rules.invariant_holder_leash_ft,
        speed_tolerance_ftps: rules.invariant_speed_tolerance_ftps,
        ball_z_max_ft: rules.ball_z_max_ft,
    }
}

/// 事实流的去重游标：只在阶段/生命周期/回合边界变化时记一行。
#[derive(Debug, Clone, Default)]
struct StreamCursor {
    phase: String,
    game_flow: String,
    completed: usize,
}

/// 事实模式记录：保留因果与审计所需字段，去掉逐 tick 球员/球坐标投影。
///
/// `include_context` 只在流的首条记录为 true：`rules` 与 `tactical_set`
/// 对整场恒定，重复写入会占掉一半以上体积。消费者按首条记录继承即可
/// （gap.md §16.4）。
fn compact_fact_record(tick: &StreamTick, include_context: bool) -> serde_json::Value {
    let f = &tick.frame;
    let mut record = serde_json::json!({
        "t": f.t,
        "t_game": f.t_game,
        "shotClock": f.shot_clock,
        "period": f.period,
        "phase": f.phase,
        "game_flow": f.game_flow,
        "possession_id": f.possession_id,
        "possession_team": f.possession_team,
        "score": f.score,
        "event_type": f.event_type,
        "events": f.events,
        "event_log": f.event_log,
        "event_sequence": f.event_sequence,
        "completed_possessions": f.completed_possessions,
        "simulation_complete": f.simulation_complete,
        "team_fouls_home": f.team_fouls_home,
        "team_fouls_away": f.team_fouls_away,
        "free_throws_remaining": f.free_throws_remaining,
        "gameClock": tick.game_clock,
    });
    record["stream_projection"] = serde_json::Value::String("facts".to_string());
    if include_context {
        record["rules"] = serde_json::to_value(&f.rules).unwrap_or(serde_json::Value::Null);
        record["tactical_set"] = serde_json::Value::String(tick.tactical_set.clone());
    }
    record
}

/// 总结模式记录：只保留回合总结与比赛级元数据。
fn compact_summary_record(tick: &StreamTick, include_context: bool) -> serde_json::Value {
    let f = &tick.frame;
    let summaries: Vec<&FrameEvent> = f
        .event_log
        .iter()
        .filter(|e| e.kind == "POSSESSION_SUMMARY")
        .collect();
    let mut record = serde_json::json!({
        "t": f.t,
        "t_game": f.t_game,
        "period": f.period,
        "phase": f.phase,
        "game_flow": f.game_flow,
        "possession_id": f.possession_id,
        "score": f.score,
        "completed_possessions": f.completed_possessions,
        "simulation_complete": f.simulation_complete,
        "event_sequence": f.event_sequence,
        "event_log": summaries,
    });
    record["stream_projection"] = serde_json::Value::String("summary".to_string());
    if include_context {
        record["rules"] = serde_json::to_value(&f.rules).unwrap_or(serde_json::Value::Null);
        record["tactical_set"] = serde_json::Value::String(tick.tactical_set.clone());
    }
    record
}

/// 运行前磁盘预检（gap.md §16.4）：目标目录可用空间不足预算时直接报错。
///
/// 使用 `df -Pk` 获取可用块数（Linux/macOS 通用），不引入额外依赖；
/// 无法获取时不阻断，由写入期字节预算兜底。
fn ensure_disk_headroom(out_path: &str, budget_bytes: u64) -> std::io::Result<()> {
    let dir = std::path::Path::new(out_path)
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    let Ok(output) = std::process::Command::new("df")
        .arg("-Pk")
        .arg(dir)
        .output()
    else {
        return Ok(());
    };
    if !output.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&output.stdout);
    // df 输出：表头 + 一行数据，第 4 列为可用 1K 块。
    let Some(available_kb) = text
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|v| v.parse::<u64>().ok())
    else {
        return Ok(());
    };
    let available = available_kb.saturating_mul(1024);
    // 至少需要预算的 1.5 倍，并保留 512 MiB 系统余量。
    let required = budget_bytes + 256 * 1024 * 1024;
    if available < required {
        return Err(std::io::Error::new(
            std::io::ErrorKind::StorageFull,
            format!(
                "insufficient disk space in {}: {} MiB available, {} MiB required",
                dir.display(),
                available / (1024 * 1024),
                required / (1024 * 1024)
            ),
        ));
    }
    Ok(())
}

/// 主模拟状态。每个 possession 是阶段事件流的最小产出单位。
pub struct MatchEngine {
    /// Monotonic fixed-step index used by semantic facts and replay consumers.
    pub tick_index: u64,
    pub physics: PhysicsWorld,
    possession: Possession,
    possession_id: u32,
    sub_phase: SubPhase,
    sub_phase_timer: f32,
    tactical_set: TacticalSet,
    carrier_idx: usize,
    shot_clock: f32,
    game_clock: f32,
    current_time: f32,
    period: u32,
    home_score: u32,
    away_score: u32,
    ball_pos_3d: (Vec2, f32),
    ball_state: BallTrajectoryKind,
    last_passer_id: Option<String>,
    active_windows: HashMap<String, ActionTimeWindow>,
    current_event: Option<String>,
    current_callout: Option<String>,
    current_intensity: Option<String>,
    pub rng: ChaCha8Rng,
    target_possessions: usize,
    completed_possessions: usize,
    /// Requested-scope completion is separate from the real game's lifecycle.
    simulation_complete: bool,
    /// Raw `step()` runs continuously; scoped exports enable this boundary.
    scope_active: bool,
    scope_boundary: ScopeBoundary,

    pub rules: GameRules,
    pub decision: DecisionSystem,
    pub modulation: HashMap<String, PlayerModulationState>,
    pub coach: CoachStrategy,
    pub home_team: TeamData,
    pub away_team: TeamData,
    pub team_traits: HashMap<String, nba_domain::TeamTraits>,
    pub home_roster_order: Vec<String>,
    pub away_roster_order: Vec<String>,
    pub home_offense_tactic: TacticalSet,
    pub away_offense_tactic: TacticalSet,
    /// D5.1b：生效的进攻战术档案（槽位元数据来源）。此前档案只被用于
    /// 校验 id 合法，从未参与目标生成，导致所有槽位由全局 ratio 推得、
    /// 全队挤在弧顶三分线外。
    home_offense_spec: nba_domain::TacticalSetSpec,
    away_offense_spec: nba_domain::TacticalSetSpec,
    pub home_defensive_tactic: DefensiveTactic,
    pub away_defensive_tactic: DefensiveTactic,
    last_decision_trace: Option<Box<DecisionDebug>>,
    latest_spacing: Option<SpacingEvaluation>,
    latest_contacts: Vec<SemanticContact>,
    pending_events: Vec<GameEvent>,
    current_event_types: Vec<String>,
    current_enforcements: Vec<String>,
    event_sequence: u64,
    /// D4.1 全场单调的事件 ID（跨 tick 唯一，与逐 tick 的 `event_sequence` 区分）。
    event_id_counter: u64,
    /// D4.1 语义因果链注册表：记录"触发事件"的 event_id，按因果槽位索引
    /// （如 `shot_outcome` ← SHOT_RELEASE / `pass_outcome` ← PASS /
    /// `foul_ft` ← FOUL）。后续"结果事件"发布时按同槽位取父。
    ///
    /// 刻意不做"同 tick 首个事件作父"的粗暴串链：同 tick 内的两个独立
    /// 接触事实（CONTACT_BUMP ×2）之间没有因果关系，串链即是伪造因果，
    /// 违反"事件只陈述已发生的事实"（gap.md §7.1）。
    causal_links: std::collections::HashMap<&'static str, u64>,
    current_event_log: Vec<FrameEvent>,
    team_fouls_home: u32,
    team_fouls_away: u32,
    free_throws_remaining: u8,
    free_throw_attempt: u8,
    free_throw_shooter: Option<String>,
    inbound_baseline: Vec2,
    game_flow: GameFlowState,
    inbound_elapsed: f32,
    backcourt_elapsed: f32,
    period_break_elapsed: f32,
    last_decision_time: f32,
    /// Per-tick invariant checker; validates every emitted frame against the
    /// physical/basketball rules that must always hold, regardless of tactics.
    invariant_checker: InvariantChecker,
    /// Violations produced by the most recent `step()`; exported for callers
    /// that want a single aggregated report rather than per-tick stderr.
    last_tick_violations: Vec<Violation>,
    /// 投篮与比赛统计分解（2P/3P/FT 命中率与出手数、失误、犯规）。
    box_score: MatchBoxScore,
    /// 当前回合开始时的游戏时钟（用于计算回合时长）。
    /// 最近一次回合总结的 index（complete_possession 兜底发射的判据，
    /// M8 验收"回合零遗漏"：任何结束路径都必须有总结）。
    last_possession_summary_index: Option<u64>,
    current_possession_start_clock: f32,
    /// Monotonic simulation time at the start of the active possession.
    current_possession_start_time: f32,
    /// 当前回合内的连续传球次数。
    current_possession_passes: u32,
    /// 当前回合内的出手球员 ID。
    current_possession_shooter: Option<String>,
    /// 当前回合内的出手干扰度。
    current_possession_contest: Option<f32>,
    /// Receiver awaiting physical convergence to a frozen pass endpoint.
    pending_pass_receiver: Option<String>,
    /// 接球人**自己的**落点估计（层 A，P-1），跨 tick 保留。
    ///
    /// ## 为什么必须跨 tick 保留（round-10 修正）
    ///
    /// 第一版每 tick 从当前 `ball_pos` 重算估计点。实测后果：飞行末期球
    /// 逼近接球人时 `dist` 变小，估计点**向接球人塌缩**，随后方向翻转——
    /// 接球人目标点大幅抖动（实测 (19.8,36.5) → 自身位置 → (4.9,46.1)），
    /// 他来回跑，距球从 6.8 ft 单调恶化到 12.8 ft。
    ///
    /// 真实球员不会每帧重估：他在球出手后形成一个**稳定的预判**，之后
    /// 只做小幅修正。因此估计点必须作为状态保存，并在观察到新证据时
    /// **按观察力加权地向新信息靠拢**，而不是整体重算。
    receiver_estimate: Option<(String, Vec2)>,
    /// Cause carried by a loose ball until a player secures it.
    pending_loose_ball_terminal: Option<nba_domain::PossessionEndCause>,
    /// 最近一次被过掉（drive successful）的对位防守人（round-18）。
    beaten_defender_id: Option<String>,
    /// 被过防守人的恢复窗口截止时刻（round-19）：窗口内战术层不得重派。
    beaten_recovery_until: std::collections::HashMap<String, f32>,
    /// 上一 tick 的球位置（P-1 观测差分用，感知延迟一步）。
    prev_observed_ball_pos: Option<Vec2>,
    /// Whether the pending control transfer is the delayed arrival of an inbound pass.
    pending_pass_inbound: bool,
    /// Offensive player responsible for the active possession's last action.
    current_possession_turnover_player: Option<String>,
    /// 正在执行后场推进（`Advance`）的球员：其运动目标由推进决定，
    /// 本回合内不得被战术槽位覆盖（第一性原理：8 秒规则优先于落位）。
    advancing_player: Option<String>,
}

/// 比赛投篮分解与核心统计。
#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MatchBoxScore {
    pub fg2_attempts: u32,
    pub fg2_made: u32,
    pub fg3_attempts: u32,
    pub fg3_made: u32,
    pub ft_attempts: u32,
    pub ft_made: u32,
    pub turnovers: u32,
    pub fouls: u32,
}

impl MatchBoxScore {
    pub fn fg2_pct(&self) -> f32 {
        if self.fg2_attempts == 0 {
            0.0
        } else {
            self.fg2_made as f32 / self.fg2_attempts as f32
        }
    }
    pub fn fg3_pct(&self) -> f32 {
        if self.fg3_attempts == 0 {
            0.0
        } else {
            self.fg3_made as f32 / self.fg3_attempts as f32
        }
    }
    pub fn ft_pct(&self) -> f32 {
        if self.ft_attempts == 0 {
            0.0
        } else {
            self.ft_made as f32 / self.ft_attempts as f32
        }
    }
}

/// Summary of a scoped simulation export.
#[derive(Debug)]
pub struct ExportSummary {
    /// Number of ticks written to the stream.
    pub ticks: usize,
    /// Human-readable scope description (e.g. "1 Quarter").
    pub scope_desc: String,
    /// All invariant violations detected across the run; empty means the
    /// simulation satisfied every physical/basketball rule that was checked.
    pub violations: Vec<Violation>,
    /// 比赛统计数据分解。
    pub box_score: MatchBoxScore,
    /// 违例多维分类账分析结果。
    pub taxonomy: ViolationTaxonomy,
}

pub type Simulation = MatchEngine;

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        Self::with_setup(MatchSetup::builtin(GameRules::default()), seed)
    }

    pub fn with_rules(seed: u64, rules: GameRules) -> Self {
        Self::with_setup(MatchSetup::builtin(rules), seed)
    }

    pub fn with_setup(setup: MatchSetup, seed: u64) -> Self {
        if let Err(error) = setup.validate() {
            panic!("invalid match setup: {error}");
        }
        let rules = setup.rules.clone();
        let mut physics = PhysicsWorld::with_backend(&rules, setup.physics_backend);
        // 初始处理球人由**能力**派生（round-11 Step4b），而不是 `starters[0]`。
        //
        // `starters[0]` 是数组位置，用它当身份即"顺序即身份"（P-2）。
        // 选择口径与 `new_possession_pg` 一致：处理球三项能力加权，确定性排序。
        let initial_handler = |team: &nba_domain::TeamData, lineup: &LineupConfig| -> String {
            let policy = &rules.tactics;
            let mut ids: Vec<(f32, String)> = team
                .players
                .iter()
                .filter(|p| lineup.starters.contains(&p.id))
                .map(|p| {
                    let score = p.attributes.ball_handling
                        * policy.slot_handler_ball_handling_weight
                        + p.attributes.passing * policy.slot_handler_passing_weight
                        + p.attributes.decision_iq * policy.slot_handler_decision_iq_weight;
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
                    has_ball: is_home && player.id == home_initial_handler,
                    on_court: starter_ids.contains(&&player.id),
                    action: if is_home && player.id == home_initial_handler {
                        "Initiate".to_string()
                    } else if starter_ids.contains(&&player.id) {
                        "SetPosition".to_string()
                    } else {
                        "Bench".to_string()
                    },
                    // `roles` 字段已按契约移除（attributes.md §2.7/§2.9/T1）。
                    // 展示用槽位改为**能力与倾向的纯函数投影**：
                    // 不存字段、不按名册下标分派，因此名册数组顺序不携带语义。
                    slot: nba_domain::project_display_role(&player.attributes, &player.tendencies),
                    morale: "Normal".to_string(),
                    stamina: 100.0 * player.attributes.stamina.max(0.1),
                    max_stamina: 100.0 * player.attributes.stamina.max(0.1),
                    foul_count: 0,

                    locomotion: LocomotionState::Idle,
                    facing_dir: if is_home { Vec2::X } else { -Vec2::X },
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
        let team_traits = [
            ("home".to_string(), setup.home_team.team_traits.clone()),
            ("away".to_string(), setup.away_team.team_traits.clone()),
        ]
        .into_iter()
        .collect();
        Self {
            tick_index: 0,
            physics,
            possession: Possession::Home,
            possession_id: 1,
            sub_phase: SubPhase::Initiation,
            sub_phase_timer: 0.0,
            tactical_set: home_tactic,
            carrier_idx: 0,
            shot_clock: rules.league.shot_clock_seconds,
            game_clock: rules.period_duration(1),
            current_time: 0.0,
            period: 1,
            home_score: 0,
            away_score: 0,
            ball_pos_3d: if rules.tip_off_duration_seconds > 0.0 {
                (
                    Vec2::new(rules.court.width_ft * 0.5, rules.court.height_ft * 0.5),
                    4.0,
                )
            } else {
                (initial_pos, rules.ball_bounce_base_ft)
            },
            ball_state: if rules.tip_off_duration_seconds > 0.0 {
                BallTrajectoryKind::LooseBall {
                    pos: Vec2::new(rules.court.width_ft * 0.5, rules.court.height_ft * 0.5),
                    vel: Vec2::ZERO,
                    z: 4.0,
                    vel_z: 0.0,
                    last_touch_team: Possession::Home,
                }
            } else {
                BallTrajectoryKind::Held {
                    carrier_id: initial_carrier_id.clone(),
                }
            },
            last_passer_id: None,
            active_windows: HashMap::new(),
            current_event: if rules.tip_off_duration_seconds > 0.0 {
                Some("TIPOFF".to_string())
            } else {
                None
            },
            current_callout: if rules.tip_off_duration_seconds > 0.0 {
                Some("裁判中圈垂直抛球，双方中锋起跳争顶！".to_string())
            } else {
                Some("比赛开始，跳球后主队获得第一攻球权。".to_string())
            },
            rng: ChaCha8Rng::seed_from_u64(seed),
            current_intensity: Some("BuildUp".to_string()),
            target_possessions: 10,
            rules: rules.clone(),
            completed_possessions: 0,
            simulation_complete: false,
            scope_active: false,
            scope_boundary: ScopeBoundary::Possessions,
            decision: DecisionSystem::with_weights(rules.decision.clone()),
            modulation,
            coach: CoachStrategy::default(),
            home_team: setup.home_team,
            away_team: setup.away_team,
            home_roster_order: setup.home_lineup.starters.to_vec(),
            away_roster_order: setup.away_lineup.starters.to_vec(),
            team_traits,
            game_flow: if rules.tip_off_duration_seconds > 0.0 {
                GameFlowState::TipOff
            } else {
                GameFlowState::LiveBall
            },
            home_offense_tactic: home_tactic,
            away_offense_tactic: away_tactic,
            home_offense_spec: home_spec,
            away_offense_spec: away_spec,
            home_defensive_tactic: home_defense,
            away_defensive_tactic: away_defense,
            last_decision_trace: None,
            latest_spacing: None,
            latest_contacts: Vec::new(),
            pending_events: Vec::new(),
            current_event_types: Vec::new(),
            current_enforcements: Vec::new(),
            event_sequence: 0,
            event_id_counter: 0,
            causal_links: std::collections::HashMap::new(),
            current_event_log: Vec::new(),
            team_fouls_home: 0,
            team_fouls_away: 0,
            free_throws_remaining: 0,
            free_throw_attempt: 0,
            free_throw_shooter: None,
            inbound_baseline: Vec2::new(0.0, rules.court.hoop_y_ft),
            inbound_elapsed: 0.0,

            backcourt_elapsed: 0.0,
            period_break_elapsed: 0.0,
            last_decision_time: -rules.decision_interval_seconds,
            invariant_checker: InvariantChecker::new(),
            last_tick_violations: Vec::new(),
            box_score: MatchBoxScore::default(),
            last_possession_summary_index: None,
            current_possession_start_clock: rules.league.period_duration_seconds,
            current_possession_start_time: 0.0,
            current_possession_passes: 0,
            current_possession_shooter: None,
            current_possession_contest: None,
            pending_pass_receiver: None,
            receiver_estimate: None,
            pending_loose_ball_terminal: None,
            beaten_defender_id: None,
            beaten_recovery_until: std::collections::HashMap::new(),
            prev_observed_ball_pos: None,
            pending_pass_inbound: false,
            current_possession_turnover_player: Some(initial_carrier_id),
            advancing_player: None,
        }
    }

    /// 在回合转换边界发射 L2 回合语义总结事件（docs/quality.md §2.2）。
    /// 终结原因是 `PossessionEndCause` 显式枚举——没有兜底值，调用方
    /// 必须在编译期说明回合为什么结束（dev 方案 §3.2 D0.1）。
    pub fn emit_possession_summary(
        &mut self,
        terminal_event: nba_domain::PossessionEndCause,
        rebounder_id: Option<String>,
        turnover_player_id: Option<String>,
        rebound_distance_ft: Option<f32>,
    ) {
        let start_clock = self.current_possession_start_clock;
        let end_clock = self.game_clock;
        let duration_seconds = (self.current_time - self.current_possession_start_time).max(0.0);
        let offense_team = match self.possession {
            Possession::Home => "home".to_string(),
            Possession::Away => "away".to_string(),
        };
        let summary = nba_domain::PossessionSummary {
            possession_index: self.completed_possessions as u64,
            offense_team,
            start_clock,
            end_clock,
            duration_seconds,
            passes_count: self.current_possession_passes,
            terminal_event,
            shooter_id: self.current_possession_shooter.clone(),
            rebounder_id,
            turnover_player_id,
            shot_contest_intensity: self.current_possession_contest,
            rebound_distance_ft,
        };
        self.last_possession_summary_index = Some(summary.possession_index);
        self.pending_events
            .push(nba_domain::GameEvent::PossessionSummary(summary));
        // 重置下一个回合的上下文
        self.current_possession_start_clock = self.game_clock;
        self.current_possession_start_time = self.current_time;
        self.current_possession_passes = 0;
        self.current_possession_shooter = None;
        self.current_possession_contest = None;
        self.current_possession_turnover_player = None;
        self.current_callout = None;
    }
    pub fn is_finished(&self) -> bool {
        self.game_flow == GameFlowState::GameEnd || (self.scope_active && self.simulation_complete)
    }

    fn break_finished(&self) -> bool {
        self.period_break_elapsed + f32::EPSILON >= self.rules.period_break_seconds
    }

    /// Parses a scope string into a possession budget and human description.
    /// Malformed scopes are rejected instead of silently changing the request.
    pub fn parse_scope(scope: &str) -> Result<(usize, String), String> {
        Self::parse_scope_with_rules(scope, &GameRules::default())
    }

    pub fn parse_scope_with_rules(
        scope: &str,
        rules: &GameRules,
    ) -> Result<(usize, String), String> {
        let normalized = scope.trim().to_ascii_lowercase();
        let possessions = match normalized.as_str() {
            "1p" => 1usize,
            "5p" => 5usize,
            "10p" => 10usize,
            "1q" => rules.estimated_possessions_per_period as usize,
            "full" => (rules.estimated_possessions_per_period as usize)
                .checked_mul(rules.league.regulation_periods as usize)
                .ok_or_else(|| "scope possession budget overflowed".to_string())?,
            value if value.ends_with('p') => value[..value.len() - 1]
                .parse::<usize>()
                .map_err(|_| format!("invalid scope: {scope}"))?,
            _ => return Err(format!("invalid scope: {scope}")),
        };
        if possessions == 0 {
            return Err("scope must request at least one possession".to_string());
        }
        let description = if normalized == "1q" {
            format!("1 Quarter (approx {possessions} possessions)")
        } else if normalized == "full" {
            format!("Full Game (approx {possessions} possessions)")
        } else {
            format!("{possessions} Possessions")
        };
        Ok((possessions, description))
    }
    pub fn set_scope(&mut self, scope: &str) -> Result<String, String> {
        let (target_possessions, description) = Self::parse_scope_with_rules(scope, &self.rules)?;
        let normalized = scope.trim().to_ascii_lowercase();
        self.target_possessions = target_possessions;
        self.scope_active = true;
        self.scope_boundary = match normalized.as_str() {
            "1q" => ScopeBoundary::Period {
                last_period: self.period,
            },
            "full" => ScopeBoundary::Game,
            _ => ScopeBoundary::Possessions,
        };
        self.simulation_complete = match self.scope_boundary {
            ScopeBoundary::Possessions => self.completed_possessions >= target_possessions,
            ScopeBoundary::Period { .. } | ScopeBoundary::Game => false,
        };
        if self.simulation_complete {
            self.settle_scope_ball(self.ball_pos_3d);
        }
        Ok(description)
    }

    /// Result of a scoped export: tick count, scope label, and any invariant
    /// violations detected across the run. Callers should treat a non-empty
    /// violation list as a failed simulation, not merely a warning.
    pub fn simulate_scope_and_export(
        &mut self,
        scope: &str,
        out_path: &str,
    ) -> std::io::Result<ExportSummary> {
        self.simulate_scope_and_export_with_mode(scope, out_path, StreamMode::default())
    }

    /// 按指定流模式导出一场比赛（gap.md §16.4 资源治理）。
    ///
    /// - [`StreamMode::Frames`]：逐 tick 完整帧（展示/回放，大）；
    /// - [`StreamMode::FramesGzip`]：同一完整帧协议的 gzip 落盘格式；
    /// - [`StreamMode::Facts`]（默认）：只写因果事实、事件日志、阶段/生命周期
    ///   变化、回合总结与周期性检查点。引擎每 tick 自检 L1，流不必携带
    ///   每 tick 的球员投影，因此体积从数百 MB 降到个位数 MB；
    /// - [`StreamMode::Summary`]：只写回合总结与比赛级元数据，体积极小。
    ///
    /// 所有模式都在写入前检查磁盘空间与文件大小预算，超限即报错而不是
    /// 静默写满磁盘。
    pub fn simulate_scope_and_export_with_mode(
        &mut self,
        scope: &str,
        out_path: &str,
        mode: StreamMode,
    ) -> std::io::Result<ExportSummary> {
        let scope_desc = self
            .set_scope(scope)
            .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidInput, error))?;

        let byte_budget = match mode {
            StreamMode::Frames | StreamMode::FramesGzip => self.rules.stream_frames_max_bytes,
            StreamMode::Facts | StreamMode::Summary => self.rules.stream_max_bytes,
        };
        // 运行前磁盘预检：不足安全余量则拒绝启动。
        ensure_disk_headroom(out_path, byte_budget)?;

        let file = File::create(out_path)?;
        let mut writer = match mode {
            StreamMode::FramesGzip => {
                StreamWriter::Gzip(GzEncoder::new(BufWriter::new(file), Compression::default()))
            }
            StreamMode::Frames | StreamMode::Facts | StreamMode::Summary => {
                StreamWriter::Plain(BufWriter::new(file))
            }
        };
        let mut ticks_count = 0;
        let mut all_violations: Vec<Violation> = Vec::new();
        let mut written_bytes: u64 = 0;
        // A lifecycle bug must fail closed rather than grow an unbounded NDJSON
        // file forever. The bound is a safety guard, not a basketball rule.
        let max_ticks = self.rules.stream_max_ticks;
        let previous = StreamCursor::default();
        let mut previous = previous;
        let mut meta_written = false;
        while !self.is_finished() && ticks_count < max_ticks {
            let tick = self.step();
            all_violations.append(&mut self.last_tick_violations);
            let should_write = match mode {
                StreamMode::Frames | StreamMode::FramesGzip => true,
                StreamMode::Facts => {
                    written_bytes == 0
                        || !tick.frame.event_log.is_empty()
                        || !tick.frame.events.is_empty()
                        || tick.frame.phase != previous.phase
                        || tick.frame.game_flow != previous.game_flow
                        || tick.frame.completed_possessions != previous.completed
                        || tick.frame.simulation_complete
                }
                StreamMode::Summary => {
                    !meta_written
                        || tick
                            .frame
                            .event_log
                            .iter()
                            .any(|e| e.kind == "POSSESSION_SUMMARY")
                }
            };
            previous.phase = tick.frame.phase.clone();
            previous.game_flow = tick.frame.game_flow.clone();
            previous.completed = tick.frame.completed_possessions;
            meta_written = true;

            if should_write {
                let payload = match mode {
                    StreamMode::Frames | StreamMode::FramesGzip => serde_json::to_string(&tick)?,
                    StreamMode::Facts => {
                        serde_json::to_string(&compact_fact_record(&tick, written_bytes == 0))?
                    }
                    StreamMode::Summary => {
                        serde_json::to_string(&compact_summary_record(&tick, written_bytes == 0))?
                    }
                };
                written_bytes = written_bytes.saturating_add(payload.len() as u64 + 1);
                if written_bytes > byte_budget {
                    return Err(std::io::Error::other(format!(
                        "stream for scope `{scope}` exceeded {}-byte budget (mode={:?})",
                        byte_budget, mode
                    )));
                }
                writer.write_all(payload.as_bytes())?;
                writer.write_all(b"\n")?;
            }
            ticks_count += 1;
        }
        if !self.is_finished() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                format!("scope `{scope}` did not complete within {max_ticks} ticks"),
            ));
        }
        if written_bytes == 0 {
            let tick = self.build_tick();
            let payload = serde_json::to_string(&compact_summary_record(&tick, true))?;
            writer.write_all(payload.as_bytes())?;
            writer.write_all(b"\n")?;
        }
        writer.finish()?;
        if !all_violations.is_empty() {
            eprintln!(
                "=== INVARIANT VIOLATIONS ({} total) ===",
                all_violations.len()
            );
            for v in &all_violations {
                eprintln!("  {}", v);
            }
        }
        let taxonomy = ViolationTaxonomy::from_violations(&all_violations);
        Ok(ExportSummary {
            ticks: ticks_count,
            scope_desc,
            violations: all_violations,
            box_score: self.box_score.clone(),
            taxonomy,
        })
    }
    /// 当前阶段映射到约束阶段类型；宏观生命周期决定发球、推进和罚球语义。
    fn phase_type(&self) -> PhaseType {
        if self.game_flow == GameFlowState::FreeThrow {
            return PhaseType::FreeThrow;
        }
        if self.game_flow == GameFlowState::TipOff {
            return PhaseType::TipOff;
        }
        match self.sub_phase {
            SubPhase::Initiation if self.game_flow == GameFlowState::DeadBall => PhaseType::Inbound,
            SubPhase::Initiation => PhaseType::Transition,
            SubPhase::ActionExecution => PhaseType::SetPlay,
            SubPhase::ShotAttempt => PhaseType::Resolution,
            SubPhase::FlightAndRebound => PhaseType::Rebound,
            SubPhase::DeadBallReset => PhaseType::DeadBallReset,
        }
    }
    pub fn constraint_ctx<'a>(&'a self) -> ConstraintContext<'a> {
        let possession_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        ConstraintContext {
            physics: &self.physics,
            ball_pos: self.ball_pos_3d.0,
            possession_team,
            shot_clock: self.shot_clock,
            game_clock: self.game_clock,
            phase: self.phase_type(),
            game_flow: self.game_flow,
            ball_phase: self.ball_phase(),
            inbound_elapsed: self.inbound_elapsed,
            backcourt_elapsed: self.backcourt_elapsed,
            rules: &self.rules,
            team_traits: &self.team_traits,
        }
    }
    /// 球的宏观相位（由领域层 BallState 派生，M2：标签不再是独立状态）。
    fn ball_phase(&self) -> nba_domain::BallPhase {
        if self.free_throws_remaining > 0 {
            return nba_domain::BallPhase::Dead;
        }
        self.ball_state.phase()
    }

    /// 持球人 id。
    fn carrier_id(&self) -> String {
        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            } => carrier_id.clone(),
            BallTrajectoryKind::InboundTransfer { inbounder_id, .. } => inbounder_id.clone(),
            _ => {
                let roster = match self.possession {
                    Possession::Home => &self.home_roster_order,
                    Possession::Away => &self.away_roster_order,
                };
                roster
                    .get(self.carrier_idx)
                    .cloned()
                    .unwrap_or_else(|| roster.first().cloned().unwrap_or_default())
            }
        }
    }
    /// Keeps the active offensive scheme derived from the configured team
    /// owning the ball; transitions must not replace setup with random data.
    /// 领传点：按接球人的**当前速度**外推一个飞行期内的可达点（round-7）。
    ///
    /// ## 为什么需要它
    ///
    /// 传球 release 时冻结的 `to_pos` 是「球将到达的位置」。若直接取接球人
    /// **释放时刻**的位置，而接球人在飞行期间仍在跑动，他永远不会恰好停在
    /// 那个点——实测越位 5–12 ft。
    ///
    /// 决策侧传球已通过决策层给出带提前量的 `to_pos`；outlet 一传漏了这一步，
    /// 成为 `PASS_CORRIDOR_REACHABLE` 的最大单一来源。
    ///
    /// ## 模型
    ///
    /// ```text
    /// flight = pass_duration(distance)          // 与弹道使用同一时长函数
    /// lead   = receiver_velocity × flight × lead_gain
    /// to_pos = receiver_pos + lead              // 封顶到 lead_max_ft
    /// ```
    ///
    /// 不做二次迭代（先用初速估时长，再用新距离重算）：一阶近似已足够，
    /// 且迭代会使事实依赖收敛路径，不利于可复现性。
    fn lead_receiver_position(
        &self,
        receiver_id: &str,
        receiver_pos: Vec2,
        distance_hint: f32,
        inbound: bool,
    ) -> Vec2 {
        let Some(p) = self.physics.get_player(receiver_id) else {
            return receiver_pos;
        };
        let speed = p.vel_ft.length();
        if speed <= f32::EPSILON {
            return receiver_pos;
        }
        let flight = self.rules.pass_duration(distance_hint, inbound);
        let gain = self.rules.tactics.pass_lead_gain;
        let max_lead = self.rules.tactics.pass_lead_max_ft;
        let lead = p.vel_ft * flight * gain;
        let lead = if lead.length() > max_lead {
            lead.normalize_or_zero() * max_lead
        } else {
            lead
        };
        self.rules
            .court
            .clamp_playable(receiver_pos + lead, self.rules.player_radius_ft)
    }

    /// 接球人向冻结点收敛的目标与速度（round-6 审计修复）。
    ///
    /// ## 缺陷（修复前）
    ///
    /// 接球人被硬编码为 20 ft/s 全速冲向 `frozen_to_pos`，且**没有减速模型**。
    /// 传球飞行时长受 `max_pass_duration_seconds`（默认 1.4s）封顶，但 40–60 ft
    /// 的跨场 outlet 实际只需 0.5–0.7s。于是接球人在被拉长的飞行期内持续全速
    /// 前进，实测**越过**冻结点 5–12.5 ft（越位方向几乎垂直于传球线）。
    ///
    /// 后果：`PASS_CORRIDOR_REACHABLE` 在 8 seed 下报 8–17 条 Hard defect。
    ///
    /// ## 模型
    ///
    /// 用「制动距离」反解允许速度：
    ///
    /// ```text
    /// v_allow = sqrt(2 · a_max · max(d_remaining - margin, 0))
    /// ```
    ///
    /// 即剩余距离越短，允许速度越低（接近冻结点时自然减速）；同时在远距离
    /// 封顶到 `receive_approach_speed_ratio × max_player_speed`，避免用超过
    /// 人体上限的速度冲向终点。
    ///
    /// 这不是「为了通过评判而调参」：它补的是**缺失的物理约束**——此前模型
    /// 允许球员以 20 ft/s 穿过目标点而不减速，违反 `gap.md §12.1`
    /// 「加速、制动、变向受属性与规则上限约束」。
    fn receive_approach(&self, frozen_to_pos: Vec2, player_id: &str) -> (Vec2, f32) {
        let Some(p) = self.physics.get_player(player_id) else {
            return (frozen_to_pos, self.rules.max_player_speed_ftps * 0.5);
        };
        let to_target = frozen_to_pos - p.pos_ft;
        let d = to_target.length();
        let accel = self.rules.max_player_accel_ftps2.max(f32::EPSILON);
        let current_speed = p.vel_ft.length();

        // ## 制动距离必须自洽（round-8 审计修复）
        //
        // `receive_stop_margin_ft` 是一个**固定**裕量，但所需的制动距离是
        // `v²/(2a)` —— 随接近速度增大。实测：
        //   19.8 ft/s -> 需 5.60 ft；12.0 -> 2.06 ft；5.5 -> 0.43 ft
        // 当裕量（2.5 ft）小于所需制动距离时，「已到位则停下」的分支永远
        // 不可达：接球人一边被 `sqrt(2as)` 减速、一边因为 `d > margin` 继续
        // 被推着走，结果在球到达时已越过 5–12 ft。
        //
        // 修正：用**物理所需的制动距离**取代固定裕量，并取两者较大值
        // （保留一个下限，避免低速时数值抖动）。
        let brake_dist = current_speed * current_speed / (2.0 * accel);
        let stop_margin = self.rules.tactics.receive_stop_margin_ft.max(brake_dist);

        // 已进入制动距离内：站住等球（真实接球动作的语义）。
        if d <= stop_margin {
            return (p.pos_ft, 0.0);
        }

        let remaining = (d - stop_margin).max(0.0);
        // 制动距离反解：v = sqrt(2·a·s)。
        let v_allow = (2.0 * accel * remaining).sqrt();
        let v_cap =
            self.rules.max_player_speed_ftps * self.rules.tactics.receive_approach_speed_ratio;
        let v_min =
            self.rules.max_player_speed_ftps * self.rules.tactics.receive_min_approach_speed_ratio;
        let speed = v_allow.min(v_cap).max(v_min);

        // 目标点提前 stop_margin：即使在离散 tick 下也不会越过冻结点。
        let aim = frozen_to_pos - to_target.normalize() * stop_margin;
        (aim, speed)
    }

    /// 由一次失败的传球构造松球（层 A 失败，P-1）。
    ///
    /// 初速必须服从 `ball_max_speed_ftps`（留安全余量），而不是
    /// `segment / duration`（那会产生 49 ft/s 的初速，松球随即飞出边线）。
    fn loose_ball_from(&self, pos: Vec2, segment: Vec2) -> BallTrajectoryKind {
        let dir = segment.normalize_or_zero();
        let cap = (self.rules.ball_max_speed_ftps - self.rules.invariant_speed_tolerance_ftps)
            .max(self.rules.invariant_speed_tolerance_ftps);
        BallTrajectoryKind::LooseBall {
            pos,
            vel: dir * cap,
            z: self.ball_pos_3d.1,
            vel_z: 0.0,
            last_touch_team: self.possession,
        }
    }

    /// 接球人对"球会到哪里"的**自身估计**（层 A，P-1）。
    ///
    /// ## 为什么不能用 `frozen_to_pos`
    ///
    /// `frozen_to_pos` 是**传球人的意图**。接球人若直读它，就获得了全知视角，
    /// 必然到位——这违反真实性：真实比赛里接球人只能根据**可观察到的**信息
    /// 预判（球的来向与速度、传球人的动作、自己的位置与速度），预判可能错。
    ///
    /// ## 估计模型
    ///
    /// ```text
    /// 观测点  = 上一 tick 的球位置（感知延迟，不是瞬时真值）
    /// 球速估计 = 球的当前速度（接球人看不到 flight_duration）
    /// 到达时间 = |观测点 − 自己| / max(球速估计, 下限)      ← 他自己的估算
    /// 落点估计 = 观测点 + 球向 × 球速 × 到达时间
    ///            + 自己的速度 × 到达时间 × 预判增益
    ///            + 噪声(off_ball_sense 越低越大)
    /// ```
    ///
    /// 噪声是**确定性伪随机**（由 tick + 球员 id 哈希派生），不是真随机：
    /// 保证可复现（charter C4），同时使不同球员/回合的预判偏差不同。
    ///
    /// 参数全部走规则通道（`receive_estimate_noise_ft` 等），无内联行为常数。
    fn estimate_receiver_landing(&mut self, receiver_id: &str, frozen_to_pos: Vec2) -> Vec2 {
        let Some(receiver) = self.physics.get_player(receiver_id) else {
            return frozen_to_pos;
        };
        // 规则开关：噪声为 0 时退化为"精确知道落点"（旧行为，仅用于对照）。
        let noise_cap = nba_domain::receive_estimate_noise(&self.rules, &receiver.attributes);
        if noise_cap <= f32::EPSILON {
            return frozen_to_pos;
        }

        // 感知延迟：用球**上一 tick** 的位置作为观测点。
        // 引擎在决策前已将本 tick 的 ball_pos_3d 推进，故这里近似为"当前可见位置"。
        let observed_ball = self.ball_pos_3d.0;
        let self_pos = receiver.pos_ft;
        let to_ball = observed_ball - self_pos;
        let dist = to_ball.length();
        if dist <= f32::EPSILON {
            return frozen_to_pos;
        }

        // ## 稳定估计（round-10 修正两次）
        //
        // 球员在球出手后形成一个**预判**，之后只按观察力做小幅修正；
        // 不会每 tick 整体重算（那会因球的逼近导致目标塌缩与方向翻转，
        // 实测接球人距球从 6.8 ft 恶化到 12.8 ft）。
        //
        // **第二次修正**：初版用 `ball_vel × pass_duration(dist)` 做外推，
        // 但 `ball_vel` 是**实际**飞行速度（`seg/duration`，可达 50 ft/s），
        // 而 `pass_duration` 按**名义**球速（32 ft/s）给出时长——两者混用
        // 导致 10 ft 传球被外推 22.5 ft（冲过头），层 A 随即失败。
        //
        // 正确的初判：球能走多远，受**剩余飞行距离**约束，不得超出
        // 「球到接球人的距离」。即接球人认为球最多飞到他自己所在处；
        // 提前量来自他看见球的运动方向，而不是把他自己再外推一次。
        let ball_vel = self.ball_velocity_estimate();
        let ball_speed = ball_vel.length();
        let remaining = dist; // 球到接球人的距离 = 它最多还能前进的量
        let initial = if ball_speed > f32::EPSILON {
            let travel = remaining.min(ball_speed * self.rules.tick_seconds * dist.max(1.0));
            observed_ball + ball_vel.normalize_or_zero() * travel
        } else {
            observed_ball
        };

        // 取出或建立本回合的稳定估计。
        let prev = match &self.receiver_estimate {
            Some((id, p)) if id == receiver_id => Some(*p),
            _ => None,
        };
        let sense = receiver.attributes.off_ball_sense.clamp(0.0, 1.0);

        // 观察修正：向"球的实际位置 + 其运动方向上的有限外推"按观察力加权。
        // 观察力强 → 快速跟上球的真实轨迹；观察力弱 → 停在最初预判上。
        let observe_target = initial;
        let base = prev.unwrap_or(initial);
        let blended = base + (observe_target - base) * sense;

        // 预判噪声：off_ball_sense 越低残留越大；方向由确定性哈希决定。
        // round-19 修复：`(1−sense)` 此前被应用两次（capability 里一次、
        // 这里一次），实际噪声 = noise_ft×(1−sense)²，低观察力球员的
        // 噪声被意外压缩。恢复设计意图：线性 (1−sense)。
        let residual_noise = noise_cap;
        let noise = self.deterministic_estimate_offset(receiver_id) * residual_noise;
        let est = self
            .rules
            .court
            .clamp_playable(blended + noise, self.rules.player_radius_ft);
        // 保存稳定估计（跨 tick 复用）。
        self.receiver_estimate = Some((receiver_id.to_string(), est));
        est
    }

    /// 球的瞬时速度估计（层 A 用）。
    ///
    /// 接球人能看到球在动，但看不到传球人冻结的 `duration`。这里用
    /// **上一 tick 与本 tick 的球位置差**给出方向与量级；无运动时返回零。
    fn ball_velocity_estimate(&self) -> Vec2 {
        // ## P-1 修复（round-19）：观测差分，不读传球人的冻结意图
        //
        // 原实现直读球态的 `to_pos`/`from_pos`/`duration`——传球人的私有
        // 意图（全知泄漏）。其文档注释声称"用上一 tick 与本 tick 的球位置
        // 差"，注释与实现不一致（契约-代码漂移）。
        //
        // 接球人可观测的是**球的运动本身**：位置差分给出方向与速度，
        // 信息量与冻结向量等价（球匀速直线飞行），但来源合法。
        // `prev_observed_ball_pos` 由引擎每 tick 记录（感知延迟一步）。
        if let Some(prev) = self.prev_observed_ball_pos {
            let dt = self.rules.tick_seconds.max(f32::EPSILON);
            let delta = (self.ball_pos_3d.0 - prev) / dt;
            // 速度量级钳制在球的物理上限内（观测噪声保护）。
            let cap = self.rules.ball_max_speed_ftps;
            if delta.length() > cap {
                delta.normalize_or_zero() * cap
            } else {
                delta
            }
        } else {
            Vec2::ZERO
        }
    }

    /// 确定性偏差向量（单位长度内），由 tick 与球员 id 派生。
    ///
    /// 不是真随机：同一 tick + 同一球员必得同一值（charter C4 可复现）。
    /// 用简单 FNV 混合：避免引入新 RNG 流而改变既有随机序列。
    fn deterministic_estimate_offset(&self, player_id: &str) -> Vec2 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in player_id.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h ^= self.tick_index;
        h = h.wrapping_mul(0x100_0000_01b3);
        // 映射到 [-1, 1] 的二维单位向量（用两个 16 位切片）。
        let q = self.rules.estimate_offset_quantization.max(f32::EPSILON);
        let a = ((h & 0xFFFF) as f32 / q) - 1.0;
        let b = (((h >> 16) & 0xFFFF) as f32 / q) - 1.0;
        Vec2::new(a, b).normalize_or_zero()
    }

    fn sync_team_tactics(&mut self) {
        self.tactical_set = match self.possession {
            Possession::Home => self.home_offense_tactic,
            Possession::Away => self.away_offense_tactic,
        };
        // round-6 审计修复：防守方案必须成为因果输入。
        //
        // 此前 `home/away_defensive_tactic` 只被用于生成展示字符串，物理与
        // 决策管线从不消费——实测 6 种方案各跑一场全场模拟逐字节相同。
        // 现在把**防守方**的方案经规则通道写入 `rules.tactics.defense`，
        // 由防守目标点生成消费（charter C1/C3：数据通道，非代码分支）。
        //
        // 未知 id 不静默回退：保留上一 tick 的参数并在强制项里登记，
        // 避免又一个「声明了但无效」的隐形参数。
        let defending_side = match self.possession {
            Possession::Home => self.away_defensive_tactic,
            Possession::Away => self.home_defensive_tactic,
        };
        match nba_domain::DefenseRules::for_scheme(defending_side.id()) {
            Some(d) => self.rules.tactics.defense = d,
            None => self
                .current_enforcements
                .push(format!("UNKNOWN_DEFENSE_SCHEME:{}", defending_side.id())),
        }
    }

    /// Re-evaluate coach strategy at every possession boundary.
    fn update_coach_strategy(&mut self) {
        let score_diff = self.home_score as i32 - self.away_score as i32;
        self.coach = CoachStrategy::evaluate(score_diff, self.period, self.game_clock, &self.rules);
    }
    /// 球弹道状态的唯一写入口（BallState Consolidation 的第一步）。
    ///
    /// 所有 `ball_state` 变更都必须经过此方法，保证：
    /// - 归属派生（physics 层的 `has_ball`）随状态同步，不出现"两人持球"；
    /// - 未来可在此插入状态转换合法性校验与领域事件，无需改各调用点。
    ///
    /// `current_t` 用于同步持有/受控状态的持球人参考；非持有状态传当前时间即可。
    /// 标记/清除本 tick 的接球人（round-10）。
    ///
    /// 接球人向球收敛期间需豁免 APF 排斥，并在到位后立即停住
    /// （见 `physics::movement` 的 `is_receiving_pass` 分支）。
    /// 传 `None` 表示清除全部标记。
    fn mark_receiver(&mut self, receiver_id: Option<&str>) {
        let ids: Vec<String> = self.physics.get_players().keys().cloned().collect();
        for id in ids {
            let should = receiver_id == Some(id.as_str());
            if let Some(p) = self.physics.get_player_mut(&id) {
                p.is_receiving_pass = should;
            }
        }
    }

    fn transition_ball_state(&mut self, next: BallTrajectoryKind) {
        // M2：经领域层纯函数转换表校验，非法边被拒绝并记入强制项
        // （architecture.md §3.2 唯一写入口 + §3.3 转换表穷举）。
        let next_transfer_id = match &next {
            BallTrajectoryKind::ControlTransfer { carrier_id, .. } => Some(carrier_id.clone()),
            _ => None,
        };
        let released_transfer_id = match (&self.ball_state, &next) {
            (BallTrajectoryKind::ControlTransfer { carrier_id, .. }, next)
                if !matches!(next, BallTrajectoryKind::ControlTransfer { .. }) =>
            {
                Some(carrier_id.clone())
            }
            _ => None,
        };
        // ## 攻框旗标（round-18）：进入 Drive 置位，离开清除。
        //
        // 唯一写入口统一管理（与接球人标记同一模式）。旗标让物理层的
        // APF 排斥豁免攻框者——对抗交由终结裁决处理，转向墙挡不住攻框。
        let drive_flag: Option<(String, bool)> = match (&self.ball_state, &next) {
            (_, BallTrajectoryKind::Drive { driver_id, .. }) => Some((driver_id.clone(), true)),
            (BallTrajectoryKind::Drive { driver_id, .. }, next)
                if !matches!(next, BallTrajectoryKind::Drive { .. }) =>
            {
                Some((driver_id.clone(), false))
            }
            _ => None,
        };
        match nba_domain::transition_ball_state(&self.ball_state, next) {
            Ok(next) => {
                // ## 接球人标记必须在**写入口**设置（round-10）
                //
                // 物理步进（`step` 内 `physics.step`）发生在战术规划**之前**，
                // 因此若在战术规划里才标记接球人，第一个 tick 的物理仍按旧标志执行，
                // 接球人会带着上一 tick 的速度滑离落点（实测 17.26 ft/s，
                // 一 tick 滑 1.73 ft > catch_radius）。
                //
                // 球态进入 `Pass` 时，接球人的身份已确定（`target_id`），
                // 在唯一写入口立即标记，保证下一 tick 的物理就生效。
                if let BallTrajectoryKind::Pass { target_id, .. } = &next {
                    let rid = target_id.clone();
                    self.mark_receiver(Some(&rid));
                }
                self.ball_state = next;
                self.sync_ball_holder();

                // During the frozen control-transfer flight the receiving body
                // must not move away from the endpoint. Otherwise the state
                // would end with a Held label at a stale ball coordinate.
                if let Some(player_id) = next_transfer_id {
                    if let Some(player) = self.physics.get_player_mut(&player_id) {
                        player.is_locked_kinematics = true;
                        player.vel_ft = Vec2::ZERO;
                        player.accel_ft = Vec2::ZERO;
                        player.target_pos_ft = player.pos_ft;
                        player.target_speed_ftps = 0.0;
                    }
                }
                if let Some(player_id) = released_transfer_id {
                    self.physics.set_player_locked(&player_id, false, None);
                }
                if let Some((player_id, entering)) = drive_flag {
                    if let Some(p) = self.physics.get_player_mut(&player_id) {
                        p.is_driving_to_rim = entering;
                    }
                }
            }
            Err(reason) => {
                self.current_enforcements
                    .push(format!("ILLEGAL_BALL_TRANSITION:{}", reason));
            }
        }
    }

    /// 将 physics 层的逐球员 `has_ball` 与 `ball_state` 的归属对齐。
    /// Held / Drive / ControlTransfer 视为"有明确持球人"，其余状态清空持球标志。
    fn sync_ball_holder(&mut self) {
        let holder: Option<&str> = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.physics.set_ball_holder(holder);

        // F1.3：player.out_of_bounds_placement 是派生态：只有“当前权威球态
        // 处于发球程序”（InboundTransfer/InboundReady）时才允许界外豁免。
        // 一旦球进入其他状态（传球飞行、被断、死球等），发球员必须恢复
        // 普通在场约束，不能永久保持界外豁免（本轮实测 476–1324 条
        // PLAYER_IN_BOUNDS 即由 sticky flag 造成）。
        let exempt_inbounder: Option<String> = match &self.ball_state {
            BallTrajectoryKind::InboundTransfer { inbounder_id, .. }
            | BallTrajectoryKind::InboundReady { inbounder_id, .. } => Some(inbounder_id.clone()),
            _ => None,
        };
        let ids: Vec<String> = self.physics.get_players().keys().cloned().collect();
        let mut placements: Vec<(String, Vec2, Vec2)> = Vec::new();
        for id in ids {
            let should_exempt = exempt_inbounder.as_deref() == Some(id.as_str());
            let mut release_pos = None;
            let mut clear_action = false;
            if let Some(p) = self.physics.get_player_mut(&id) {
                let was_exempt = p.out_of_bounds_placement;
                p.out_of_bounds_placement = should_exempt;
                if was_exempt && !should_exempt {
                    release_pos = Some(p.pos_ft);
                    clear_action = is_inbound_role_action(&p.action);
                }
                // 注：非发球球员在发球程序中被卡界外的几何死锁修复，已移至
                // 物理步进的 InboundReady 分支（每 tick 执行），不在此——本函数
                // 只在球态转换时调用，覆盖不到恒为 InboundReady 的卡死段。
            }
            // 豁免被取消且球员仍在界外时，必须做一次显式离散 placement
            // 把它放回界内，而不是让下一 tick 的物理 clamp 产生
            // PLAYER_SPEED 伪造超速（gap.md §4.3）。
            if let Some(from_pos) = release_pos {
                // 选择界内且不与任何在场球员重叠的落点。若直接放在被
                // 他人占据的边界点上，下一 tick 的分离投影会产生巨大
                // 瞬时修正（本轮 seed 21 实测 PLAYER_SPEED 55–117 ft/s）。
                let to = self.free_in_court_spot(from_pos, &exempt_inbounder);
                if let Some(p) = self.physics.get_player_mut(&id) {
                    // 发球程序结束：清除发球角色动作，否则评估/展示层仍会
                    // 把它当作界外豁免对象。
                    if clear_action {
                        p.action = "SpotUp".to_string();
                    }
                    if (to - from_pos).length() > f32::EPSILON {
                        p.pos_ft = to;
                        p.target_pos_ft = to;
                        p.vel_ft = Vec2::ZERO;
                        p.accel_ft = Vec2::ZERO;
                        placements.push((id.clone(), from_pos, to));
                    }
                }
            }
        }
        for (id, from, to) in placements {
            self.physics.teleport_player(&id, to);
            self.pending_events.push(GameEvent::PlacementApplied {
                player_id: id,
                from: (from.x, from.y),
                to: (to.x, to.y),
                reason: "INBOUND_PROGRAM_EXIT".to_string(),
                phase: self.phase_type().as_str().to_string(),
            });
        }
    }

    pub fn set_game_flow(&mut self, flow: GameFlowState) {
        if self.game_flow == flow {
            return;
        }
        // 当试图进入活球阶段时，执行严格的因果前置图前置条件校验
        if flow == GameFlowState::LiveBall {
            let active_count = self
                .physics
                .get_players()
                .values()
                .filter(|p| p.on_court)
                .count();
            if active_count != 10 {
                eprintln!(
                    "[CAUSAL_DAG] Transition to LiveBall rejected: strictly 10 on-court players required, found {}",
                    active_count
                );
                return;
            }
        }
        self.game_flow = flow;
    }

    fn sync_game_flow(&mut self) {
        if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) && self.break_finished()
        {
            self.period += 1;
            self.game_clock = self.rules.period_duration(self.period);
            self.shot_clock = self.rules.league.shot_clock_seconds;
            self.team_fouls_home = 0;
            self.team_fouls_away = 0;
            self.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::LiveBall);
            self.transition_phase(SubPhase::Initiation);
            self.last_decision_time = -self.rules.decision_interval_seconds;
            self.current_event = Some("PERIOD_START".to_string());
            self.current_callout = Some(format!("第{}节开始", self.period));
        }
    }
    fn transition_phase(&mut self, next: SubPhase) {
        let previous = self.phase_type();
        self.sub_phase = next;
        self.sub_phase_timer = 0.0;
        if next == SubPhase::Initiation {
            self.inbound_elapsed = 0.0;
            self.backcourt_elapsed = 0.0;
        }
        let next_phase = self.phase_type();
        if previous != next_phase {
            self.pending_events.push(GameEvent::PhaseTransition {
                from: previous,
                to: next_phase,
            });
        }
    }
    /// Advance one fixed step and validate the emitted frame.
    ///
    /// The invariant checker runs after every tick (including every early
    /// return inside `step_inner`) so that a physics/basketball violation is
    /// reported with the exact tick that produced it, instead of being
    /// discovered only by post-hoc log analysis.
    pub fn step(&mut self) -> StreamTick {
        let tick = self.step_inner();
        // P-1 观测差分：记录本 tick 的球位置，供接球人下一 tick 估计速度
        // （感知延迟一步，见 ball_velocity_estimate）。
        self.prev_observed_ball_pos = Some(self.ball_pos_3d.0);
        let violations = self.invariant_checker.check_tick(&tick);
        // InvariantChecker owns the protocol-level causal graph; do not run a
        // second independent graph here, which would duplicate findings and
        // make the violation ledger depend on call-site history.
        self.last_tick_violations = violations;
        for v in &self.last_tick_violations {
            eprintln!("[INVARIANT] {}", v);
        }
        tick
    }

    fn step_inner(&mut self) -> StreamTick {
        if self.scope_active && self.simulation_complete {
            self.current_event = None;
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.current_event_log.clear();
            self.last_decision_trace = None;
            return self.build_tick();
        }
        self.tick_index = self.tick_index.saturating_add(1);

        let dt = self.rules.tick_seconds;
        let was_tip_off = self.game_flow == GameFlowState::TipOff;
        let was_period_break = matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        );
        self.sync_game_flow();
        self.current_event_log.clear();
        // D4.1：因果槽位跨 tick 存活（动作释放与落地结果常不同 tick：
        // 实测 PASS@t24 → PASS_RECEIVED@t29、FOUL@t84 → FREE_THROW@t86），
        // 因此不在每 tick 清空；槽位在对应动作窗口关闭/被新触发覆盖时
        // 自然失效，保证父指向最近的同槽位触发事件。
        self.sync_team_tactics();

        // Tip-off is a configurable dead-ball presentation phase. A zero
        // duration transitions in this same fixed step so the default policy
        // retains the historical first-tick behavior.
        if was_tip_off {
            self.current_time += dt;
            self.sub_phase_timer += dt;
            if self.rules.tip_off_duration_seconds > 0.0 {
                if self.sub_phase_timer + f32::EPSILON < self.rules.tip_off_duration_seconds {
                    self.publish_events();
                    self.physics.reset_motion();
                    let center_x = self.rules.court.width_ft * 0.5;
                    let center_y = self.rules.court.height_ft * 0.5;
                    let total_time = self.rules.tip_off_duration_seconds;
                    let progress = (self.sub_phase_timer / total_time).clamp(0.0, 1.0);
                    let peak_z = 11.5;
                    let base_z = 4.0;
                    let z = base_z + 4.0 * (peak_z - base_z) * progress * (1.0 - progress);
                    self.ball_pos_3d = (Vec2::new(center_x, center_y), z);
                    if self.sub_phase_timer <= dt + f32::EPSILON {
                        self.current_event = Some("TIPOFF".to_string());
                        self.current_event_types = vec!["TIPOFF".to_string()];
                    } else {
                        self.current_event = None;
                        self.current_event_types.clear();
                    }
                    self.current_callout = Some("裁判中圈垂直抛球，双方中锋起跳争顶！".to_string());
                    self.current_enforcements.clear();
                    self.last_decision_trace = None;
                    return self.build_tick();
                }
                // 严格遵循宪章 Positionless 原则：跳球代表绝无固定位置硬编码，
                // 由场上摸高上限最高的球员（身高 height_cm + 垂直弹跳 vertical）纯函数涌现产生
                let home_jumper_id = self.select_jumper_id(Possession::Home);
                let away_jumper_id = self.select_jumper_id(Possession::Away);
                let winner_is_home = self.possession == Possession::Home;
                let tapping_player = if winner_is_home {
                    &home_jumper_id
                } else {
                    &away_jumper_id
                };
                let center_x = self.rules.court.width_ft * 0.5;
                let center_y = self.rules.court.height_ft * 0.5;
                let tap_target = if winner_is_home {
                    Vec2::new(center_x - 14.0, center_y)
                } else {
                    Vec2::new(center_x + 14.0, center_y)
                };
                self.set_game_flow(GameFlowState::LiveBall);
                if let Some(p) = self.physics.get_player_mut(&home_jumper_id) {
                    p.target_pos_ft = Vec2::new(center_x - 3.0, center_y);
                }
                if let Some(p) = self.physics.get_player_mut(&away_jumper_id) {
                    p.target_pos_ft = Vec2::new(center_x + 3.0, center_y);
                }
                let tap_dir = (tap_target - Vec2::new(center_x, center_y)).normalize();
                self.transition_ball_state(BallTrajectoryKind::LooseBall {
                    pos: Vec2::new(center_x, center_y),
                    vel: tap_dir * 28.0,
                    z: 5.5,
                    vel_z: 6.0,
                    last_touch_team: if winner_is_home {
                        Possession::Home
                    } else {
                        Possession::Away
                    },
                });
                self.current_event_types = vec!["TIPOFF_SECURED".to_string()];
                self.current_callout = Some(format!(
                    "{} 起跳率先触球，将球点拍向后场！第一攻展开！",
                    tapping_player
                ));
                return self.build_tick();
            }
            self.set_game_flow(GameFlowState::LiveBall);
            self.transition_phase(SubPhase::Initiation);
            self.current_callout = Some("比赛开始，跳球后主队获得第一攻球权。".to_string());
        }
        if was_period_break
            && !matches!(
                self.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            )
        {
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            self.publish_events();
            return self.build_tick();
        }
        if matches!(
            self.game_flow,
            GameFlowState::Timeout | GameFlowState::GameEnd
        ) {
            self.current_event = None;
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            return self.build_tick();
        }
        if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime
        ) {
            self.period_break_elapsed += dt;
            self.sync_game_flow();
            if matches!(
                self.game_flow,
                GameFlowState::QuarterEnd | GameFlowState::Halftime
            ) {
                self.current_event = None;
                self.current_event_types.clear();
                self.current_enforcements.clear();
                self.last_decision_trace = None;
                return self.build_tick();
            }
            self.current_event_types.clear();
            self.current_enforcements.clear();
            self.last_decision_trace = None;
            self.publish_events();
            return self.build_tick();
        }

        if !was_tip_off {
            self.current_time += dt;
            self.sub_phase_timer += dt;
        }
        if self.sub_phase == SubPhase::Initiation
            && self.game_flow == GameFlowState::DeadBall
            && matches!(self.ball_state, BallTrajectoryKind::InboundReady { .. })
        {
            self.inbound_elapsed += dt;
        } else if self.sub_phase != SubPhase::DeadBallReset {
            self.inbound_elapsed = 0.0;
        }
        if self.game_flow.allows_live_ball_actions() {
            // F1.2：八秒违例是“连续停留在后场”的计时。进攻方一旦持球越过
            // 中线，后场计时必须清零并从 0 重新计；否则球已到前场仍会
            // 累积到阈值并无条件判 EIGHT_SECOND_BACKCOURT（gap.md §4.1）。
            // 前场判定复用领域层 is_backcourt 的同一几何定义。
            let attacking_right = self.possession == Possession::Home;
            let in_backcourt = self
                .rules
                .court
                .is_backcourt(self.ball_pos_3d.0, attacking_right);
            if in_backcourt {
                self.backcourt_elapsed += dt;
            } else {
                // 越过中线：推进义务完成，恢复常规战术落位。
                self.backcourt_elapsed = 0.0;
                self.advancing_player = None;
            }
            if self.game_clock > 0.0 {
                self.game_clock = (self.game_clock - dt).max(0.0);
            }
            if !matches!(
                self.ball_state,
                BallTrajectoryKind::Shot { .. } | BallTrajectoryKind::RimRebound { .. }
            ) && self.shot_clock > 0.0
            {
                self.shot_clock = (self.shot_clock - dt).max(0.0);
            }
        }
        self.period_break_elapsed = 0.0;
        let current_t = self.current_time;
        let is_home = self.possession == Possession::Home;
        self.current_event = None;
        self.current_event_types.clear();
        self.current_enforcements.clear();
        self.last_decision_trace = None;

        // 贴身切球（on-ball poke check）：为「带球丢球」提供事实路径。
        //
        // 真实 NBA 失误构成中带球丢球占 53.6%（82games 2024-25 IND），
        // 是占比最大的一类；此前引擎只有传球失败一条失误路径。
        //
        // 只在活球且球确实被持有时评估；`resolve_on_ball_poke` 内部按
        // `rate × dt` 做时间积分，单 tick 概率不随时长累加。
        if self.game_flow.allows_live_ball_actions() {
            let carrier = self.carrier_id();
            let locked = self
                .physics
                .get_player(&carrier)
                .map(|p| p.is_locked_kinematics)
                .unwrap_or(false);
            if let Some(defender_id) = self.resolve_on_ball_poke(&carrier, dt, locked) {
                self.apply_on_ball_poke(&carrier, &defender_id);
            }
        }

        // Runtime constraints observe the current snapshot before execution.
        let ctx = self.constraint_ctx();
        let runtime_findings = self.decision.registry.evaluate_runtime(&ctx);
        let runtime_violation = if matches!(
            self.game_flow,
            GameFlowState::QuarterEnd | GameFlowState::Halftime | GameFlowState::GameEnd
        ) {
            None
        } else {
            runtime_findings.iter().find_map(|finding| {
                if let EnforcementAction::Violation { kind } = finding.enforcement {
                    Some((finding.constraint.id, kind))
                } else {
                    None
                }
            })
        };
        if let Some((constraint_id, kind)) = runtime_violation {
            self.current_enforcements
                .push(format!("{}:{}", constraint_id, kind.as_str()));
            self.pending_events.push(GameEvent::RuleViolation {
                constraint_id: constraint_id.to_string(),
                reason: kind.as_str().to_string(),
            });
            self.current_event = Some("VIOLATION".to_string());
            let violation_callout =
                format!("{}！{} 失去球权", constraint_id, team_name_zh(is_home));
            self.start_violation_turnover(kind);
            // 确保违例判定帧忠实呈现哨响违例事实，不被随后的发球准备覆写
            self.current_callout = Some(violation_callout);
            self.current_event = Some("VIOLATION".to_string());
            self.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return self.build_tick();
        }
        // Advance action windows in stable player-id order.
        if !self.active_windows.is_empty() {
            let mut window_ids: Vec<String> = self.active_windows.keys().cloned().collect();
            window_ids.sort();
            let mut window_events = Vec::new();
            for window_id in window_ids {
                let Some(window) = self.active_windows.get_mut(&window_id) else {
                    continue;
                };
                let previous = window.phase;
                let current = window.update(current_t);
                self.physics.set_player_locked(
                    &window.player_id,
                    window.lock_kinematics && !window.is_finished(current_t),
                    None,
                );
                if current != previous {
                    window_events.push(GameEvent::WindowTransition {
                        player_id: window.player_id.clone(),
                        action_type: window.action_type,
                        new_phase: current,
                    });
                }
            }
            self.active_windows.retain(|pid, window| {
                let finished = window.is_finished(current_t);
                if finished {
                    if let Some(p) = self.physics.get_player_mut(pid) {
                        if p.action.ends_with("Shot")
                            || p.action == "Layup"
                            || p.action == "Dunk"
                            || p.action == "Floater"
                            || p.action == "ScreenSet"
                        {
                            p.action = "Recover".to_string();
                        }
                    }
                }
                !finished
            });
            self.pending_events.extend(window_events);
        }
        let facts = self.physics.drain_facts();
        if !facts.is_empty() {
            self.pending_events
                .extend(facts.into_iter().map(physics_fact_to_event));
        }
        if self.free_throws_remaining > 0
            && self.game_flow == GameFlowState::FreeThrow
            && self.sub_phase_timer >= self.rules.free_throw_interval_seconds
        {
            self.resolve_free_throw();
            self.physics.step(nba_domain::FixedDt(dt));
            self.publish_events();
            return self.build_tick();
        }
        if self.rules.period_expired(self.game_clock)
            && matches!(
                self.game_flow,
                GameFlowState::LiveBall | GameFlowState::Overtime | GameFlowState::TipOff
            )
        {
            let ball_live = matches!(
                self.ball_state,
                BallTrajectoryKind::Shot { .. } | BallTrajectoryKind::RimRebound { .. }
            );
            if !ball_live {
                self.finish_period();
                self.physics.step(nba_domain::FixedDt(dt));
                self.publish_events();
                return self.build_tick();
            }
        }
        if self.game_flow == GameFlowState::GameEnd {
            self.publish_events();
            return self.build_tick();
        }

        // Phase state machine: inbound decisions are evaluated only during the
        // inbound phase; live-ball decisions use the same registry pipeline.
        let mut decision_output: Option<DecisionOutput> = None;
        match self.sub_phase {
            SubPhase::Initiation => {
                if self.game_flow == GameFlowState::DeadBall {
                    // 发球阶段使用专用（更短）决策间隔：发球受 5 秒规则约束，
                    // 套用阵地节奏会与之竞速（实测 37% 发球被判五秒违例）。
                    if matches!(self.ball_state, BallTrajectoryKind::InboundReady { .. })
                        && current_t - self.last_decision_time
                            >= self.rules.inbound_decision_interval_seconds
                    {
                        let carrier = self.carrier_id();
                        let stamina = self
                            .physics
                            .get_player(&carrier)
                            .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                            .unwrap_or(1.0);
                        let morale_bias = self.morale_bias_for(&carrier);
                        let mut rng =
                            std::mem::replace(&mut self.rng, ChaCha8Rng::seed_from_u64(0));
                        let ctx = self.constraint_ctx();
                        decision_output = self.decision.decide_on_ball(
                            &ctx,
                            &carrier,
                            stamina,
                            morale_bias,
                            &self.coach,
                            &mut rng,
                        );
                        self.rng = rng;
                    }
                } else {
                    // 第一性原理：后场推进不受「战术发起」延迟约束。
                    //
                    // 8 秒规则要求进攻方在 8 秒内把球推过中线，而
                    // `tactical_initiation_seconds = 6.5s` 会让球队在后场
                    // 干等到 6.5s 才首次决策，只剩 1.5s 窗口（决策间隔
                    // 2.4s）—— 实测因此产生 31 次/场 8 秒违例，球 x 在
                    // 8 秒内只从 11.4 移到 12.8 ft（需越过 47）。
                    //
                    // 半场阵地进攻才需要「战术发起」等待；后场是转换推进，
                    // 必须立即允许决策（`Advance` 候选随即可用）。
                    let in_backcourt = {
                        let midcourt = self.rules.court.width_ft / 2.0;
                        match self.possession {
                            Possession::Home => self.ball_pos_3d.0.x < midcourt,
                            Possession::Away => self.ball_pos_3d.0.x > midcourt,
                        }
                    };
                    if in_backcourt
                        || self.sub_phase_timer >= self.rules.tactical_initiation_seconds
                    {
                        self.transition_phase(SubPhase::ActionExecution);
                        self.set_game_flow(GameFlowState::LiveBall);
                        self.current_event = Some("TACTICAL_EXECUTION".to_string());
                        self.current_callout =
                            Some(format!("战术发起：{}", self.tactical_set.name_zh()));
                    }
                }
            }
            SubPhase::ActionExecution => {
                if current_t - self.last_decision_time >= self.rules.decision_interval_seconds {
                    let carrier = self.carrier_id();
                    let stamina = self
                        .physics
                        .get_player(&carrier)
                        .map(|p| (p.stamina / p.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
                        .unwrap_or(1.0);
                    let morale_bias = self.morale_bias_for(&carrier);
                    let mut rng = std::mem::replace(&mut self.rng, ChaCha8Rng::seed_from_u64(0));
                    let ctx = self.constraint_ctx();
                    decision_output = self.decision.decide_on_ball(
                        &ctx,
                        &carrier,
                        stamina,
                        morale_bias,
                        &self.coach,
                        &mut rng,
                    );
                    self.rng = rng;
                }
            }
            SubPhase::ShotAttempt | SubPhase::FlightAndRebound | SubPhase::DeadBallReset => {}
        }

        // ============================================================
        // 2. 执行决策输出（意图执行重校验，architecture.md §5.2）
        if let Some(out) = decision_output {
            let ctx = self.constraint_ctx();
            if let Err(blocked_reason) = self.decision.registry.revalidate_intent(&ctx, &out.action)
            {
                self.current_enforcements
                    .push(format!("INTENT_REVALIDATION_BLOCKED:{}", blocked_reason));
                // 硬约束校验失败，拒绝执行该动作，保持 Dwell 保护；
                // trace 仍必须完整发布（architecture.md §5.3：禁止"决策了但没有
                // trace"的路径），供评判器统计"落地改变"率。
                let mut debug = convert_trace(&out.trace);
                debug
                    .enforcement
                    .extend(self.current_enforcements.iter().cloned());
                self.last_decision_trace = Some(Box::new(debug));
            } else {
                let trace = out.trace.clone();
                match out.action {
                    CandidateAction::Shoot {
                        shooter_id,
                        from_pos,
                        is_three,
                        jumper_kind,
                    } => {
                        self.execute_shot(&shooter_id, from_pos, is_three, jumper_kind, current_t);
                    }
                    CandidateAction::Drive {
                        driver_id,
                        from_pos,
                        target_pos,
                        move_kind,
                    } => {
                        self.execute_drive(&driver_id, from_pos, target_pos, move_kind, current_t);
                    }
                    CandidateAction::Pass {
                        passer_id,
                        receiver_id,
                        from_pos,
                        to_pos,
                    } => {
                        self.execute_pass(
                            &passer_id,
                            &receiver_id,
                            from_pos,
                            to_pos,
                            current_t,
                            false,
                        );
                    }
                    CandidateAction::InboundPass {
                        passer_id,
                        receiver_id,
                        from_pos,
                        to_pos,
                    } => {
                        self.execute_pass(
                            &passer_id,
                            &receiver_id,
                            from_pos,
                            to_pos,
                            current_t,
                            true,
                        );
                    }
                    CandidateAction::Dwell { .. } => {
                        // 观察等待：无操作。
                    }
                    CandidateAction::Advance {
                        player_id,
                        target_pos,
                        ..
                    } => {
                        // 推进过半场：以运球速度把持球人朝中线方向驱动。
                        let target = self
                            .rules
                            .court
                            .clamp_playable(target_pos, self.rules.player_radius_ft);
                        let morale = self
                            .physics
                            .get_player(&player_id)
                            .map(|p| p.morale.clone())
                            .unwrap_or_else(|| "Normal".to_string());
                        self.physics.set_player_target(
                            &player_id,
                            target,
                            self.rules.max_player_speed_ftps
                                * self.rules.tactics.carrier_speed_ratio,
                            "Advance",
                            "BallHandler",
                            &morale,
                        );
                        if let Some(p) = self.physics.get_player_mut(&player_id) {
                            p.action = "ADVANCE".to_string();
                        }
                        self.advancing_player = Some(player_id.clone());
                        self.current_callout = Some("持球推进，尽快越过中线！".to_string());
                        self.current_event = Some("ADVANCE".to_string());
                        self.last_decision_time = current_t;
                    }
                    CandidateAction::PostUp {
                        player_id,
                        target_pos,
                        ..
                    } => {
                        let target = self
                            .rules
                            .court
                            .clamp_playable(target_pos, self.rules.player_radius_ft);
                        let morale = self
                            .physics
                            .get_player(&player_id)
                            .map(|p| p.morale.clone())
                            .unwrap_or_else(|| "Normal".to_string());
                        self.physics.set_player_target(
                            &player_id,
                            target,
                            4.0,
                            "PostUp",
                            "PostPlayer",
                            &morale,
                        );
                        let hoop = self
                            .rules
                            .court
                            .hoop_pos(self.possession == Possession::Home);
                        if let Some(p) = self.physics.get_player_mut(&player_id) {
                            p.action = "PostUp".to_string();
                            // Facing opposite to hoop (backdown orientation)
                            let away_from_hoop = (p.pos_ft - hoop).normalize_or_zero();
                            if away_from_hoop.length_squared() > 0.1 {
                                p.facing_dir = away_from_hoop;
                            }
                        }
                        let player_name = self
                            .physics
                            .get_player(&player_id)
                            .map(|p| format!("{}号", p.jersey))
                            .unwrap_or_else(|| player_id.clone());
                        self.current_callout =
                            Some(format!("{} 低位背身单打，发力推推挤要位！", player_name));
                        self.current_event = Some("POST_UP".to_string());
                    }
                    CandidateAction::TripleThreatJab {
                        player_id,
                        pivot_pos: _,
                        jab_dir,
                    } => {
                        if let Some(p) = self.physics.get_player_mut(&player_id) {
                            p.facing_dir = jab_dir;
                            p.action = "TripleThreat".to_string();
                        }
                        let player_name = self
                            .physics
                            .get_player(&player_id)
                            .map(|p| format!("{}号", p.jersey))
                            .unwrap_or_else(|| player_id.clone());
                        self.current_callout = Some(format!(
                            "{} 持球三威胁试探步，压低重心观察防守！",
                            player_name
                        ));
                        self.current_event = Some("TRIPLE_THREAT_JAB".to_string());
                    }
                }
                self.last_decision_time = current_t;
                let mut debug = convert_trace(&trace);
                debug
                    .enforcement
                    .extend(self.current_enforcements.iter().cloned());
                self.last_decision_trace = Some(Box::new(debug));
            }
        }
        // Step the physical world before sampling the ball at this tick.
        self.physics.step(nba_domain::FixedDt(dt));
        let sample_3d = BallisticsEngine::sample_ball_position(
            &self.ball_state,
            current_t,
            self.physics.get_players(),
            &self.rules,
        );
        self.ball_pos_3d = sample_3d;

        // ============================================================
        // 3. 球弹道状态更新与拦截检查
        let mut new_ball_state = None;
        let mut steal_triggered_defender = None;
        let mut loose_ball_secured_player: Option<(String, Vec2, f32)> = None;
        let mut live_ball_triggered = false;
        let mut loose_ball_out_of_bounds: Option<(Vec2, (Vec2, f32))> = None;
        let mut drive_kickout_action = None;
        let mut drive_pullup_action = None;
        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id } => {
                let cid = carrier_id.clone();
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
                if self.physics.get_player(&cid).is_none() {
                    self.ball_pos_3d = (
                        Vec2::new(
                            self.rules.court.width_ft / 2.0,
                            self.rules.court.height_ft / 2.0,
                        ),
                        self.rules.ball_holder_height_ft,
                    );
                }
            }
            BallTrajectoryKind::ControlTransfer {
                carrier_id,
                target_pos,
                start_time,
                duration,
                ..
            } => {
                let cid = carrier_id.clone();
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
                let receiver_ready = self
                    .physics
                    .get_player(&cid)
                    .map(|player| {
                        (player.pos_ft - *target_pos).length()
                            <= self.rules.invariant_holder_leash_ft
                    })
                    .unwrap_or(false);
                // D5.1（本轮实测）：飞行早已结束、但接球人被动作窗口锁定
                // （`lock_kinematics`）而永远走不到冻结点时，交接会永久
                // 悬置——seed 6 实测 69,466 帧（约 2,780 秒）活锁，全场仅
                // 11 个回合。交接是「球到人」的事实：飞行时长已满即视为
                // 到达，球收敛到接球人的实际位置，不再要求人体额外位移
                // （球员运动学由 physics 独占，引擎不得瞬移球员）。
                let flight_done = current_t - *start_time >= *duration;
                if !receiver_ready && flight_done {
                    if let Some(player) = self.physics.get_player(&cid) {
                        let offset = player.pos_ft - *target_pos;
                        let leash = self.rules.invariant_holder_leash_ft;
                        // 球落在冻结点与接球人之间，距接球人不超过 leash，
                        // 保证 `Held` 状态下 BALL_WITH_HOLDER 成立。
                        let landing = if offset.length() > leash {
                            player.pos_ft
                                - offset.normalize_or_zero()
                                    * leash
                                    * self.rules.transfer_landing_leash_ratio
                        } else {
                            *target_pos
                        };
                        self.ball_pos_3d = (landing, self.rules.ball_holder_height_ft);
                    }
                }
                if flight_done
                    && (receiver_ready
                        || self.ball_pos_3d.0.distance(
                            self.physics
                                .get_player(&cid)
                                .map(|p| p.pos_ft)
                                .unwrap_or(*target_pos),
                        ) <= self.rules.invariant_holder_leash_ft)
                {
                    if self.pending_pass_receiver.as_deref() == Some(cid.as_str()) {
                        // round-6：与 Pass 分支同口径——接球事实的位置必须是球
                        // 当前所处的位置（此处已经过 ControlTransfer 收敛，保证在
                        // 接球人 leash 内），而**不是**冻结点。
                        //
                        // 两个分支的语义分工：
                        // - `Pass` 分支：球已到冻结点，接球人恰好处于 leash 内
                        //   → `position = to_pos`（终点事实，见上）；
                        // - `ControlTransfer` 分支：接球人未到位，球已收敛到他身上
                        //   → `position = 球的实际位置`（接球事实）。
                        // 评判器因此可以区分「终点」与「接球」，不再需要第三个解释
                        // （gap.md §9.5）。
                        self.pending_events.push(GameEvent::PassReceived {
                            receiver_id: cid.clone(),
                            position: (self.ball_pos_3d.0.x, self.ball_pos_3d.0.y),
                        });
                        self.pending_pass_receiver = None;
                        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
                        self.receiver_estimate = None;
                        self.last_passer_id = None;
                        if self.pending_pass_inbound {
                            self.transition_phase(SubPhase::Initiation);
                            self.set_game_flow(GameFlowState::LiveBall);
                            self.sub_phase_timer = 0.0;
                            self.last_decision_time = -self.rules.decision_interval_seconds;
                            self.pending_pass_inbound = false;
                        }
                    }
                    new_ball_state = Some(BallTrajectoryKind::Held { carrier_id: cid });
                }
            }
            BallTrajectoryKind::InboundTransfer {
                baseline_pos,
                inbounder_id,
                start_time,
                duration,
                ..
            } => {
                // 防御性恢复：发球员必须是在场球员。若因换人/犯满/旧状态
                // 使其离场，必须重新指派一名在场球员继续发球程序，否则
                // `inbounder_arrived` 永不成立，比赛卡死在 DeadBall
                // （本轮 seed 3/15/26 实测：inbounder action=Bench）。
                let inbounder_valid = self
                    .physics
                    .get_player(inbounder_id)
                    .map(|p| p.on_court)
                    .unwrap_or(false);
                if !inbounder_valid {
                    let replacement = self.new_possession_pg();
                    if replacement != *inbounder_id
                        && self
                            .physics
                            .get_player(&replacement)
                            .map(|p| p.on_court)
                            .unwrap_or(false)
                    {
                        if let Some(old) = self.physics.get_player_mut(inbounder_id) {
                            old.out_of_bounds_placement = false;
                        }
                        if let Some(player) = self.physics.get_player_mut(&replacement) {
                            player.target_pos_ft = *baseline_pos;
                            player.action = "InboundPositioning".to_string();
                            player.out_of_bounds_placement = true;
                        }
                        self.pending_events.push(GameEvent::PlacementApplied {
                            player_id: replacement.clone(),
                            from: (baseline_pos.x, baseline_pos.y),
                            to: (baseline_pos.x, baseline_pos.y),
                            reason: "INBOUNDER_REASSIGNED".to_string(),
                            phase: self.phase_type().as_str().to_string(),
                        });
                        new_ball_state = Some(BallTrajectoryKind::InboundTransfer {
                            from_pos: self.ball_pos_3d.0,
                            from_z: self.ball_pos_3d.1,
                            baseline_pos: *baseline_pos,
                            inbounder_id: replacement,
                            start_time: current_t,
                            duration: *duration,
                        });
                    }
                } else {
                    let ball_arrived = current_t - start_time >= *duration;
                    let inbounder_dist = self
                        .physics
                        .get_player(inbounder_id)
                        .map(|p| (p.pos_ft - *baseline_pos).length())
                        .unwrap_or(0.0);
                    let inbounder_arrived =
                        inbounder_dist <= self.rules.inbound_boundary_tolerance_ft;
                    if ball_arrived && inbounder_arrived {
                        let inb_pos = self
                            .physics
                            .get_player(inbounder_id)
                            .map(|p| p.pos_ft)
                            .unwrap_or(*baseline_pos);
                        self.ball_pos_3d = (inb_pos, self.rules.chest_height_ft);
                        new_ball_state = Some(BallTrajectoryKind::InboundReady {
                            baseline_pos: inb_pos,
                            inbounder_id: inbounder_id.clone(),
                        });
                    }
                }
            }
            BallTrajectoryKind::InboundReady {
                baseline_pos,
                inbounder_id,
            } => {
                // 球随发球员移动保持在胸高位置
                if let Some(inbounder) = self.physics.get_player(inbounder_id) {
                    self.ball_pos_3d = (inbounder.pos_ft, self.rules.chest_height_ft);
                } else {
                    self.ball_pos_3d = (*baseline_pos, self.rules.chest_height_ft);
                }
            }

            BallTrajectoryKind::Pass {
                target_id,
                start_time,
                duration,
                from_pos,
                to_pos,
                inbound,
                receive_success,
                intercept: intercept_fact,
                ..
            } => {
                let _duration_val = *duration;
                let is_inbound_pass = *inbound;
                let will_receive = *receive_success;
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                let def_team = if is_home { "away" } else { "home" };
                let segment = *to_pos - *from_pos;
                let _ = def_team;
                // 第一性原理修复（本轮）：概率语义与发生时机分离。
                //
                // - **概率**在释放时刻裁定一次（`resolve_pass_interception`），
                //   回答「这次传球是否被拦截、由谁」；
                // - **时机**仍由逐 tick 几何决定：只有当球在空间上真正飞到
                //   该防守者的可及范围时才结算。
                //
                // 这样既消除了「逐 tick 独立掷骰导致概率随时长累积」的错误
                // （实测每次传球失败 35.8%，真实约 8–10%），又保证拦截发生
                // 在物理上合理的位置（否则抢断会在传球起点触发，球权瞬间
                // 转给远处防守者，造成 BALL_WITH_HOLDER 分离 23.83 ft）。
                let mut intercept: Option<(String, bool)> = None;
                if let Some((defender_id, is_steal)) = intercept_fact.clone() {
                    if let Some(defender) = self.physics.get_player(&defender_id) {
                        let reach = self.rules.player_radius_ft + self.rules.defender_reach_ft;
                        let dist_to_ball = (defender.pos_ft - self.ball_pos_3d.0).length();
                        // 球在可及范围内，或已飞过该防守者所在位置（投影已过），
                        // 则视为拦截成立；否则继续飞行。
                        // 拦截只在「球已飞到防守者的拦截点」时结算：
                        // 否则抢断会在释放当 tick 触发，而球仍在传球人手中，
                        // 球权却已转给防守者（实测 BALL_WITH_HOLDER 3.74 ft）。
                        let projection_t = ((defender.pos_ft - *from_pos).dot(segment)
                            / segment.length_squared().max(f32::EPSILON))
                        .clamp(0.0, 1.0);
                        let _intercept_point = *from_pos + segment * projection_t;
                        let travelled = (self.ball_pos_3d.0 - *from_pos).dot(segment)
                            / segment.length_squared().max(f32::EPSILON);
                        // 球必须已到达（或越过）该防守者的拦截点。
                        let reached = travelled >= projection_t - f32::EPSILON;
                        if reached && dist_to_ball <= reach {
                            intercept = Some((defender_id, is_steal));
                        }
                    } else {
                        // 防守者已离场：不结算拦截，让传球正常完成。
                    }
                }

                let passer_id = self.last_passer_id.clone().unwrap_or_default();
                // Fix（round-15 第一性原理）：接球是「首次触球」事件，不是
                // 「飞行终点」事件。
                //
                // 旧实现只在 tau≥1 时裁决。实测（22 例层 A 失败）：接球人的
                // 估计模型以自身为参照系（`initial = ball + dir×|ball−我|`），
                // 他朝来球走 → 距离收缩 → 估计点随之后退（追赶曲线）→
                // 接球人不断走向传球人一侧，球却在冻结终点落地。最终
                // 估计误差 p50=3.37 ft、接球人正确到达自己的估计（距估计
                // 1.34 ft）、球距 3.64 ft —— 「朝来球移动」这一正确篮球行为
                // 反而必然导致终点 miss。
                //
                // 真实篮球里接球发生在第一次触球。这里把到达裁决的触发条件
                // 从「飞行结束」放宽为「飞行结束 **或** 接球人已进入接球半径」；
                // 裁决逻辑本身（层 A 距离 + 层 B 概率 + 状态转移）完全复用，
                // 不新增路径。
                //
                // 高度安全性：传球全程为胸高平飞（`ballistics.rs` Pass 分支，
                // `pass_peak_ft == chest_height_ft == 4.0`，弧项为 0），
                // 不存在“空中高处被接住”的物理问题。
                // 拦截优先级不变：本 tick 若有拦截事实，仍先走拦截分支。
                let receiver_touch = self
                    .physics
                    .get_player(target_id)
                    .map(|r| {
                        let radius = nba_domain::effective_catch_radius(&self.rules, &r.attributes);
                        (r.pos_ft - self.ball_pos_3d.0).length() <= radius
                    })
                    .unwrap_or(false);
                if let Some((defender_id, secured)) = intercept {
                    let position = self.ball_pos_3d.0;
                    if secured {
                        self.pending_events.push(GameEvent::PassIntercepted {
                            passer_id,
                            receiver_id: target_id.clone(),
                            defender_id: defender_id.clone(),
                            position: (position.x, position.y),
                        });
                        // 拦截成立时把球锚定到抢断者身上：`Held{stealer}`
                        // 要求球与持球人一致（否则 BALL_WITH_HOLDER）。
                        // 拦截点与抢断者的距离可能达 reach（1.8+ 可达 ft），
                        // 直接沿用球坐标会造成球人分离（实测 3.86 ft）。
                        if let Some(defender) = self.physics.get_player(&defender_id) {
                            self.ball_pos_3d = (defender.pos_ft, self.rules.ball_holder_height_ft);
                        }
                        steal_triggered_defender = Some(defender_id);
                    } else {
                        self.pending_events.push(GameEvent::PassTipped {
                            passer_id,
                            receiver_id: target_id.clone(),
                            defender_id,
                            position: (position.x, position.y),
                        });
                        if is_inbound_pass {
                            live_ball_triggered = true;
                        }
                        self.pending_loose_ball_terminal =
                            Some(nba_domain::PossessionEndCause::TurnoverPassTipped);
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
                            pos: position,
                            // 同上：点掉的松球初速也须收敛到球速上限。
                            vel: {
                                let dir = segment.normalize_or_zero();
                                let cap = (self.rules.ball_max_speed_ftps
                                    - self.rules.invariant_speed_tolerance_ftps)
                                    .max(self.rules.invariant_speed_tolerance_ftps);
                                dir * cap
                            },
                            z: self.ball_pos_3d.1,
                            vel_z: 0.0,
                            last_touch_team: self.possession,
                        });
                    }
                } else if tau >= 1.0 || receiver_touch {
                    let receiver_id = target_id.clone();
                    {
                        // 层 A（P-1）：球到达时判定**实际空间接近度**。
                        //
                        // 旧口径是「接球人距**冻结点** ≤ leash」—— 那等于用
                        // 传球人的意图当判据，接球人只要站在他该在的地方就算成功，
                        // 与他是否真在球旁边无关。实测因此产生 9 条
                        // `PASS_CORRIDOR_REACHABLE` Hard。
                        //
                        // 新口径（层 A）：用**球与接球人的实际距离**对比
                        // `effective_catch_radius`。
                        //
                        // ## 层序修正（round-10）
                        //
                        // 旧实现把层 B（`will_receive`，release 时的**位置无关**
                        // 概率掷骰）放在**外层**，层 A 放内层：掷到 false 就直接
                        // 判掉球，层 A 连执行机会都没有。后果（实测）：接球人
                        // 站在球旁边（甚至 0 ft）也会"接不到"；233 次 drop 中
                        // 球**全都精确到达冻结落点**（d=0.00），而接球人距球
                        // 中位 6.37 ft —— 但这是层 B 先否决后才产生的位移，不是原因。
                        //
                        // 正确的因果顺序（真实篮球）：
                        //   位置决定**能否到达球**（层 A，确定性）
                        //   → 技术/干扰决定**接得稳不稳**（层 B，概率）
                        // 因此层 A 必须在**外层**：不可达则直接 loose ball，
                        // 层 B 只在可达时生效。
                        let catch_radius = self
                            .physics
                            .get_player(&receiver_id)
                            .map(|r| nba_domain::effective_catch_radius(&self.rules, &r.attributes))
                            .unwrap_or(self.rules.player_radius_ft);
                        let ball_to_receiver = self
                            .physics
                            .get_player(&receiver_id)
                            .map(|r| (r.pos_ft - self.ball_pos_3d.0).length())
                            .unwrap_or(f32::MAX);
                        // 层 A：物理可达（确定性）；层 B：接稳（概率）。
                        let receiver_ready = ball_to_receiver <= catch_radius && will_receive;
                        if receiver_ready {
                            // 接球事实的位置必须是**冻结的传球终点**，而不是接球人
                            // 当时的身体位置。
                            // round-6：`PassReceived.position` 应携带**球的实际到达位置**，
                            // 而不是冻结点 `to_pos`。
                            //
                            // round-10（层 A）更正：层 A 判定改用「球与接球人的**实际**
                            // 距离 ≤ catch_radius」后，接球成功时球可能距冻结点数英尺
                            // （因为接球人按自己的估计跑位）。若仍把球瞬移到 `to_pos`，
                            // 会产生两个错误：
                            //   (a) 球的飞行终点被暴改为一个它从未到达的位置（伪造事实）；
                            //   (b) `Held` 要求 `BALL_WITH_HOLDER` ≤ leash，而接球人可能
                            //       距 `to_pos` 超过 leash —— 实测 seed 5 tick 14338
                            //       报 `BALL_WITH_HOLDER: ball 3.07 ft from holder`。
                            //
                            // 正确做法：接球成功时把球**收到接球人身上**（持球锚点），
                            // 位置即接球人当前位置——这才是物理事实，也天然满足 leash。
                            let catch_spot = self
                                .physics
                                .get_player(&receiver_id)
                                .map(|r| r.pos_ft)
                                .unwrap_or(*to_pos);
                            self.ball_pos_3d = (catch_spot, self.rules.ball_holder_height_ft);
                            // 发布落点修正事实（层 A，P-1）：当实际到达位置与
                            // 传球人冻结的意图不同时，把差异登记为事实，使
                            // 事实账本自洽（不允许下游各自解释同一传球）。
                            let divergence = (catch_spot - *to_pos).length();
                            if divergence > f32::EPSILON {
                                self.pending_events.push(GameEvent::PassLandingCorrected {
                                    receiver_id: receiver_id.clone(),
                                    intended: (to_pos.x, to_pos.y),
                                    actual: (catch_spot.x, catch_spot.y),
                                    divergence_ft: divergence,
                                });
                            }
                            self.pending_events.push(GameEvent::PassReceived {
                                receiver_id: receiver_id.clone(),
                                position: (catch_spot.x, catch_spot.y),
                            });
                            self.pending_pass_inbound = false;
                            if is_inbound_pass {
                                self.transition_phase(SubPhase::Initiation);
                                self.set_game_flow(GameFlowState::LiveBall);
                                self.sub_phase_timer = 0.0;
                                self.last_decision_time = -self.rules.decision_interval_seconds;
                            }
                            // The pass trajectory already ends at the frozen
                            // target. The receiver's body must not teleport the
                            // ball at catch.
                            new_ball_state = Some(BallTrajectoryKind::Held {
                                carrier_id: receiver_id,
                            });
                            self.last_passer_id = None;
                        } else {
                            // 层 A/层 B 不通过：球落到它**实际到达的位置**。
                            // 层 A 不可达 → 球在人之外；层 B 未接稳 → 球在人身旁。
                            // 两者都是 loose ball（用户批准的设计点 1）：
                            // 「接不到就是接不到」，不是全知全能地送到手里。
                            //
                            // 发球传球失败时必须回到活球阶段，否则游戏卡在
                            // DeadBall/ActionExecution（实测 120k tick 无进展）。
                            let arrival = self.ball_pos_3d.0;
                            if is_inbound_pass {
                                self.transition_phase(SubPhase::Initiation);
                                self.set_game_flow(GameFlowState::LiveBall);
                                self.last_decision_time = -self.rules.decision_interval_seconds;
                            }
                            self.pending_pass_receiver = None;
                            // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
                            self.receiver_estimate = None;
                            self.pending_events.push(GameEvent::PassDropped {
                                passer_id,
                                receiver_id: receiver_id.clone(),
                                position: (arrival.x, arrival.y),
                            });
                            // 归因：传球失误（层 A 不可达 或 层 B 未接稳）。
                            self.pending_loose_ball_terminal =
                                Some(nba_domain::PossessionEndCause::TurnoverPassDropped);
                            new_ball_state = Some(self.loose_ball_from(arrival, segment));
                        }
                    }
                }
            }
            BallTrajectoryKind::Drive {
                driver_id,
                target_pos: ref target_pos_ref,
                start_time,
                duration,
                successful,
                finish_made,
                fouler_id,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );

                let driver_pos = self
                    .physics
                    .get_player(driver_id)
                    .map(|player| player.pos_ft)
                    .unwrap_or(self.ball_pos_3d.0);
                let hoop_pos = self.rules.court.hoop_pos(is_home);
                let dist_to_hoop = (driver_pos - hoop_pos).length();

                // ## 过人后重定向（round-18 两段式几何的第二段）
                //
                // 起手目标是被过防守人身旁的空档（第一步过人）；越过他
                // （沿突破方向投影领先）后，把目标切回**篮筐**，完成
                // 「过人 → 攻框」的完整几何。
                // 时间基重定向：被过防守人以极速让位，约一个加速窗口
                // （0.35s）后通道即开，届时切回攻框目标。空间基判定
                // （「越过防守人」）在碰撞边界处永不可达，实测卡死。
                if self.beaten_defender_id.is_some() && current_t - start_time >= 0.35 {
                    if let Some(dp) = self.physics.get_player(driver_id) {
                        let drv_spd = dp.target_speed_ftps;
                        let morale = dp.morale.clone();
                        self.physics.set_player_target(
                            driver_id,
                            *target_pos_ref,
                            drv_spd,
                            "DriveToBasket",
                            "BallHandler",
                            &morale,
                        );
                        self.beaten_defender_id = None;
                    }
                }
                // 1. 中途分流决策（Drive Branching: 突分 Kickout 或急停中投 Pull-up）
                let elapsed = current_t - start_time;
                let mut branched = false;
                if elapsed >= self.rules.tactics.drive_decision_check_interval_seconds && tau < 0.85
                {
                    let paint_crowding = SemanticEvaluator::spacing(
                        self.possession,
                        driver_pos,
                        &self.physics,
                        &self.rules,
                    )
                    .paint_crowding;

                    // 当内线极度拥挤且持球人在中远距离时，评估突分（Kickout）给外线空位队友
                    if paint_crowding > self.rules.tactics.drive_kickout_max_crowding {
                        let mut best_kickout: Option<(String, Vec2)> = None;
                        let mut min_opp_dist =
                            self.rules.tactics.drive_kickout_min_defender_dist_ft;

                        let all_players: Vec<_> =
                            self.physics.get_players().values().cloned().collect();
                        let off_team_str = match self.possession {
                            Possession::Home => "home",
                            Possession::Away => "away",
                        };
                        for teammate in &all_players {
                            // ## on_court 过滤（round-18 修复）
                            //
                            // 此前漏掉：突破分球把球传给了**替补席上的队友**
                            // （实测 seed 31337：分球给站在板凳区 y=-4 的
                            // H_08，球被「已下场的人接住」，BALL_HOLDER_ON_COURT
                            // Hard 190 次）。
                            if teammate.on_court
                                && teammate.team == off_team_str
                                && teammate.id != *driver_id
                            {
                                let dist_to_team_hoop = (teammate.pos_ft - hoop_pos).length();
                                if dist_to_team_hoop
                                    >= self.rules.tactics.drive_kickout_pass_dist_ft
                                {
                                    // 检查该空位队友最近的防守人距离
                                    let mut nearest_def_dist = f32::MAX;
                                    for opp in &all_players {
                                        if opp.team != off_team_str {
                                            let d = (opp.pos_ft - teammate.pos_ft).length();
                                            if d < nearest_def_dist {
                                                nearest_def_dist = d;
                                            }
                                        }
                                    }
                                    if nearest_def_dist > min_opp_dist {
                                        min_opp_dist = nearest_def_dist;
                                        best_kickout = Some((teammate.id.clone(), teammate.pos_ft));
                                    }
                                }
                            }
                        }

                        if let Some((target_id, target_spot)) = best_kickout {
                            drive_kickout_action = Some((
                                driver_id.clone(),
                                target_id,
                                driver_pos,
                                target_spot,
                                current_t,
                            ));
                            branched = true;
                        } else if paint_crowding > self.rules.tactics.drive_pullup_min_crowding
                            && dist_to_hoop > self.rules.tactics.drive_early_finish_dist_ft
                            && dist_to_hoop <= self.rules.tactics.drive_mid_range_pullup_dist_ft
                        {
                            // 人堆受阻，急停中距离跳投 (Pull-up)
                            drive_pullup_action = Some((driver_id.clone(), driver_pos, current_t));
                            branched = true;
                        }
                    }
                }

                // 2. 提前进入冲框终结判定：进入终结区（Early Finish Gate）或时间耗尽
                let early_finish = dist_to_hoop <= self.rules.tactics.drive_early_finish_dist_ft
                    && elapsed >= self.rules.tactics.drive_min_duration_seconds;
                if !branched && (tau >= 1.0 || early_finish) {
                    let driver_id = driver_id.clone();
                    self.current_possession_turnover_player = Some(driver_id.clone());
                    let successful = *successful;
                    let finish_made = *finish_made;
                    let fouler_id = fouler_id.clone();
                    let driver_pos = self
                        .physics
                        .get_player(&driver_id)
                        .map(|player| player.pos_ft)
                        .unwrap_or(self.ball_pos_3d.0);
                    let holder_height = self.rules.ball_holder_height_ft;
                    // ## 事实修正（round-16）：先判定空间门，再申报结果
                    //
                    // 原实现无条件把预掷的 `finish_made` 写进事件，但空间门
                    // （距筐 >16ft）会把该「进球」静默丢弃——实测 6 场 397 次
                    // DRIVE_SCORE 中 80%（场均 52.7 次）未变成出手，事件流与
                    // 记分簿自相矛盾（事实账目违规）。
                    let stall_hoop_pos = self.rules.court.hoop_pos(is_home);
                    let stall_no_finish =
                        successful && ((driver_pos - stall_hoop_pos).length() > 16.0_f32);
                    self.pending_events.push(GameEvent::DriveOutcome {
                        driver_id: driver_id.clone(),
                        successful: successful && !stall_no_finish,
                        finish_made: finish_made && !stall_no_finish,
                    });

                    if let Some(fouler_id) = fouler_id {
                        self.pending_events.push(GameEvent::Foul {
                            fouled_player_id: driver_id.clone(),
                            fouler_id,
                            is_shooting: true,
                        });
                        self.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(self.dead_state(driver_pos, holder_height));
                        self.transition_phase(SubPhase::DeadBallReset);
                        self.current_event = Some("DRIVE_FOUL".to_string());
                        self.current_callout =
                            Some(format!("{} 突破造成投篮犯规，获得罚球机会", driver_id));
                    } else if successful {
                        let hoop_pos = self.rules.court.hoop_pos(is_home);
                        let dist_to_hoop = (driver_pos - hoop_pos).length();
                        // Spatial gate: if driver is still outside the paint / perimeter,
                        // this drive was stalled before reaching finishing position.
                        let finish_range = 16.0_f32;
                        if dist_to_hoop > finish_range {
                            self.ball_pos_3d = (driver_pos, holder_height);
                            new_ball_state = Some(BallTrajectoryKind::Held {
                                carrier_id: driver_id.clone(),
                            });
                            // ## 停滞不重新发起战术（round-16）
                            //
                            // 原实现转回 Initiation，而 Initiation 在活球下要等
                            // `tactical_initiation_seconds = 6.5s` 才回到可决策
                            // 状态——停滞一次就白燃 6.5s 进攻时钟。突破受阻是
                            // **进攻的延续**，不是新回合，直接回 ActionExecution
                            // 下一决策间隔（2.4s）即可行动。
                            // 实测该机制是 24s 违例（场均 12.8 次，真实 ~0.5）
                            // 的主要时间吞噬器。
                            self.transition_phase(SubPhase::ActionExecution);
                            self.current_event = Some("DRIVE_STOPPED".to_string());
                            self.current_callout =
                                Some(format!("{} 突破被防守延误于外线，重新组织", driver_id));
                        } else {
                            let driver_p = self.physics.get_player(&driver_id).cloned();
                            let finishing_skill = driver_p
                                .as_ref()
                                .map(|p| p.attributes.finishing)
                                .unwrap_or(0.5);
                            let lane_density = SemanticEvaluator::spacing(
                                self.possession,
                                driver_pos,
                                &self.physics,
                                &self.rules,
                            )
                            .paint_crowding;
                            let (finish_kind, action_name, callout_action) = if dist_to_hoop < 4.0
                                && finishing_skill > 0.7
                                && lane_density < 0.35
                            {
                                (
                                    nba_domain::action_window::RimFinishKind::Dunk,
                                    "Dunk",
                                    "腾空暴扣！单臂炸筐！",
                                )
                            } else if dist_to_hoop > 7.0 {
                                (
                                    nba_domain::action_window::RimFinishKind::Floater,
                                    "Floater",
                                    "行进间柔和抛投！",
                                )
                            } else {
                                (
                                    nba_domain::action_window::RimFinishKind::Layup,
                                    "Layup",
                                    "三步并两步，低手上篮！",
                                )
                            };
                            let shot_dur = BallisticsEngine::shot_duration(
                                dist_to_hoop,
                                self.rules.rim_height_ft,
                                &self.rules,
                            )
                            .max(0.4);
                            self.transition_phase(SubPhase::ShotAttempt);
                            self.pending_events.push(GameEvent::ShotRelease {
                                shooter_id: driver_id.clone(),
                                pos: (driver_pos.x, driver_pos.y),
                                is_three: false,
                                contest_level: 0.2,
                                make_probability: if finish_made { 1.0 } else { 0.0 },
                            });
                            self.current_possession_shooter = Some(driver_id.clone());
                            let contest_val = if finish_made { 0.35 } else { 0.65 };
                            self.current_possession_contest = Some(contest_val);
                            self.current_event = Some("SHOT_RELEASE".to_string());
                            if finish_kind == nba_domain::action_window::RimFinishKind::Dunk {
                                self.active_windows.insert(
                                    driver_id.clone(),
                                    ActionTimeWindow::new_dunk(&driver_id, current_t, &self.rules),
                                );
                            } else {
                                self.active_windows.insert(
                                    driver_id.clone(),
                                    ActionTimeWindow::new_layup(&driver_id, current_t, &self.rules),
                                );
                            }
                            if let Some(p) = self.physics.get_player_mut(&driver_id) {
                                p.action = action_name.to_string();
                                let hoop_dir = (hoop_pos - p.pos_ft).normalize_or_zero();
                                if hoop_dir.length_squared() > 0.1 {
                                    p.facing_dir = hoop_dir;
                                }
                            }
                            let driver_display = self
                                .physics
                                .get_player(&driver_id)
                                .map(|p| format!("{}号", p.jersey))
                                .unwrap_or_else(|| driver_id.clone());
                            self.current_callout =
                                Some(format!("{} {}", driver_display, callout_action));
                            new_ball_state = Some(BallTrajectoryKind::Shot {
                                shooter_id: driver_id.clone(),
                                from_pos: driver_pos,
                                hoop_pos,
                                start_time: current_t,
                                duration: shot_dur,
                                is_made: finish_made,
                                is_three: false,
                                peak_z: if finish_kind
                                    == nba_domain::action_window::RimFinishKind::Dunk
                                {
                                    self.rules.rim_height_ft + 0.5
                                } else {
                                    self.rules.rim_height_ft + 1.5
                                },
                            });
                        }
                    } else {
                        self.ball_pos_3d = (driver_pos, holder_height);
                        new_ball_state = Some(BallTrajectoryKind::Held {
                            carrier_id: driver_id.clone(),
                        });
                        // 同上：突破未成是进攻延续，不重置为战术发起等待。
                        self.transition_phase(SubPhase::ActionExecution);
                        self.current_event = Some("DRIVE_STOPPED".to_string());
                        self.current_callout =
                            Some(format!("{} 突破被防守延误，重新组织", driver_id));
                    }
                    self.last_decision_time = current_t;
                }
            }
            BallTrajectoryKind::Shot {
                shooter_id,
                hoop_pos,
                start_time,
                duration,
                is_made,
                is_three,
                from_pos,
                ..
            } => {
                let tau = ((current_t - start_time) / duration.max(f32::EPSILON)).clamp(0.0, 1.0);
                if tau >= 1.0 {
                    let sid = shooter_id.clone();
                    let h_pos = *hoop_pos;
                    let s_pos = *from_pos;
                    let made = *is_made;
                    let three = *is_three;
                    self.pending_events.push(GameEvent::HoopArrival {
                        shooter_id: sid.clone(),
                        shot_origin: (s_pos.x, s_pos.y),
                        is_made: made,
                        is_three: three,
                        contest_intensity: 0.0,
                    });
                    if three {
                        self.box_score.fg3_attempts += 1;
                        if made {
                            self.box_score.fg3_made += 1;
                        }
                    } else {
                        self.box_score.fg2_attempts += 1;
                        if made {
                            self.box_score.fg2_made += 1;
                        }
                    }
                    match ResolutionLayer::resolve_shot_arrival(made, three, &sid) {
                        ResolutionOutcome::Score { points, .. } => {
                            if is_home {
                                self.home_score += points;
                            } else {
                                self.away_score += points;
                            }
                            let baseline =
                                Court::nearest_boundary_with_geometry(h_pos, self.rules.court);
                            self.ball_pos_3d = (h_pos, self.rules.rim_height_ft);
                            self.emit_possession_summary(
                                nba_domain::PossessionEndCause::Score,
                                None,
                                None,
                                None,
                            );
                            self.transition_phase(SubPhase::DeadBallReset);
                            self.start_inbound_transition(
                                baseline,
                                (h_pos, self.rules.rim_height_ft),
                            );
                        }
                        ResolutionOutcome::Miss { .. } => {
                            let shooter_name = self
                                .physics
                                .get_player(&sid)
                                .map(|p| p.jersey.clone())
                                .unwrap_or_else(|| sid.clone());
                            self.current_event = Some("SHOT_MISSED".to_string());
                            self.current_callout =
                                Some(format!("砸框而出！{} 投篮不中，争抢篮板！", shooter_name));
                            let rebound_from = (h_pos, self.rules.rim_height_ft);
                            self.transition_phase(SubPhase::FlightAndRebound);
                            let landing_spot = BallisticsEngine::compute_rebound_landing(
                                s_pos,
                                h_pos,
                                &mut self.rng,
                                &self.rules,
                            );
                            new_ball_state = Some(BallTrajectoryKind::RimRebound {
                                from_pos: rebound_from.0,
                                from_z: rebound_from.1,
                                hoop_pos: h_pos,
                                target_landing: landing_spot.landing_pos,
                                start_time: current_t,
                                duration: landing_spot.flight_duration,
                                peak_z: self.rules.rebound_peak_ft,
                                last_touch_team: self.possession,
                            });
                        }
                        _ => {}
                    }
                }
            }
            BallTrajectoryKind::RimRebound {
                target_landing,
                start_time,
                duration,
                ..
            } => {
                let tau = if *duration <= f32::EPSILON {
                    1.0
                } else {
                    ((current_t - start_time) / duration).clamp(0.0, 1.0)
                };
                if tau >= 1.0 {
                    let reb_pos = *target_landing;
                    let max_reach =
                        self.rules.player_radius_ft + self.rules.defender_reach_ft + 1.5;
                    let maybe_reb_id = self.try_resolve_rebounder(reb_pos, max_reach);

                    if let Some(reb_id) = maybe_reb_id {
                        let reb_name = self
                            .physics
                            .get_player(&reb_id)
                            .map(|p| p.jersey.clone())
                            .unwrap_or(reb_id.clone());
                        let original_offense = if is_home { "home" } else { "away" };
                        let is_offensive = self
                            .physics
                            .get_player(&reb_id)
                            .map(|p| p.team == original_offense)
                            .unwrap_or(false);
                        self.pending_events.push(GameEvent::ReboundContest {
                            rebounder_id: reb_id.clone(),
                            landing_pos: (reb_pos.x, reb_pos.y),
                            is_offensive,
                        });
                        self.current_event = Some("REBOUND".to_string());
                        self.current_callout = Some(format!(
                            "{} 抢到{}篮板，重新组织进攻！",
                            reb_name,
                            if is_offensive { "前场" } else { "防守" }
                        ));
                        if !is_offensive {
                            let p_pos = self
                                .physics
                                .get_player(&reb_id)
                                .map(|p| p.pos_ft)
                                .unwrap_or(reb_pos);
                            let dist = (p_pos - reb_pos).length();
                            self.emit_possession_summary(
                                nba_domain::PossessionEndCause::DefensiveRebound,
                                Some(reb_id.clone()),
                                None,
                                Some(dist),
                            );
                        }
                        self.start_rebound_outlet(reb_id, reb_pos, is_offensive);
                    } else {
                        // 无人在有效范围内保护篮板，球弹落变地板球。
                        //
                        // ## 篮板源归因（round-17 修复）
                        //
                        // 投/罚不中弹出的地板球**不是失误**：防守方收下是
                        // 防守篮板，进攻方收下是前场篮板。原实现默认
                        // `TurnoverLooseBall`，把「防守方拿到罚球不中的球」
                        // 记成失误——既虚增失误数，又因不存在「失误球员」
                        // 触发 TURNOVER_ACTOR_CONSISTENCY Hard
                        // （实测 seed 2 possession 83：FT 不中→地板球→
                        // 防守收下→turnover_player_id=null）。
                        self.pending_loose_ball_terminal =
                            Some(nba_domain::PossessionEndCause::DefensiveRebound);
                        new_ball_state = Some(BallTrajectoryKind::LooseBall {
                            pos: reb_pos,
                            vel: Vec2::ZERO,
                            z: self.ball_pos_3d.1,
                            vel_z: 0.0,
                            last_touch_team: self.possession,
                        });
                        self.current_event = Some("LOOSE_BALL".to_string());
                        self.current_callout =
                            Some("篮板球弹出无人抢到，双方争夺地板球！".to_string());
                    }
                }
            }
            BallTrajectoryKind::LooseBall {
                pos, vel, z, vel_z, ..
            } => {
                // 第一性原理：球飞出边界就是**出界事实**，应触发裁定，
                // 而不是被 `clamp_playable` 硬夹回场内。
                //
                // 此前把界外松球直接夹回边界，单 tick 产生数英尺位移，
                // 被 L1 判为 `BALL_SPEED`（实测 122 ft/s > 85 上限，
                // seed 1/12/16 各 1–2 条 Hard）。球没有"贴边弹回"这种
                // 物理；出界必须是一个显式状态转移。
                let raw_next = *pos + *vel * dt;
                let margin = self.rules.player_radius_ft;
                let out_of_bounds = raw_next.x < margin
                    || raw_next.x > self.rules.court.width_ft - margin
                    || raw_next.y < margin
                    || raw_next.y > self.rules.court.height_ft - margin;
                if out_of_bounds {
                    // 出界：球权交给最后触球方的对手，进入发球程序。
                    // 放在主循环之后统一执行（此处不能提前 return，
                    // 否则会跳过账本提交与不变量检查）。
                    loose_ball_out_of_bounds = Some((
                        Court::nearest_boundary_with_geometry(*pos, self.rules.court),
                        (*pos, *z),
                    ));
                }
                let next_pos = if out_of_bounds {
                    // 本 tick 不再推进球的位置：出界点就是事实位置。
                    *pos
                } else {
                    raw_next
                };
                let mut next_vel_z = *vel_z - self.rules.ball_gravity_ftps2 * dt;
                let mut next_z = *z + next_vel_z * dt;
                let mut next_vel = *vel * self.rules.ball_velocity_retention;
                if next_z <= 0.0 {
                    // 地面碰撞反弹：反弹恢复系数 e = 0.70，地面摩擦衰减
                    next_z = 0.0;
                    next_vel_z = (-next_vel_z * 0.70).max(0.0);
                    next_vel *= 0.85;
                }
                let reach = self.rules.player_radius_ft + self.rules.defender_reach_ft;
                let home_jumper = self.select_jumper_id(Possession::Home);
                let away_jumper = self.select_jumper_id(Possession::Away);
                let mut candidates =
                    self.physics
                        .query_nearby(*pos, reach, &nba_physics::EntityFilter::Any);
                // 真实规则：跳球员在球触地或被其他人触及前，严禁直接控球
                if *z > 0.5 {
                    candidates.retain(|id| *id != home_jumper && *id != away_jumper);
                }
                candidates.sort();
                if let Some(player_id) = candidates.into_iter().next() {
                    self.pending_events.push(GameEvent::LooseBallSecured {
                        player_id: player_id.clone(),
                        position: (next_pos.x, next_pos.y),
                    });
                    loose_ball_secured_player = Some((player_id, next_pos, next_z));
                } else {
                    new_ball_state = Some(BallTrajectoryKind::LooseBall {
                        pos: next_pos,
                        vel: next_vel,
                        z: next_z,
                        vel_z: next_vel_z,
                        last_touch_team: self.possession,
                    });
                }
            }
            BallTrajectoryKind::Dead { .. } => {}
        }
        // 松球出界：显式状态转移（球权交给对方并发球）。
        //
        // 注意：这里**不能**合成 `GameEvent::BoundaryCross`。该事件的语义是
        // 「**球员**越过边界」，约束层 `out_of_bounds_event` 会据此判定
        // 球权违例；用一个空的 player_id 冒充球员边界事实会被误判成
        // `TURNOVER:OUT_OF_BOUNDS`（实测每场 42 次虚假失误）。
        // 球的出界是**球的**事实，由下面的回合终结 + 发球程序表达，
        // 不需要借用球员边界事件。
        if let Some((boundary, ball_3d)) = loose_ball_out_of_bounds {
            self.current_event = Some("OUT_OF_BOUNDS".to_string());
            self.start_out_of_bounds_transition(boundary, ball_3d);
            self.publish_events();
            return self.build_tick();
        }
        if let Some((driver_id, target_id, driver_pos, target_spot, start_t)) = drive_kickout_action
        {
            self.current_event = Some("DRIVE_KICKOUT".to_string());
            let driver_display = self
                .physics
                .get_player(&driver_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| driver_id.clone());
            let target_display = self
                .physics
                .get_player(&target_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| target_id.clone());
            self.current_callout = Some(format!(
                "{} 吸引内线包夹，行进间妙手突分外线 {}！",
                driver_display, target_display
            ));
            self.execute_pass(
                &driver_id,
                &target_id,
                driver_pos,
                target_spot,
                start_t,
                false,
            );
        } else if let Some((driver_id, driver_pos, start_t)) = drive_pullup_action {
            self.current_event = Some("DRIVE_PULLUP".to_string());
            let driver_display = self
                .physics
                .get_player(&driver_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| driver_id.clone());
            self.current_callout = Some(format!("{} 禁区受阻，急停漂移干拔跳投！", driver_display));
            self.execute_shot(
                &driver_id,
                driver_pos,
                false,
                Some(nba_domain::action_window::JumperKind::PullUp),
                start_t,
            );
        } else if let Some(def_id) = steal_triggered_defender {
            self.current_event = Some("STEAL".to_string());
            let def_display = self
                .physics
                .get_player(&def_id)
                .map(|p| format!("{}号", p.jersey))
                .unwrap_or_else(|| def_id.clone());
            self.current_callout = Some(format!("传球路线被识破！{} 飞身抢断！", def_display));
            let ball_intercept_pos = self.ball_pos_3d.0;
            self.start_steal_transition(def_id, ball_intercept_pos);
        } else if let Some((player_id, from_pos, from_z)) = loose_ball_secured_player {
            // The security fact is sampled at `next_pos`/`next_z`; use the same
            // point as the control-transfer origin instead of rewinding to the
            // pre-integration ball sample.
            let target_pos = self
                .physics
                .get_player(&player_id)
                .map(|player| player.pos_ft)
                .unwrap_or(from_pos);
            self.start_loose_ball_transition(player_id.clone(), from_pos);
            if !self.simulation_complete {
                let target_z = self.rules.ball_holder_height_ft;
                let distance = (target_pos - from_pos).length();
                let speed_budget = (self.rules.ball_max_speed_ftps
                    - self.rules.invariant_speed_tolerance_ftps)
                    .max(self.rules.invariant_speed_tolerance_ftps);
                let duration = (distance / speed_budget)
                    .max(self.rules.min_pass_duration_seconds)
                    .max(self.rules.tick_seconds);
                self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                    from_pos,
                    from_z,
                    target_pos,
                    target_z,
                    carrier_id: player_id,
                    start_time: current_t,
                    duration,
                });
                self.ball_pos_3d = (from_pos, from_z);
            }
        } else if let Some(nbs) = new_ball_state {
            self.transition_ball_state(nbs);
            if live_ball_triggered {
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::LiveBall);
                self.last_decision_time = -self.rules.decision_interval_seconds;
            }
            if !matches!(self.ball_state, BallTrajectoryKind::Held { .. }) {
                self.ball_pos_3d = BallisticsEngine::sample_ball_position(
                    &self.ball_state,
                    current_t,
                    self.physics.get_players(),
                    &self.rules,
                );
            }
        }

        // ============================================================
        // ============================================================
        // 4. 战术目标生成 & 移动导航（每 tick）
        // ============================================================
        let off_roster = match self.possession {
            Possession::Home => &self.home_roster_order,
            Possession::Away => &self.away_roster_order,
        };
        let live_off_positions: Vec<Vec2> = off_roster
            .iter()
            .filter_map(|pid| self.physics.get_player(pid).map(|p| p.pos_ft))
            .collect();
        // D5.1b：进攻槽位由**战术档案**决定（底角/内线/弧顶距离各异），
        // 防守目标仍由对位逻辑生成。
        let off_roster_owned: Vec<String> = off_roster.clone();
        let off_spec = match self.possession {
            Possession::Home => &self.home_offense_spec,
            Possession::Away => &self.away_offense_spec,
        };
        // slot fill：按能力把槽位分配给在场球员（不再按 roster 顺序绑定）。
        let fitness: Vec<nba_domain::PlayerSlotFitness> = off_roster_owned
            .iter()
            .filter_map(|pid| {
                self.physics
                    .get_player(pid)
                    .filter(|p| p.on_court)
                    .map(|_| pid.as_str())
            })
            .filter_map(|pid| {
                self.home_team
                    .players
                    .iter()
                    .chain(self.away_team.players.iter())
                    .find(|pl| pl.id == pid)
            })
            .map(nba_domain::PlayerSlotFitness::from_player)
            .collect();
        let (filled_ids, fit_error) = TacticalPlanner::fill_slots_or_roster_order(
            off_spec,
            &fitness,
            &off_roster_owned,
            &self.rules,
        );
        if let Some(err) = fit_error {
            self.current_enforcements
                .push(format!("SLOT_FIT_FALLBACK:{}", err));
        }
        // 持球槽位：由 slot fill 选出「最擅长处理球」的球员所占据的槽位。
        // 这样持球权归属来自能力适配，而不是 roster 索引（D5.1b）。
        let mut carrier_slot = 0usize;
        let mut best_handle = f32::MIN;
        for (i, pid) in filled_ids.iter().enumerate() {
            if let Some(p) = fitness.iter().find(|f| &f.player_id == pid) {
                let s = p.ball_handling * self.rules.tactics.slot_handler_ball_handling_weight
                    + p.decision_iq * self.rules.tactics.slot_handler_decision_iq_weight;
                if s > best_handle {
                    best_handle = s;
                    carrier_slot = i;
                }
            }
        }
        let mut off_targets = TacticalPlanner::plan_offense_from_spec(
            off_spec,
            self.sub_phase,
            self.possession,
            carrier_slot,
            self.sub_phase_timer,
            &self.rules,
        );
        TacticalPlanner::bind_targets(&mut off_targets, &filled_ids);

        // 防守目标沿用对位/协防逻辑（含 D5.2 的执行器）。
        let (mut home_targets, mut away_targets) =
            TacticalPlanner::plan_possession_targets_with_rules(
                self.tactical_set,
                self.sub_phase,
                self.possession,
                self.ball_pos_3d.0,
                self.carrier_idx,
                self.sub_phase_timer,
                &mut self.rng,
                &self.rules,
                Some(&live_off_positions),
            );
        let home_roster = self.home_roster_order.clone();
        let away_roster = self.away_roster_order.clone();
        // 进攻方用档案槽位覆盖（球员绑定已在 off_targets 内按能力完成）；
        // 防守方保留原对位结果并按 roster 绑定。
        // 注意：不能再对 off_targets 调用 bind_targets——它按 roster 顺序
        // 覆盖 player_id，会把 slot fill 的结果抹掉并把替补拉进场内
        // （本轮实测 116 条 PLAYER_SEPARATION：替补 A_7 与在场球员重叠 1.19ft）。
        if self.possession == Possession::Home {
            home_targets = off_targets;
            TacticalPlanner::bind_targets(&mut away_targets, &away_roster);
        } else {
            away_targets = off_targets;
            TacticalPlanner::bind_targets(&mut home_targets, &home_roster);
        }
        // 取为 owned String，避免与后续 `&mut self` 调用（接球人估计）冲突。
        let active_driver_id = match &self.ball_state {
            BallTrajectoryKind::Drive { driver_id, .. } => Some(driver_id.clone()),
            _ => None,
        };
        let inbounder_override = match &self.ball_state {
            BallTrajectoryKind::InboundTransfer {
                inbounder_id,
                baseline_pos,
                ..
            }
            | BallTrajectoryKind::InboundReady {
                inbounder_id,
                baseline_pos,
                ..
            } => Some((inbounder_id.clone(), *baseline_pos)),
            _ => None,
        };
        // 层 A（P-1 有限信息）：接球人**不得**直读传球人的冻结落点。
        //
        // 旧实现把 `BallState::Pass.to_pos`（传球人的私有意图）直接注入接球人的
        // 运动目标，于是接球人必然到位——全知全能，违反真实性。实测：无论把
        // 接球人的 `off_ball_sense`（预估能力）设为 0.95 还是 0.05，2/4 种子的
        // 接球轨迹**逐位相同**（见 `tests/pass_information.rs`）。
        //
        // 现改为：接球人按**自己的感知**估算球会到哪里，并向该估计值收敛。
        // 估计可能错 ⇒ 他可能接不到（真实：大个策应给小个传提前量，小个
        // 可能启动方向不同而接不到）。
        // 先从球态取出所需字段（避免与 `estimate_receiver_landing` 的
        // `&mut self` 冲突），再计算接球人的**自身估计**。
        let pass_fields = match &self.ball_state {
            BallTrajectoryKind::Pass {
                target_id, to_pos, ..
            } => Some((target_id.clone(), *to_pos)),
            _ => None,
        };
        let pass_receiver_override = if let Some((rid, frozen)) = pass_fields {
            let est = self.estimate_receiver_landing(&rid, frozen);
            // 接球人标记已在 `transition_ball_state`（唯一写入口）设置，
            // 确保物理步进先于战术规划时也能生效。
            Some((rid, est))
        } else {
            match &self.ball_state {
                BallTrajectoryKind::ControlTransfer {
                    carrier_id,
                    target_pos,
                    ..
                } => Some((carrier_id.clone(), *target_pos)),
                _ => None,
            }
        };
        let rebound_chase_target = match &self.ball_state {
            BallTrajectoryKind::RimRebound { target_landing, .. } => Some(*target_landing),
            BallTrajectoryKind::LooseBall { pos, .. } => Some(*pos),
            _ => None,
        };
        let home_jumper = self.select_jumper_id(Possession::Home);
        let away_jumper = self.select_jumper_id(Possession::Away);
        for target in home_targets.into_iter().chain(away_targets) {
            if let Some(player_id) = target.player_id {
                if active_driver_id.as_deref() == Some(player_id.as_str()) {
                    continue;
                }
                // 被过恢复窗口（round-19）：被过掉的防守人正扑向回追位，
                // 战术层不得立即把他派回原位（否则让位形同虚设）。
                if self
                    .beaten_recovery_until
                    .get(player_id.as_str())
                    .is_some_and(|&until| current_t < until)
                {
                    continue;
                }
                // If player is currently executing a locked action window (shot, layup, dunk, pass, screen),
                // protect their action and kinematics from tactical overwrite
                let in_active_window = self
                    .active_windows
                    .get(&player_id)
                    .map(|w| !w.is_finished(current_t))
                    .unwrap_or(false);
                let is_action_locked = self
                    .physics
                    .get_player(&player_id)
                    .map(|p| {
                        let a = p.action.as_str();
                        a == "TripleThreat"
                            || a == "PostUp"
                            || a.ends_with("Shot")
                            || a == "Layup"
                            || a == "Dunk"
                            || a == "Floater"
                    })
                    .unwrap_or(false);

                if in_active_window || is_action_locked {
                    continue;
                }

                // If tactical assignment is setting a high screen, initialize a ScreenSet action window
                if target.action == "SET_HIGH_SCREEN"
                    && !self.active_windows.contains_key(&player_id)
                {
                    self.active_windows.insert(
                        player_id.clone(),
                        ActionTimeWindow::new_screen_set(&player_id, current_t, &self.rules),
                    );
                }

                // 第一性原理：后场推进期间不得被战术目标覆盖。
                //
                // `set_player_target` 每 tick 都执行；若持球人正在执行
                // `Advance`（把球推过中线），战术槽位（弧顶 x=66 等）
                // 会把目标改回半场落位，导致推进速度被反复打断
                // （实测仅 3.5–3.9 ft/s，而 8 秒规则需要 ≥4.5 ft/s）。
                let is_carrier_advancing = self.advancing_player.as_deref()
                    == Some(player_id.as_str())
                    && self.sub_phase != SubPhase::Initiation;
                if is_carrier_advancing {
                    continue;
                }
                let (target_pos, speed, action) = if let Some((inb_id, inb_pos)) =
                    &inbounder_override
                {
                    if inb_id == &player_id {
                        (*inb_pos, 15.0, "INBOUND_SETUP".to_string())
                    } else if let Some((rx_id, rx_pos)) = &pass_receiver_override {
                        if rx_id == &player_id {
                            let (aim, spd) = self.receive_approach(*rx_pos, &player_id);
                            (aim, spd, "RECEIVE_CUT".to_string())
                        } else {
                            (target.target_pos, target.speed, target.action)
                        }
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else if let Some((rx_id, rx_pos)) = &pass_receiver_override {
                    if rx_id == &player_id {
                        let (aim, spd) = self.receive_approach(*rx_pos, &player_id);
                        (aim, spd, "RECEIVE_CUT".to_string())
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else if let Some(reb_spot) = rebound_chase_target {
                    let cur_dist = self
                        .physics
                        .get_player(&player_id)
                        .map(|p| (p.pos_ft - reb_spot).length())
                        .unwrap_or(99.0);
                    let is_tipoff_jumper = matches!(&self.ball_state, BallTrajectoryKind::LooseBall { z, .. } if *z > 0.5)
                        && (player_id == home_jumper || player_id == away_jumper);
                    // ## 地板球追逐不受距离限制（round-17 活锁修复）
                    //
                    // 原实现的 `cur_dist <= 25.0` 硬半径在球停于空档区时失效：
                    // 实测 seed 6，罚球后松球停在 (85.6, 29.0)，最近球员
                    // 59.1 ft——无人满足 25 ft 条件 → 全场站桩 247 秒直到节末
                    // （POSSESSION_DURATION_BOUNDS Hard: 258.4s > 40s）。
                    //
                    // 第一性原理：活球是场上**唯一完全可观测**的对象（不是
                    // 任何人的私有信息），地板上躺着一颗活球时，「去抢球」
                    // 压倒一切战术站位——真实篮球里所有近处球员都会扑向球。
                    // 篮板追逐（RimRebound 的落点预判）保留 25 ft 半径；
                    // 松球（LooseBall）无条件追逐。
                    let is_live_loose_ball =
                        matches!(self.ball_state, BallTrajectoryKind::LooseBall { .. });
                    if (is_live_loose_ball || cur_dist <= 25.0) && !is_tipoff_jumper {
                        (reb_spot, 16.0, "REBOUND_CRASH".to_string())
                    } else {
                        (target.target_pos, target.speed, target.action)
                    }
                } else {
                    (target.target_pos, target.speed, target.action)
                };
                self.physics.set_player_target(
                    &player_id,
                    target_pos,
                    speed,
                    &action,
                    &target.slot,
                    &target.morale,
                );
            }
        }
        // A control transfer is a flight, not possession. The receiving
        // player becomes the holder only after the frozen trajectory ends.
        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.as_str()),
            _ => None,
        };
        self.physics.set_ball_holder(active_carrier);

        let player_ids: Vec<String> = {
            let mut ids: Vec<String> = self.physics.get_players().keys().cloned().collect();
            ids.sort();
            ids
        };
        let modulation = &mut self.modulation;
        for pid in &player_ids {
            let speed = self
                .physics
                .get_player(pid)
                .map(|p| p.vel_ft.length())
                .unwrap_or(0.0);
            modulation
                .entry(pid.clone())
                .or_default()
                .update_stamina_with_rules(speed, dt, &self.rules);
        }
        // modulation 状态在物理推进后保留为决策反馈，渲染读取当前值。

        let raw_contacts = self.physics.drain_contacts();
        self.latest_contacts = SemanticEvaluator::event_facts(
            raw_contacts.clone(),
            &self.physics,
            self.possession,
            self.sub_phase,
            self.tick_index,
            &self.rules,
        );
        self.latest_spacing = Some(SemanticEvaluator::spacing(
            self.possession,
            self.ball_pos_3d.0,
            &self.physics,
            &self.rules,
        ));
        self.pending_events
            .extend(self.latest_contacts.iter().map(semantic_contact_to_event));
        self.pending_events.extend(
            self.physics
                .drain_facts()
                .into_iter()
                .map(physics_fact_to_event),
        );
        for player_id in &player_ids {
            if let Some(state) = self.modulation.get(player_id) {
                if let Some(player) = self.physics.get_player_mut(player_id) {
                    player.stamina = state.stamina * player.max_stamina;
                    player.morale = format!("{:?}", state.morale);
                }
            }
        }

        self.publish_events();

        self.build_tick()
    }

    fn morale_bias_for(&self, player_id: &str) -> f32 {
        let policy = &self.rules.modulation;
        match self.modulation.get(player_id).map(|m| m.morale) {
            Some(nba_decision::modulation::MoraleState::HotHand) => policy.hot_hand_bias,
            Some(nba_decision::modulation::MoraleState::Clutch) => policy.clutch_bias,
            Some(nba_decision::modulation::MoraleState::Normal) => 0.0,
            Some(nba_decision::modulation::MoraleState::Frustrated) => policy.frustrated_bias,
            Some(nba_decision::modulation::MoraleState::Exhausted) => policy.exhausted_bias,
            None => 0.0,
        }
    }

    fn publish_events(&mut self) {
        while !self.pending_events.is_empty() {
            let events = std::mem::take(&mut self.pending_events);
            let mut adjudicated_events = Vec::with_capacity(events.len() + 2);
            for event in events {
                if let GameEvent::Contact {
                    player_a, player_b, ..
                } = &event
                {
                    if let Some(contact) = self.latest_contacts.iter().find(|contact| {
                        contact.raw.entity_a == *player_a && contact.raw.entity_b == *player_b
                    }) {
                        if let ResolutionOutcome::Foul {
                            fouled_player_id,
                            fouler_id,
                            is_shooting,
                        } = ResolutionLayer::resolve_semantic_contact(
                            contact,
                            self.physics.get_players(),
                            &self.rules.resolve.contact,
                            &mut self.rng,
                        ) {
                            adjudicated_events.push(GameEvent::Foul {
                                fouled_player_id,
                                fouler_id,
                                is_shooting,
                            });
                        }
                    }
                }
                adjudicated_events.push(event);
            }
            let ctx = self.constraint_ctx();
            let findings = self
                .decision
                .registry
                .evaluate_events(&ctx, &adjudicated_events);

            for event in &adjudicated_events {
                if let GameEvent::Foul {
                    fouled_player_id,
                    fouler_id,
                    is_shooting,
                } = event
                {
                    if let Some(state) = self.modulation.get_mut(fouled_player_id) {
                        state.catch_equilibrium = (state.catch_equilibrium
                            - self.rules.semantics.contact_minor_speed_ratio * 0.2)
                            .clamp(0.4, 1.0);
                    }
                    if let Some(state) = self.modulation.get_mut(fouler_id) {
                        state.turnover_count = state.turnover_count.saturating_add(1);
                    }
                    let fouler_is_home = self
                        .physics
                        .get_player(fouler_id)
                        .map(|p| p.team == "home")
                        .unwrap_or(false);
                    let team_fouls = if fouler_is_home {
                        self.team_fouls_home = self.team_fouls_home.saturating_add(1);
                        self.team_fouls_home
                    } else {
                        self.team_fouls_away = self.team_fouls_away.saturating_add(1);
                        self.team_fouls_away
                    };
                    if let Some(player) = self.physics.get_player_mut(fouler_id) {
                        player.foul_count = player.foul_count.saturating_add(1);
                    }
                    // 犯满离场（charter 7：个人犯满上限为联赛档案参数）。
                    if self
                        .physics
                        .get_player(fouler_id)
                        .is_some_and(|p| p.foul_count >= self.rules.league.max_personal_fouls)
                    {
                        self.forced_substitution(fouler_id);
                    }
                    self.current_event = Some(
                        if *is_shooting {
                            "SHOOTING_FOUL"
                        } else {
                            "FOUL"
                        }
                        .to_string(),
                    );
                    if *is_shooting || team_fouls >= self.rules.league.bonus_fouls_per_period {
                        self.free_throws_remaining = if *is_shooting {
                            self.rules.league.shooting_foul_free_throws
                        } else {
                            self.rules.league.bonus_free_throws
                        };
                        // The fouled team keeps possession and shoots.
                        self.free_throw_shooter = Some(fouled_player_id.clone());
                        self.set_game_flow(GameFlowState::FreeThrow);
                        self.transition_phase(SubPhase::DeadBallReset);
                        // F1.1：罚球程序把球权威态收敛为 Dead（置于罚球点），
                        // 否则权威态仍是 Held{持球人} 而球在篮筐/罚球点，
                        // 投影出 BALL_WITH_HOLDER Hard（gap.md §5.1）。
                        let shooter_is_home = self
                            .physics
                            .get_player(fouled_player_id)
                            .map(|p| p.team == "home")
                            .unwrap_or(self.possession == Possession::Home);
                        let ft_spot = Court::free_throw_pos(shooter_is_home, &self.rules);
                        self.ball_pos_3d = (ft_spot, self.rules.ball_holder_height_ft);
                        self.transition_ball_state(
                            self.dead_state(ft_spot, self.rules.ball_holder_height_ft),
                        );
                    }
                }
            }

            let mut applied_keys = std::collections::HashSet::new();
            for finding in &findings {
                let key = finding.summary();
                // 只有 Violate 才触发强制项：Flagged 是咨询性信号（如非持球人的
                // BOUNDARY_CROSSING），不能生成 RuleViolation 事件——否则每个
                // 边界事实都会附带一条伪 VIOLATION/ENFORCEMENT_APPLIED。
                let is_violate = matches!(
                    finding.result.status,
                    nba_decision::constraint::ConstraintStatus::Violate { .. }
                );
                if is_violate
                    && finding.enforcement != EnforcementAction::None
                    && applied_keys.insert(key)
                {
                    self.apply_enforcement(finding);
                }
            }

            self.current_event_types.extend(
                adjudicated_events
                    .iter()
                    .map(|event| event.event_type_str().to_string()),
            );
            for event in adjudicated_events {
                self.event_sequence = self.event_sequence.saturating_add(1);
                // D4.1：分配全场唯一 event_id，并按因果父子关系链接：
                // 本 tick 的第一个事件成为该 tick 的因果根，后续同 tick 事件
                // 以它为父——这使得同一决策触发的多条事实（如
                // SHOT_RELEASE → SCORE）在账本上构成一条因果链，
                // 评判器不再需要用"事件窗口猜测"重建因果（gap.md §7.1/§7.2）。
                self.event_id_counter = self.event_id_counter.saturating_add(1);
                let event_id = self.event_id_counter;
                let kind = event.event_type_str().to_string();
                // D4.1：按语义槽位解析父事件，并登记本事件作为新的触发事件。
                let parent_event_id =
                    causal_parent_of(&kind).and_then(|slot| self.causal_links.get(slot).copied());
                if let Some(slot) = causal_trigger_slot(&kind) {
                    self.causal_links.insert(slot, event_id);
                }
                let data = serde_json::to_value(&event).ok();
                self.current_event_log.push(FrameEvent {
                    sequence: self.event_sequence,
                    time: (self.current_time * 100.0).round() / 100.0,
                    kind,
                    data,
                    event_id,
                    parent_event_id,
                });
            }
            if let Some(primary) = self.current_event_types.last() {
                self.current_event = Some(primary.clone());
            }
        }
    }

    fn apply_enforcement(&mut self, finding: &nba_decision::constraint::ConstraintFinding) {
        match &finding.enforcement {
            EnforcementAction::None
            | EnforcementAction::BlockAction
            | EnforcementAction::ModifyAction { .. }
            | EnforcementAction::DelayAction { .. }
            | EnforcementAction::TriggerFoul { .. }
            | EnforcementAction::ResetPosition => {}
            EnforcementAction::Violation { kind } => {
                let action = kind.as_str().to_string();
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.pending_events.push(GameEvent::RuleViolation {
                    constraint_id: finding.constraint.id.to_string(),
                    reason: action.clone(),
                });
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action,
                });
            }
            EnforcementAction::ChangePossession { reason } => {
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, reason));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action: format!("CHANGE_POSSESSION:{}", reason),
                });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
            EnforcementAction::EndPhase { next } => {
                let action = format!("END_PHASE:{}", next.as_str());
                self.current_enforcements
                    .push(format!("{}:{}", finding.constraint.id, action));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action,
                });
                if *next == PhaseType::DeadBallReset {
                    self.set_game_flow(GameFlowState::DeadBall);
                    self.transition_phase(SubPhase::DeadBallReset);
                }
            }
            EnforcementAction::Turnover { reason } => {
                self.current_enforcements
                    .push(format!("{}:TURNOVER:{}", finding.constraint.id, reason));
                self.pending_events.push(GameEvent::EnforcementApplied {
                    constraint_id: finding.constraint.id.to_string(),
                    action: format!("TURNOVER:{}", reason),
                });
                self.start_violation_turnover(ViolationKind::IllegalAction);
            }
        }
    }

    fn execute_drive(
        &mut self,
        driver_id: &str,
        from_pos: Vec2,
        target_pos: Vec2,
        move_kind: Option<nba_domain::action_window::DribbleMoveKind>,
        current_t: f32,
    ) {
        let Some(driver) = self.physics.get_player(driver_id).cloned() else {
            return;
        };
        let defender = self.physics.openness(driver_id);
        let lane_density =
            SemanticEvaluator::spacing(self.possession, target_pos, &self.physics, &self.rules)
                .paint_crowding;
        let stamina = (driver.stamina / driver.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0);
        let defender_id = defender.closest_defender_id.clone();
        let foul_rate = defender_id
            .as_ref()
            .map(|_| self.rules.resolve.base_rates.foul_on_drive_rate)
            .unwrap_or(0.0);
        let resolution = DriveResolution::resolve(
            self.rules.resolve.base_rates.drive_success,
            self.rules.resolve.shot_type_rates.drive_finish_2pt,
            foul_rate,
            driver.attributes.finishing,
            stamina,
            lane_density,
            defender.contest_intensity,
            self.rules.resolve.shot_type_block_bias.drive_finish,
            self.rules.resolve.player_skill.finishing_weight,
            &self.rules.resolve.drive,
            &mut self.rng,
        );
        let fouler_id = resolution.shooting_foul.then_some(defender_id).flatten();
        let target_pos = self
            .rules
            .court
            .clamp_playable(target_pos, self.rules.player_radius_ft);
        let drive_dist = (target_pos - from_pos).length();
        let drive_speed = (driver.max_speed_ftps * self.rules.tactics.drive_speed_ratio).max(1.0);
        // ## 时长必须包含加速坡（round-18）
        //
        // 原公式 `dist/speed` 假设瞬时达到极速。实测从静止加速
        // （max_player_accel 35 ft/s²）到 ~25 ft/s 需 ~0.7s、损失 ~9 ft
        // 里程——tau=1 时持球人仍距目标 5-10 ft，只能在 7-16 ft 抛投
        // （篮下≤4ft 出手占比 2.4%，真实 25-50%）。加入 `speed/accel`
        // 的加速坡项，使时长覆盖真实到达时间。
        let accel = self.rules.max_player_accel_ftps2.max(f32::EPSILON);
        let drive_duration = (drive_dist / drive_speed + drive_speed / accel).clamp(
            self.rules.tactics.drive_min_duration_seconds,
            self.rules.tactics.drive_max_duration_seconds,
        );
        let action_str = match move_kind {
            Some(nba_domain::action_window::DribbleMoveKind::Crossover) => "Crossover",
            Some(nba_domain::action_window::DribbleMoveKind::BetweenTheLegs) => "BetweenTheLegs",
            Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => "BehindTheBack",
            Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => "SpinMove",
            _ => "DriveToBasket",
        };
        let callout_text = match move_kind {
            Some(nba_domain::action_window::DribbleMoveKind::Crossover) => {
                format!("{} 变向晃开防守，大幅变向突破！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::BetweenTheLegs) => {
                format!("{} 胯下换手运球，加速直插内线！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::BehindTheBack) => {
                format!("{} 背后运球摆脱，直切篮下！", driver.jersey)
            }
            Some(nba_domain::action_window::DribbleMoveKind::SpinMove) => {
                format!("{} 陀螺转身过人，撕裂防线！", driver.jersey)
            }
            _ => format!("{} 持球强突，冲击篮筐！", driver.jersey),
        };
        // ## 被过掉的防守人必须真的被过掉（round-18）
        //
        // 根因链：突破预掷 `successful=true`，但物理层持球人直撞贴身防守人
        // （最近防守人 3.7 ft ≈ min_player_separation），碰撞消解每 tick
        // 清零速度——实测 0% 突破到达 ≤4.5ft，65% 停在离筐 18.6 ft。
        //
        // 语义对齐（与拦截锚定球到抢断者同一原则——概率在事件时刻裁定，
        // 事实随后回放）：`successful` 意味着**过掉了对位防守人**。把该
        // 防守人实际位移出突破走廊（侧向 + 分离余量），他随后由防守战术
        // 重新追防——这正是真实篮球「被过掉后回追」的几何。
        let mut target_pos_override: Option<Vec2> = None;
        if resolution.successful {
            // ## 过人变向（round-18 修订版：绕行而非瞬移）
            //
            // 初版把被过的防守人瞬移出通道——触发 PLAYER_TELEPORT 不变量
            // （实测 70-94 Hard/seed）。不变量是对的：位置跳变是伪造事实。
            //
            // 物理一致的过人语义：**持球人变向绕过**防守人（真实的 crossover
            // 几何）。突破目标点侧移一个分离余量，路径绕开贴身防守人；
            // 防守人随后由战术层追防（真实「被过掉后回追」）。
            let drive_dir = (target_pos - from_pos).normalize_or_zero();
            let perp = Vec2::new(-drive_dir.y, drive_dir.x);
            // ## 让位整条通道（round-19：从单人到全体）
            //
            // round-18 只让位**最近的一名**通道内防守人——过掉第一人对
            // 后，护框者（第二道防线）仍在篮下挡住最后几米，实测篮下
            // ≤4ft 出手仅 3.8%（真实 25-50%）。
            //
            // 语义：`successful` 预掷的是「这次突破**整体**打成了」——
            // 包括过掉对位人与顶开/绕过护框。因此通道内**所有**防守人
            // 都应让位（各自向远离突破方向的侧向清空点极速移动）；
            // 对抗强度已由 successful 的掷骰（防守能力加权）承担，
            // 几何层只负责让事实成立。
            let clear = self.rules.min_player_separation_ft * 2.0 + self.rules.player_radius_ft;
            let mut beaten_ids: Vec<String> = Vec::new();
            let mut beat_spots: Vec<(String, Vec2, f32, String, String)> = Vec::new();
            for q in self.physics.get_players().values() {
                if !q.on_court || q.team == driver.team {
                    continue;
                }
                let rel = q.pos_ft - from_pos;
                let along = rel.dot(drive_dir);
                let lateral = (rel - drive_dir * along).length();
                if along > 0.0
                    && along < drive_dist
                    && lateral < self.rules.tactics.drive_lane_offset_ft.max(4.0)
                {
                    let side = if perp.dot(rel) >= 0.0 { -1.0 } else { 1.0 };
                    let spot = self.rules.court.clamp_playable(
                        q.pos_ft + perp * side * clear,
                        self.rules.player_radius_ft,
                    );
                    beat_spots.push((
                        q.id.clone(),
                        spot,
                        q.max_speed_ftps,
                        q.slot.clone(),
                        q.morale.clone(),
                    ));
                    beaten_ids.push(q.id.clone());
                }
            }
            // 主对位人（离持球人最近者）驱动两段式过人几何；其余
            // （护框者等）只让位，不参与重定向判定。
            let primary = self
                .physics
                .get_players()
                .values()
                .filter(|q| beaten_ids.contains(&q.id))
                .min_by(|a, b| {
                    (a.pos_ft - from_pos)
                        .length_squared()
                        .partial_cmp(&(b.pos_ft - from_pos).length_squared())
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .map(|q| q.id.clone());
            for (bid, spot, sp, slot, morale) in beat_spots {
                self.physics
                    .set_player_target(&bid, spot, sp, "BeatenRecovery", &slot, &morale);
                self.beaten_recovery_until.insert(
                    bid,
                    self.current_time + self.rules.tactics.drive_beaten_recovery_seconds,
                );
            }
            if let Some(pid) = primary {
                if let Some(q) = self.physics.get_player(&pid) {
                    let side = if perp.dot(q.pos_ft - from_pos) >= 0.0 {
                        -1.0
                    } else {
                        1.0
                    };
                    let beat_spot = self.rules.court.clamp_playable(
                        q.pos_ft + perp * side * clear,
                        self.rules.player_radius_ft,
                    );
                    // 两段式过人几何（真实 crossover）：
                    //   第一段：持球人目标 = 过人点；第二段：重定向攻框。
                    target_pos_override = Some(beat_spot);
                    self.beaten_defender_id = Some(pid);
                }
            }
        }
        let initial_target = target_pos_override.unwrap_or(target_pos);
        self.physics.set_player_target(
            driver_id,
            initial_target,
            drive_speed,
            action_str,
            "BallHandler",
            &driver.morale,
        );
        self.transition_ball_state(BallTrajectoryKind::Drive {
            driver_id: driver_id.to_string(),
            from_pos,
            target_pos,
            move_kind,
            start_time: current_t,
            duration: drive_duration,
            successful: resolution.successful,
            finish_made: resolution.finish_made,
            fouler_id,
        });
        self.ball_pos_3d = (from_pos, self.rules.ball_holder_height_ft);
        self.transition_phase(SubPhase::ActionExecution);
        self.pending_events.push(GameEvent::DriveInitiated {
            driver_id: driver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            target_pos: (target_pos.x, target_pos.y),
        });
        self.current_event = Some("DRIVE_INITIATED".to_string());
        self.current_callout = Some(callout_text);
        self.current_intensity = Some("Climax".to_string());
    }

    /// Resolve shot quality at release; the resulting outcome is then replayable
    /// independently of the later presentation trajectory.
    fn execute_shot(
        &mut self,
        shooter_id: &str,
        from_pos: Vec2,
        is_three_hint: bool,
        jumper_kind: Option<nba_domain::action_window::JumperKind>,
        current_t: f32,
    ) {
        let is_home = self.possession == Possession::Home;
        let hoop = self.rules.court.hoop_pos(is_home);
        let shooter = self.physics.get_player(shooter_id);
        let shooter_pos = shooter.map(|player| player.pos_ft).unwrap_or(from_pos);
        let dist_to_hoop = (shooter_pos - hoop).length();
        // 底角三分是更近的直线（NBA 22ft vs 弧顶 23.75ft），必须几何判定。
        let is_three_by_distance = self.rules.court.is_three_point_attempt(
            shooter_pos,
            is_home,
            self.rules.league.three_point_distance_ft,
            self.rules.league.corner_three_distance_ft,
        );
        let is_three = is_three_by_distance || is_three_hint;
        let openness = self.physics.openness(shooter_id);
        let spacing_bonus = self
            .latest_spacing
            .map(|spacing| spacing.shot_quality_bonus)
            .unwrap_or(0.0);
        let skill = shooter
            .map(|player| {
                if dist_to_hoop < self.rules.rim_shot_distance_ft {
                    player.attributes.finishing
                } else if is_three {
                    player.attributes.shooting_three
                } else {
                    player.attributes.shooting_mid
                }
            })
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let stamina = shooter
            .map(|player| (player.stamina / player.max_stamina.max(f32::EPSILON)).clamp(0.0, 1.0))
            .unwrap_or(1.0);
        let skill_adjustment =
            (skill - 0.5) * self.rules.resolve.player_skill.shooting_weight * 2.0;
        let stamina_adjustment = (stamina - 1.0) * self.rules.resolve.player_skill.shooting_weight;
        let contest_penalty = openness.contest_intensity * self.rules.shot_contest_sensitivity;
        let base_fg = if dist_to_hoop < self.rules.rim_shot_distance_ft {
            self.rules.resolve.base_rates.shot_make_2pt
        } else if is_three {
            self.rules.resolve.base_rates.shot_make_3pt
        } else {
            self.rules.resolve.base_rates.shot_make_2pt
        };
        let final_fg_pct = (base_fg + skill_adjustment + stamina_adjustment + spacing_bonus
            - contest_penalty)
            .clamp(self.rules.shot_pct_floor, self.rules.shot_pct_ceiling);
        let is_made = self.rng.gen_bool(final_fg_pct as f64);

        // 峰值必须服从规则通道的高度上限：base + dist×factor 在超远距离
        // （约 >84ft）会算出高于 `ball_z_max_ft` 的弧顶，直接违反
        // BALL_HEIGHT_BOUNDS（本轮 seed 6 full 实测 35.17ft > 35.0ft）。
        // 在生成端收敛到上限，而不是事后由不变量检查器发现。
        let peak_z = (self.rules.shot_peak_base_ft
            + dist_to_hoop * self.rules.shot_peak_distance_factor)
            .min(self.rules.ball_z_max_ft);
        let flight_time = BallisticsEngine::shot_duration(dist_to_hoop, peak_z, &self.rules);
        self.active_windows.insert(
            shooter_id.to_string(),
            ActionTimeWindow::new_jump_shot(shooter_id, current_t, &self.rules),
        );
        self.current_possession_shooter = Some(shooter_id.to_string());
        self.current_possession_contest = Some(openness.contest_intensity);

        let release_pos = self.ball_pos_3d.0;
        self.transition_ball_state(BallTrajectoryKind::Shot {
            shooter_id: shooter_id.to_string(),
            from_pos: release_pos,
            hoop_pos: hoop,
            start_time: current_t,
            duration: flight_time,
            is_made,
            is_three,
            peak_z,
        });
        self.transition_phase(SubPhase::ShotAttempt);
        self.pending_events.push(GameEvent::ShotRelease {
            shooter_id: shooter_id.to_string(),
            pos: (release_pos.x, release_pos.y),
            is_three,
            contest_level: openness.contest_intensity,
            make_probability: final_fg_pct,
        });
        let action_name = match jumper_kind {
            Some(nba_domain::action_window::JumperKind::StepBack) => "StepBackShot",
            Some(nba_domain::action_window::JumperKind::PullUp) => "PullUpShot",
            Some(nba_domain::action_window::JumperKind::TurnaroundFadeaway) => "TurnaroundFadeaway",
            _ => {
                if is_three {
                    "ThreePointShot"
                } else {
                    "JumpShot"
                }
            }
        };
        let callout_detail = match jumper_kind {
            Some(nba_domain::action_window::JumperKind::StepBack) => {
                "撤步拉开空间，命中高难度后撤步！"
            }
            Some(nba_domain::action_window::JumperKind::PullUp) => "急停干拔，教科书般起跳出手！",
            Some(nba_domain::action_window::JumperKind::TurnaroundFadeaway) => {
                "翻身极致后仰，飘逸出手！"
            }
            _ => {
                if is_three {
                    "果断张手三分出手！"
                } else {
                    "迎着防守干拔跳投！"
                }
            }
        };
        if let Some(p) = self.physics.get_player_mut(shooter_id) {
            p.action = action_name.to_string();
            let hoop_dir = (hoop - p.pos_ft).normalize_or_zero();
            if hoop_dir.length_squared() > 0.1 {
                p.facing_dir = hoop_dir;
            }
        }
        self.current_possession_turnover_player = Some(shooter_id.to_string());
        let shooter_name = self
            .physics
            .get_player(shooter_id)
            .map(|p| p.jersey.clone())
            .unwrap_or_else(|| shooter_id.to_string());
        self.current_callout = Some(format!("{} {}", shooter_name, callout_detail));
        self.current_intensity = Some("Climax".to_string());
    }

    fn resolve_free_throw(&mut self) {
        if self.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_attributes = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.attributes.clone())
            .unwrap_or_default();
        let made = self
            .rng
            .gen_bool(nba_domain::free_throw_probability(&self.rules, &shooter_attributes) as f64);
        let shooter_is_home = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.possession == Possession::Home);
        let attempt = self.free_throw_attempt.saturating_add(1);
        let ft_pos = Court::free_throw_pos(shooter_is_home, &self.rules);
        // F1.1：罚球是停表的显式事件链。出手前把球权威态保持在罚球点
        // 的 Dead 状态，不得让 ball_pos_3d 指向篮筐而权威态仍为 Held。
        self.ball_pos_3d = (ft_pos, self.rules.ball_holder_height_ft);
        if !matches!(self.ball_state, BallTrajectoryKind::Dead { .. }) {
            self.transition_ball_state(self.dead_state(ft_pos, self.rules.ball_holder_height_ft));
        } else if let BallTrajectoryKind::Dead { pos, z, .. } = &mut self.ball_state {
            *pos = ft_pos;
            *z = self.rules.ball_holder_height_ft;
        }
        self.pending_events.push(GameEvent::FreeThrowAttempt {
            shooter_id: shooter_id.clone(),
            attempt,
            made,
        });
        self.box_score.ft_attempts += 1;
        if made {
            self.box_score.ft_made += 1;
            if shooter_is_home {
                self.home_score += 1;
            } else {
                self.away_score += 1;
            }
        }
        self.free_throw_attempt = attempt;
        self.free_throws_remaining = self.free_throws_remaining.saturating_sub(1);
        self.sub_phase_timer = 0.0;
        if self.free_throws_remaining == 0 {
            self.free_throw_shooter = None;
            self.free_throw_attempt = 0;
            if made {
                // A made final free throw closes the offense's possession.
                // Emit the score summary before the inbound helper calls
                // complete_possession(), so the boundary has a causal fact.
                self.current_possession_shooter = Some(shooter_id.clone());
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::Score,
                    None,
                    None,
                    None,
                );
                let hoop = self.rules.court.hoop_pos(shooter_is_home);
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(hoop, self.ball_pos_3d);
            } else {
                self.start_free_throw_rebound(shooter_is_home, ft_pos);
            }
        }
    }
    fn start_free_throw_rebound(&mut self, shooter_is_home: bool, ft_pos: Vec2) {
        let hoop = self.rules.court.hoop_pos(shooter_is_home);
        let rebound_from = (hoop, self.rules.rim_height_ft);
        let landing_spot =
            BallisticsEngine::compute_rebound_landing(ft_pos, hoop, &mut self.rng, &self.rules);
        self.ball_pos_3d = (rebound_from.0, rebound_from.1);
        self.shot_clock = self.rules.league.offensive_rebound_shot_clock_seconds;
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::FlightAndRebound);
        let rebound = BallTrajectoryKind::RimRebound {
            from_pos: rebound_from.0,
            from_z: rebound_from.1,
            hoop_pos: hoop,
            target_landing: landing_spot.landing_pos,
            start_time: self.current_time,
            duration: landing_spot.flight_duration,
            peak_z: self.rules.rebound_peak_ft,
            last_touch_team: self.possession,
        };
        if matches!(
            self.ball_state,
            BallTrajectoryKind::Pass { .. }
                | BallTrajectoryKind::ControlTransfer { .. }
                | BallTrajectoryKind::InboundTransfer { .. }
                | BallTrajectoryKind::InboundReady { .. }
                | BallTrajectoryKind::LooseBall { .. }
        ) {
            self.transition_ball_state(self.dead_state(rebound_from.0, rebound_from.1));
        }
        self.transition_ball_state(rebound);
    }

    /// Test/diagnostic hook: resolve the current free throw with a forced outcome.
    pub fn resolve_forced_free_throw(&mut self, made: bool) {
        if self.free_throws_remaining == 0 {
            return;
        }
        let shooter_id = self
            .free_throw_shooter
            .clone()
            .unwrap_or_else(|| self.new_possession_pg());
        let shooter_is_home = self
            .physics
            .get_player(&shooter_id)
            .map(|p| p.team == "home")
            .unwrap_or(self.possession == Possession::Home);
        let attempt = self.free_throw_attempt.saturating_add(1);
        self.pending_events.push(GameEvent::FreeThrowAttempt {
            shooter_id: shooter_id.clone(),
            attempt,
            made,
        });
        if made {
            if shooter_is_home {
                self.home_score += 1;
            } else {
                self.away_score += 1;
            }
        }
        self.free_throw_attempt = attempt;
        self.free_throws_remaining = self.free_throws_remaining.saturating_sub(1);
        self.sub_phase_timer = 0.0;
        if self.free_throws_remaining == 0 {
            self.free_throw_shooter = None;
            self.free_throw_attempt = 0;
            if made {
                self.current_possession_shooter = Some(shooter_id.clone());
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::Score,
                    None,
                    None,
                    None,
                );
                self.transition_phase(SubPhase::Initiation);
                self.set_game_flow(GameFlowState::DeadBall);
                self.start_inbound_transition(Court::hoop_pos(shooter_is_home), self.ball_pos_3d);
            } else {
                let hoop = self.rules.court.hoop_pos(shooter_is_home);
                self.ball_pos_3d = (hoop, self.rules.rim_height_ft);
                self.start_free_throw_rebound(
                    shooter_is_home,
                    Court::free_throw_pos(shooter_is_home, &self.rules),
                );
            }
        }
    }

    fn execute_pass(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        _from_pos: Vec2,
        to_pos: Vec2,
        current_t: f32,
        inbound: bool,
    ) {
        let from_pos = self.ball_pos_3d.0;

        let _initial_dist = self
            .physics
            .get_player(receiver_id)
            .map(|p| (p.pos_ft - from_pos).length())
            .unwrap_or(20.0);
        // 领传由**决策层**给出（`CandidateAction::Pass.to_pos` 已含提前量），
        // 此处**不得**再叠加一次——实测叠加后 `PASS_CORRIDOR_REACHABLE`
        // 由 9 条恶化到 28 条（接收人因减速模型无法到达过远的落点）。
        // outlet 一传走 `start_rebound_outlet`，那条路径没有决策层，故单独领传。
        let target_lead_pos = to_pos;
        self.active_windows.insert(
            passer_id.to_string(),
            ActionTimeWindow::new_pass(passer_id, current_t, &self.rules),
        );
        let pass_dist = (target_lead_pos - from_pos).length();
        let duration = self.rules.pass_duration(pass_dist, inbound);
        self.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.receiver_estimate = None;
        self.pending_pass_inbound = inbound;
        let receive_success =
            self.resolve_pass_success(passer_id, receiver_id, from_pos, target_lead_pos);
        // 第一性原理修复（本轮）：拦截必须在**传球释放时裁定一次**。
        //
        // 原实现把拦截放在逐 tick 的弹道循环里：每个 tick 遍历所有防守者、
        // 每人独立掷骰。于是失败概率随时长累积 —— 一次 0.45–1.4s（11–35 tick）
        // 的传球，若 1–2 名防守者处于判定范围内，至少失败一次的概率接近 1
        // （实测每次传球失败 35.8%，真实 NBA 约 8–10%）。
        //
        // 概率的语义是「这次传球是否被拦截」，不是「这个 tick 是否被拦截」，
        // 因此必须一次性裁定，并把结果作为事实随弹道携带（与
        // `receive_success` 同一模式）。
        let intercept =
            self.resolve_pass_interception(passer_id, receiver_id, from_pos, target_lead_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos,
            to_pos: target_lead_pos,
            target_id: receiver_id.to_string(),
            start_time: current_t,
            duration,
            peak_z: self.rules.pass_peak_ft,
            inbound,
            receive_success,
            intercept,
        });
        self.last_passer_id = Some(passer_id.to_string());
        self.current_possession_turnover_player = Some(passer_id.to_string());
        self.transition_phase(SubPhase::ActionExecution);
        // 第一性原理：`passes_count` 是「**尝试**传球次数」，应在释放时计数。
        //
        // 原实现只在 `PASS_RECEIVED` 时 `+= 1`，于是掉球/点掉/抢断的传球
        // 完全不被计入——实测 17/60 回合的 `passes_count` 与事件流不一致
        // （申报 0、实际 1–3）。这既污染了 L2 的 ACTION_COMPOSITION_PASSES
        // 准则，也让"每回合传球 1.26 次"的结论本身不可信。
        self.current_possession_passes += 1;
        self.pending_events.push(GameEvent::PassRelease {
            passer_id: passer_id.to_string(),
            receiver_id: receiver_id.to_string(),
            from_pos: (from_pos.x, from_pos.y),
            to_pos: (target_lead_pos.x, target_lead_pos.y),
        });
        if let Some(index) = self.player_index_for_id(receiver_id) {
            self.carrier_idx = index;
        }
        // F1.3：发球员从界外 placement 回场内由 `sync_ball_holder` 在
        // 球态离开 InboundTransfer/InboundReady 时统一处理（单一机制）。
        self.current_event = Some(if inbound { "INBOUND_PASS" } else { "PASS" }.to_string());
        self.current_callout = Some(if inbound {
            "界外发球进入飞行，接应点开始读取防守".to_string()
        } else {
            "突分策应！外线转移球创造空位机会".to_string()
        });
    }

    /// 传球拦截的一次性裁定（第一性原理修复）。
    ///
    /// 语义：**这次传球**是否被某名防守者拦截，而不是「某个 tick 是否被拦截」。
    /// 因此只在释放时刻对每名相关防守者评估一次，取风险最高者作为拦截者。
    ///
    /// 判定依据（几何 + 能力，均为释放时刻的事实）：
    /// - 防守者到传球线段的垂距（lane clearance）：越近越可能碰到球；
    /// - 防守者是否处于球道高度可达范围（球在 4ft，防守者可伸手）；
    /// - 防守者的 `steal` 能力。
    ///
    /// 返回 `Some((defender_id, is_steal))`：`true` = 抢断，`false` = 点掉。
    fn resolve_pass_interception(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        from_pos: Vec2,
        to_pos: Vec2,
    ) -> Option<(String, bool)> {
        let def_team = match self.physics.get_player(passer_id).map(|p| p.team.as_str()) {
            Some("home") => "away",
            Some("away") => "home",
            _ => return None,
        };
        let segment = to_pos - from_pos;
        let segment_length_sq = segment.length_squared();
        if segment_length_sq <= f32::EPSILON {
            return None;
        }
        let policy = &self.rules.resolve.base_rates;
        let zero = f32::from(0u8);
        let one = f32::from(1u8);
        let half = one / f32::from(2u8);
        let scale = policy.intercept_clearance_scale_ft.max(f32::EPSILON);
        let reach = self.rules.player_radius_ft + self.rules.defender_reach_ft;

        let mut defenders: Vec<(String, f32, f32)> = self
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == def_team && p.id != receiver_id)
            .filter_map(|p| {
                let projection_t =
                    ((p.pos_ft - from_pos).dot(segment) / segment_length_sq).clamp(0.0, 1.0);
                let closest = from_pos + segment * projection_t;
                let clearance = (p.pos_ft - closest).length();
                // 只有在球道可达范围内才算「有机会碰到球」。
                if clearance > reach {
                    return None;
                }
                Some((p.id.clone(), clearance, p.attributes.steal))
            })
            .collect();
        // 确定性顺序：风险最高者优先评估（数值相同时按 id 排序）。
        defenders.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        for (defender_id, clearance, steal_skill) in defenders {
            let base_contest = (one - (clearance / scale)).clamp(zero, one);
            let skill_factor = half + steal_skill.clamp(zero, one);
            let steal_prob = (base_contest * policy.intercept_steal_slope * skill_factor)
                .clamp(policy.intercept_steal_floor, policy.intercept_steal_ceiling);
            let tip_prob = (base_contest * policy.intercept_tip_slope * skill_factor)
                .clamp(policy.intercept_tip_floor, policy.intercept_tip_ceiling);
            let roll = self.rng.gen::<f32>();
            if roll < steal_prob {
                return Some((defender_id, true));
            }
            if roll < steal_prob + tip_prob {
                return Some((defender_id, false));
            }
        }
        None
    }

    /// 贴身切球成立时的事实应用：球脱手 → loose ball → 失误归因。
    ///
    /// 与 `PassTipped` 的 loose ball 路径同构：球从持球人处弹出，
    /// 双方争抢；`pending_loose_ball_terminal` 记录**若攻方未能夺回**时的
    /// 回合终止原因（`TurnoverLooseBall` = 带球丢球）。
    ///
    /// 弹出方向：以防守人→持球人方向为基准（切球动作把球从护球位置拨走），
    /// 叠加由 `poke_deflection_spread_rad` 限幅的确定性伪随机偏转。
    fn apply_on_ball_poke(&mut self, carrier_id: &str, defender_id: &str) {
        let Some(carrier_pos) = self.physics.get_player(carrier_id).map(|p| p.pos_ft) else {
            return;
        };
        let Some(defender_pos) = self.physics.get_player(defender_id).map(|p| p.pos_ft) else {
            return;
        };
        let spread = self.rules.resolve.ball_security.poke_deflection_spread_rad;
        // 基准方向：防守人 → 持球人（球被从持球人身上拨离防守人方向）。
        let base_dir = (carrier_pos - defender_pos).normalize_or_zero();
        let base_dir = if base_dir.length_squared() <= f32::EPSILON {
            // 完全重叠时退化：取持球人朝进攻篮筐方向，保持确定性。
            let hoop = self
                .rules
                .court
                .hoop_pos(self.possession == Possession::Home);
            (hoop - carrier_pos).normalize_or_zero()
        } else {
            base_dir
        };
        let angle = (self.rng.gen::<f32>() * 2.0 - 1.0) * spread;
        let (sin_a, cos_a) = angle.sin_cos();
        let dir = Vec2::new(
            base_dir.x * cos_a - base_dir.y * sin_a,
            base_dir.x * sin_a + base_dir.y * cos_a,
        );
        // 球的位置取**球当前的实际坐标**，而不是持球人的身体坐标。
        //
        // 持球时球有 `ball_holder_offset_ft` 的前向/侧向偏移与弹跳相位
        // （`ballistics.rs` 的 `Held` 分支），两者并不相等。若用
        // `carrier_pos` 作为松球起点，球会在单帧内跳变该偏移量——实测
        // 3.87 ft/tick，被 `BALL_SPEED` 不变量判为 96.72 ft/s（超上限 85）。
        let ball_pos = self.ball_pos_3d.0;
        // 切球是**小幅拨离**，不是全速发射：球原在持球人手里（近乎静止），
        // 被拨一下只能获得有限初速。取「绝对上限」与「本次球速上限的比例」
        // 的较小者，保证不同规则档案下都不越过 `BALL_SPEED` 不变量。
        let policy = &self.rules.resolve.ball_security;
        let cap = (self.rules.ball_max_speed_ftps - self.rules.invariant_speed_tolerance_ftps)
            .max(self.rules.invariant_speed_tolerance_ftps);
        let speed = policy
            .poke_ball_speed_ftps
            .min(cap * policy.poke_ball_speed_ratio)
            .max(0.0);

        self.pending_events.push(GameEvent::BallPokedLoose {
            handler_id: carrier_id.to_string(),
            defender_id: defender_id.to_string(),
            position: (ball_pos.x, ball_pos.y),
        });
        // 归因：带球丢球（真实 NBA 占比最大的失误类型）。
        self.pending_loose_ball_terminal = Some(nba_domain::PossessionEndCause::TurnoverLooseBall);
        self.current_possession_turnover_player = Some(carrier_id.to_string());
        // 传球链断：丢弃接球人估计与传球人记录。
        self.receiver_estimate = None;
        self.last_passer_id = None;
        self.pending_pass_receiver = None;
        self.transition_ball_state(BallTrajectoryKind::LooseBall {
            pos: ball_pos,
            vel: dir * speed,
            // 沿用球**当前的** z，与 `loose_ball_from` 同源。
            //
            // 不可硬置 `ball_holder_height_ft`：那会在长下落中让重力把
            // |v| 累积到超过 `ball_max_speed_ftps`，触发 `BALL_SPEED` Hard
            // 违规（实测 seed 31337 tick 3755 → 96.72 ft/s > 85）。
            // 球被切掉时的高度是**事实**，不应被重置。
            z: self.ball_pos_3d.1,
            vel_z: 0.0,
            last_touch_team: self.possession,
        });
    }

    /// 贴身切球（on-ball poke check）的一次性裁定。
    ///
    /// ## 语义
    ///
    /// 回答「**这次持球暴露**是否被防守者切掉」，而不是「这个 tick 是否被切」。
    /// 与 `resolve_pass_interception` 同形：**在事件发生的那一刻裁定一次**，
    /// 结果作为事实（loose ball）传播。因此不存在逐 tick 概率累积。
    ///
    /// ## 为什么需要它（round-13 结构发现）
    ///
    /// 真实 NBA 失误构成中「带球丢球」占 **53.6%**（82games 2024-25 IND），
    /// 是占比最大的一类。此前引擎只有传球失败一条失误路径，
    /// `TurnoverLooseBall` 在 718 回合中只出现 1 次。
    ///
    /// ## 判定依据（均为当前时刻的事实）
    ///
    /// - 防守者是否进入 `poke_pressure_radius_ft`（几何可达）；
    /// - 持球人是否确实处于 `Held` 且未被动作窗口锁定
    ///   （锁定 = 正在投篮/传球，球不在护球状态，由调用方保证）；
    /// - 概率由 `capability::poke_check_success` 从**技能与倾向**派生；
    /// - 逐 tick 按 `poke_attempt_rate_per_sec × dt` 折算尝试频率，
    ///   使「贴身持续越久、被切风险越大」在**时间积分**意义上成立，
    ///   而不是让单 tick 概率随时长累积成必然。
    ///
    /// 返回 `Some(defender_id)` 表示切球成立。
    fn resolve_on_ball_poke(
        &mut self,
        carrier_id: &str,
        dt: f32,
        lock_kinematics: bool,
    ) -> Option<String> {
        // 动作窗口锁定（投篮/传球/上篮进行中）时球不在护球状态。
        if lock_kinematics {
            return None;
        }
        if !matches!(self.ball_state, BallTrajectoryKind::Held { .. }) {
            return None;
        }
        let policy = self.rules.resolve.ball_security.clone();
        let offense_team = self
            .physics
            .get_player(carrier_id)
            .map(|p| p.team.clone())?;
        let def_team = match offense_team.as_str() {
            "home" => "away",
            _ => "home",
        };
        let handler_attrs = self
            .physics
            .get_player(carrier_id)
            .map(|p| p.attributes.clone())?;
        let handler_risk = self
            .physics
            .get_player(carrier_id)
            .map(|p| p.tendencies.risk_tolerance)
            .unwrap_or(0.5);
        let carrier_pos = self
            .physics
            .get_player(carrier_id)
            .map(|p| p.pos_ft)
            .unwrap_or(self.ball_pos_3d.0);

        // 只有贴身到压力半径内的防守者才构成切球威胁。
        let mut threats: Vec<(String, f32)> = self
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == def_team)
            .filter_map(|p| {
                let distance = (p.pos_ft - carrier_pos).length();
                if distance > policy.poke_pressure_radius_ft {
                    return None;
                }
                Some((p.id.clone(), distance))
            })
            .collect();
        // 确定性顺序：距离最近者优先（数值相同时按 id 排序）。
        threats.sort_by(|a, b| {
            a.1.partial_cmp(&b.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.0.cmp(&b.0))
        });

        // 时间积分：贴身持续 dt 秒相当于 rate × dt 次尝试机会。
        // 单次尝试概率 p，至少成功一次的概率 = 1 − (1−p)^trials。
        // 这样「贴得更久」以幂律逼近 1，但单 tick 概率不随时长累积。
        let trials = (policy.poke_attempt_rate_per_sec * dt.max(0.0)).max(0.0);
        if trials <= f32::EPSILON {
            return None;
        }
        for (defender_id, distance) in threats {
            let Some(defender_attrs) = self
                .physics
                .get_player(&defender_id)
                .map(|p| p.attributes.clone())
            else {
                continue;
            };
            let per_try = nba_domain::capability::poke_check_success(
                &self.rules,
                &defender_attrs,
                &handler_attrs,
                handler_risk,
            );
            // 距离越远越难切到：压力半径边缘处线性衰减至 0。
            let proximity = (1.0 - distance / policy.poke_pressure_radius_ft).clamp(0.0, 1.0);
            let p = (per_try * proximity).clamp(0.0, 1.0);
            let miss_all = (1.0 - p).powf(trials);
            let hit_chance = 1.0 - miss_all;
            if self.rng.gen::<f32>() < hit_chance {
                return Some(defender_id);
            }
        }
        None
    }

    /// Resolve a pass once at release from spatial facts, player capabilities,
    /// and the configured officiating policy. Arrival only replays this fact.
    fn resolve_pass_success(
        &mut self,
        passer_id: &str,
        receiver_id: &str,
        from_pos: Vec2,
        to_pos: Vec2,
    ) -> bool {
        let (Some(passer), Some(receiver)) = (
            self.physics.get_player(passer_id).cloned(),
            self.physics.get_player(receiver_id).cloned(),
        ) else {
            return false;
        };
        let evaluation = SemanticEvaluator::pass(
            from_pos,
            to_pos,
            passer_id,
            receiver_id,
            &self.physics,
            &self.rules,
        );
        let catch_equilibrium = self
            .modulation
            .get(receiver_id)
            .map(|state| state.catch_equilibrium)
            .unwrap_or(1.0);
        matches!(
            ResolutionLayer::resolve_pass_arrival(
                &passer,
                &receiver,
                evaluation.target_openness,
                catch_equilibrium,
                &self.rules.resolve.pass,
                self.rules.resolve.base_rates.pass_success,
                &mut self.rng,
            ),
            ResolutionOutcome::PassReceived { .. }
        )
    }

    /// Resolves the rebound winner from the landing window, then delegates
    /// the contest probability to the officiating layer.
    fn try_resolve_rebounder(&mut self, landing: Vec2, max_reach: f32) -> Option<String> {
        let defensive_team = match self.possession {
            Possession::Home => "away",
            Possession::Away => "home",
        };
        let offensive_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let mut offensive_candidates = self.rebound_candidates(landing, offensive_team, max_reach);
        let mut defensive_candidates = self.rebound_candidates(landing, defensive_team, max_reach);
        offensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        defensive_candidates.sort_by(|left, right| {
            left.1
                .total_cmp(&right.1)
                .then_with(|| left.0.cmp(&right.0))
        });

        if offensive_candidates.is_empty() && defensive_candidates.is_empty() {
            return None;
        }
        let Some((offensive_id, offensive_distance)) = offensive_candidates.first() else {
            return defensive_candidates.first().map(|(id, _)| id.clone());
        };
        let Some((defensive_id, defensive_distance)) = defensive_candidates.first() else {
            return Some(offensive_id.clone());
        };
        let Some(offensive_player) = self.physics.get_player(offensive_id).cloned() else {
            return Some(defensive_id.clone());
        };
        let Some(defensive_player) = self.physics.get_player(defensive_id).cloned() else {
            return Some(offensive_id.clone());
        };

        match ResolutionLayer::resolve_rebound(
            &offensive_player,
            &defensive_player,
            *offensive_distance,
            *defensive_distance,
            &self.rules.resolve.rebound,
            &mut self.rng,
        ) {
            ResolutionOutcome::ReboundSecured { rebounder_id, .. } => Some(rebounder_id),
            _ => Some(defensive_id.clone()),
        }
    }

    #[allow(dead_code)]
    fn resolve_rebounder(&mut self, landing: Vec2) -> String {
        let max_r = self.rules.player_radius_ft + self.rules.defender_reach_ft + 1.5;
        self.try_resolve_rebounder(landing, max_r)
            .or_else(|| {
                let search_radius = self.rules.court.width_ft.hypot(self.rules.court.height_ft);
                self.try_resolve_rebounder(landing, search_radius)
            })
            .unwrap_or_else(|| self.new_possession_pg())
    }

    fn rebound_candidates(
        &self,
        landing: Vec2,
        team: &str,
        search_radius: f32,
    ) -> Vec<(String, f32)> {
        self.physics
            .query_nearby(
                landing,
                search_radius,
                &nba_physics::EntityFilter::Team(team.to_string()),
            )
            .into_iter()
            .filter_map(|id| {
                self.physics
                    .get_player(&id)
                    .map(|player| (id, (player.pos_ft - landing).length()))
            })
            .collect()
    }

    fn update_scope_completion(&mut self) {
        if !self.scope_active || self.simulation_complete {
            return;
        }
        let reached = match self.scope_boundary {
            ScopeBoundary::Possessions => self.completed_possessions >= self.target_possessions,
            ScopeBoundary::Period { last_period } => {
                self.period > last_period
                    || (self.period == last_period
                        && matches!(
                            self.game_flow,
                            GameFlowState::QuarterEnd | GameFlowState::Halftime
                        ))
            }
            ScopeBoundary::Game => self.game_flow == GameFlowState::GameEnd,
        };
        if reached {
            self.simulation_complete = true;
            self.settle_scope_ball(self.ball_pos_3d);
        }
    }

    // ========================================================================
    // Possession boundary
    fn complete_possession(&mut self) {
        // D0.1（dev 方案 §3.2）：`UNATTRIBUTED_END` 兜底已物理删除。
        // 到达回合边界时必须已有带显式 `PossessionEndCause` 的总结；
        // 缺总结 = 因果链破缺，构成 Hard 缺陷，测试断言其不发生而非兜底。
        let count = self.completed_possessions as u64;
        debug_assert_eq!(
            self.last_possession_summary_index,
            Some(count),
            "possession {count} reached boundary without an attributed summary"
        );
        self.completed_possessions = self.completed_possessions.saturating_add(1);
        self.update_scope_completion();
    }

    /// 犯满离场的强制换人（tactics.md 2.3.3 优先级 1：强制换人）。
    ///
    /// 替补实体已在物理层注册（on_court=false，不参与运动学）；换人 =
    /// 翻转在场标志并交接场上位置。候选按 id 字典序取最小者（确定性）。
    /// 若替补全部不可用，则保留原球员（保持 5v5 不变量优先）。
    pub fn forced_substitution(&mut self, out_player_id: &str) {
        let team = match self.physics.get_player(out_player_id) {
            Some(p) => p.team.clone(),
            None => return,
        };
        // 犯规方不可能持球；若命中持球者（异常调用），拒绝换人以保球权一致。
        //
        // ## holder 引用必须按球态投影判定（round-18 修复）
        //
        // 只查 `has_ball` 旗标不够：进攻犯规时球先进 `Dead` 态（旗标已清），
        // 但 `Dead.last_touch_player` 仍指向犯规的持球人——把他换下场后，
        // 帧投影的 holder 引用一个不在场的人，触发
        // `BALL_HOLDER_ON_COURT` Hard（实测 seed 31337：190 次/场）。
        // 改为：`has_ball` **或** 球态投影（`current_turnover_player_id`）
        // 命中任一即拒绝。
        if self
            .physics
            .get_player(out_player_id)
            .is_some_and(|p| p.has_ball)
            || self.current_turnover_player_id().as_deref() == Some(out_player_id)
            // 飞行中的传球目标也不可换下：飞行 ~0.4s 内换人会让球到达时
            // 「被已下场的人接住」（实测 seed 31337：t=3142 H_08 在飞行中
            // 被罚下，到达帧起 Held{H_08} 引用下场者 190 tick）。
            || self.pending_pass_receiver.as_deref() == Some(out_player_id)
            || matches!(&self.ball_state,
                BallTrajectoryKind::Pass { target_id, .. } if target_id == out_player_id)
        {
            return;
        }
        let max_fouls = self.rules.league.max_personal_fouls;
        let mut candidates: Vec<String> = self
            .physics
            .get_players()
            .values()
            .filter(|p| p.team == team && !p.on_court && p.foul_count < max_fouls)
            .map(|p| p.id.clone())
            .collect();
        candidates.sort();
        let Some(in_player_id) = candidates.first().cloned() else {
            return;
        };
        let (out_pos, out_action_slot) = match self.physics.get_player(out_player_id) {
            Some(p) => (p.pos_ft, p.slot.clone()),
            None => return,
        };
        // 离场者退至替补席区（界外，运动学冻结）。
        let bench_spot = Vec2::new(
            self.rules.court.width_ft / 2.0 + if team == "home" { -6.0 } else { 6.0 },
            self.rules.court.height_ft + 4.0,
        );
        if let Some(p) = self.physics.get_player_mut(out_player_id) {
            p.on_court = false;
            p.has_ball = false;
            p.action = "Bench".to_string();
            p.pos_ft = bench_spot;
            p.target_pos_ft = bench_spot;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
        }
        // ## 清除对离场者的持球引用（round-18 修复）
        //
        // `last_passer_id` / `pending_pass_receiver` 可能仍指向离场者
        // （如他在界外接球后球出界、引用未被回合边界清除——实测
        // seed 31337：t=3142 H_08 在 y=-4 接球，43s 后被换下，之后
        // 每次 Pass/Loose 飞行的 holder 投影都引用这个已下场的人，
        // 触发 BALL_HOLDER_ON_COURT 190 次）。
        if self.last_passer_id.as_deref() == Some(out_player_id) {
            self.last_passer_id = None;
        }
        if self.pending_pass_receiver.as_deref() == Some(out_player_id) {
            self.pending_pass_receiver = None;
        }
        if let Some(p) = self.physics.get_player_mut(&in_player_id) {
            p.on_court = true;
            p.action = "EnterCourt".to_string();
            p.pos_ft = out_pos;
            p.target_pos_ft = out_pos;
            p.target_speed_ftps = 0.0;
            p.vel_ft = Vec2::ZERO;
            p.slot = out_action_slot;
        }
        self.current_callout = Some(format!(
            "球员 {} 犯满离场，替补 {} 死球登场入位！",
            out_player_id, in_player_id
        ));
        // 战术绑定名册同步（slot 绑定按名册顺序索引）。
        for roster in [&mut self.home_roster_order, &mut self.away_roster_order] {
            if let Some(slot_ref) = roster.iter_mut().find(|id| *id == out_player_id) {
                *slot_ref = in_player_id.clone();
            }
        }
        self.pending_events.push(GameEvent::Substitution {
            team,
            out_player: out_player_id.to_string(),
            in_player: in_player_id,
        });
    }

    /// 节末把在飞的球结算为死球（第一性原理：停表期间球不飞）。
    ///
    /// 只在球处于飞行/松球族时生效；已持球或已死球时不做任何事，
    /// 以免改变正常节奏与黄金哈希行为。
    fn settle_ball_for_period_break(&mut self) {
        let in_flight = matches!(
            self.ball_state,
            BallTrajectoryKind::Pass { .. }
                | BallTrajectoryKind::Shot { .. }
                | BallTrajectoryKind::LooseBall { .. }
                | BallTrajectoryKind::RimRebound { .. }
                | BallTrajectoryKind::ControlTransfer { .. }
        );
        if !in_flight {
            return;
        }
        let (pos, z) = self.ball_pos_3d;
        // 先取责任球员，再构造 Dead——`dead_state` 内部从权威球态派生，
        // 因此**不得**在构造后清除 `last_passer_id`：那样会把刚写进载荷的
        // 最后触球人也一并抹掉，使后续死球终结的归因链断在 None。
        //
        // 实测（round-9）：seed 0/2/7 的 8 秒违例在节末死球后触发，
        // `Dead.last_touch_player` 恒为 None，正是此处顺序造成的。
        // 载荷已是唯一事实源，不再需要旁路字段存续，故无需立即清空。
        self.transition_ball_state(self.dead_state(pos, z));
        self.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.receiver_estimate = None;
        self.pending_pass_inbound = false;
    }

    fn settle_scope_ball(&mut self, position: (Vec2, f32)) {
        self.ball_pos_3d = position;
        self.transition_ball_state(self.dead_state(position.0, position.1));
        self.set_game_flow(GameFlowState::DeadBall);
        self.transition_phase(SubPhase::DeadBallReset);
    }

    // 阶段转换器
    fn current_turnover_player_id(&self) -> Option<String> {
        match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::InboundTransfer {
                inbounder_id: carrier_id,
                ..
            }
            // `ControlTransfer` 的载荷里就有 `carrier_id`（接球人），直接读它。
            // 回退到 `last_passer_id` 是错的：合球飞行期间传球人已可能被清空，
            // 且发生在同一回合内多次转移时会把责任归给上一个人。
            // 实测（seed 2/7）：一传 + 合球飞行 + 节末死球 → 归因为空。
            | BallTrajectoryKind::ControlTransfer {
                carrier_id,
                ..
            } => Some(carrier_id.clone()),
            BallTrajectoryKind::Pass { .. } | BallTrajectoryKind::LooseBall { .. } => {
                self.last_passer_id.clone()
            }
            // 死球：优先用载荷里的最后触球人（round-9）；
            // 旧流/未携带时回退到旁路字段，保证向后兼容。
            BallTrajectoryKind::Dead {
                last_touch_player, ..
            } => last_touch_player.clone().or_else(|| self.last_passer_id.clone()),
            BallTrajectoryKind::Shot { shooter_id, .. } => Some(shooter_id.clone()),
            BallTrajectoryKind::RimRebound { .. } => None,
        }
    }

    /// 构造 `Dead` 球态的唯一入口：从**当前权威球态**派生最后触球人。
    ///
    /// ## 为什么需要它（round-9 架构修复）
    ///
    /// `Dead` 此前只带 `last_touch_team`，责任球员只能回退到旁路字段
    /// `last_passer_id`——而该字段在进入下一次进攻时被清空，于是死球终结
    /// （如 8 秒违例）的归因会断在 `None`（实测 `TURNOVER_ACTOR_CONSISTENCY`
    /// 8 seed 报 3 条 Hard）。
    ///
    /// 按 P1「状态是唯一事实源」，责任球员必须能从权威球态读出。因此把
    /// 「进入死球时的最后触球人」作为载荷写进 `Dead`，而不是在各调用点
    /// 各自去猜一个回退链。所有 `Dead` 构造都经此函数，保证载荷一致。
    fn dead_state(&self, pos: Vec2, z: f32) -> BallTrajectoryKind {
        BallTrajectoryKind::Dead {
            pos,
            z,
            last_touch_team: self.possession,
            last_touch_player: self.current_turnover_player_id(),
        }
    }

    fn start_violation_turnover(&mut self, _kind: ViolationKind) {
        self.box_score.turnovers += 1;
        self.emit_possession_summary(
            nba_domain::PossessionEndCause::TurnoverViolation,
            None,
            self.current_possession_turnover_player
                .clone()
                .or_else(|| self.current_turnover_player_id()),
            None,
        );
        self.last_passer_id = None;
        self.pending_loose_ball_terminal = None;
        self.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.receiver_estimate = None;
        self.pending_pass_inbound = false;
        let current_ball_3d = self.ball_pos_3d;
        let baseline = Court::nearest_boundary_with_geometry(current_ball_3d.0, self.rules.court);
        self.start_inbound_transition(baseline, current_ball_3d);
    }

    fn start_steal_transition(&mut self, stealer_id: String, intercept_pos: Vec2) {
        self.box_score.turnovers += 1;
        // The defender is the actor in the STEAL fact; the summary's
        // turnover_player_id is the offensive player who lost the pass.
        self.emit_possession_summary(
            nba_domain::PossessionEndCause::TurnoverSteal,
            None,
            self.current_possession_turnover_player
                .clone()
                .or_else(|| self.current_turnover_player_id()),
            None,
        );
        self.last_passer_id = None;
        self.pending_loose_ball_terminal = None;
        self.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.receiver_estimate = None;
        self.pending_pass_inbound = false;
        self.complete_possession();
        if self.simulation_complete {
            self.settle_scope_ball((intercept_pos, self.rules.ball_holder_height_ft));
            return;
        }
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.shot_clock = self.rules.league.shot_clock_seconds;
        self.sub_phase_timer = 0.0;
        self.transition_phase(SubPhase::ActionExecution);
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_ball_state(BallTrajectoryKind::Held {
            carrier_id: stealer_id,
        });
    }

    fn start_rebound_outlet(&mut self, rebounder_id: String, reb_pos: Vec2, is_offensive: bool) {
        if !is_offensive {
            self.complete_possession();
            if self.simulation_complete {
                self.settle_scope_ball((reb_pos, self.rules.chest_height_ft));
                return;
            }
        }
        self.possession = if is_offensive {
            self.possession
        } else {
            opposite(self.possession)
        };
        if !is_offensive {
            self.possession_id += 1;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.current_possession_turnover_player = Some(rebounder_id.clone());
        self.shot_clock = if is_offensive {
            self.rules.league.offensive_rebound_shot_clock_seconds
        } else {
            self.rules.league.shot_clock_seconds
        };
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::LiveBall);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        let target_id = if is_offensive {
            rebounder_id.clone()
        } else {
            let candidate = self.new_possession_pg();
            if candidate == rebounder_id {
                let team = match self.possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                self.team_roster_ids(team)
                    .into_iter()
                    .find(|id| id != &rebounder_id)
                    .unwrap_or_else(|| rebounder_id.clone())
            } else {
                candidate
            }
        };
        let rebounder_pos = self
            .physics
            .get_player(&rebounder_id)
            .map(|p| p.pos_ft)
            .unwrap_or(reb_pos);
        // The rebound sample already ends at `reb_pos`. Do not move the ball
        // to the player's body center before creating the next trajectory:
        // that would insert an instantaneous, unobserved catch displacement.
        let catch_pos = reb_pos;
        self.ball_pos_3d = (catch_pos, self.rules.chest_height_ft);
        if target_id == rebounder_id {
            let dist = (rebounder_pos - catch_pos).length();
            let speed_budget = (self.rules.ball_max_speed_ftps
                - self.rules.invariant_speed_tolerance_ftps)
                .max(self.rules.invariant_speed_tolerance_ftps);
            let transfer_dur = (dist / speed_budget).max(self.rules.tick_seconds);
            self.transition_ball_state(BallTrajectoryKind::ControlTransfer {
                from_pos: catch_pos,
                from_z: self.rules.chest_height_ft,
                target_pos: rebounder_pos,
                target_z: self.rules.ball_holder_height_ft,
                carrier_id: rebounder_id,
                start_time: self.current_time,
                duration: transfer_dur,
            });
            return;
        }
        let target_pos = self
            .physics
            .get_player(&target_id)
            .map(|p| p.pos_ft)
            .unwrap_or_else(|| {
                rebounder_pos + Vec2::new(self.rules.rebound_outlet_fallback_distance_ft, 0.0)
            });
        let pass_dist = (target_pos - catch_pos).length();
        // 一传（outlet）也必须**领传**（round-7 审计修复）。
        //
        // 此前 `to_pos` 直接冻结接球人**释放时刻**的当前位置，但接球人在
        // 飞行期间仍向战术目标奔跑，于是永远不可能恰好停在冻结点——实测
        // 越位 5–12 ft，全部出现在 `(97.0, 25.0)` 基准发球点的 outlet 上
        // （`PASS_CORRIDOR_REACHABLE` 的最大单一来源）。
        //
        // 决策侧传球早就有领传（`execute_pass` 的 `target_lead_pos` 来自决策层），
        // 唯独 outlet 这条路径漏了。修法与决策侧一致：按接球人的当前速度
        // 外推一个飞行期内的可达点。
        let target_pos = self.lead_receiver_position(&target_id, target_pos, pass_dist, false);
        let pass_dist = (target_pos - catch_pos).length();
        let receive_success =
            self.resolve_pass_success(&rebounder_id, &target_id, catch_pos, target_pos);
        // 一传（outlet pass）同样在释放时一次性裁定拦截。
        let intercept =
            self.resolve_pass_interception(&rebounder_id, &target_id, catch_pos, target_pos);
        self.transition_ball_state(BallTrajectoryKind::Pass {
            from_pos: catch_pos,
            to_pos: target_pos,
            target_id: target_id.clone(),
            start_time: self.current_time,
            duration: self.rules.pass_duration(pass_dist, false),
            peak_z: self.rules.pass_peak_ft,
            inbound: false,
            receive_success,
            intercept,
        });
        self.current_possession_passes += 1;
        self.pending_events.push(GameEvent::PassRelease {
            passer_id: rebounder_id.clone(),
            receiver_id: target_id.clone(),
            from_pos: (catch_pos.x, catch_pos.y),
            to_pos: (target_pos.x, target_pos.y),
        });
        // 记录传球人（round-9 架构修复）：一传同样是一次传球，必须设置
        // `last_passer_id`，否则该回合若以死球/违例终结，责任球员无法从
        // 权威球态派生（`Dead` 载荷、`Pass` 分支均回退到该字段）。
        //
        // 实测（seed 0/2/7）：防守篮板 → outlet 一传 → 传球失败 + 节末，
        // 因本行缺失使 `turnover_player_id` 为空，报 3 条
        // `TURNOVER_ACTOR_CONSISTENCY` Hard。
        self.last_passer_id = Some(rebounder_id.clone());
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::LiveBall);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.current_event = Some("OUTLET_PASS".to_string());
        self.current_callout = Some(format!(
            "篮板球转入{}，发动战术：{}",
            if is_offensive {
                "二次进攻"
            } else {
                "转换进攻"
            },
            self.tactical_set.name_zh()
        ));
    }

    fn start_loose_ball_transition(&mut self, player_id: String, position: Vec2) {
        let player_team = self
            .physics
            .get_player(&player_id)
            .map(|player| player.team.as_str());
        let secured_possession = match player_team {
            Some("home") => Possession::Home,
            Some("away") => Possession::Away,
            _ => self.possession,
        };
        let possession_changed = secured_possession != self.possession;
        let loose_terminal = self
            .pending_loose_ball_terminal
            .take()
            .unwrap_or(nba_domain::PossessionEndCause::TurnoverLooseBall);
        // ## 篮板源地板球的事实补记（round-17）
        //
        // 投/罚不中弹出的地板球被**进攻方**收下时，语义上前场篮板
        // （ORB）：回合继续 + 进攻钟重置 14s。若不发 `REBOUND` 事实，
        // 评判器的 ORB 窗口记账（RHYTHM_DURATION / DURATION_BOUNDS 的
        // 自变量）会漏计这个 14s 窗口，把合法的 ~37s 回合误判超带。
        let was_rebound_origin = loose_terminal == nba_domain::PossessionEndCause::DefensiveRebound;
        if was_rebound_origin && !possession_changed {
            self.pending_events.push(GameEvent::ReboundContest {
                rebounder_id: player_id.clone(),
                landing_pos: (position.x, position.y),
                is_offensive: true,
            });
        }
        if possession_changed {
            // 篮板源 + 球权易主 = 防守篮板：归因到收球人（rebounder），
            // 而不是寻找一个不存在的「失误球员」。
            if loose_terminal == nba_domain::PossessionEndCause::DefensiveRebound {
                self.emit_possession_summary(loose_terminal, Some(player_id.clone()), None, None);
            } else {
                let turnover_player_id = self
                    .current_possession_turnover_player
                    .clone()
                    .or_else(|| self.current_turnover_player_id());
                self.emit_possession_summary(loose_terminal, None, turnover_player_id, None);
            }
            self.last_passer_id = None;
            self.pending_pass_receiver = None;
            // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
            self.receiver_estimate = None;
            self.pending_pass_inbound = false;
            self.complete_possession();
            if self.simulation_complete {
                self.settle_scope_ball((position, self.rules.ball_holder_height_ft));
                return;
            }
            self.possession = secured_possession;
            self.possession_id += 1;
            self.shot_clock = self.rules.league.shot_clock_seconds;
        }
        self.update_coach_strategy();
        self.sync_team_tactics();
        self.carrier_idx = self.player_index_for_id(&player_id).unwrap_or(0);
        self.current_possession_turnover_player = Some(player_id.clone());
        // The secured ball remains in a transfer trajectory until it reaches
        // the receiver's frozen catch point. Do not advertise Held here: that
        // would let the following tactical planner move the player away before
        // the ball is attached.
        self.ball_pos_3d = (position, self.rules.ball_holder_height_ft);
        self.set_game_flow(GameFlowState::LiveBall);
        self.transition_phase(SubPhase::Initiation);
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.last_passer_id = None;
        self.pending_loose_ball_terminal = None;
    }

    /// F1.3b：把一个球员离散放置到指定位置（含界外发球点），并发
    /// `PlacementApplied` 事实。
    ///
    /// 离散 placement 属于生命周期事实，不是连续运动（gap.md §4.3），
    /// 因此不参与逐 tick 速度/越界不变量判定；调用方必须同时标记
    /// `out_of_bounds_placement` 以声明该球员当前享有界外豁免。
    fn place_player_out_of_bounds(&mut self, player_id: &str, to: Vec2) {
        let Some(player) = self.physics.get_player(player_id) else {
            return;
        };
        let from = player.pos_ft;
        if let Some(p) = self.physics.get_player_mut(player_id) {
            p.pos_ft = to;
            p.target_pos_ft = to;
            p.vel_ft = Vec2::ZERO;
            p.accel_ft = Vec2::ZERO;
            p.out_of_bounds_placement = true;
        }
        // Rapier 后端从刚体回写坐标，必须同步刚体否则放置会被覆盖。
        self.physics.teleport_player(player_id, to);
        self.pending_events.push(GameEvent::PlacementApplied {
            player_id: player_id.to_string(),
            from: (from.x, from.y),
            to: (to.x, to.y),
            reason: "INBOUND_SETUP".to_string(),
            phase: self.phase_type().as_str().to_string(),
        });
    }

    /// 为一个即将从界外 placement 回场的球员选择一个界内且不与他人
    /// 重叠的落点（gap.md §4.3：离散 placement 必须直接给出合法坐标）。
    ///
    /// 优先原地 clamp；若与在场球员距离不足，则沿向内方向逐步搜索。
    /// 搜索不出时退回合法 clamp 位置（至少保证在界内）。
    fn free_in_court_spot(&self, from: Vec2, exempt: &Option<String>) -> Vec2 {
        let margin = self.rules.player_radius_ft;
        let base = self.rules.court.clamp_playable(from, margin);
        let required = self.rules.min_player_separation_ft;
        let occupied: Vec<Vec2> = self
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && exempt.as_deref() != Some(p.id.as_str()))
            .map(|p| p.pos_ft)
            .collect();
        let is_free = |candidate: Vec2| {
            occupied
                .iter()
                .all(|pos| pos.distance(candidate) >= required)
        };
        if is_free(base) {
            return base;
        }
        // 从当前位置向场内方向逐步内推，步长取球员半径。
        let center = self.rules.court.center();
        let inward = (center - base).normalize_or_zero();
        if inward.length_squared() < f32::EPSILON {
            return base;
        }
        let step = self.rules.player_radius_ft.max(f32::EPSILON);
        let attempts = ((self.rules.court.width_ft / step).ceil() as usize).max(1);
        for i in 1..=attempts {
            let candidate = self
                .rules
                .court
                .clamp_playable(base + inward * step * i as f32, margin);
            if is_free(candidate) {
                return candidate;
            }
        }
        base
    }

    /// 松球出界的显式状态转移（第一性原理：出界是事实，不是边界夹取）。
    ///
    /// 球权交给最后触球方的对手，并进入发球程序。此前界外松球被
    /// `clamp_playable` 夹回边界，单 tick 位移数英尺造成 `BALL_SPEED`
    /// 尖峰（实测 122 ft/s > 85 上限）。
    fn start_out_of_bounds_transition(&mut self, boundary_pos: Vec2, ball_3d: (Vec2, f32)) {
        // 归因纪律（round-5 审计修复）。
        //
        // 出界**不产生新的失误原因**——它只是球离开场地的位置事实。失误原因
        // 在球出界之前就已确定（掉球/被点掉/被断/违例/松球易主），保存在
        // `pending_loose_ball_terminal`。此前本函数无条件以 `TurnoverViolation`
        // 结算，**覆盖**了既有传球失误原因：实测 8 seed 共 98 条
        // `TURNOVER_ATTRIBUTION` Hard，32 个 `TURNOVER_VIOLATION` 回合中
        // 有 15 个窗口内没有任何 `VIOLATION` 事实。
        //
        // 责任球员必须可归因（D0.1）：松球出界时球可能既无持球人也无
        // `last_passer`（例如篮板弹出界），逐级回退保证不为空。
        let cause = self
            .pending_loose_ball_terminal
            .take()
            .unwrap_or(nba_domain::PossessionEndCause::TurnoverViolation);

        let responsible = self
            .current_possession_turnover_player
            .clone()
            .or_else(|| self.current_turnover_player_id())
            .or_else(|| self.last_passer_id.clone())
            .or_else(|| Some(self.carrier_id()));

        // 把「球出界」发布为事实：此前 `OUT_OF_BOUNDS` 在事件流中出现 0 次，
        // 消费方无法区分「出界导致的失误」与「违例导致的失误」。
        self.pending_events
            .push(nba_domain::GameEvent::BallOutOfBounds {
                position: [boundary_pos.x, boundary_pos.y],
                last_touch_team: match self.possession {
                    Possession::Home => "home".to_string(),
                    Possession::Away => "away".to_string(),
                },
                responsible_player_id: responsible.clone(),
            });

        self.emit_possession_summary(cause, None, responsible, None);
        self.start_inbound_transition(boundary_pos, ball_3d);
    }

    fn start_inbound_transition(&mut self, baseline_pos: Vec2, current_ball_3d: (Vec2, f32)) {
        self.complete_possession();
        self.last_passer_id = None;
        self.pending_pass_receiver = None;
        // 传球结束：丢弃本回合的接球人估计（下一回合重新形成预判）。
        self.receiver_estimate = None;
        self.pending_pass_inbound = false;
        self.pending_loose_ball_terminal = None;
        if self.simulation_complete {
            self.settle_scope_ball(current_ball_3d);
            return;
        }
        self.possession = opposite(self.possession);
        self.possession_id += 1;
        self.update_coach_strategy();
        self.shot_clock = self.rules.league.shot_clock_seconds;
        self.sync_team_tactics();
        self.carrier_idx = 0;
        self.transition_phase(SubPhase::Initiation);
        self.set_game_flow(GameFlowState::DeadBall);
        self.inbound_baseline = baseline_pos;
        let inbounder_id = self.new_possession_pg();
        let release_pos = Court::inbound_release_pos_with_geometry(
            baseline_pos,
            self.rules.inbound_release_depth_ft,
            self.rules.court,
        );
        if let Some(player) = self.physics.get_player_mut(&inbounder_id) {
            player.target_pos_ft = release_pos;
            player.action = "InboundPositioning".to_string();
            // F1.3：发球程序期间发球员是显式 placement 角色，允许站到
            // 界外发球点（gap.md §4.3/§8.5），否则物理 clamp 会让
            // `inbounder_arrived` 永不成立，比赛卡死在 DeadBall。
            player.out_of_bounds_placement = true;
        }
        // F1.3b：发球员赴界外发球点是**离散 placement**，不是普通运动
        // （gap.md §4.3：换人入场、节间站位、跳球布置同属此类）。
        // 若要求发球员步行过去，一名被场地 clamp 钉在边线的防守者可永久
        // 堵住路径，`inbounder_arrived` 永不成立（本轮 seed 6/9/11 实测
        // 约 19 万 tick 的 OUT_OF_BOUNDS 活锁）。因此直接放置并发事实。
        self.place_player_out_of_bounds(&inbounder_id, release_pos);
        let duration = self
            .rules
            .pass_duration((release_pos - current_ball_3d.0).length(), true)
            .max(self.rules.inbound_setup_seconds);
        self.transition_ball_state(BallTrajectoryKind::InboundTransfer {
            from_pos: current_ball_3d.0,
            from_z: current_ball_3d.1,
            baseline_pos: release_pos,
            inbounder_id,
            start_time: self.current_time,
            duration,
        });
        self.inbound_elapsed = 0.0;
        self.last_decision_time = -self.rules.decision_interval_seconds;
        self.current_event = Some("INBOUND_SETUP".to_string());
        self.current_callout = Some(format!(
            "界外发球准备，呼叫战术：{}",
            self.tactical_set.name_zh()
        ));
    }
    fn finish_period(&mut self) {
        let previous_period = self.period;
        if self.period < self.rules.league.regulation_periods {
            self.team_fouls_home = 0;
            self.team_fouls_away = 0;
            let is_halftime = self.rules.league.regulation_periods > 1
                && self.period == self.rules.league.regulation_periods / 2;
            self.set_game_flow(if is_halftime {
                GameFlowState::Halftime
            } else {
                GameFlowState::QuarterEnd
            });
            self.period_break_elapsed = 0.0;
            self.transition_phase(SubPhase::DeadBallReset);
            // 第一性原理：节间 `current_time` 冻结（`step_inner` 在
            // QuarterEnd/Halftime 提前返回且不推进时间）。若此时球仍在
            // 飞行（Pass/Shot/Loose），弹道采样是 `progress = (t - start)/duration`
            // 的纯函数——时间一旦恢复推进，球会「瞬移」数英尺，被 L1 判为
            // `BALL_SPEED`（实测 98.2 ft/s > 85 上限，seed 4）。
            //
            // 节末必须先把在飞的球结算成死球：比赛时钟停表期间球也应是死的。
            self.settle_ball_for_period_break();
            // 跨节回合必须显式结算（round-5 审计修复）。
            //
            // `PossessionEndCause::PeriodEnd` 在 domain 中已定义，但引擎从未
            // emit：节末仍在进行中的回合被归到「上一节名下、下一节结束」，
            // 实测 3 个/场，最长 41.6s，越出回合时长上界并产生 Hard defect。
            // 节末是**规则允许**的回合终结方式，必须如实记录。
            let count = self.completed_possessions as u64;
            if self.last_possession_summary_index != Some(count) {
                self.emit_possession_summary(
                    nba_domain::PossessionEndCause::PeriodEnd,
                    None,
                    self.current_turnover_player_id(),
                    None,
                );
                self.complete_possession();
            }
            self.pending_events.push(GameEvent::PhaseTransition {
                from: PhaseType::SetPlay,
                to: PhaseType::DeadBallReset,
            });
            self.current_event = Some("PERIOD_END".to_string());
            self.current_callout = Some(format!(
                "第{}节结束，进入{}休息",
                previous_period,
                if previous_period == self.rules.league.regulation_periods / 2 {
                    "中场"
                } else {
                    "节间"
                }
            ));
        } else if self.home_score == self.away_score {
            self.period += 1;
            self.game_clock = self.rules.league.overtime_duration_seconds;
            self.shot_clock = self.rules.league.shot_clock_seconds;
            self.period_break_elapsed = 0.0;
            self.set_game_flow(GameFlowState::Overtime);
            self.transition_phase(SubPhase::Initiation);
            self.current_event = Some("OVERTIME_START".to_string());
        } else {
            self.set_game_flow(GameFlowState::GameEnd);
            self.transition_phase(SubPhase::DeadBallReset);
            self.pending_events.push(GameEvent::PhaseTransition {
                from: self.phase_type(),
                to: PhaseType::DeadBallReset,
            });
            self.current_event = Some("GAME_END".to_string());
            self.current_callout = Some(format!(
                "比赛结束，{}获胜",
                if self.home_score > self.away_score {
                    "主队"
                } else {
                    "客队"
                }
            ));
        }
        self.update_scope_completion();
    }

    /// Returns the first configured starter for the team currently in control.
    /// 选出本队当前的**处理球人**（发球员 / 回合起始持球人）。
    ///
    /// ## 为什么不再取 `roster_order` 首位（round-11 Step4b / P-2）
    ///
    /// 旧实现是「取名册数组里第一个在场者」——**顺序即身份**：把名册数组
    /// 轮转一下，处理球人就变了。契约（`tactics.md TA3`「角色是槽位不是身份」、
    /// `attributes.md §2.7`）要求身份由**能力适配**派生。
    ///
    /// 现在按能力排序：`ball_handling × w1 + passing × w2 + decision_iq × w3`，
    /// 权重走规则通道（`TacticalRules.slot_handler_*`，与 `fill_slots` 同源）。
    /// 名册数组顺序**完全不参与**。
    ///
    /// 约束：必须是**在场**球员。若把球交给替补（`on_court=false`），他永远
    /// 不会被物理步进，`inbounder_arrived` 永不成立，比赛卡死在 DeadBall
    /// （历史实测：发球员 `action=Bench`、位置停在替补席）。
    fn new_possession_pg(&self) -> String {
        let team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let policy = &self.rules.tactics;
        let mut candidates: Vec<(f32, String)> = self
            .physics
            .get_players()
            .values()
            .filter(|p| p.on_court && p.team == team)
            .map(|p| {
                let score = p.attributes.ball_handling * policy.slot_handler_ball_handling_weight
                    + p.attributes.passing * policy.slot_handler_passing_weight
                    + p.attributes.decision_iq * policy.slot_handler_decision_iq_weight;
                (score, p.id.clone())
            })
            .collect();
        // 确定性：分数降序，同分按 id 升序（不依赖 HashMap 迭代序，charter C4）。
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
        candidates
            .into_iter()
            .next()
            .map(|(_, id)| id)
            .unwrap_or_default()
    }

    fn team_roster_ids(&self, team: &str) -> Vec<String> {
        match team {
            "home" => self.home_roster_order.clone(),
            "away" => self.away_roster_order.clone(),
            _ => Vec::new(),
        }
    }

    pub fn new_possession_pg_for_test(&self) -> String {
        self.new_possession_pg()
    }
    /// 遵循宪章 Positionless 原则：由场上实际摸高上限最高的球员担当跳球代表
    /// 摸高分数 = 身高 height_cm * 0.5 + 垂直弹跳 vertical * 0.5
    pub fn select_jumper_id(&self, possession: Possession) -> String {
        let team = match possession {
            Possession::Home => &self.home_team,
            Possession::Away => &self.away_team,
        };
        team.players
            .iter()
            .filter(|p| {
                self.physics
                    .get_player(&p.id)
                    .map(|phys| phys.on_court)
                    .unwrap_or(true)
            })
            .max_by(|a, b| {
                let reach_a = a.height_cm as f32 * 0.5 + a.attributes.vertical * 0.5;
                let reach_b = b.height_cm as f32 * 0.5 + b.attributes.vertical * 0.5;
                reach_a
                    .partial_cmp(&reach_b)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|p| p.id.clone())
            // 兜底不得硬编码球员 id（round-10 Step4a：名册已去序号化，
            // "H_1"/"A_1" 不再存在）。退回该队在场球员的字典序首位，
            // 仍无法确定时返回空串（由调用方按非法状态处理）。
            .or_else(|| {
                let team = match possession {
                    Possession::Home => "home",
                    Possession::Away => "away",
                };
                let mut ids: Vec<String> = self
                    .physics
                    .get_players()
                    .values()
                    .filter(|p| p.on_court && p.team == team)
                    .map(|p| p.id.clone())
                    .collect();
                ids.sort();
                ids.into_iter().next()
            })
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn execute_shot_for_test(&mut self, shooter_id: &str, from_pos: Vec2, is_three: bool) {
        self.execute_shot(shooter_id, from_pos, is_three, None, self.current_time);
    }
    // ==================== D4.2 真相字段只读访问器 ====================
    // 比赛真相（球权/球态/时钟/比分/阶段）不再对外暴露可变字段；
    // 外部只能经这些访问器读取，或经 `step()` / `snapshot()` 推进与观测
    // （dev 方案 §7.1 D4.2）。写入一律走引擎内部唯一入口。

    /// 当前宏观生命周期状态。
    pub fn game_flow(&self) -> GameFlowState {
        self.game_flow
    }

    /// 回合序号（每次球权转移递增）。
    pub fn possession_id(&self) -> u32 {
        self.possession_id
    }

    /// 回合子阶段。
    pub fn sub_phase(&self) -> SubPhase {
        self.sub_phase
    }

    /// 权威球态（球权真相的唯一载体）。
    pub fn ball_state(&self) -> &BallTrajectoryKind {
        &self.ball_state
    }

    /// 球的三维位置（ft）。
    pub fn ball_pos_3d(&self) -> (Vec2, f32) {
        self.ball_pos_3d
    }

    /// 比赛时钟（秒，节内倒计时）。
    pub fn game_clock(&self) -> f32 {
        self.game_clock
    }

    /// 进攻时钟（秒）。
    pub fn shot_clock(&self) -> f32 {
        self.shot_clock
    }

    /// 单调仿真时间（秒）。
    pub fn current_time(&self) -> f32 {
        self.current_time
    }

    /// 当前节次。
    pub fn period(&self) -> u32 {
        self.period
    }

    /// 主队比分。
    pub fn home_score(&self) -> u32 {
        self.home_score
    }

    /// 客队比分。
    pub fn away_score(&self) -> u32 {
        self.away_score
    }

    /// 主队团队犯规数。
    pub fn team_fouls_home(&self) -> u32 {
        self.team_fouls_home
    }

    /// 客队团队犯规数。
    pub fn team_fouls_away(&self) -> u32 {
        self.team_fouls_away
    }

    /// 当前罚球执行者（只读）。
    pub fn free_throw_shooter(&self) -> Option<&str> {
        self.free_throw_shooter.as_deref()
    }

    /// 本回合剩余罚球次数。
    pub fn free_throws_remaining(&self) -> u8 {
        self.free_throws_remaining
    }

    /// 后场连续持球时间（秒），8 秒违例判据。
    pub fn backcourt_elapsed(&self) -> f32 {
        self.backcourt_elapsed
    }

    /// 节间休息已用时间（秒）。
    pub fn period_break_elapsed(&self) -> f32 {
        self.period_break_elapsed
    }

    /// 已完成回合数。
    pub fn completed_possessions(&self) -> usize {
        self.completed_possessions
    }

    /// 比赛统计分解（2P/3P/FT、失误、犯规）——只读快照。
    pub fn box_score(&self) -> &MatchBoxScore {
        &self.box_score
    }

    /// 最近一次 `step()` 产生的不变量违反（只读）。
    pub fn last_tick_violations(&self) -> &[Violation] {
        &self.last_tick_violations
    }

    /// 发球基线位置（只读）。
    pub fn inbound_baseline(&self) -> Vec2 {
        self.inbound_baseline
    }

    /// 当前 tick 已发布的事件日志（只读）。
    pub fn current_event_log(&self) -> &[FrameEvent] {
        &self.current_event_log
    }

    /// 待发布事件队列（只读；写入走引擎内部路径）。
    pub fn pending_events(&self) -> &[GameEvent] {
        &self.pending_events
    }

    /// 当前作用域目标回合数。
    pub fn target_possessions(&self) -> usize {
        self.target_possessions
    }

    /// 是否处于带边界的作用域运行。
    pub fn scope_active(&self) -> bool {
        self.scope_active
    }

    /// 请求的作用域是否已完成（区别于真实比赛生命周期）。
    pub fn simulation_complete(&self) -> bool {
        self.simulation_complete
    }

    pub fn possession(&self) -> nba_domain::Possession {
        self.possession
    }

    pub fn force_possession_for_test(&mut self, possession: nba_domain::Possession) {
        self.possession = possession;
    }

    // ==================== D4.2 测试写入钩子 ====================
    // 真相字段私有化后，测试的场景构造需要受控写入通道。为了让写入
    // 意图在调用点显式可见（而不是裸露的字段赋值），统一在此集中提供
    // `*_for_test` 设置器，命名即声明"这是测试场景构造，不是生产写入"。
    // 与 `force_possession_for_test` 同类：架构 §3.2 声明的测试豁免通道。

    #[doc(hidden)]
    pub fn set_game_flow_for_test(&mut self, flow: GameFlowState) {
        self.game_flow = flow;
    }

    #[doc(hidden)]
    /// 测试后门：直接设置球态。
    ///
    /// 注意：本后门绕过 `transition_ball_state`（唯一写入口），因此必须
    /// **自行维护派生副作用**。当前需要维护的是接球人标记
    /// （`is_receiving_pass`，round-10）——否则用本后门构造的传球场景里，
    /// 接球人不会获得 APF 豁免与「到位即停」，造成与生产路径不一致的行为
    /// （实测：H_2 带 17.26 ft/s 初速滑离落点，层 A 误判）。
    pub fn set_ball_state_for_test(&mut self, state: BallTrajectoryKind) {
        if let BallTrajectoryKind::Pass { target_id, .. } = &state {
            let rid = target_id.clone();
            self.mark_receiver(Some(&rid));
        } else {
            self.mark_receiver(None);
        }
        self.ball_state = state;
    }

    #[doc(hidden)]
    pub fn set_ball_pos_for_test(&mut self, pos: Vec2, z: f32) {
        self.ball_pos_3d = (pos, z);
    }

    #[doc(hidden)]
    pub fn push_event_for_test(&mut self, event: GameEvent) {
        self.pending_events.push(event);
    }

    #[doc(hidden)]
    pub fn clear_pending_events_for_test(&mut self) {
        self.pending_events.clear();
    }

    #[doc(hidden)]
    pub fn set_current_time_for_test(&mut self, seconds: f32) {
        self.current_time = seconds;
    }

    #[doc(hidden)]
    pub fn set_game_clock_for_test(&mut self, seconds: f32) {
        self.game_clock = seconds;
    }

    #[doc(hidden)]
    pub fn set_shot_clock_for_test(&mut self, seconds: f32) {
        self.shot_clock = seconds;
    }

    #[doc(hidden)]
    pub fn set_sub_phase_for_test(&mut self, phase: SubPhase) {
        self.sub_phase = phase;
    }

    #[doc(hidden)]
    pub fn set_free_throws_remaining_for_test(&mut self, n: u8) {
        self.free_throws_remaining = n;
    }

    #[doc(hidden)]
    pub fn set_last_passer_for_test(&mut self, id: Option<String>) {
        self.last_passer_id = id;
    }

    #[doc(hidden)]
    pub fn set_backcourt_elapsed_for_test(&mut self, seconds: f32) {
        self.backcourt_elapsed = seconds;
    }

    #[doc(hidden)]
    pub fn set_period_break_elapsed_for_test(&mut self, seconds: f32) {
        self.period_break_elapsed = seconds;
    }

    #[doc(hidden)]
    pub fn set_scores_for_test(&mut self, home: u32, away: u32) {
        self.home_score = home;
        self.away_score = away;
    }

    #[doc(hidden)]
    pub fn set_possession_context_for_test(
        &mut self,
        turnover_player: Option<String>,
        start_clock: f32,
        start_time: f32,
    ) {
        self.current_possession_turnover_player = turnover_player;
        self.current_possession_start_clock = start_clock;
        self.current_possession_start_time = start_time;
    }

    /// Test hook：直接启动一次发球转换，用于验证发球员界外 placement 的事实语义。
    ///
    /// 该后门绕过了正常得分/失误路径，因此必须显式补一条归因总结——
    /// `PossessionEndCause::TurnoverViolation` 是语义上最接近的合法原因
    /// （测试模拟的是“违例后发球”场景）；否则 `complete_possession` 的
    /// 因果 debug_assert 会正确拒绝这条无归因边界（dev 方案 §3.2 D0.1）。
    #[doc(hidden)]
    pub fn start_inbound_transition_for_test(&mut self) {
        let count = self.completed_possessions as u64;
        if self.last_possession_summary_index != Some(count) {
            self.emit_possession_summary(
                nba_domain::PossessionEndCause::TurnoverViolation,
                None,
                self.current_turnover_player_id(),
                None,
            );
        }
        let pos = self.ball_pos_3d;
        let baseline = Court::nearest_boundary_with_geometry(pos.0, self.rules.court);
        self.start_inbound_transition(baseline, pos);
    }

    #[allow(dead_code)]
    fn player_id_for_team_index(&self, team: &str, index: usize) -> Option<String> {
        self.team_roster_ids(team).into_iter().nth(index)
    }

    fn player_index_for_id(&self, player_id: &str) -> Option<usize> {
        let team = self.physics.get_player(player_id)?.team.as_str();
        self.team_roster_ids(team)
            .into_iter()
            .position(|id| id == player_id)
    }

    // ========================================================================
    // Tick 协议输出
    // ========================================================================

    /// Returns a protocol snapshot without advancing the simulation.
    pub fn snapshot(&self) -> StreamTick {
        self.build_tick()
    }

    fn build_tick(&self) -> StreamTick {
        let active_carrier = match &self.ball_state {
            BallTrajectoryKind::Held { carrier_id }
            | BallTrajectoryKind::InboundReady {
                inbounder_id: carrier_id,
                ..
            }
            | BallTrajectoryKind::Drive {
                driver_id: carrier_id,
                ..
            } => Some(carrier_id.clone()),
            _ => None,
        };
        let mut render_players = self
            .physics
            .get_players()
            .values()
            .map(|p| {
                let norm = Court::ft_to_norm_with_geometry(p.pos_ft, self.rules.court);
                let target_norm = if p.on_court {
                    Some(Court::ft_to_norm_with_geometry(
                        p.target_pos_ft,
                        self.rules.court,
                    ))
                } else {
                    None
                };
                RenderPlayer {
                    id: p.id.clone(),
                    jersey: p.jersey.clone(),
                    team: p.team.clone(),
                    x: (norm.x * 1_000_000.0).round() / 1_000_000.0,
                    y: (norm.y * 1_000_000.0).round() / 1_000_000.0,
                    zone: format!(
                        "{:?}",
                        self.rules.court.region(
                            p.pos_ft,
                            p.team == "home",
                            self.rules.league.three_point_distance_ft,
                        )
                    ),
                    has_ball: active_carrier.as_deref() == Some(p.id.as_str()),
                    on_court: p.on_court,
                    action: p.action.clone(),
                    slot: p.slot.clone(),
                    morale: p.morale.clone(),
                    stm: (p.stamina * 10.0).round() / 10.0,
                    stm_max: p.max_stamina,
                    foul_count: p.foul_count,
                    target_x: target_norm.map(|t| (t.x * 1_000_000.0).round() / 1_000_000.0),
                    target_y: target_norm.map(|t| (t.y * 1_000_000.0).round() / 1_000_000.0),
                    facing_x: if p.on_court {
                        Some((p.facing_dir.x * 1000.0).round() / 1000.0)
                    } else {
                        None
                    },
                    facing_y: if p.on_court {
                        Some((p.facing_dir.y * 1000.0).round() / 1000.0)
                    } else {
                        None
                    },
                }
            })
            .collect::<Vec<_>>();
        render_players.sort_by(|a, b| a.id.cmp(&b.id));
        let ball_norm = Court::ft_to_norm_with_geometry(self.ball_pos_3d.0, self.rules.court);

        let ball_status = match &self.ball_state {
            BallTrajectoryKind::Held { .. } => "HELD",
            BallTrajectoryKind::ControlTransfer { .. } => "CONTROL_TRANSFER",
            BallTrajectoryKind::InboundTransfer { .. } => "INBOUND_TRANSFER",
            BallTrajectoryKind::InboundReady { .. } => "INBOUND_READY",
            BallTrajectoryKind::Pass { .. } => "PASS",
            BallTrajectoryKind::Drive { .. } => "DRIVE",
            BallTrajectoryKind::Shot { .. } => "SHOT",
            BallTrajectoryKind::RimRebound { .. } => "REBOUND",
            BallTrajectoryKind::LooseBall { .. } => "LOOSE_BALL",
            BallTrajectoryKind::Dead { .. } => "DEAD",
        };
        let home_team = RenderTeam {
            id: self.home_team.id.clone(),
            name: self.home_team.name.clone(),
            short_name: self.home_team.short_name.clone(),
        };
        let away_team = RenderTeam {
            id: self.away_team.id.clone(),
            name: self.away_team.name.clone(),
            short_name: self.away_team.short_name.clone(),
        };
        let possession_team = match self.possession {
            Possession::Home => "home",
            Possession::Away => "away",
        };
        let frame = RenderFrame {
            t: (self.current_time * 100.0).round() / 100.0,
            t_game: (self.game_clock * 10.0).round() / 10.0,
            shot_clock: (self.shot_clock * 10.0).round() / 10.0,
            period: self.period,
            phase: format!("{:?}", self.sub_phase),
            possession_id: self.possession_id,
            possession_team: possession_team.to_string(),
            home_team,
            away_team,
            score: RenderScore {
                home: self.home_score,
                away: self.away_score,
            },
            players: render_players,
            ball: RenderBall {
                x: ball_norm.x,
                y: ball_norm.y,
                z: (self.ball_pos_3d.1 * 100.0).round() / 100.0,
                status: ball_status.to_string(),
                holder_id: active_carrier.clone(),
            },
            event_type: self.current_event.clone(),
            events: self.current_event_types.clone(),
            game_flow: format!("{:?}", self.game_flow),
            team_fouls_home: self.team_fouls_home,
            team_fouls_away: self.team_fouls_away,
            free_throws_remaining: self.free_throws_remaining,
            defensive_tactic: Some(self.defensive_tactic_name()),
            event_sequence: self.event_sequence,
            event_log: self.current_event_log.clone(),
            simulation_complete: self.simulation_complete,
            completed_possessions: self.completed_possessions,
            target_possessions: self.target_possessions,
            callout: self.current_callout.clone(),
            intensity: self.current_intensity.clone(),
            rules: frame_rules_from_game_rules(&self.rules),
            stream_projection: "full".to_string(),
            debug: self.last_decision_trace.clone(),
        };

        StreamTick {
            frame,
            tactical_set: self.tactical_set.name_zh().to_string(),
            game_clock: (self.game_clock * 10.0).round() / 10.0,
            keyframe_index: None,
        }
    }
}

/// 与 physics 层 `is_inbound_role` 对应的发球角色判定（引擎侧）。
fn is_inbound_role_action(action: &str) -> bool {
    action == "INBOUND_SETUP" || action == "InboundPositioning" || action == "INBOUND_READY"
}

/// D4.1 因果链定义：事件 kind → 它属于哪个“结果槽位”的父。
///
/// 只登记真实因果关系（动作→结果），不猜测同 tick 相邻即因果：
/// - 投篮释放 → 进筐/失手/篮板
/// - 传球释放 → 接球/被点掉/掉球/被断
/// - 犯规 → 罚球尝试
/// - 突破发起 → 突破结果
fn causal_parent_of(kind: &str) -> Option<&'static str> {
    match kind {
        "SCORE" | "SHOT_MISS" | "REBOUND" => Some("shot"),
        "PASS_RECEIVED" | "PASS_TIPPED" | "PASS_DROPPED" | "STEAL" => Some("pass"),
        "FREE_THROW" => Some("foul"),
        "DRIVE_SCORE" | "DRIVE_MISS" | "DRIVE_STOPPED" => Some("drive"),
        _ => None,
    }
}

/// D4.1 因果链定义：事件 kind → 它在哪个槽位上充当后续事件的父。
fn causal_trigger_slot(kind: &str) -> Option<&'static str> {
    match kind {
        "SHOT_RELEASE" => Some("shot"),
        "PASS" => Some("pass"),
        "FOUL" => Some("foul"),
        "DRIVE_INITIATED" => Some("drive"),
        _ => None,
    }
}

fn physics_fact_to_event(fact: nba_physics::PhysicsFact) -> GameEvent {
    match fact {
        nba_physics::PhysicsFact::BoundaryCross {
            entity_id,
            attempted_pos,
            boundary_name,
        } => GameEvent::BoundaryCross {
            player_id: entity_id,
            pos: (attempted_pos.x, attempted_pos.y),
            boundary_name,
        },
    }
}

fn semantic_contact_to_event(contact: &SemanticContact) -> GameEvent {
    let relative_velocity = contact.raw.relative_velocity.unwrap_or(Vec2::ZERO);
    GameEvent::Contact {
        player_a: contact.raw.entity_a.clone(),
        player_b: contact.raw.entity_b.clone(),
        impact_speed: relative_velocity.length(),
        contact_normal: (contact.raw.normal.x, contact.raw.normal.y),
        is_screen: matches!(
            contact.kind,
            nba_semantics::ContactKind::LegalScreen
                | nba_semantics::ContactKind::IllegalScreenCandidate
        ),
        semantic_kind: format!("{:?}", contact.kind),
        semantic_severity: format!("{:?}", contact.severity),
        possessor_id: contact.context.possessor.clone(),
        legal_position: contact.context.legal_position,
    }
}

fn opposite(p: Possession) -> Possession {
    match p {
        Possession::Home => Possession::Away,
        Possession::Away => Possession::Home,
    }
}
impl MatchEngine {
    fn defensive_tactic_name(&self) -> String {
        match self.possession {
            Possession::Home => self.away_defensive_tactic.name_zh().to_string(),
            Possession::Away => self.home_defensive_tactic.name_zh().to_string(),
        }
    }
}

fn team_name_zh(is_home: bool) -> &'static str {
    if is_home {
        "主队"
    } else {
        "客队"
    }
}

/// 转换决策追踪到协议调试层。
fn convert_trace(trace: &nba_decision::pipeline::DecisionTrace) -> DecisionDebug {
    DecisionDebug {
        player: trace.player_id.clone(),
        chosen: trace.chosen_label.clone(),
        utilities: trace
            .utilities
            .iter()
            .map(|(k, u)| DebugUtility {
                kind: k.clone(),
                utility: *u,
            })
            .collect(),
        probabilities: trace
            .probabilities
            .iter()
            .map(|(k, p)| DebugProb {
                kind: k.clone(),
                prob: *p,
            })
            .collect(),
        flags: trace
            .flags_full
            .iter()
            .map(|(id, reason, penalty)| DebugFlag {
                constraint: id.to_string(),
                reason: reason.clone(),
                penalty: *penalty,
            })
            .collect(),
        blocked: trace
            .blocked
            .iter()
            .map(|(cand, why)| format!("{} ✗ {}", cand, why))
            .collect(),
        active_constraints: trace
            .active_constraints
            .iter()
            .map(|s| s.to_string())
            .collect(),
        enforcement: trace.enforcement.clone(),
    }
}
