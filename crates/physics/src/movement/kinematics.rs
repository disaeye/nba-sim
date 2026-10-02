//! 运动学求解：从意图到速度、碰撞分离与端点投影。
//!
//! 本模块是**两个后端共用的**运动学核心，不依赖 Rapier 或任何具体后端：
//! `make_motion_proposals` 把每个球员的目标点与属性转成速度提案，
//! `resolve_motion_collisions` 在提案层面解相对速度的不等式，
//! `apply_motion_proposals` 做权威的端点投影与边界事实发射。
//!
//! 与后端的边界：后端只负责积分与接触检测，规则化的运动学约束全部在此。
//! 分离出来是因为它同时被 `RapierSpatialPhysics` 与 `SimpleCirclePhysics`
//! 调用，放在任一后端里都会让另一个反向依赖。

use glam::Vec2;
use std::collections::{HashMap, HashSet};

use nba_domain::action_window::BallOrientation;
use nba_domain::{FixedDt, GameRules};

use super::{
    EntityFilter, LocomotionState, PhysicsFact, PlayerPhysicsState, RawContact, RayHit,
    ShapeCastHit,
};

#[derive(Clone, Copy)]
struct VelocityLimits {
    max_speed_ftps: f32,
    max_accel_ftps2: f32,
    max_braking_accel_ftps2: f32,
    max_lateral_accel_ftps2: f32,
}

/// 单步速度提案：`make_motion_proposals` 的产物、碰撞求解的输入。
pub(super) struct MotionProposal {
    pub(super) id: String,
    pub(super) current_pos: Vec2,
    pub(super) current_vel: Vec2,
    pub(super) next_pos: Vec2,
    pub(super) next_vel: Vec2,
    pub(super) max_speed_ftps: f32,
    pub(super) max_accel_ftps2: f32,
    pub(super) max_braking_accel_ftps2: f32,
    pub(super) max_lateral_accel_ftps2: f32,
}

pub(super) fn make_motion_proposals(
    players: &mut HashMap<String, PlayerPhysicsState>,
    rules: &GameRules,
    dt: FixedDt,
) -> Vec<MotionProposal> {
    let dt = dt.0;
    assert!(
        dt.is_finite() && dt > 0.0,
        "movement timestep must be finite and positive"
    );
    assert!(
        rules.max_player_speed_ftps.is_finite()
            && rules.max_player_accel_ftps2.is_finite()
            && rules.max_player_braking_accel_ftps2.is_finite()
            && rules.max_player_lateral_accel_ftps2.is_finite()
            && rules.attribute_response_floor.is_finite(),
        "movement rules must contain finite kinematic limits"
    );
    let mut ids: Vec<String> = players.keys().cloned().collect();
    ids.sort();

    // 预收集场上球员物理状态快照，用于人造势能场（APF）多体排斥合力计算
    let on_court_snapshots: Vec<(String, String, Vec2, bool)> = ids
        .iter()
        .filter_map(|id| {
            let p = players.get(id)?;
            if p.on_court {
                Some((p.id.clone(), p.team.clone(), p.pos_ft, p.has_ball))
            } else {
                None
            }
        })
        .collect();

    ids.into_iter()
        .filter_map(|id| {
            let player = players.get_mut(&id)?;
            if !player.on_court {
                player.vel_ft = Vec2::ZERO;
                player.accel_ft = Vec2::ZERO;
                return None;
            }
            assert!(
                player.pos_ft.is_finite()
                    && player.vel_ft.is_finite()
                    && player.target_pos_ft.is_finite()
                    && player.target_speed_ftps.is_finite()
                    && player.max_speed_ftps.is_finite()
                    && player.max_accel_ftps2.is_finite()
                    && player.turn_decel_timer.is_finite()
                    && player.attributes.acceleration.is_finite()
                    && player.attributes.agility.is_finite(),
                "player movement state must be finite"
            );
            assert!(
                player.max_speed_ftps >= 0.0 && player.max_accel_ftps2 >= 0.0,
                "player movement limits must be nonnegative"
            );
            let current_pos = player.pos_ft;
            let current_vel = player.vel_ft;
            let current_speed = current_vel.length();
            let max_speed = player.max_speed_ftps.min(rules.max_player_speed_ftps);
            let max_accel =
                player
                    .max_accel_ftps2
                    .min(nba_domain::capability::effective_max_accel(
                        rules,
                        &player.attributes,
                    ));
            let max_braking_accel =
                nba_domain::capability::effective_max_braking_accel(rules, &player.attributes)
                    .min(max_accel);
            let max_lateral_accel =
                nba_domain::capability::effective_max_lateral_accel(rules, &player.attributes)
                    .min(max_accel);
            assert!(
                max_speed.is_finite()
                    && max_accel.is_finite()
                    && max_braking_accel.is_finite()
                    && max_lateral_accel.is_finite()
                    && current_speed.is_finite(),
                "derived movement limits must be finite"
            );
            assert!(
                current_speed <= max_speed + max_speed * 1e-5,
                "player speed {current_speed} exceeds its configured movement limit {max_speed}"
            );
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

            assert!(
                rules.arrival_epsilon_ft.is_finite()
                    && rules.arrival_speed_scale.is_finite()
                    && rules.arrival_speed_floor.is_finite()
                    && rules.turnaround_min_decel_seconds.is_finite()
                    && rules.player_linear_damping.is_finite()
                    && rules.turn_decel_retention.is_finite()
                    && rules.turn_decel_retention_floor.is_finite()
                    && rules.turn_decel_retention_ceiling.is_finite()
                    && rules.tactics.apf_repulsion_radius_ft.is_finite()
                    && rules.tactics.apf_teammate_repulsion_accel.is_finite()
                    && rules.tactics.apf_opponent_repulsion_accel.is_finite(),
                "movement steering rules must be finite"
            );
            // 期望速度按剩余距离限制，确保能够在目标点前用规则制动力停下。
            let seek_target = |distance: f32| -> Vec2 {
                if distance > rules.arrival_epsilon_ft && player.target_speed_ftps > 0.0 {
                    let speed_by_distance = (player.target_speed_ftps
                        * (distance / (max_speed * rules.arrival_speed_scale + 1.0))
                            .clamp(rules.arrival_speed_floor, 1.0))
                    .min(max_speed);
                    let stopping_speed =
                        (2.0 * max_braking_accel * (distance - rules.arrival_epsilon_ft).max(0.0))
                            .sqrt();
                    to_target.normalize_or_zero() * speed_by_distance.min(stopping_speed)
                } else {
                    Vec2::ZERO
                }
            };

            let mut next_vel = if player.is_locked_kinematics {
                current_vel * (1.0 - rules.player_linear_damping * dt).max(0.0)
            } else if player.is_receiving_pass {
                // ## 接球人不受转身减速影响（round-10）
                //
                // 接球人的速度由 `engine::receive_approach` 按制动距离给出，
                // 后者已经保证「到位即停」。而 `turn_decel_timer` 分支会把它
                // 覆盖为 `current_vel * 0.4`，使接球人带着上一 tick 的战术跑位
                // 速度滑离接球点。
                //
                // 实测（seed 1201）：H_2 初速 43.2 ft/s → 保留 40% = 17.26 ft/s，
                // 一 tick 滑 1.73 ft > catch_radius 2.6 ft，层 A 误判「接不到」。
                //
                // 因此接球人**无条件**向目标收敛，跳过转身减速分支。
                seek_target(distance)
            } else if player.turn_decel_timer > 0.0 {
                current_vel * nba_domain::effective_turn_decel_retention(rules, &player.attributes)
            } else {
                seek_target(distance)
            };
            // 人造势能场（APF）：计算周围球员对当前球员的平滑排斥加速度
            //
            // ## 接球人豁免（round-10）
            //
            // 接球人向球收敛时不得被 APF 推开：实测 H_2 初始恰在冻结接球点
            // (d=0, 目标速度 0)，仍被 12 ft 半径的队友排斥场以 ~16 ft/s 推离，
            // 2 tick 后距球 3.2 ft > catch_radius 2.6 → 层 A 误判「接不到」。
            //
            // `is_receiving` 标志由引擎在设置接球目标时置位（见
            // `MatchEngine::sync_team_tactics` 的 `pass_receiver_override` 分支），
            // 此处据此豁免 APF —— 接球是比「保持间距」更强的意图。
            let mut apf_repulsion_accel = Vec2::ZERO;
            let rep_radius = rules.tactics.apf_repulsion_radius_ft;
            if !player.is_locked_kinematics
                && rep_radius > f32::EPSILON
                && !player.is_receiving_pass
                && !player.is_driving_to_rim
            {
                for (other_id, other_team, other_pos, other_has_ball) in &on_court_snapshots {
                    if other_id == &player.id {
                        continue;
                    }
                    let diff = current_pos - *other_pos;
                    let dist = diff.length();
                    if dist < rep_radius && dist > 0.1 {
                        let is_teammate = other_team == &player.team;
                        // 距离越近排斥越强，随距离平滑二次衰减：(1 - d/R)^2
                        let decay = (1.0 - dist / rep_radius).powi(2);
                        let base_accel = if is_teammate {
                            // 队友间：若对方持球，自身必须让出进攻走廊，增加额外斥力
                            if *other_has_ball {
                                rules.tactics.apf_teammate_repulsion_accel * 1.5
                            } else {
                                rules.tactics.apf_teammate_repulsion_accel
                            }
                        } else {
                            // 对手：作为动态障碍物产生回避斥力
                            rules.tactics.apf_opponent_repulsion_accel
                        };
                        apf_repulsion_accel += (diff / dist) * (base_accel * decay);
                    }
                }
            }

            // 将 APF 斥力加速度叠加进速度积分，再按纵向制动与横向抓地限制更新。
            //
            // ## 侧向偏转投影（发球回合实测缺陷修复）
            //
            // 斥力若原样叠加，其中与目标方向共线的逆向分量会直接对冲
            // 速度幅值：贴防者以 min_separation 3.6ft 恒定贴住时，斥力
            // 恒为 ~4.8 ft/s²，与 seek 的每帧修正量（total_limit =
            // max_accel×dt ≈ 0.7 ft/s）形成稳态对抗，实测把落位跑动
            // 压到 0.2~0.5 ft/s（seed42 Q1 11:04，SPOT_UP_3PT 目标在
            // 75ft 外却全场钉死在后场）——「贴防」变成了「运动压制」。
            //
            // 真实行为是绕行：速度幅值不损失，方向避开障碍。因此把
            // 斥力投影到目标方向的垂直平面后再叠加（切向逃逸），速度
            // 幅值由 seek 保持，方向自然绕开。分离硬约束仍由
            // resolve_motion_collisions 保证，不依赖斥力减速。
            let apf_lateral = if next_vel.length_squared() > f32::EPSILON {
                let dir = next_vel.normalize_or_zero();
                let along = apf_repulsion_accel.dot(dir);
                apf_repulsion_accel - dir * along
            } else {
                apf_repulsion_accel
            };
            let apf_steered_vel = next_vel + apf_lateral * dt;
            next_vel = traction_limited_velocity(
                current_vel,
                apf_steered_vel,
                max_speed,
                max_accel,
                max_braking_accel,
                max_lateral_accel,
                dt,
            );
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
            } else if speed > 0.5
                && !(player.has_ball && player.ball_orientation == BallOrientation::BackToBasket)
            {
                // 背身持球人顶人时朝向被姿态锚定（背对篮筐面向传球侧），
                // 速度方向不得覆盖，否则背身顶进瞬间就「变回」面框。
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
                max_braking_accel_ftps2: max_braking_accel,
                max_lateral_accel_ftps2: max_lateral_accel,
            })
        })
        .collect()
}

pub(super) fn resolve_motion_collisions(
    proposals: &mut [MotionProposal],
    rules: &GameRules,
    dt: FixedDt,
) {
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
                    let relative_accel = (proposals[left_index].max_braking_accel_ftps2
                        + proposals[right_index].max_braking_accel_ftps2)
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
                let relative_accel = (proposals[left_index].max_braking_accel_ftps2
                    + proposals[right_index].max_braking_accel_ftps2)
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
                        proposals[left_index].max_braking_accel_ftps2,
                        proposals[left_index].max_lateral_accel_ftps2,
                        dt,
                    );
                    let right_result = bounded_velocity(
                        proposals[right_index].current_vel,
                        right_target,
                        proposals[right_index].max_speed_ftps,
                        proposals[right_index].max_accel_ftps2,
                        proposals[right_index].max_braking_accel_ftps2,
                        proposals[right_index].max_lateral_accel_ftps2,
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
    let (left_current, left_next, right_current, right_next) = {
        let left = &proposals[left_index];
        let right = &proposals[right_index];
        (
            left.current_vel,
            left.next_vel,
            right.current_vel,
            right.next_vel,
        )
    };
    let left = &proposals[left_index];
    let left_capacity = max_feasible_velocity_shift(
        left_current,
        left_next,
        -normal,
        VelocityLimits {
            max_speed_ftps: left.max_speed_ftps,
            max_accel_ftps2: left.max_accel_ftps2,
            max_braking_accel_ftps2: left.max_braking_accel_ftps2,
            max_lateral_accel_ftps2: left.max_lateral_accel_ftps2,
        },
        dt,
    );
    let right = &proposals[right_index];
    let right_capacity = max_feasible_velocity_shift(
        right_current,
        right_next,
        normal,
        VelocityLimits {
            max_speed_ftps: right.max_speed_ftps,
            max_accel_ftps2: right.max_accel_ftps2,
            max_braking_accel_ftps2: right.max_braking_accel_ftps2,
            max_lateral_accel_ftps2: right.max_lateral_accel_ftps2,
        },
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
    limits: VelocityLimits,
    dt: f32,
) -> f32 {
    let direction = direction.normalize_or_zero();
    if direction.length_squared() <= f32::EPSILON
        || !velocity_is_feasible(
            candidate,
            current,
            limits.max_speed_ftps,
            limits.max_accel_ftps2,
            limits.max_braking_accel_ftps2,
            limits.max_lateral_accel_ftps2,
            dt,
        )
    {
        return 0.0;
    }
    let mut low = 0.0;
    let mut high = candidate.length() + limits.max_speed_ftps + limits.max_accel_ftps2 * dt + 1.0;
    for _ in 0..32 {
        let middle = (low + high) * 0.5;
        if velocity_is_feasible(
            candidate + direction * middle,
            current,
            limits.max_speed_ftps,
            limits.max_accel_ftps2,
            limits.max_braking_accel_ftps2,
            limits.max_lateral_accel_ftps2,
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
    max_braking_accel: f32,
    max_lateral_accel: f32,
    dt: f32,
) -> bool {
    let delta = velocity - current;
    let longitudinal_axis = if current.length_squared() > f32::EPSILON {
        current.normalize_or_zero()
    } else {
        velocity.normalize_or_zero()
    };
    let longitudinal = delta.dot(longitudinal_axis);
    let lateral = delta - longitudinal_axis * longitudinal;
    let tolerance = 1e-5;
    velocity.length_squared() <= max_speed.powi(2) + tolerance
        && delta.length_squared() <= (max_accel * dt).powi(2) + tolerance
        && longitudinal.max(0.0) <= max_accel * dt + tolerance
        && (-longitudinal).max(0.0) <= max_braking_accel * dt + tolerance
        && longitudinal >= -current.length() - tolerance
        && lateral.length_squared() <= (max_lateral_accel * dt).powi(2) + tolerance
}
pub(super) fn apply_motion_proposals(
    players: &mut HashMap<String, PlayerPhysicsState>,
    proposals: &[MotionProposal],
    rules: &GameRules,
    dt: FixedDt,
    pending_facts: &mut Vec<PhysicsFact>,
) {
    let dt = dt.0;
    assert!(
        dt.is_finite() && dt > 0.0,
        "movement timestep must be finite and positive"
    );
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
        let next_pos = if player.out_of_bounds_placement {
            raw_pos
        } else {
            rules.court.clamp_playable(raw_pos, margin)
        };
        player.pos_ft = next_pos;
        // 边界事实（唯一发射点）+ 边沿锁存：只有当球员从"可站立区域"真正
        // 越出时才发事实。目标点恰在 clamp 边界线时 `raw_pos != next_pos`
        // 会因浮点误差恒为真，故以几何容差判定，并且只在上升沿发射
        // （dev 方案 D3.1：实测单球员连续 675–755 tick 刷屏）。
        // 第一性原理：`BoundaryCross` 表达的是「球员**实质性**越出边界」，
        // 而不是「目标点比 clamp 边界多出零点几英尺」。
        //
        // 战术槽位/防守目标若贴着边线（例如底角 y=2.5 而可站立下限 1.8），
        // 球员会被**永久顶在边界**上，raw 与 clamped 每 tick 相差 0.06–0.17 ft。
        // 若把这种亚英尺级钳制也算作越界事实，持球人就会被反复判成出界失误
        // （实测每场 42–52 次虚假 `TURNOVER:OUT_OF_BOUNDS`）。
        //
        // 用规则化的 `boundary_epsilon_ft` 作为"实质性越界"的门槛：
        // 只有真正把身体推出边界（例如强制位移、碰撞挤出）才算事实。
        let epsilon = rules
            .boundary_epsilon_ft
            .max(rules.semantics.minimum_entity_distance_ft)
            .max(f32::EPSILON);
        let out_of_bounds = (raw_pos - next_pos).length() > epsilon;
        let was_out_of_bounds = player.boundary_cross_latched;
        player.boundary_cross_latched = out_of_bounds;
        if out_of_bounds && !was_out_of_bounds {
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
                let correction = (minimum - distance) * rules.separation_correction_share;
                // 显式 placement 球员（发球程序中的发球员）不参与分离投影：
                // 否则投影会把它推回场内并卡在边界，`inbounder_arrived`
                // 永不成立，造成第二个 DeadBall 活锁（本轮 seed 6/9/11 实测）。
                // 其他球员承担全部修正量，发球员保持在合法发球点。
                let left_exempt = left.out_of_bounds_placement;
                let right_exempt = right.out_of_bounds_placement;
                if left_exempt && right_exempt {
                    continue;
                }
                let (left_correction, right_correction) = (correction, correction);
                let left_pos = rules
                    .court
                    .clamp_playable(left.pos_ft - normal * left_correction, margin);
                let right_pos = rules
                    .court
                    .clamp_playable(right.pos_ft + normal * right_correction, margin);
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
        // 包线断言针对运动学提案终点（碰撞/边界/分离投影之前）：它们只
        // 修正空间合法性，不改写运动学承诺的位移上界。分离投影可能把
        // 球员额外推开，那份位移由空间契约负责，速度报告按实际终点重算。
        let proposal_speed = (proposal.next_pos - proposal.current_pos).length() / dt;
        assert!(
            proposal_speed.is_finite() && proposal_speed <= proposal.max_speed_ftps + 1e-3,
            "projected movement endpoint violates speed envelope"
        );
        let final_vel = (player.pos_ft - proposal.current_pos) / dt;
        assert!(
            final_vel.is_finite(),
            "projected movement endpoint is not finite"
        );
        // 分离投影的额外位移不计入运动学速度上界：报告速度按包线截断，
        // 下一 tick 的提案从合法速度出发（否则投影推挤会自我累积）。
        player.vel_ft = final_vel.clamp_length_max(proposal.max_speed_ftps);
        player.accel_ft = (final_vel - proposal.current_vel) / dt;
    }
}
fn traction_limited_velocity(
    current: Vec2,
    desired: Vec2,
    max_speed: f32,
    max_accel: f32,
    max_braking_accel: f32,
    max_lateral_accel: f32,
    dt: f32,
) -> Vec2 {
    let delta = desired - current;
    let longitudinal_axis = if current.length_squared() > f32::EPSILON {
        current.normalize_or_zero()
    } else {
        desired.normalize_or_zero()
    };
    let longitudinal_amount = delta.dot(longitudinal_axis);
    let lateral_delta = delta - longitudinal_axis * longitudinal_amount;
    let braking_limit = (max_braking_accel * dt).min(current.length());
    let mut longitudinal =
        longitudinal_axis * longitudinal_amount.clamp(-braking_limit, max_accel * dt);
    let mut lateral = lateral_delta.clamp_length_max(max_lateral_accel * dt);
    let total_delta = (longitudinal + lateral).length();
    let total_limit = max_accel * dt;
    if total_delta > total_limit {
        let scale = total_limit / total_delta;
        longitudinal *= scale;
        lateral *= scale;
    }
    (current + longitudinal + lateral).clamp_length_max(max_speed)
}

fn bounded_velocity(
    current: Vec2,
    desired: Vec2,
    max_speed: f32,
    max_accel: f32,
    max_braking_accel: f32,
    max_lateral_accel: f32,
    dt: f32,
) -> Vec2 {
    traction_limited_velocity(
        current,
        desired,
        max_speed,
        max_accel,
        max_braking_accel,
        max_lateral_accel,
        dt,
    )
}

fn is_defensive_action(action: &str) -> bool {
    action.contains("DEFEND")
        || action.contains("Defend")
        || action.contains("DROP")
        || action.contains("Drop")
        || action.contains("CLOSEOUT")
        || action.contains("Closeout")
}

pub(super) fn collect_contact_facts(
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

pub(super) fn query_nearby_players(
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

pub(super) fn cast_capsule_players(
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

pub(super) fn raycast_players(
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
