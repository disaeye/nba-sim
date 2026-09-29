//! 物理子系统：可替换后端与共用的运动学核心。
//!
//! 本模块按职责分为三层，边界以“能否被两个后端共用”为准：
//!
//! - `mod.rs`（本文件）：对外值类型（`PlayerPhysicsState` / `EntityFilter` /
//!   `RawContact` 等）、后端接口 `SpatialPhysics`、引擎面向的门面 `PhysicsWorld`、
//!   以及两个具体后端（`RapierSpatialPhysics` / `SimpleCirclePhysics`）；
//! - `kinematics.rs`：两个后端共用的规则化运动学（速度提案、碰撞求解、
//!   端点投影与边界事实发射）。它不依赖任何具体后端，因此单独立文件。
//!
//! 分层依据：后端只负责积分与接触检测，规则化的运动学约束全部在
//! `kinematics`；放在任一后端里都会让另一个反向依赖。

mod kinematics;

use kinematics::{
    apply_motion_proposals, cast_capsule_players, collect_contact_facts, make_motion_proposals,
    query_nearby_players, raycast_players, resolve_motion_collisions,
};

use glam::Vec2;
use rapier2d::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use nba_domain::action_window::BallOrientation;
use nba_domain::{FixedDt, GameRules};

use crate::spatial::{OpennessMetric, PassCorridorStatus, SpatialGeometry};

#[derive(Debug, Clone, PartialEq)]
pub enum PhysicsFact {
    /// A kinematic body attempted to cross the playable court boundary.
    BoundaryCross {
        entity_id: String,
        attempted_pos: Vec2,
        boundary_name: String,
    },
}

/// Legacy names are intentionally kept private to the physics implementation;
/// all runtime limits come from `GameRules`.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LocomotionState {
    Idle,
    Accelerating,
    Sprinting,
    Decelerating,
    Stopping,
    Shuffling,
    Airborne,
}

impl LocomotionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Accelerating => "Accelerating",
            Self::Sprinting => "Sprinting",
            Self::Decelerating => "Decelerating",
            Self::Stopping => "Stopping",
            Self::Shuffling => "Shuffling",
            Self::Airborne => "Airborne",
        }
    }
}

/// Runtime body data. Static skill and tendency data is carried through the
/// physics snapshot so decision and officiating layers can read one coherent
/// player view without coupling physics to their outcomes.
#[derive(Debug, Clone)]
pub struct PlayerPhysicsState {
    pub id: String,
    pub jersey: String,
    pub team: String,
    pub pos_ft: Vec2,
    pub vel_ft: Vec2,
    pub accel_ft: Vec2,
    pub target_pos_ft: Vec2,
    pub target_speed_ftps: f32,
    pub max_speed_ftps: f32,
    pub max_accel_ftps2: f32,
    /// Whether this selected roster member participates in court physics.
    pub on_court: bool,
    pub action: String,
    pub slot: String,
    pub morale: String,
    pub stamina: f32,
    pub max_stamina: f32,
    /// Personal fouls accumulated in the current match.
    pub foul_count: u8,
    pub locomotion: LocomotionState,
    pub facing_dir: Vec2,
    /// 持球姿态（面框/背身）：新持球确立时由决策层评估的技术选择，
    /// 失去球权即重置为面框。非持球人恒为面框。
    pub ball_orientation: BallOrientation,
    pub turn_decel_timer: f32,
    pub is_locked_kinematics: bool,
    /// 显式 placement 豁免（gap.md §4.3）：发球程序中的发球员允许被
    /// 放置到界外发球点，且不产生伪造的边界/violation 事实。
    /// 这是结构化状态，不是按 action 字符串匹配。
    pub out_of_bounds_placement: bool,
    /// 正在执行接球跑位（round-10）：接球人向球收敛期间豁免 APF 排斥，
    /// 否则「保持间距」会把他推离球，造成层 A 误判（实测被推 3.2 ft）。
    pub is_receiving_pass: bool,
    /// 持球攻框中（Drive 状态且目标为篮筐方向）：豁免 APF 排斥。
    ///
    /// 真实篮球里攻框者**顶着对抗完成终结**——对抗的代价由终结裁决
    /// （命中率/犯规掷骰）处理，而不是被一个转向力场挡在 7-16 ft 外
    /// （round-18 前实测：88% 突破停滞、篮下出手仅 3%）。
    /// 硬性碰撞分离（min_player_separation）仍然生效——豁免的只是
    /// 转向，不是穿人。
    pub is_driving_to_rim: bool,
    /// BoundaryCross 的边沿锁存：记录上一 tick 是否处于越界（被 clamp）
    /// 状态，使边界事实只在上升沿发射而非逐 tick 重复（电平→边沿）。
    pub boundary_cross_latched: bool,
    pub attributes: nba_domain::PlayerAttributes,
    pub tendencies: nba_domain::PlayerTendencies,
}

/// Backend-independent entity filter used by spatial queries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntityFilter {
    Any,
    Team(String),
    OpposingTeam(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ShapeCastHit {
    pub entity_id: String,
    pub point: Vec2,
    pub distance: f32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RayHit {
    pub entity_id: String,
    pub point: Vec2,
    pub distance: f32,
}

/// Physical contact fact. It intentionally contains no basketball
/// interpretation such as foul, screen, charge, or possession outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct RawContact {
    pub entity_a: String,
    pub entity_b: String,
    pub point: Vec2,
    pub normal: Vec2,
    pub penetration: f32,
    pub relative_velocity: Option<Vec2>,
    pub entity_a_action: Option<String>,
    pub entity_b_action: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PhysicsBackend {
    #[default]
    Rapier,
    SimpleCircle,
}
/// Backend contract. Implementations expose spatial and contact facts only;
/// rule, score, foul, turnover, and tactical outcomes belong above this layer.
pub trait SpatialPhysics {
    fn backend_kind(&self) -> PhysicsBackend;
    fn register_player(&mut self, state: PlayerPhysicsState);
    fn set_player_target(
        &mut self,
        id: &str,
        target_pos_ft: Vec2,
        speed_ftps: f32,
        action: &str,
        slot: &str,
        morale: &str,
    );
    fn set_player_locked(&mut self, id: &str, locked: bool, locomotion: Option<LocomotionState>);
    fn reset_motion(&mut self);
    fn step(&mut self, dt: FixedDt, ball_holder_id: Option<&str>);
    fn drain_contacts(&mut self) -> Vec<RawContact>;
    fn drain_facts(&mut self) -> Vec<PhysicsFact>;
    fn get_players(&self) -> &HashMap<String, PlayerPhysicsState>;
    fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState>;
    fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState>;
    fn teleport_player(&mut self, id: &str, pos: Vec2);
    /// 持球权变更边界：重置失去球权球员的持球姿态（面框基准）。
    fn set_ball_holder(&mut self, holder_id: Option<&str>);
    fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String>;
    fn overlap_circle(&self, center: Vec2, radius: f32) -> Vec<String>;
    fn cast_capsule(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        filter: &EntityFilter,
    ) -> Option<ShapeCastHit>;
    fn raycast(
        &self,
        from: Vec2,
        dir: Vec2,
        max_distance: f32,
        filter: &EntityFilter,
    ) -> Option<RayHit>;
    /// Evaluate a pass corridor against this backend's authoritative view.
    fn pass_corridor(
        &self,
        from: Vec2,
        to: Vec2,
        corridor_radius: f32,
        passer_id: &str,
        receiver_id: &str,
    ) -> PassCorridorStatus {
        SpatialGeometry::check_pass_corridor(
            from,
            to,
            corridor_radius,
            self.get_players(),
            passer_id,
            receiver_id,
            self.rules(),
        )
    }
    /// Evaluate defender proximity against this backend's authoritative view.
    fn openness(&self, player_id: &str) -> OpennessMetric {
        SpatialGeometry::get_openness(player_id, self.get_players(), self.rules())
    }
    /// Return the policy used by this backend for geometry interpretation.
    fn rules(&self) -> &GameRules;
}

/// Stable engine-facing façade. The selected backend is an implementation
/// detail, so the basketball layer can run against Rapier or simple geometry.
pub struct PhysicsWorld {
    backend: Box<dyn SpatialPhysics>,
}

impl Default for PhysicsWorld {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicsWorld {
    pub fn new() -> Self {
        Self::with_backend(&GameRules::default(), PhysicsBackend::Rapier)
    }

    pub fn with_rules(rules: &GameRules) -> Self {
        Self::with_backend(rules, PhysicsBackend::Rapier)
    }

    pub fn with_backend(rules: &GameRules, backend: PhysicsBackend) -> Self {
        let backend: Box<dyn SpatialPhysics> = match backend {
            PhysicsBackend::Rapier => Box::new(RapierSpatialPhysics::with_rules(rules)),
            PhysicsBackend::SimpleCircle => Box::new(SimpleCirclePhysics::with_rules(rules)),
        };
        Self { backend }
    }

    pub fn backend(&self) -> PhysicsBackend {
        self.backend.backend_kind()
    }

    pub fn register_player(&mut self, state: PlayerPhysicsState) {
        self.backend.register_player(state);
    }

    pub fn set_player_target(
        &mut self,
        id: &str,
        target_pos_ft: Vec2,
        speed_ftps: f32,
        action: &str,
        slot: &str,
        morale: &str,
    ) {
        // 几何自洽（dev 方案 D3.1 暴露的缺陷）：任何移动目标点必须是
        // "可站立"的界内坐标。实测站位生成器会给出恰好位于 clamp 边界
        // 线上的目标（如 y = height - player_radius），球员被固定在该点后
        // 每 tick 都满足 `raw_pos != clamped`，边界事实刷屏且发球程序
        // 被长期阻塞（seed 21 单球员连续 675 tick）。
        //
        // 豁免：显式 placement 的球员（发球程序中的发球员）允许在界外。
        // 这里统一约束目标点，使位置与目标口径一致，而不是只在物理步进
        // 时反复把位置 clamp 回来。
        let target_pos_ft = {
            let clamped = self
                .backend
                .rules()
                .court
                .clamp_playable(target_pos_ft, self.backend.rules().player_radius_ft);
            let is_placement_exempt = self
                .backend
                .get_player(id)
                .is_some_and(|p| p.out_of_bounds_placement);
            if is_placement_exempt {
                target_pos_ft
            } else {
                clamped
            }
        };
        self.backend
            .set_player_target(id, target_pos_ft, speed_ftps, action, slot, morale);
    }

    pub fn set_player_locked(
        &mut self,
        id: &str,
        locked: bool,
        locomotion: Option<LocomotionState>,
    ) {
        self.backend.set_player_locked(id, locked, locomotion);
    }
    pub fn reset_motion(&mut self) {
        self.backend.reset_motion();
    }

    pub fn step(&mut self, dt: FixedDt) {
        self.backend.step(dt, None);
    }

    pub fn step_with_ball_holder(&mut self, dt: FixedDt, ball_holder_id: Option<&str>) {
        self.backend.step(dt, ball_holder_id);
    }

    pub fn drain_contacts(&mut self) -> Vec<RawContact> {
        self.backend.drain_contacts()
    }

    pub fn drain_facts(&mut self) -> Vec<PhysicsFact> {
        self.backend.drain_facts()
    }

    pub fn get_players(&self) -> &HashMap<String, PlayerPhysicsState> {
        self.backend.get_players()
    }

    pub fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState> {
        self.backend.get_player(id)
    }

    pub fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState> {
        self.backend.get_player_mut(id)
    }

    pub fn teleport_player(&mut self, id: &str, pos: Vec2) {
        self.backend.teleport_player(id, pos);
    }

    pub fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        self.backend.set_ball_holder(holder_id);
    }

    pub fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String> {
        self.backend.query_nearby(center, radius, filter)
    }

    pub fn overlap_circle(&self, center: Vec2, radius: f32) -> Vec<String> {
        self.backend.overlap_circle(center, radius)
    }

    pub fn cast_capsule(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        filter: &EntityFilter,
    ) -> Option<ShapeCastHit> {
        self.backend.cast_capsule(from, to, radius, filter)
    }

    pub fn raycast(
        &self,
        from: Vec2,
        dir: Vec2,
        max_distance: f32,
        filter: &EntityFilter,
    ) -> Option<RayHit> {
        self.backend.raycast(from, dir, max_distance, filter)
    }
    pub fn pass_corridor(
        &self,
        from: Vec2,
        to: Vec2,
        corridor_radius: f32,
        passer_id: &str,
        receiver_id: &str,
    ) -> PassCorridorStatus {
        self.backend
            .pass_corridor(from, to, corridor_radius, passer_id, receiver_id)
    }

    pub fn openness(&self, player_id: &str) -> OpennessMetric {
        self.backend.openness(player_id)
    }

    pub fn rules(&self) -> &GameRules {
        self.backend.rules()
    }
}
impl SpatialPhysics for PhysicsWorld {
    fn backend_kind(&self) -> PhysicsBackend {
        self.backend.backend_kind()
    }
    fn register_player(&mut self, state: PlayerPhysicsState) {
        self.backend.register_player(state);
    }
    fn set_player_target(
        &mut self,
        id: &str,
        target_pos_ft: Vec2,
        speed_ftps: f32,
        action: &str,
        slot: &str,
        morale: &str,
    ) {
        self.backend
            .set_player_target(id, target_pos_ft, speed_ftps, action, slot, morale);
    }
    fn set_player_locked(&mut self, id: &str, locked: bool, locomotion: Option<LocomotionState>) {
        self.backend.set_player_locked(id, locked, locomotion);
    }
    fn reset_motion(&mut self) {
        self.backend.reset_motion();
    }
    fn step(&mut self, dt: FixedDt, ball_holder_id: Option<&str>) {
        self.backend.step(dt, ball_holder_id);
    }
    fn drain_contacts(&mut self) -> Vec<RawContact> {
        self.backend.drain_contacts()
    }
    fn drain_facts(&mut self) -> Vec<PhysicsFact> {
        self.backend.drain_facts()
    }
    fn get_players(&self) -> &HashMap<String, PlayerPhysicsState> {
        self.backend.get_players()
    }
    fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState> {
        self.backend.get_player(id)
    }
    fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState> {
        self.backend.get_player_mut(id)
    }
    fn teleport_player(&mut self, id: &str, pos: Vec2) {
        self.backend.teleport_player(id, pos);
    }
    fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        self.backend.set_ball_holder(holder_id);
    }
    fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String> {
        self.backend.query_nearby(center, radius, filter)
    }
    fn overlap_circle(&self, center: Vec2, radius: f32) -> Vec<String> {
        self.backend.overlap_circle(center, radius)
    }
    fn cast_capsule(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        filter: &EntityFilter,
    ) -> Option<ShapeCastHit> {
        self.backend.cast_capsule(from, to, radius, filter)
    }
    fn raycast(
        &self,
        from: Vec2,
        dir: Vec2,
        max_distance: f32,
        filter: &EntityFilter,
    ) -> Option<RayHit> {
        self.backend.raycast(from, dir, max_distance, filter)
    }
    fn rules(&self) -> &GameRules {
        self.backend.rules()
    }
}

struct RapierSpatialPhysics {
    rigid_body_set: RigidBodySet,
    collider_set: ColliderSet,
    physics_pipeline: PhysicsPipeline,
    island_manager: IslandManager,
    broad_phase: BroadPhase,
    narrow_phase: NarrowPhase,
    impulse_joint_set: ImpulseJointSet,
    multibody_joint_set: MultibodyJointSet,
    ccd_solver: CCDSolver,
    player_handles: HashMap<String, (RigidBodyHandle, ColliderHandle)>,
    players: HashMap<String, PlayerPhysicsState>,
    active_contacts: HashSet<(String, String)>,
    rules: GameRules,
    pending_contacts: Vec<RawContact>,
    pending_facts: Vec<PhysicsFact>,
}

impl RapierSpatialPhysics {
    fn with_rules(rules: &GameRules) -> Self {
        let rigid_body_set = RigidBodySet::new();
        let mut collider_set = ColliderSet::new();
        let wall_thickness = (rules.player_radius_ft * 2.0).max(f32::EPSILON);
        let friction = rules.player_linear_damping / (rules.player_linear_damping + 1.0);
        let court = rules.court;
        for (half_w, half_h, x, y) in [
            (
                wall_thickness / 2.0,
                court.height_ft / 2.0 + wall_thickness,
                -wall_thickness / 2.0,
                court.height_ft / 2.0,
            ),
            (
                wall_thickness / 2.0,
                court.height_ft / 2.0 + wall_thickness,
                court.width_ft + wall_thickness / 2.0,
                court.height_ft / 2.0,
            ),
            (
                court.width_ft / 2.0 + wall_thickness,
                wall_thickness / 2.0,
                court.width_ft / 2.0,
                -wall_thickness / 2.0,
            ),
            (
                court.width_ft / 2.0 + wall_thickness,
                wall_thickness / 2.0,
                court.width_ft / 2.0,
                court.height_ft + wall_thickness / 2.0,
            ),
        ] {
            collider_set.insert(
                ColliderBuilder::cuboid(half_w, half_h)
                    .translation(vector![x, y])
                    .restitution(0.1)
                    .friction(friction)
                    .build(),
            );
        }
        Self {
            rigid_body_set,
            collider_set,
            physics_pipeline: PhysicsPipeline::new(),
            island_manager: IslandManager::new(),
            broad_phase: BroadPhase::new(),
            narrow_phase: NarrowPhase::new(),
            impulse_joint_set: ImpulseJointSet::new(),
            multibody_joint_set: MultibodyJointSet::new(),
            ccd_solver: CCDSolver::new(),
            player_handles: HashMap::new(),
            players: HashMap::new(),
            active_contacts: HashSet::new(),
            rules: rules.clone(),
            pending_contacts: Vec::new(),
            pending_facts: Vec::new(),
        }
    }

    fn sync_positions(&mut self) {
        let mut ids: Vec<String> = self.player_handles.keys().cloned().collect();
        ids.sort();
        for id in ids {
            let Some((body_handle, _)) = self.player_handles.get(&id).copied() else {
                continue;
            };
            let Some(body) = self.rigid_body_set.get(body_handle) else {
                continue;
            };
            let raw_pos = Vec2::new(body.translation().x, body.translation().y);
            let margin = self
                .rules
                .player_radius_ft
                .min(self.rules.court.width_ft.min(self.rules.court.height_ft) / 2.0);
            if let Some(player) = self.players.get_mut(&id) {
                // 替补席在场地边界之外，且不参与比赛物理：既不回写坐标，
                // 也不产生边界事实（每 tick 6 条伪造 OUT_OF_BOUNDS 会污染
                // 全部事件流与不变量统计，2026-09-02 GAP 复审实证）。
                if !player.on_court {
                    continue;
                }
                if player.out_of_bounds_placement {
                    player.pos_ft = raw_pos;
                    continue;
                }
                // 仅在刚体坐标比运动学结果更"靠内"时采纳，避免用刚体积分
                // 产物覆盖 `apply_motion_proposals` 已 clamp 的权威位置——
                // 否则每 tick 都会重新把位置推到边界外，下一 tick 又产生
                // 新的上升沿（无限循环刷屏）。
                let body_inside = self.rules.court.clamp_playable(raw_pos, margin);
                if (body_inside - raw_pos).length() <= f32::EPSILON {
                    player.pos_ft = raw_pos;
                }
                // 边界事实的唯一发射点是 `apply_motion_proposals`（运动学权威，
                // 在本函数之前运行并已回写 `player.pos_ft`）。本函数只从 Rapier
                // 刚体同步坐标，不再发 BoundaryCross：
                // 双发射点 + 单锁存会让两个源交替产生上升沿，实测导致
                // 单球员连续 675–755 tick 刷屏（dev 方案 D3.1 诊断）。
                // 保留位置 clamp 作为兜底，但不发事实。
            }
        }
        collect_contact_facts(
            &self.players,
            &mut self.active_contacts,
            &mut self.pending_contacts,
            &self.rules,
        );
    }
}

impl SpatialPhysics for RapierSpatialPhysics {
    fn backend_kind(&self) -> PhysicsBackend {
        PhysicsBackend::Rapier
    }

    fn register_player(&mut self, state: PlayerPhysicsState) {
        let body = RigidBodyBuilder::kinematic_position_based()
            .translation(vector![state.pos_ft.x, state.pos_ft.y])
            .lock_rotations()
            .ccd_enabled(true)
            .build();
        let body_handle = self.rigid_body_set.insert(body);
        let collider = ColliderBuilder::ball(self.rules.player_radius_ft)
            .restitution(0.0)
            .friction(self.rules.player_linear_damping / (self.rules.player_linear_damping + 1.0))
            .density(1.0)
            .build();
        let collider_handle =
            self.collider_set
                .insert_with_parent(collider, body_handle, &mut self.rigid_body_set);
        let id = state.id.clone();
        self.player_handles
            .insert(id.clone(), (body_handle, collider_handle));
        self.players.insert(id, state);
    }

    fn set_player_target(
        &mut self,
        id: &str,
        target_pos_ft: Vec2,
        speed_ftps: f32,
        action: &str,
        slot: &str,
        morale: &str,
    ) {
        assert!(
            target_pos_ft.is_finite() && speed_ftps.is_finite(),
            "movement target and speed must be finite"
        );
        if let Some(player) = self.players.get_mut(id) {
            player.target_pos_ft = target_pos_ft;
            player.target_speed_ftps = speed_ftps
                .max(0.0)
                .min(player.max_speed_ftps.min(self.rules.max_player_speed_ftps));
            player.action = action.to_string();
            player.slot = slot.to_string();
            player.morale = morale.to_string();
        }
    }

    fn set_player_locked(&mut self, id: &str, locked: bool, locomotion: Option<LocomotionState>) {
        if let Some(player) = self.players.get_mut(id) {
            player.is_locked_kinematics = locked;
            if let Some(locomotion) = locomotion {
                player.locomotion = locomotion;
            }
        }
    }
    fn reset_motion(&mut self) {
        for player in self.players.values_mut() {
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.turn_decel_timer = 0.0;
        }
    }

    fn step(&mut self, dt: FixedDt, ball_holder_id: Option<&str>) {
        let dt = dt.0.max(f32::EPSILON);
        let fixed_dt = FixedDt(dt);
        let mut proposals =
            make_motion_proposals(&mut self.players, &self.rules, fixed_dt, ball_holder_id);
        resolve_motion_collisions(&mut proposals, &self.rules, fixed_dt);
        apply_motion_proposals(
            &mut self.players,
            &proposals,
            &self.rules,
            fixed_dt,
            &mut self.pending_facts,
        );

        for proposal in &proposals {
            let Some((body_handle, _)) = self.player_handles.get(&proposal.id).copied() else {
                continue;
            };
            if let Some(body) = self.rigid_body_set.get_mut(body_handle) {
                let player = self.players.get(&proposal.id);
                let position = player.map(|p| p.pos_ft).unwrap_or(proposal.next_pos);
                body.set_next_kinematic_translation(vector![position.x, position.y]);
            }
        }

        let gravity = vector![0.0, 0.0];
        let integration_parameters = IntegrationParameters {
            dt,
            ..Default::default()
        };
        self.physics_pipeline.step(
            &gravity,
            &integration_parameters,
            &mut self.island_manager,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.rigid_body_set,
            &mut self.collider_set,
            &mut self.impulse_joint_set,
            &mut self.multibody_joint_set,
            &mut self.ccd_solver,
            None,
            &(),
            &(),
        );
        self.sync_positions();
    }

    fn drain_contacts(&mut self) -> Vec<RawContact> {
        std::mem::take(&mut self.pending_contacts)
    }

    fn drain_facts(&mut self) -> Vec<PhysicsFact> {
        std::mem::take(&mut self.pending_facts)
    }

    fn get_players(&self) -> &HashMap<String, PlayerPhysicsState> {
        &self.players
    }

    fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState> {
        self.players.get(id)
    }

    fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState> {
        self.players.get_mut(id)
    }

    fn teleport_player(&mut self, id: &str, pos: Vec2) {
        if let Some(player) = self.players.get_mut(id) {
            player.pos_ft = pos;
            player.target_pos_ft = pos;
            player.vel_ft = Vec2::ZERO;
        }
        if let Some((body_handle, _)) = self.player_handles.get(id).copied() {
            if let Some(body) = self.rigid_body_set.get_mut(body_handle) {
                body.set_translation(vector![pos.x, pos.y], true);
                body.set_next_kinematic_translation(vector![pos.x, pos.y]);
            }
        }
    }
    fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        // 持球权变更是姿态生命周期边界：失去球权的球员回到面框基准，
        // 新持球人的姿态由决策层评估后写入。
        for (id, player) in &mut self.players {
            if player.on_court && holder_id != Some(id.as_str()) {
                player.ball_orientation = BallOrientation::FaceUp;
            }
        }
    }

    fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String> {
        query_nearby_players(&self.players, center, radius, filter)
    }

    fn overlap_circle(&self, center: Vec2, radius: f32) -> Vec<String> {
        query_nearby_players(&self.players, center, radius, &EntityFilter::Any)
    }

    fn cast_capsule(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        filter: &EntityFilter,
    ) -> Option<ShapeCastHit> {
        cast_capsule_players(
            &self.players,
            from,
            to,
            radius,
            filter,
            self.rules.player_radius_ft,
        )
    }

    fn raycast(
        &self,
        from: Vec2,
        dir: Vec2,
        max_distance: f32,
        filter: &EntityFilter,
    ) -> Option<RayHit> {
        raycast_players(
            &self.players,
            from,
            dir,
            max_distance,
            filter,
            self.rules.player_radius_ft,
        )
    }
    fn rules(&self) -> &GameRules {
        &self.rules
    }
}

/// Dependency-light kinematic backend used by deterministic tests and headless runs.
pub struct SimpleCirclePhysics {
    players: HashMap<String, PlayerPhysicsState>,
    active_contacts: HashSet<(String, String)>,
    rules: GameRules,
    pending_contacts: Vec<RawContact>,
    pending_facts: Vec<PhysicsFact>,
}

impl SimpleCirclePhysics {
    pub fn with_rules(rules: &GameRules) -> Self {
        Self {
            players: HashMap::new(),
            active_contacts: HashSet::new(),
            rules: rules.clone(),
            pending_contacts: Vec::new(),
            pending_facts: Vec::new(),
        }
    }
}

impl SpatialPhysics for SimpleCirclePhysics {
    fn backend_kind(&self) -> PhysicsBackend {
        PhysicsBackend::SimpleCircle
    }

    fn register_player(&mut self, state: PlayerPhysicsState) {
        self.players.insert(state.id.clone(), state);
    }

    fn set_player_target(
        &mut self,
        id: &str,
        target_pos_ft: Vec2,
        speed_ftps: f32,
        action: &str,
        slot: &str,
        morale: &str,
    ) {
        assert!(
            target_pos_ft.is_finite() && speed_ftps.is_finite(),
            "movement target and speed must be finite"
        );
        if let Some(player) = self.players.get_mut(id) {
            player.target_pos_ft = target_pos_ft;
            player.target_speed_ftps = speed_ftps
                .max(0.0)
                .min(player.max_speed_ftps.min(self.rules.max_player_speed_ftps));
            player.action = action.to_string();
            player.slot = slot.to_string();
            player.morale = morale.to_string();
        }
    }

    fn set_player_locked(&mut self, id: &str, locked: bool, locomotion: Option<LocomotionState>) {
        if let Some(player) = self.players.get_mut(id) {
            player.is_locked_kinematics = locked;
            if let Some(locomotion) = locomotion {
                player.locomotion = locomotion;
            }
        }
    }
    fn reset_motion(&mut self) {
        for player in self.players.values_mut() {
            player.vel_ft = Vec2::ZERO;
            player.accel_ft = Vec2::ZERO;
            player.turn_decel_timer = 0.0;
        }
    }

    fn step(&mut self, dt: FixedDt, ball_holder_id: Option<&str>) {
        let dt = FixedDt(dt.0.max(f32::EPSILON));
        let mut proposals =
            make_motion_proposals(&mut self.players, &self.rules, dt, ball_holder_id);
        resolve_motion_collisions(&mut proposals, &self.rules, dt);
        apply_motion_proposals(
            &mut self.players,
            &proposals,
            &self.rules,
            dt,
            &mut self.pending_facts,
        );
        collect_contact_facts(
            &self.players,
            &mut self.active_contacts,
            &mut self.pending_contacts,
            &self.rules,
        );
    }

    fn drain_contacts(&mut self) -> Vec<RawContact> {
        std::mem::take(&mut self.pending_contacts)
    }

    fn drain_facts(&mut self) -> Vec<PhysicsFact> {
        std::mem::take(&mut self.pending_facts)
    }

    fn get_players(&self) -> &HashMap<String, PlayerPhysicsState> {
        &self.players
    }

    fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState> {
        self.players.get(id)
    }

    fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState> {
        self.players.get_mut(id)
    }
    fn teleport_player(&mut self, id: &str, pos: Vec2) {
        if let Some(player) = self.players.get_mut(id) {
            player.pos_ft = pos;
            player.target_pos_ft = pos;
            player.vel_ft = Vec2::ZERO;
        }
    }

    fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        // 与 Rapier 后端同步：姿态生命周期随持球权重置。
        for (id, player) in &mut self.players {
            if player.on_court && holder_id != Some(id.as_str()) {
                player.ball_orientation = BallOrientation::FaceUp;
            }
        }
    }

    fn query_nearby(&self, center: Vec2, radius: f32, filter: &EntityFilter) -> Vec<String> {
        query_nearby_players(&self.players, center, radius, filter)
    }

    fn overlap_circle(&self, center: Vec2, radius: f32) -> Vec<String> {
        query_nearby_players(&self.players, center, radius, &EntityFilter::Any)
    }

    fn cast_capsule(
        &self,
        from: Vec2,
        to: Vec2,
        radius: f32,
        filter: &EntityFilter,
    ) -> Option<ShapeCastHit> {
        cast_capsule_players(
            &self.players,
            from,
            to,
            radius,
            filter,
            self.rules.player_radius_ft,
        )
    }

    fn raycast(
        &self,
        from: Vec2,
        dir: Vec2,
        max_distance: f32,
        filter: &EntityFilter,
    ) -> Option<RayHit> {
        raycast_players(
            &self.players,
            from,
            dir,
            max_distance,
            filter,
            self.rules.player_radius_ft,
        )
    }
    fn rules(&self) -> &GameRules {
        &self.rules
    }
}
