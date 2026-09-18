//! 只读访问器：比赛真相（球权/球态/时钟/比分/阶段与回合上下文）的观察通道。
//!
//! 依据 `current/plan.md` D7 与 `gap.md` §20.1：权威时间、球权和终态没有外部
//! 可写真相字段。字段在 `MatchEngine` 上私有，外部只能经这里的只读访问器或
//! `step()` / `snapshot()` 推进与观测；需要变可写的测试场景走
//! `test_hooks.rs` 里显式命名的 `*_for_test` 钩子。
//!
//! 这些字段曾为 `pub`，使任何一个库调用方都能直接改写比赛状态（如
//! `engine.rules.tick_seconds = f32::MAX`），因此访问器集中在本文件便于审计
//! 「外部究竟能观察什么」。
//!
//! 本文件另有唯一一个写入方法 [`MatchEngine::sync_to_world`]：它把引擎状态
//! 单向投影到 `MatchWorld` 镜像（ADR-011）。它不是访问器，保留在此是因为
//! 它与上述字段一一对应，分散到别处反而看不出两边字段是否同步。

use glam::Vec2;
use nba_domain::{GameEvent, GameFlowState, GameRules, Possession, SubPhase};
use nba_invariants::Violation;
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::PhysicsWorld;

use super::{MatchBoxScore, MatchEngine};

impl MatchEngine {
    // ==================== D4.2 真相字段只读访问器 ====================
    // 比赛真相（球权/球态/时钟/比分/阶段）不再对外暴露可变字段；
    // 外部只能经这些访问器读取，或经 `step()` / `snapshot()` 推进与观测
    // （dev 方案 §7.1 D4.2）。写入一律走引擎内部唯一入口。

    /// 当前宏观生命周期状态。
    pub fn game_flow(&self) -> GameFlowState {
        self.flow.game_flow
    }

    /// 回合序号（每次球权转移递增）。
    pub fn possession_id(&self) -> u32 {
        self.flow.possession_id
    }

    /// 回合子阶段。
    pub fn sub_phase(&self) -> SubPhase {
        self.clock.sub_phase
    }

    /// 权威球态（球权真相的唯一载体）。
    pub fn ball_state(&self) -> &BallTrajectoryKind {
        &self.ball.ball_state
    }

    /// 球的三维位置（ft）。
    pub fn ball_pos_3d(&self) -> (Vec2, f32) {
        self.ball.ball_pos_3d
    }

    /// 比赛时钟（秒，节内倒计时）。
    pub fn game_clock(&self) -> f32 {
        self.clock.game_clock
    }

    /// 进攻时钟（秒）。
    pub fn shot_clock(&self) -> f32 {
        self.clock.shot_clock
    }

    /// 单调仿真时间（秒）。
    pub fn current_time(&self) -> f32 {
        self.clock.current_time
    }

    /// 当前节次。
    pub fn period(&self) -> u32 {
        self.clock.period
    }

    /// 主队比分。
    pub fn home_score(&self) -> u32 {
        self.ledger.home_score
    }

    /// 客队比分。
    pub fn away_score(&self) -> u32 {
        self.ledger.away_score
    }

    /// 客队团队犯规数。
    pub fn team_fouls_away(&self) -> u32 {
        self.ledger.team_fouls_away
    }

    /// 当前罚球执行者（只读）。
    pub fn free_throw_shooter(&self) -> Option<&str> {
        self.ledger.free_throw_shooter.as_deref()
    }

    /// 本回合剩余罚球次数。
    pub fn free_throws_remaining(&self) -> u8 {
        self.ledger.free_throws_remaining
    }

    /// 后场连续持球时间（秒），8 秒违例判据。
    pub fn backcourt_elapsed(&self) -> f32 {
        self.clock.backcourt_elapsed
    }

    /// 已完成回合数。
    pub fn completed_possessions(&self) -> usize {
        self.flow.completed_possessions
    }

    /// 比赛统计分解（2P/3P/FT、失误、犯规）——只读快照。
    pub fn box_score(&self) -> &MatchBoxScore {
        &self.ledger.box_score
    }

    // ------------------------------------------------------------------
    // 只读访问器（`current/plan.md` D7：外部不能改真相，只能观察）。
    //
    // 这些字段曾为 `pub`，使任何一个库调用方都能直接改写比赛状态（如
    // `engine.rules.tick_seconds = f32::MAX`），违反 `gap.md` §20.1
    // 「权威时间、球权和终态没有外部可写真相字段」。现改为私有 +
    // 只读访问器；需要变可写的测试场景走显式命名的 `*_for_test` 钩子。
    // ------------------------------------------------------------------

    /// 单调固定步索引（事实与回放消费者用它定位 tick）。
    pub fn tick_index(&self) -> u64 {
        self.clock.tick_index
    }

    /// 本场生效的规则（只读）。外部需自定义规则时用 `with_rules` 构造。
    pub fn rules(&self) -> &GameRules {
        &self.config.rules
    }

    /// 物理世界（只读）：位置、属性、openness 等查询走这里。
    pub fn physics(&self) -> &PhysicsWorld {
        &self.systems.physics
    }

    /// 获取底层 ECS MatchWorld 纯数据实体世界核的只读引用 (ADR-011)。
    pub fn world(&self) -> &crate::world::MatchWorld {
        &self.systems.world
    }

    /// 同步 MatchEngine 内部状态至 MatchWorld 纯数据实体世界核 (ADR-011)。
    pub fn sync_to_world(&mut self) {
        self.systems.world.clock.period = self.clock.period;
        self.systems.world.clock.game_clock = self.clock.game_clock;
        self.systems.world.clock.shot_clock = self.clock.shot_clock;
        self.systems.world.clock.current_time = self.clock.current_time;
        self.systems.world.clock.sub_phase = self.clock.sub_phase;
        self.systems.world.clock.sub_phase_timer = self.clock.sub_phase_timer;

        self.systems.world.ledger.home_score = self.ledger.home_score;
        self.systems.world.ledger.away_score = self.ledger.away_score;
        self.systems.world.ledger.possession = self.flow.possession;
        self.systems.world.ledger.possession_id = self.flow.possession_id;
        self.systems.world.ledger.home_fouls_in_period = self.ledger.team_fouls_home as u8;
        self.systems.world.ledger.away_fouls_in_period = self.ledger.team_fouls_away as u8;
        self.systems.world.ledger.possession_arrow = self.flow.possession_arrow;

        self.systems.world.ball.pos_3d = self.ball.ball_pos_3d;
        self.systems.world.ball.state = self.ball.ball_state.clone();
        self.systems.world.ball.associated_player_id =
            self.ball.ball_state.associated_player().map(str::to_string);
        self.systems.world.ball.last_touch_team = self
            .ball
            .ball_state
            .possessing_team()
            .unwrap_or(self.flow.possession);

        self.systems.world.game_flow = self.flow.game_flow;
        self.systems.world.tactical_set = self.config.tactical_set;

        self.sync_world_players();
    }

    /// 把物理层的在册球员投影到 `MatchWorld` 的实体数组。
    ///
    /// ## 为何需要（D23）
    ///
    /// `sync_to_world` 此前只写时钟、账本、球与生命周期四个标量组，
    /// `transforms` / `states` / `limits` 永远是空向量。后果是
    /// `PerceptionSystem::evaluate` 的循环 `for i in 0..n` 一次都不执行
    /// （`n == 0`），每个空间核心（Voronoi 开阔度、压迫密度、弱侧探测）都在
    /// 空世界上计算，返回值被丢弃——「接了感知系统」不成立。
    ///
    /// 数组按 player id 排序后逐个 push，使三个平行数组的下标与
    /// `states[i]` 一一对应（`PerceptionSystem` 按下标取两队身份与位置）。
    ///
    /// ## 只投影在场球员
    ///
    /// `physics.get_players()` 含场上与替补（后者为换人而保持注册），
    /// 而空间拓扑只能看到场上十人。过滤条件用 `on_court`，与物理层的
    /// 分离射线、接触检测口径一致。
    fn sync_world_players(&mut self) {
        let mut ids: Vec<String> = self.systems.physics.get_players().keys().cloned().collect();
        ids.sort();
        let world = &mut self.systems.world;
        world.transforms.clear();
        world.states.clear();
        world.limits.clear();
        let players = self.systems.physics.get_players();
        for id in ids {
            let Some(player) = players.get(&id) else {
                continue;
            };
            if !player.on_court {
                continue;
            }
            world.transforms.push(crate::world::TransformComponent {
                pos_ft: player.pos_ft,
                vel_ft: player.vel_ft,
                accel_ft: player.accel_ft,
                facing: player.facing_dir,
            });
            world.limits.push(crate::world::PhysicalLimitComponent {
                max_speed_ftps: player.max_speed_ftps,
                max_accel_ftps2: player.max_accel_ftps2,
                // 生产物理对「速度改变量」的唯一约束是 `velocity_is_feasible`
                // 的 `|v - current| <= max_accel * dt`（movement.rs）；侧向变向
                // 与前进共用同一上限，不存在单独的抓地力常数。因此镜像值取同一量。
                traction_limit: player.max_accel_ftps2,
            });
            world.states.push(crate::world::PlayerRuntimeComponent {
                id: player.id.clone(),
                team: player.team.clone(),
                jersey: player.jersey.clone(),
                on_court: player.on_court,
                stamina: player.stamina,
                max_stamina: player.max_stamina,
                foul_count: player.foul_count,
                morale: player.morale.clone(),
                action: player.action.clone(),
                slot: player.slot.clone(),
                locomotion: player.locomotion,
            });
        }
    }

    /// 所属方名单顺序（只读）；用于验证顺序不携带语义（ADR-005）。
    pub fn away_roster_order(&self) -> &[String] {
        &self.config.away_roster_order
    }

    /// FIBA 交替拥有箭头指向（只读，D20）。
    pub fn possession_arrow(&self) -> Option<Possession> {
        self.flow.possession_arrow
    }

    /// 裁决争球 / 纠缠球（Held Ball，D20）。
    /// 在 FIBA 模式下（use_alternate_possession_arrow=true）依据球权箭头裁定，并翻转箭头；
    /// 在 NBA 模式下执行跳球争顶程序。
    pub fn resolve_held_ball(&mut self) -> Possession {
        if self.config.rules.league.use_alternate_possession_arrow {
            let awarded = self.flow.possession_arrow.unwrap_or(Possession::Away);
            let next_arrow = match awarded {
                Possession::Home => Possession::Away,
                Possession::Away => Possession::Home,
            };
            self.flow.possession_arrow = Some(next_arrow);
            self.flow.possession = awarded;
            awarded
        } else {
            // NBA: 跳球
            self.flow.possession
        }
    }

    /// 最近一次 `step()` 产生的不变量违反（只读）。
    pub fn last_tick_violations(&self) -> &[Violation] {
        &self.audit.last_tick_violations
    }

    /// 发球基线位置（只读）。
    pub fn inbound_baseline(&self) -> Vec2 {
        self.flow.inbound_baseline
    }

    /// 待发布事件队列（只读；写入走引擎内部路径）。
    pub fn pending_events(&self) -> &[GameEvent] {
        &self.journal.pending_events
    }

    pub fn possession(&self) -> nba_domain::Possession {
        self.flow.possession
    }
}
