//! D14 MatchEngine 最小只读 snapshot 投影视图。
//!
//! 依据 docs/architecture.md §2 [12] EmitPhase 与 §4 无副作用快照规定：
//! 包含回合状态、时钟、比分、球态、阵容与只读物理世界视图，
//! 供 CLI、评判器、测试、回放等统一获取只读视图，不产生任何副作用。

use glam::Vec2;
use nba_decision::tactics::TacticalSet;
use nba_domain::{GameFlowState, GameRules, Possession, SubPhase};
use nba_physics::ballistics::BallTrajectoryKind;
use nba_physics::movement::{PhysicsWorld, PlayerPhysicsState};

use crate::match_engine::MatchBoxScore;

/// MatchEngine 内部状态的只读零拷贝借用投影。
pub struct EngineSnapshot<'a> {
    pub game: GameStateView<'a>,
    pub ball: BallStateView<'a>,
    pub lineups: LineupStateView<'a>,
    pub rules: &'a GameRules,
    pub box_score: &'a MatchBoxScore,
    pub physics: &'a PhysicsWorld,
}

impl<'a> std::fmt::Debug for EngineSnapshot<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineSnapshot")
            .field("game", &self.game)
            .field("ball", &self.ball)
            .field("lineups", &self.lineups)
            .field("rules", &self.rules)
            .field("box_score", &self.box_score)
            .finish()
    }
}

/// 比赛进度与回合只读视图。
#[derive(Debug, Clone)]
pub struct GameStateView<'a> {
    pub period: u32,
    pub game_clock: f32,
    pub shot_clock: f32,
    pub current_time: f32,
    pub home_score: u32,
    pub away_score: u32,
    pub possession: Possession,
    pub possession_id: u32,
    pub sub_phase: SubPhase,
    pub sub_phase_timer: f32,
    pub tactical_set: TacticalSet,
    pub game_flow: &'a GameFlowState,
    pub possession_arrow: Option<Possession>,
}

/// 篮球位置与状态只读视图。
#[derive(Debug, Clone)]
pub struct BallStateView<'a> {
    pub pos_3d: (Vec2, f32),
    pub state: &'a BallTrajectoryKind,
    pub active_carrier_or_focus_id: String,
}

/// 场上阵容只读视图。
#[derive(Debug, Clone)]
pub struct LineupStateView<'a> {
    pub home_roster_order: &'a [String],
    pub away_roster_order: &'a [String],
}

impl<'a> EngineSnapshot<'a> {
    /// 依球员 ID 获取球员只读物理与运动状态。
    pub fn get_player(&self, player_id: &str) -> Option<&'a PlayerPhysicsState> {
        self.physics.get_player(player_id)
    }
}
