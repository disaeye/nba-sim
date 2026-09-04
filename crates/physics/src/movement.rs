use glam::Vec2;
use rapier2d::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

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
    pub has_ball: bool,
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
    pub turn_decel_timer: f32,
    pub is_locked_kinematics: bool,
    pub attributes: nba_domain::PlayerAttributes,
    pub roles: Vec<nba_domain::PlayerRole>,
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
    fn step(&mut self, dt: FixedDt);
    fn drain_contacts(&mut self) -> Vec<RawContact>;
    fn drain_facts(&mut self) -> Vec<PhysicsFact>;
    fn get_players(&self) -> &HashMap<String, PlayerPhysicsState>;
    fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState>;
    fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState>;
    fn set_ball_holder(&mut self, holder_id: Option<&str>);
    fn teleport_player(&mut self, id: &str, pos: Vec2);
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
        self.backend.step(dt);
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

    pub fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        self.backend.set_ball_holder(holder_id);
    }
    pub fn teleport_player(&mut self, id: &str, pos: Vec2) {
        self.backend.teleport_player(id, pos);
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
    fn step(&mut self, dt: FixedDt) {
        self.backend.step(dt);
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
            let clamped = self.rules.court.clamp_playable(raw_pos, margin);
            if let Some(player) = self.players.get_mut(&id) {
                // 替补席在场地边界之外，且不参与比赛物理：既不回写坐标，
                // 也不产生边界事实（每 tick 6 条伪造 OUT_OF_BOUNDS 会污染
                // 全部事件流与不变量统计，2026-09-02 GAP 复审实证）。
                if !player.on_court {
                    continue;
                }
                player.pos_ft = clamped;
                // Kinematic velocity is owned by the custom movement solver.
                // Rapier's `linvel` is an integration artifact here: the body
                // target is applied across sub-steps, so exposing it would make
                // acceleration depend on the adapter's internal sub-step count.
                if raw_pos != clamped {
                    self.pending_facts.push(PhysicsFact::BoundaryCross {
                        entity_id: player.id.clone(),
                        attempted_pos: raw_pos,
                        boundary_name: "COURT".to_string(),
                    });
                }
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

    fn step(&mut self, dt: FixedDt) {
        let dt = dt.0.max(f32::EPSILON);
        let fixed_dt = FixedDt(dt);
        let mut proposals = make_motion_proposals(&mut self.players, &self.rules, fixed_dt);
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
        for (id, player) in &mut self.players {
            player.has_ball =
                player.on_court && holder_id.map(|holder| holder == id).unwrap_or(false);
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

    fn step(&mut self, dt: FixedDt) {
        let dt = FixedDt(dt.0.max(f32::EPSILON));
        let mut proposals = make_motion_proposals(&mut self.players, &self.rules, dt);
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
        for (id, player) in &mut self.players {
            player.has_ball =
                player.on_court && holder_id.map(|holder| holder == id).unwrap_or(false);
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

#[derive(Debug, Clone)]
struct MotionProposal {
    id: String,
    current_pos: Vec2,
    current_vel: Vec2,
    next_pos: Vec2,
    next_vel: Vec2,
    max_speed_ftps: f32,
    max_accel_ftps2: f32,
}

fn make_motion_proposals(
    players: &mut HashMap<String, PlayerPhysicsState>,
    rules: &GameRules,
    dt: FixedDt,
) -> Vec<MotionProposal> {
    let dt = dt.0.max(f32::EPSILON);
    let mut ids: Vec<String> = players.keys().cloned().collect();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            let player = players.get_mut(&id)?;
            if !player.on_court {
                player.vel_ft = Vec2::ZERO;
                player.accel_ft = Vec2::ZERO;
                return None;
            }
            let current_pos = player.pos_ft;
            let current_vel = player.vel_ft;
            let current_speed = current_vel.length();
            let max_speed = player
                .max_speed_ftps
                .min(rules.max_player_speed_ftps)
                .max(0.0);
            let max_accel = player
                .max_accel_ftps2
                .min(rules.max_player_accel_ftps2)
                .max(0.0);
            let to_target = player.target_pos_ft - current_pos;
            let distance = to_target.length();

            if current_speed > 4.0 && distance > 0.5 && current_vel.length_squared() > f32::EPSILON
            {
                let dot = current_vel.normalize().dot(to_target.normalize_or_zero());
                if dot < 0.0 && player.turn_decel_timer <= 0.0 {
                    player.turn_decel_timer = rules.turnaround_min_decel_seconds;
                }
            }
            player.turn_decel_timer = (player.turn_decel_timer - dt).max(0.0);

            let mut next_vel = if player.is_locked_kinematics {
                current_vel * (1.0 - rules.player_linear_damping * dt).max(0.0)
            } else if player.turn_decel_timer > 0.0 {
                current_vel * 0.4
            } else if distance > 0.15 && player.target_speed_ftps > 0.0 {
                let target_speed = (player.target_speed_ftps
                    * (distance / (max_speed * 0.25 + 1.0)).clamp(0.15, 1.0))
                .min(max_speed);
                to_target.normalize_or_zero() * target_speed
            } else {
                Vec2::ZERO
            };

            let velocity_delta = next_vel - current_vel;
            let max_delta = max_accel * dt;
            if velocity_delta.length() > max_delta {
                next_vel = current_vel + velocity_delta.normalize() * max_delta;
            }
            next_vel = next_vel.clamp_length_max(max_speed);

            let speed = next_vel.length();
            player.locomotion = if player.is_locked_kinematics {
                LocomotionState::Airborne
            } else if speed < 0.25 {
                if player.target_speed_ftps > 1.0 && distance > 0.5 {
                    LocomotionState::Accelerating
                } else {
                    LocomotionState::Idle
                }
            } else if player.turn_decel_timer > 0.0 || current_speed > speed + 1.0 {
                LocomotionState::Decelerating
            } else if speed > max_speed * 0.64 {
                LocomotionState::Sprinting
            } else if is_defensive_action(&player.action) {
                LocomotionState::Shuffling
            } else if speed > current_speed + 0.5 {
                LocomotionState::Accelerating
            } else {
                LocomotionState::Shuffling
            };
            let is_defense = is_defensive_action(&player.action);
            if is_defense {
                let target_vec = player.target_pos_ft - current_pos;
                if target_vec.length() > 0.2 {
                    player.facing_dir = target_vec.normalize_or_zero();
                }
            } else if speed > 0.5 {
                player.facing_dir = next_vel.normalize_or_zero();
            }

            Some(MotionProposal {
                id,
                current_pos,
                current_vel,
                next_pos: current_pos + next_vel * dt,
                next_vel,
                max_speed_ftps: max_speed,
                max_accel_ftps2: max_accel,
            })
        })
        .collect()
}

fn resolve_motion_collisions(proposals: &mut [MotionProposal], rules: &GameRules, dt: FixedDt) {
    // Keep enough numerical cushion for normalized f32 protocol coordinates;
    // the serialized stream must satisfy the configured separation contract.
    let minimum =
        rules.min_player_separation_ft.max(0.0) + rules.separation_safety_margin_ft.max(0.0);
    let dt = dt.0.max(f32::EPSILON);
    let max_iterations = proposals.len().saturating_mul(proposals.len()).max(1);
    for _ in 0..max_iterations {
        let mut changed = false;
        for left_index in 0..proposals.len() {
            for right_index in left_index + 1..proposals.len() {
                let (left_current_pos, right_current_pos) = {
                    let left = &proposals[left_index];
                    let right = &proposals[right_index];
                    (left.current_pos, right.current_pos)
                };
                let current_delta = right_current_pos - left_current_pos;
                let current_distance = current_delta.length();
                let current_normal = if current_distance > f32::EPSILON {
                    current_delta / current_distance
                } else {
                    Vec2::X
                };

                // Brake before the endpoint enters the exclusion radius.
                let gap = (current_distance - minimum).max(0.0);
                let proposed_relative = (proposals[right_index].next_vel
                    - proposals[left_index].next_vel)
                    .dot(current_normal);
                if proposed_relative < 0.0 {
                    let relative_accel = (proposals[left_index].max_accel_ftps2
                        + proposals[right_index].max_accel_ftps2)
                        .max(f32::EPSILON);
                    let safe_closing = (-relative_accel * dt
                        + (relative_accel * relative_accel * dt * dt + 2.0 * relative_accel * gap)
                            .sqrt())
                    .max(0.0);
                    changed |= shift_relative_velocity(
                        proposals,
                        left_index,
                        right_index,
                        current_normal,
                        -safe_closing - proposed_relative,
                        dt,
                    );
                }

                // Use the actual endpoint distance, not only radial velocity:
                // direction changes can close the gap while the initial radial
                // component is near zero. Keep enough gap to brake the relative
                // closing motion during the next step.
                let left_next_pos = left_current_pos + proposals[left_index].next_vel * dt;
                let right_next_pos = right_current_pos + proposals[right_index].next_vel * dt;
                let predicted_delta = right_next_pos - left_next_pos;
                let predicted_distance = predicted_delta.length();
                let relative_accel = (proposals[left_index].max_accel_ftps2
                    + proposals[right_index].max_accel_ftps2)
                    .max(f32::EPSILON);
                let closing_speed = ((current_distance - predicted_distance) / dt).max(0.0);
                let stopping_gap = closing_speed * closing_speed / (2.0 * relative_accel);
                let safe_distance = minimum + stopping_gap;
                if predicted_distance < safe_distance {
                    let endpoint_normal = if predicted_distance > f32::EPSILON {
                        predicted_delta / predicted_distance
                    } else {
                        current_normal
                    };
                    let correction = (safe_distance - predicted_distance) / dt;
                    let left_velocity = proposals[left_index].next_vel;
                    let right_velocity = proposals[right_index].next_vel;
                    let left_target = left_velocity - endpoint_normal * (correction * 0.5);
                    let right_target = right_velocity + endpoint_normal * (correction * 0.5);
                    let left_result = bounded_velocity(
                        proposals[left_index].current_vel,
                        left_target,
                        proposals[left_index].max_speed_ftps,
                        proposals[left_index].max_accel_ftps2,
                        dt,
                    );
                    let right_result = bounded_velocity(
                        proposals[right_index].current_vel,
                        right_target,
                        proposals[right_index].max_speed_ftps,
                        proposals[right_index].max_accel_ftps2,
                        dt,
                    );
                    changed |= left_result != left_velocity || right_result != right_velocity;
                    proposals[left_index].next_vel = left_result;
                    proposals[right_index].next_vel = right_result;
                }
            }
        }
        if !changed {
            break;
        }
    }

    for proposal in proposals.iter_mut() {
        proposal.next_pos = proposal.current_pos + proposal.next_vel * dt;
    }
}

fn shift_relative_velocity(
    proposals: &mut [MotionProposal],
    left_index: usize,
    right_index: usize,
    normal: Vec2,
    correction: f32,
    dt: f32,
) -> bool {
    if correction <= f32::EPSILON || normal.length_squared() <= f32::EPSILON {
        return false;
    }
    let normal = normal.normalize_or_zero();
    let (
        left_current,
        left_next,
        left_speed,
        left_accel,
        right_current,
        right_next,
        right_speed,
        right_accel,
    ) = {
        let left = &proposals[left_index];
        let right = &proposals[right_index];
        (
            left.current_vel,
            left.next_vel,
            left.max_speed_ftps,
            left.max_accel_ftps2,
            right.current_vel,
            right.next_vel,
            right.max_speed_ftps,
            right.max_accel_ftps2,
        )
    };
    let left_capacity =
        max_feasible_velocity_shift(left_current, left_next, -normal, left_speed, left_accel, dt);
    let right_capacity = max_feasible_velocity_shift(
        right_current,
        right_next,
        normal,
        right_speed,
        right_accel,
        dt,
    );
    let total_capacity = left_capacity + right_capacity;
    if total_capacity <= f32::EPSILON {
        return false;
    }
    let applied = correction.min(total_capacity);
    let left_amount = applied * (left_capacity / total_capacity);
    let right_amount = applied - left_amount;
    let left_result = left_next - normal * left_amount;
    let right_result = right_next + normal * right_amount;
    let changed = left_result != left_next || right_result != right_next;
    proposals[left_index].next_vel = left_result;
    proposals[right_index].next_vel = right_result;
    changed
}

fn max_feasible_velocity_shift(
    current: Vec2,
    candidate: Vec2,
    direction: Vec2,
    max_speed: f32,
    max_accel: f32,
    dt: f32,
) -> f32 {
    let direction = direction.normalize_or_zero();
    if direction.length_squared() <= f32::EPSILON
        || !velocity_is_feasible(candidate, current, max_speed, max_accel, dt)
    {
        return 0.0;
    }
    let mut low = 0.0;
    let mut high = candidate.length() + max_speed.max(0.0) + max_accel.max(0.0) * dt + 1.0;
    for _ in 0..32 {
        let middle = (low + high) * 0.5;
        if velocity_is_feasible(
            candidate + direction * middle,
            current,
            max_speed,
            max_accel,
            dt,
        ) {
            low = middle;
        } else {
            high = middle;
        }
    }
    low
}

fn velocity_is_feasible(
    velocity: Vec2,
    current: Vec2,
    max_speed: f32,
    max_accel: f32,
    dt: f32,
) -> bool {
    let tolerance = 1e-5;
    velocity.length_squared() <= max_speed.max(0.0).powi(2) + tolerance
        && (velocity - current).length_squared() <= (max_accel.max(0.0) * dt).powi(2) + tolerance
}
fn apply_motion_proposals(
    players: &mut HashMap<String, PlayerPhysicsState>,
    proposals: &[MotionProposal],
    rules: &GameRules,
    dt: FixedDt,
    pending_facts: &mut Vec<PhysicsFact>,
) {
    let dt = dt.0.max(f32::EPSILON);
    let margin = rules
        .player_radius_ft
        .min(rules.court.width_ft.min(rules.court.height_ft) / 2.0);
    for proposal in proposals {
        let Some(player) = players.get_mut(&proposal.id) else {
            continue;
        };
        if !player.on_court {
            continue;
        }
        let raw_pos = proposal.next_pos;
        let next_pos = rules.court.clamp_playable(raw_pos, margin);
        player.pos_ft = next_pos;
        if raw_pos != next_pos {
            pending_facts.push(PhysicsFact::BoundaryCross {
                entity_id: player.id.clone(),
                attempted_pos: raw_pos,
                boundary_name: "COURT".to_string(),
            });
        }
    }
    // Kinematic endpoint projection is the final authority for the spatial
    // contract. It is independent of backend integration and handles all
    // simultaneous pairs, including turns and converging trajectories.
    let minimum =
        rules.min_player_separation_ft.max(0.0) + rules.separation_safety_margin_ft.max(0.0);
    let mut ids: Vec<String> = players.keys().cloned().collect();
    ids.sort();
    for _ in 0..ids.len().saturating_mul(ids.len()).max(1) {
        let mut changed = false;
        for (index, left_id) in ids.iter().enumerate() {
            for right_id in ids.iter().skip(index + 1) {
                let (Some(left), Some(right)) = (players.get(left_id), players.get(right_id))
                else {
                    continue;
                };
                if !left.on_court || !right.on_court {
                    continue;
                }
                let distance = left.pos_ft.distance(right.pos_ft);
                if distance >= minimum {
                    continue;
                }
                let normal = (right.pos_ft - left.pos_ft).normalize_or_zero();
                let correction = (minimum - distance) * 0.5;
                let left_pos = rules
                    .court
                    .clamp_playable(left.pos_ft - normal * correction, margin);
                let right_pos = rules
                    .court
                    .clamp_playable(right.pos_ft + normal * correction, margin);
                if let Some(left) = players.get_mut(left_id) {
                    left.pos_ft = left_pos;
                }
                if let Some(right) = players.get_mut(right_id) {
                    right.pos_ft = right_pos;
                }
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    // The endpoint projection is part of the accepted kinematic step. Keep
    // the reported velocity and acceleration consistent with that endpoint;
    // the next proposal will apply the acceleration envelope again.
    for proposal in proposals {
        let Some(player) = players.get_mut(&proposal.id) else {
            continue;
        };
        if !player.on_court {
            continue;
        }
        let final_vel = (player.pos_ft - proposal.current_pos) / dt;
        player.vel_ft = final_vel;
        player.accel_ft = (final_vel - proposal.current_vel) / dt;
    }
}
fn bounded_velocity(current: Vec2, desired: Vec2, max_speed: f32, max_accel: f32, dt: f32) -> Vec2 {
    let delta = desired - current;
    let max_delta = max_accel.max(0.0) * dt;
    let stepped = if delta.length() > max_delta {
        current + delta.normalize() * max_delta
    } else {
        desired
    };
    stepped.clamp_length_max(max_speed.max(0.0))
}

fn is_defensive_action(action: &str) -> bool {
    action.contains("DEFEND")
        || action.contains("Defend")
        || action.contains("DROP")
        || action.contains("Drop")
        || action.contains("CLOSEOUT")
        || action.contains("Closeout")
}

fn collect_contact_facts(
    players: &HashMap<String, PlayerPhysicsState>,
    active_contacts: &mut HashSet<(String, String)>,
    pending_contacts: &mut Vec<RawContact>,
    rules: &GameRules,
) {
    let mut ids: Vec<String> = players.keys().cloned().collect();
    ids.sort();
    let contact_limit = (rules.player_radius_ft * 2.0 + rules.contact_margin_ft).max(0.0);
    let mut touching_now = HashSet::new();
    for (index, left_id) in ids.iter().enumerate() {
        for right_id in ids.iter().skip(index + 1) {
            let (Some(left), Some(right)) = (players.get(left_id), players.get(right_id)) else {
                continue;
            };
            if !left.on_court || !right.on_court {
                continue;
            }

            let delta = right.pos_ft - left.pos_ft;
            let distance = delta.length();
            if distance >= contact_limit {
                continue;
            }
            let normal = delta.normalize_or_zero();
            let edge = (left.id.clone(), right.id.clone());
            touching_now.insert(edge.clone());
            if active_contacts.contains(&edge) {
                continue;
            }
            pending_contacts.push(RawContact {
                entity_a: left.id.clone(),
                entity_b: right.id.clone(),
                point: left.pos_ft + delta * 0.5,
                normal,
                penetration: (contact_limit - distance).max(0.0),
                relative_velocity: Some(right.vel_ft - left.vel_ft),
                entity_a_action: Some(left.action.clone()),
                entity_b_action: Some(right.action.clone()),
            });
        }
    }
    *active_contacts = touching_now;
}

fn passes_filter(player: &PlayerPhysicsState, filter: &EntityFilter) -> bool {
    player.on_court
        && match filter {
            EntityFilter::Any => true,
            EntityFilter::Team(team) => &player.team == team,
            EntityFilter::OpposingTeam(team) => &player.team != team,
        }
}

fn query_nearby_players(
    players: &HashMap<String, PlayerPhysicsState>,
    center: Vec2,
    radius: f32,
    filter: &EntityFilter,
) -> Vec<String> {
    let radius = radius.max(0.0);
    let mut result: Vec<(String, f32)> = players
        .values()
        .filter(|player| passes_filter(player, filter))
        .map(|player| (player.id.clone(), (player.pos_ft - center).length_squared()))
        .filter(|(_, distance_sq)| *distance_sq <= radius * radius)
        .collect();
    result.sort_by(|left, right| {
        left.1
            .partial_cmp(&right.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    result.into_iter().map(|(id, _)| id).collect()
}

fn point_segment_distance(point: Vec2, from: Vec2, to: Vec2) -> (f32, Vec2) {
    let segment = to - from;
    let length_sq = segment.length_squared();
    if length_sq <= f32::EPSILON {
        return ((point - from).length(), from);
    }
    let t = ((point - from).dot(segment) / length_sq).clamp(0.0, 1.0);
    let projection = from + segment * t;
    ((point - projection).length(), projection)
}

fn cast_capsule_players(
    players: &HashMap<String, PlayerPhysicsState>,
    from: Vec2,
    to: Vec2,
    radius: f32,
    filter: &EntityFilter,
    player_radius: f32,
) -> Option<ShapeCastHit> {
    let effective_radius = radius.max(0.0) + player_radius.max(0.0);
    let mut hits: Vec<ShapeCastHit> = players
        .values()
        .filter(|player| passes_filter(player, filter))
        .filter_map(|player| {
            let (distance, point) = point_segment_distance(player.pos_ft, from, to);
            (distance <= effective_radius).then(|| ShapeCastHit {
                entity_id: player.id.clone(),
                point,
                distance: (point - from).length(),
            })
        })
        .collect();
    hits.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.entity_id.cmp(&right.entity_id))
    });
    hits.into_iter().next()
}

fn raycast_players(
    players: &HashMap<String, PlayerPhysicsState>,
    from: Vec2,
    dir: Vec2,
    max_distance: f32,
    filter: &EntityFilter,
    player_radius: f32,
) -> Option<RayHit> {
    let direction = dir.normalize_or_zero();
    if direction.length_squared() <= f32::EPSILON || max_distance < 0.0 {
        return None;
    }
    let radius = player_radius.max(0.0);
    let mut hits: Vec<RayHit> = players
        .values()
        .filter(|player| passes_filter(player, filter))
        .filter_map(|player| {
            let offset = player.pos_ft - from;
            let along = offset.dot(direction);
            if along < 0.0 || along > max_distance {
                return None;
            }
            let perpendicular_sq = (offset - direction * along).length_squared();
            if perpendicular_sq > radius * radius {
                return None;
            }
            let depth = (radius * radius - perpendicular_sq).sqrt();
            let distance = (along - depth).max(0.0);
            (distance <= max_distance).then(|| RayHit {
                entity_id: player.id.clone(),
                point: from + direction * distance,
                distance,
            })
        })
        .collect();
    hits.sort_by(|left, right| {
        left.distance
            .partial_cmp(&right.distance)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.entity_id.cmp(&right.entity_id))
    });
    hits.into_iter().next()
}
