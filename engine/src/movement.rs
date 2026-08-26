use glam::Vec2;
use rapier2d::prelude::*;
use std::collections::HashMap;

use crate::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT};

// Physical body radius: 1.8ft gives a 3.6ft diameter rigid body envelope, completely preventing <3.2ft overlap
pub const PLAYER_RADIUS_FT: f32 = 1.8;
pub const MAX_PLAYER_SPEED_FTPS: f32 = 22.0; // Max human sprint speed limit
pub const MAX_PLAYER_ACCEL_FTPS2: f32 = 35.0; // Max human acceleration

#[derive(Debug, Clone)]
pub struct PlayerPhysicsState {
    pub id: String,
    pub jersey: String,
    pub team: String,
    pub pos_ft: Vec2,
    pub vel_ft: Vec2,
    pub target_pos_ft: Vec2,
    pub target_speed_ftps: f32,
    pub has_ball: bool,
    pub action: String,
    pub stamina: f32,
    pub max_stamina: f32,
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

        // Build rigid court boundaries
        let court_thickness = 5.0;
        
        // Left wall (x = -court_thickness/2)
        let left_wall = ColliderBuilder::cuboid(court_thickness / 2.0, COURT_HEIGHT_FT / 2.0 + court_thickness)
            .translation(vector![-court_thickness / 2.0, COURT_HEIGHT_FT / 2.0])
            .restitution(0.2)
            .friction(0.8)
            .build();
        collider_set.insert(left_wall);

        // Right wall (x = COURT_WIDTH_FT + court_thickness/2)
        let right_wall = ColliderBuilder::cuboid(court_thickness / 2.0, COURT_HEIGHT_FT / 2.0 + court_thickness)
            .translation(vector![COURT_WIDTH_FT + court_thickness / 2.0, COURT_HEIGHT_FT / 2.0])
            .restitution(0.2)
            .friction(0.8)
            .build();
        collider_set.insert(right_wall);

        // Bottom wall (y = -court_thickness/2)
        let bottom_wall = ColliderBuilder::cuboid(COURT_WIDTH_FT / 2.0 + court_thickness, court_thickness / 2.0)
            .translation(vector![COURT_WIDTH_FT / 2.0, -court_thickness / 2.0])
            .restitution(0.2)
            .friction(0.8)
            .build();
        collider_set.insert(bottom_wall);

        // Top wall (y = COURT_HEIGHT_FT + court_thickness/2)
        let top_wall = ColliderBuilder::cuboid(COURT_WIDTH_FT / 2.0 + court_thickness, court_thickness / 2.0)
            .translation(vector![COURT_WIDTH_FT / 2.0, COURT_HEIGHT_FT + court_thickness / 2.0])
            .restitution(0.2)
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
            .linear_damping(3.0) // Strong damping prevents sliding & oscillation
            .lock_rotations()
            .ccd_enabled(true)
            .build();
        let rb_handle = self.rigid_body_set.insert(rb);

        let collider = ColliderBuilder::ball(PLAYER_RADIUS_FT)
            .restitution(0.0) // Inelastic collision - players absorb contact
            .friction(0.8)
            .density(1.0)
            .build();
        let col_handle = self.collider_set.insert_with_parent(collider, rb_handle, &mut self.rigid_body_set);

        let id = state.id.clone();
        self.player_handles.insert(id.clone(), (rb_handle, col_handle));
        self.players.insert(id, state);
    }

    pub fn set_player_target(&mut self, id: &str, target_pos_ft: Vec2, speed_ftps: f32, action: &str) {
        if let Some(p) = self.players.get_mut(id) {
            p.target_pos_ft = target_pos_ft;
            p.target_speed_ftps = speed_ftps.clamp(0.0, MAX_PLAYER_SPEED_FTPS);
            p.action = action.to_string();
        }
    }

    pub fn step(&mut self, dt: f32) {
        // 1. Apply kinematic steering force towards target for each player
        for (id, (rb_handle, _)) in &self.player_handles {
            if let Some(player) = self.players.get(id) {
                if let Some(rb) = self.rigid_body_set.get_mut(*rb_handle) {
                    let current_pos = Vec2::new(rb.translation().x, rb.translation().y);
                    let to_target = player.target_pos_ft - current_pos;
                    let dist = to_target.length();

                    let desired_vel = if dist > 0.1 {
                        let speed = (player.target_speed_ftps * (dist / 2.5).clamp(0.1, 1.0))
                            .min(MAX_PLAYER_SPEED_FTPS);
                        to_target.normalize() * speed
                    } else {
                        Vec2::ZERO
                    };

                    let current_vel = Vec2::new(rb.linvel().x, rb.linvel().y);
                    let vel_diff = desired_vel - current_vel;
                    
                    let max_delta_v = MAX_PLAYER_ACCEL_FTPS2 * dt;
                    let steer_delta_v = if vel_diff.length() > max_delta_v {
                        vel_diff.normalize() * max_delta_v
                    } else {
                        vel_diff
                    };

                    let target_vel = current_vel + steer_delta_v;
                    let clamped_vel = if target_vel.length() > MAX_PLAYER_SPEED_FTPS {
                        target_vel.normalize() * MAX_PLAYER_SPEED_FTPS
                    } else {
                        target_vel
                    };

                    rb.set_linvel(vector![clamped_vel.x, clamped_vel.y], true);
                }
            }
        }

        // 2. Step Rapier2D Physics Pipeline with Sub-stepping for ultra-high fidelity
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

        // 3. Read back synchronized physics positions and velocities
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

    pub fn set_ball_holder(&mut self, holder_id: Option<&str>) {
        for (id, p) in self.players.iter_mut() {
            p.has_ball = match holder_id {
                Some(hid) => id == hid,
                None => false,
            };
        }
    }
}
