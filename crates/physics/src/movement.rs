use glam::Vec2;
use rapier2d::prelude::*;
use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use nba_domain::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT};

// Physical body radius: 1.8ft gives a 3.6ft diameter rigid body envelope, guaranteeing min separation >= 3.6ft
pub const PLAYER_RADIUS_FT: f32 = 1.8;
pub const MAX_PLAYER_SPEED_FTPS: f32 = 22.0; // Realistic NBA top sprint speed
pub const MAX_PLAYER_ACCEL_FTPS2: f32 = 35.0; // Realistic human acceleration limit
pub const TURNAROUND_MIN_DECEL_TIME: f32 = 0.12; // Directional change theta > 90 deg requires deceleration ticks

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
            LocomotionState::Idle => "Idle",
            LocomotionState::Accelerating => "Accelerating",
            LocomotionState::Sprinting => "Sprinting",
            LocomotionState::Decelerating => "Decelerating",
            LocomotionState::Stopping => "Stopping",
            LocomotionState::Shuffling => "Shuffling",
            LocomotionState::Airborne => "Airborne",
        }
    }
}

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
    pub has_ball: bool,
    pub action: String,
    pub slot: String,
    pub morale: String,
    pub stamina: f32,
    pub max_stamina: f32,
    pub locomotion: LocomotionState,
    pub facing_dir: Vec2,
    pub turn_decel_timer: f32,
    pub is_locked_kinematics: bool,
}

pub struct PhysicsWorld {
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
}

impl PhysicsWorld {
    pub fn new() -> Self {
        let rigid_body_set = RigidBodySet::new();
        let mut collider_set = ColliderSet::new();

        // Build rigid boundary walls around court
        let court_thickness = 5.0;
        
        let left_wall = ColliderBuilder::cuboid(court_thickness / 2.0, COURT_HEIGHT_FT / 2.0 + court_thickness)
            .translation(vector![-court_thickness / 2.0, COURT_HEIGHT_FT / 2.0])
            .restitution(0.1)
            .friction(0.8)
            .build();
        collider_set.insert(left_wall);

        let right_wall = ColliderBuilder::cuboid(court_thickness / 2.0, COURT_HEIGHT_FT / 2.0 + court_thickness)
            .translation(vector![COURT_WIDTH_FT + court_thickness / 2.0, COURT_HEIGHT_FT / 2.0])
            .restitution(0.1)
            .friction(0.8)
            .build();
        collider_set.insert(right_wall);

        let bottom_wall = ColliderBuilder::cuboid(COURT_WIDTH_FT / 2.0 + court_thickness, court_thickness / 2.0)
            .translation(vector![COURT_WIDTH_FT / 2.0, -court_thickness / 2.0])
            .restitution(0.1)
            .friction(0.8)
            .build();
        collider_set.insert(bottom_wall);

        let top_wall = ColliderBuilder::cuboid(COURT_WIDTH_FT / 2.0 + court_thickness, court_thickness / 2.0)
            .translation(vector![COURT_WIDTH_FT / 2.0, COURT_HEIGHT_FT + court_thickness / 2.0])
            .restitution(0.1)
            .friction(0.8)
            .build();
        collider_set.insert(top_wall);

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
        }
    }

    pub fn register_player(&mut self, state: PlayerPhysicsState) {
        let rb = RigidBodyBuilder::dynamic()
            .translation(vector![state.pos_ft.x, state.pos_ft.y])
            .linear_damping(4.0) // Damping ensures stability and smooth acceleration curves
            .lock_rotations()
            .ccd_enabled(true)
            .build();
        let rb_handle = self.rigid_body_set.insert(rb);

        let collider = ColliderBuilder::ball(PLAYER_RADIUS_FT)
            .restitution(0.0) // Contact without springiness
            .friction(0.8)
            .density(1.0)
            .build();
        let col_handle = self.collider_set.insert_with_parent(collider, rb_handle, &mut self.rigid_body_set);

        let id = state.id.clone();
        self.player_handles.insert(id.clone(), (rb_handle, col_handle));
        self.players.insert(id, state);
    }

    pub fn set_player_target(&mut self, id: &str, target_pos_ft: Vec2, speed_ftps: f32, action: &str, slot: &str, morale: &str) {
        if let Some(p) = self.players.get_mut(id) {
            p.target_pos_ft = target_pos_ft;
            p.target_speed_ftps = speed_ftps.clamp(0.0, MAX_PLAYER_SPEED_FTPS);
            p.action = action.to_string();
            p.slot = slot.to_string();
            p.morale = morale.to_string();
        }
    }

    pub fn set_player_locked(&mut self, id: &str, locked: bool, locomotion: Option<LocomotionState>) {
        if let Some(p) = self.players.get_mut(id) {
            p.is_locked_kinematics = locked;
            if let Some(loco) = locomotion {
                p.locomotion = loco;
            }
        }
    }

    pub fn step(&mut self, dt: f32) {
        // Kinematic steering & Locomotion FSM calculation
        for (id, (rb_handle, _)) in &self.player_handles {
            if let Some(player) = self.players.get_mut(id) {
                if let Some(rb) = self.rigid_body_set.get_mut(*rb_handle) {
                    let current_pos = Vec2::new(rb.translation().x, rb.translation().y);
                    let current_vel = Vec2::new(rb.linvel().x, rb.linvel().y);
                    let current_speed = current_vel.length();

                    if player.is_locked_kinematics {
                        // Kinematically locked (e.g. Airborne during jump shot/rebound)
                        // Preserve existing inertia with slight damping, but ignore new target steering
                        let damped_vel = current_vel * (1.0 - 0.5 * dt).max(0.0);
                        rb.set_linvel(vector![damped_vel.x, damped_vel.y], true);
                        continue;
                    }

                    let to_target = player.target_pos_ft - current_pos;
                    let dist = to_target.length();

                    // Check for sharp turnaround (theta > 90 deg) when traveling at notable speed (> 4 ft/s)
                    if current_speed > 4.0 && dist > 0.5 {
                        let dot = current_vel.normalize().dot(to_target.normalize());
                        if dot < 0.0 { // Angle > 90 deg
                            if player.turn_decel_timer <= 0.0 {
                                player.turn_decel_timer = TURNAROUND_MIN_DECEL_TIME;
                            }
                        }
                    }

                    if player.turn_decel_timer > 0.0 {
                        player.turn_decel_timer = (player.turn_decel_timer - dt).max(0.0);
                    }

                    let desired_vel = if player.turn_decel_timer > 0.0 {
                        // In turnaround penalty: force deceleration before re-accelerating in new direction
                        current_vel * 0.4
                    } else if dist > 0.15 {
                        let speed = (player.target_speed_ftps * (dist / 2.0).clamp(0.15, 1.0))
                            .min(MAX_PLAYER_SPEED_FTPS);
                        to_target.normalize() * speed
                    } else {
                        Vec2::ZERO
                    };

                    let vel_diff = desired_vel - current_vel;
                    let max_delta_v = MAX_PLAYER_ACCEL_FTPS2 * dt;
                    let steer_delta_v = if vel_diff.length() > max_delta_v {
                        vel_diff.normalize() * max_delta_v
                    } else {
                        vel_diff
                    };

                    let next_vel = current_vel + steer_delta_v;
                    let clamped_vel = if next_vel.length() > MAX_PLAYER_SPEED_FTPS {
                        next_vel.normalize() * MAX_PLAYER_SPEED_FTPS
                    } else {
                        next_vel
                    };

                    let computed_accel = (clamped_vel - current_vel) / dt;
                    player.accel_ft = computed_accel;

                    // Update Locomotion FSM state
                    let speed = clamped_vel.length();
                    player.locomotion = if player.is_locked_kinematics {
                        LocomotionState::Airborne
                    } else if speed < 0.25 {
                        if player.target_speed_ftps > 1.0 && dist > 0.5 {
                            LocomotionState::Accelerating
                        } else {
                            LocomotionState::Idle
                        }
                    } else if player.turn_decel_timer > 0.0 || (current_speed > speed + 1.0) {
                        LocomotionState::Decelerating
                    } else if speed > 14.0 {
                        LocomotionState::Sprinting
                    } else if player.action.contains("Defend") || player.action.contains("Drop") || player.action.contains("Closeout") {
                        LocomotionState::Shuffling
                    } else if speed > current_speed + 0.5 {
                        LocomotionState::Accelerating
                    } else {
                        LocomotionState::Shuffling
                    };

                    if speed > 0.5 {
                        player.facing_dir = clamped_vel.normalize();
                    }

                    rb.set_linvel(vector![clamped_vel.x, clamped_vel.y], true);
                }
            }
        }

        // Run Rapier2D Physics with 4 sub-steps for precision
        let gravity = vector![0.0, 0.0];
        let sub_steps = 4;
        let sub_dt = dt / (sub_steps as f32);
        let mut integration_parameters = IntegrationParameters::default();
        integration_parameters.dt = sub_dt;

        for _ in 0..sub_steps {
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
        }

        // Synchronize positions and clamp within court boundary
        for (id, (rb_handle, _)) in &self.player_handles {
            if let Some(rb) = self.rigid_body_set.get(*rb_handle) {
                if let Some(player) = self.players.get_mut(id) {
                    let px = rb.translation().x.clamp(1.5, COURT_WIDTH_FT - 1.5);
                    let py = rb.translation().y.clamp(1.5, COURT_HEIGHT_FT - 1.5);
                    player.pos_ft = Vec2::new(px, py);
                    player.vel_ft = Vec2::new(rb.linvel().x, rb.linvel().y);
                }
            }
        }
    }

    pub fn get_players(&self) -> &HashMap<String, PlayerPhysicsState> {
        &self.players
    }

    pub fn get_player(&self, id: &str) -> Option<&PlayerPhysicsState> {
        self.players.get(id)
    }

    pub fn get_player_mut(&mut self, id: &str) -> Option<&mut PlayerPhysicsState> {
        self.players.get_mut(id)
    }

    pub fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        for (id, player) in self.players.iter_mut() {
            player.has_ball = match holder_id {
                Some(h_id) => id == h_id,
                None => false,
            };
        }
    }
}
