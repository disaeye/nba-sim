// `GameRules::validate` 的实现（数值包络校验）。
//
// 划分依据（D29 行数门，与 rules/policies.rs 同款分层）：本文件只做
// 一件事——在比赛开始前拒绝非法档案。校验清单必须与 `GameRules` 的字段
// 声明同处一个 crate 才不会漏项，所以按「声明 / 校验」切开而不是按字段组
// 切开：字段声明与 `Default` 留在 `../rules.rs`，本文件持有 `validate`
// 本身与其逐条数值包络断言。
//
// 与 policies.rs 的策略校验分属两条通道：那边判档案自洽（策略权重、
// 倾向偏置），这边判运行前安全（时序与几何包络）。

use super::GameRules;

impl GameRules {
    /// Validate the timing and geometry envelope before a run starts.
    ///
    /// Keeping this check next to the policy object prevents each caller from
    /// inventing a partial validation list. Deserialization callers (such as
    /// the debug server) can reject invalid policy explicitly, while the
    /// simulation constructor remains a cheap value constructor.
    pub fn validate(&self) -> Result<(), String> {
        self.resolve.validate()?;

        let finite = [
            self.tick_seconds,
            self.league.period_duration_seconds,
            self.league.overtime_duration_seconds,
            self.league.shot_clock_seconds,
            self.league.offensive_rebound_shot_clock_seconds,
            self.league.bonus_final_seconds,
            self.league.timeouts_final_seconds,
            self.inbound_seconds,
            self.inbound_setup_seconds,
            self.backcourt_seconds,
            self.period_break_seconds,
            self.tactical_initiation_seconds,
            self.decision_interval_seconds,
            self.free_throw_interval_seconds,
            self.min_pass_duration_seconds,
            self.max_pass_duration_seconds,
            self.min_shot_duration_seconds,
            self.max_shot_duration_seconds,
            self.shot_peak_base_ft,
            self.shot_peak_distance_factor,
            self.chest_height_ft,
            self.rim_height_ft,
            self.rim_radius_ft,
            self.rim_contact_angle_spread_radians,
            self.rim_contact_restitution_flush,
            self.rim_contact_restitution_graze,
            self.rim_contact_vertical_restitution,
            self.rim_contact_scatter_radians,
            self.loose_ball_player_restitution,
            self.ball_radius_ft,
            self.body_contact_radius_ft,
            self.loose_ball_control_speed_ftps,
            self.tip_off_tap_speed_ftps,
            self.tip_off_tap_distance_ft,
            self.tip_off_tap_height_ft,
            self.tip_off_tap_vel_z_ftps,
            self.backboard_offset_from_baseline_ft,
            self.backboard_width_ft,
            self.backboard_bottom_height_ft,
            self.backboard_top_height_ft,
            self.backboard_restitution,
            self.pass_peak_ft,
            self.shot_clock_urgency_seconds,
            self.shot_contest_sensitivity,
            self.open_shot_distance_ft,
            self.shot_distance_reference_ft,
            self.rim_shot_distance_ft,
            self.inbound_boundary_tolerance_ft,
            self.inbound_release_depth_ft,
            self.free_throw_distance_ft,
            self.shot_pct_floor,
            self.shot_pct_ceiling,
            self.tip_off_duration_seconds,
            self.ball_bounce_base_ft,
            self.ball_bounce_amplitude_ft,
            self.ball_bounce_frequency_hz,
            self.contact_margin_ft,
            self.court.width_ft,
            self.court.height_ft,
            self.court.hoop_left_x_ft,
            self.court.hoop_right_x_ft,
            self.court.hoop_y_ft,
            self.player_radius_ft,
            self.min_player_separation_ft,
            self.separation_safety_margin_ft,
            self.max_player_speed_ftps,
            self.max_player_accel_ftps2,
            self.max_player_braking_accel_ftps2,
            self.max_player_lateral_accel_ftps2,
            self.turnaround_min_decel_seconds,
            self.player_linear_damping,
            self.ball_gravity_ftps2,
            self.ball_max_speed_ftps,
            self.defender_reach_ft,
            self.pass_corridor_radius_ft,
            self.pass_lane_clearance_reference_ft,
            self.flight_intercept_radius_ft,
            self.intercept_lane_radius_ft,
            self.rebound_outlet_fallback_distance_ft,
            self.teammate_density_radius_ft,
            self.teammate_density_capacity,
            self.court_side_margin_ratio,
            self.league.three_point_distance_ft,
            self.ball_holder_offset_ft,
            self.ball_holder_height_ft,
            self.max_ball_carrier_turn_rate_rad_per_sec,
            self.stamina_sprint_speed_ftps,
            self.stamina_recovery_speed_ftps,
            self.stamina_drain_per_second,
            self.stamina_recovery_per_second,
            self.stamina_floor,
            self.stamina_exhausted_threshold,
            self.jump_shot_prep_seconds,
            self.jump_shot_exec_seconds,
            self.jump_shot_follow_seconds,
            self.pass_prep_seconds,
            self.pass_exec_seconds,
            self.pass_follow_seconds,
            self.rebound_prep_seconds,
            self.rebound_exec_seconds,
            self.rebound_follow_seconds,
            self.semantics.contact_minor_speed_ratio,
            self.semantics.contact_positional_speed_ratio,
            self.semantics.contact_foul_candidate_speed_ratio,
            self.semantics.screen_stationary_speed_ratio,
            self.semantics.spacing_corner_weight,
            self.semantics.spacing_weak_side_weight,
            self.semantics.spacing_lane_weight,
            self.semantics.spacing_paint_penalty,
            self.semantics.opponent_density_capacity,
            self.semantics.pass_base_probability,
            self.semantics.pass_openness_weight,
            self.semantics.pass_lane_risk_weight,
            self.semantics.density_distance_weight,
            self.semantics.minimum_entity_distance_ft,
            self.semantics.perimeter_side_margin_ratio,
            self.semantics.perimeter_depth_ratio,
            self.semantics.perimeter_density_capacity,
            self.semantics.strong_side_ball_weight,
            self.semantics.strong_side_defender_weight,
            self.semantics.weak_side_defender_weight,
            self.semantics.crowded_zone_threshold,
            self.semantics.dense_zone_threshold,
            self.semantics.crowded_zone_utility_penalty,
            self.semantics.crowded_zone_risk,
            self.semantics.dense_zone_utility_penalty,
            self.semantics.dense_zone_risk,
            self.semantics.heavily_contested_threshold,
            self.semantics.heavily_contested_utility_penalty,
            self.semantics.heavily_contested_risk,
            self.semantics.contested_threshold,
            self.semantics.contested_utility_penalty,
            self.semantics.contested_risk,
            self.semantics.open_shot_contest_threshold,
        ];
        if finite.iter().any(|value| !value.is_finite()) {
            return Err("rules contain a non-finite number".to_string());
        }
        if self.tick_seconds <= 0.0
            || self.league.period_duration_seconds <= 0.0
            || self.league.overtime_duration_seconds <= 0.0
            || self.league.shot_clock_seconds <= 0.0
            || self.league.offensive_rebound_shot_clock_seconds <= 0.0
            || self.inbound_seconds <= 0.0
            || self.tip_off_tap_speed_ftps <= 0.0
            || self.tip_off_tap_distance_ft <= 0.0
            || self.tip_off_tap_height_ft <= 0.0
            || self.tip_off_tap_vel_z_ftps <= 0.0
            || self.inbound_setup_seconds < 0.0
            || self.backcourt_seconds <= 0.0
            || self.three_second_lane_seconds <= 0.0
            || self.period_break_seconds < 0.0
            || self.tactical_initiation_seconds < 0.0
            || self.decision_interval_seconds <= 0.0
            || self.free_throw_interval_seconds <= 0.0
            || self.league.regulation_periods == 0
            || self.league.bonus_final_seconds < f32::default()
            || (self.league.bonus_fouls_final != 0
                && self.league.bonus_fouls_final > self.league.bonus_fouls_per_period)
            || self.league.bonus_final_period > self.league.regulation_periods
            || self.league.timeouts_final_seconds < f32::default()
            || self.league.timeouts_final_max > self.league.timeouts_fourth_quarter_max
            || self.league.timeouts_fourth_quarter_max > self.league.timeouts_per_game
            || self.estimated_possessions_per_period == 0
        {
            return Err("clock and period policy contains an invalid value".to_string());
        }
        if self.min_pass_duration_seconds <= 0.0
            || self.max_pass_duration_seconds < self.min_pass_duration_seconds
            || self.min_shot_duration_seconds <= 0.0
            || self.max_shot_duration_seconds < self.min_shot_duration_seconds
            || self.ball_max_speed_ftps <= 0.0
            || self.pass_peak_ft < 0.0
            || self.pass_peak_distance_factor < 0.0
            || self.chest_height_ft < 0.0
            || self.rim_height_ft <= self.chest_height_ft
        {
            return Err("ball timing policy contains an invalid value".to_string());
        }
        if self.court.width_ft <= 0.0
            || self.court.height_ft <= 0.0
            || self.court.hoop_left_x_ft <= 0.0
            || self.court.hoop_left_x_ft >= self.court.hoop_right_x_ft
            || self.court.hoop_right_x_ft >= self.court.width_ft
            || self.court.hoop_y_ft <= 0.0
            || self.court.hoop_y_ft >= self.court.height_ft
            || self.free_throw_distance_ft <= 0.0
            || self.free_throw_distance_ft >= self.court.width_ft
        {
            return Err("court geometry policy contains an invalid value".to_string());
        }
        if self.court.hoop_y_ft + self.player_radius_ft > self.court.height_ft
            || self.court.hoop_y_ft < self.player_radius_ft
        {
            return Err("hoop geometry leaves no playable margin".to_string());
        }
        if self.free_throw_distance_ft >= self.court.hoop_right_x_ft
            || self.free_throw_distance_ft >= self.court.width_ft - self.court.hoop_left_x_ft
        {
            return Err("free-throw line lies outside the court geometry".to_string());
        }
        if self.player_radius_ft <= 0.0
            || self.min_player_separation_ft < self.player_radius_ft * 2.0
            || self.separation_safety_margin_ft < 0.0
            || self.max_player_speed_ftps <= 0.0
            || self.max_player_accel_ftps2 <= 0.0
            || self.max_player_braking_accel_ftps2 <= 0.0
            || self.max_player_lateral_accel_ftps2 <= 0.0
            || self.player_linear_damping < 0.0
            || self.defender_reach_ft < 0.0
            || self.pass_corridor_radius_ft < 0.0
            || self.catch_radius_base_ft <= 0.0
            || self.catch_radius_skill_gain_ft < 0.0
            || self.receive_estimate_noise_ft < 0.0
            || self.pass_lane_clearance_reference_ft <= 0.0
            || self.flight_intercept_radius_ft <= 0.0
            || self.intercept_lane_radius_ft < self.flight_intercept_radius_ft
            || self.rebound_outlet_fallback_distance_ft < 0.0
            || self.teammate_density_radius_ft <= 0.0
            || self.teammate_density_capacity <= 0.0
            || !(0.0..=0.5).contains(&self.court_side_margin_ratio)
        {
            return Err("physical policy contains an invalid value".to_string());
        }
        if !(0.0..=1.0).contains(&self.semantics.contact_minor_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.contact_positional_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.contact_foul_candidate_speed_ratio)
            || !(0.0..=1.0).contains(&self.semantics.screen_stationary_speed_ratio)
            || self.semantics.spacing_corner_weight < 0.0
            || self.semantics.spacing_weak_side_weight < 0.0
            || self.semantics.spacing_lane_weight < 0.0
            || self.semantics.spacing_paint_penalty < 0.0
            || self.semantics.opponent_density_capacity <= 0.0
            || self.semantics.pass_base_probability < 0.0
            || self.semantics.pass_base_probability > 1.0
            || self.semantics.pass_openness_weight < 0.0
            || self.semantics.pass_lane_risk_weight < 0.0
            || self.semantics.density_distance_weight < 0.0
            || self.semantics.minimum_entity_distance_ft < 0.0
            || !(0.0..=0.5).contains(&self.semantics.perimeter_side_margin_ratio)
            || !(0.0..=1.0).contains(&self.semantics.perimeter_depth_ratio)
            || self.semantics.perimeter_density_capacity <= 0.0
            || self.semantics.strong_side_ball_weight < 0.0
            || self.semantics.strong_side_defender_weight < 0.0
            || self.semantics.weak_side_defender_weight < 0.0
            || !(0.0..=1.0).contains(&self.semantics.dense_zone_threshold)
            || !(0.0..=1.0).contains(&self.semantics.crowded_zone_threshold)
            || self.semantics.crowded_zone_threshold < self.semantics.dense_zone_threshold
            || self.semantics.crowded_zone_utility_penalty < 0.0
            || self.semantics.crowded_zone_risk < 0.0
            || self.semantics.dense_zone_utility_penalty < 0.0
            || self.semantics.dense_zone_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.heavily_contested_threshold)
            || self.semantics.heavily_contested_utility_penalty < 0.0
            || self.semantics.heavily_contested_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.contested_threshold)
            || self.semantics.contested_utility_penalty < 0.0
            || self.semantics.contested_risk < 0.0
            || !(0.0..=1.0).contains(&self.semantics.open_shot_contest_threshold)
        {
            return Err("semantic policy contains an invalid value".to_string());
        }
        if !(0.0..=1.0).contains(&self.shot_pct_floor)
            || !(0.0..=1.0).contains(&self.shot_pct_ceiling)
            || self.shot_pct_floor > self.shot_pct_ceiling
            || self.shot_contest_sensitivity < 0.0
            || self.open_shot_distance_ft < 0.0
            || self.shot_distance_reference_ft <= 0.0
            || self.rim_shot_distance_ft < 0.0
            || self.league.three_point_distance_ft <= self.rim_shot_distance_ft
        {
            return Err("shot percentage and distance bounds are invalid".to_string());
        }
        if self.stream_max_bytes == 0
            || self.stream_frames_max_bytes == 0
            || self.stream_max_ticks == 0
        {
            return Err("stream byte/tick budgets must be positive".to_string());
        }
        if !(0.0..=1.0).contains(&self.separation_correction_share)
            || self.separation_correction_share == 0.0
        {
            return Err("separation correction share must be in (0, 1]".to_string());
        }
        if !(0.0..=1.0).contains(&self.transfer_landing_leash_ratio)
            || self.transfer_landing_leash_ratio == 0.0
        {
            return Err("transfer landing leash ratio must be in (0, 1]".to_string());
        }
        if !(0.0..=1.0).contains(&self.tactics.initiation_distance_ratio)
            || self.tactics.action_duration_seconds <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.drive_distance_ratio)
            || self.tactics.drive_min_duration_seconds <= 0.0
            || self.tactics.drive_max_duration_seconds < self.tactics.drive_min_duration_seconds
            || self.tactics.drive_speed_ratio <= 0.0
            || self.tactics.drive_early_finish_dist_ft <= 0.0
            || self.tactics.drive_mid_range_pullup_dist_ft
                <= self.tactics.drive_early_finish_dist_ft
            || self.tactics.drive_lane_offset_ft < 0.0
            || self.tactics.drive_rim_attack_bias < 0.0
            || self.tactics.drive_beaten_recovery_seconds < 0.0
            || self.tactics.drive_finish_range_ft <= 0.0
            || self.tactics.drive_dunk_max_dist_ft <= 0.0
            || self.tactics.drive_floater_min_dist_ft <= self.tactics.drive_dunk_max_dist_ft
            || !(0.0..=1.0).contains(&self.tactics.drive_dunk_min_finishing)
            || !(0.0..=1.0).contains(&self.tactics.drive_dunk_max_lane_density)
            || !(0.0..=1.0).contains(&self.tactics.drive_kickout_max_crowding)
            || self.tactics.drive_kickout_min_defender_dist_ft <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.drive_pullup_min_crowding)
            || self.tactics.drive_decision_check_interval_seconds <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.screen_distance_ratio)
            || self.tactics.defensive_gap_ft < 0.0
            || !(0.0..=1.0).contains(&self.tactics.help_sag_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.carrier_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.screener_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.support_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.defender_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.rebound_chase_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_defense_threshold_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_sprint_ratio)
            || self.tactics.screen_hold_separation_ft <= 0.0
            || self.tactics.screen_roll_separation_ft <= 0.0
            || self.tactics.drop_coverage_depth_ft <= 0.0
        {
            return Err("tactical movement policy contains an invalid value".to_string());
        }
        if self.rim_radius_ft <= 0.0
            || self.rim_contact_angle_spread_radians < 0.0
            || self.rim_contact_angle_spread_radians > std::f32::consts::PI
            || self.rim_contact_restitution_flush < 0.0
            || self.rim_contact_restitution_flush > 1.0
            || self.rim_contact_restitution_graze < 0.0
            || self.rim_contact_restitution_graze > self.rim_contact_restitution_flush
            || self.rim_contact_vertical_restitution < 0.0
            || self.rim_contact_vertical_restitution > 1.0
            || self.rim_contact_scatter_radians < 0.0
            || self.rim_contact_scatter_radians > std::f32::consts::PI
            || self.loose_ball_player_restitution < 0.0
            || self.loose_ball_player_restitution > 1.0
            || self.loose_ball_control_speed_ftps <= 0.0
            || !self.loose_ball_control_speed_ftps.is_finite()
            || self.backboard_width_ft <= 0.0
            || self.backboard_bottom_height_ft >= self.backboard_top_height_ft
            || self.backboard_offset_from_baseline_ft <= 0.0
            || self.backboard_offset_from_baseline_ft
                >= self.court.width_ft - self.backboard_offset_from_baseline_ft
            || !(0.0..=1.0).contains(&self.backboard_restitution)
            || self.inbound_boundary_tolerance_ft < self.player_radius_ft
            || self.inbound_release_depth_ft <= 0.0
            || self.stamina_floor < 0.0
            || self.stamina_floor > 1.0
            || self.stamina_exhausted_threshold < self.stamina_floor
            || self.stamina_exhausted_threshold > 1.0
            || self.stamina_sprint_speed_ftps < 0.0
            || self.stamina_recovery_speed_ftps < 0.0
            || self.stamina_drain_per_second < 0.0
            || self.stamina_recovery_per_second < 0.0
            || self.ball_bounce_base_ft < 0.0
            || self.ball_bounce_amplitude_ft < 0.0
            || self.ball_bounce_frequency_hz < 0.0
            || self.ball_holder_offset_ft < 0.0
            || self.ball_holder_height_ft < 0.0
            || self.max_ball_carrier_turn_rate_rad_per_sec <= f32::EPSILON
            || self.gait_stride_length_ft <= 0.0
            || self.gait_stride_min_speed_ftps < 0.0
            || self.gait_stride_speed_gain <= 0.0
            || self.gait_stride_speed_baseline < 0.0
            || self.gait_stride_stretch_min <= 0.0
            || self.gait_stride_stretch_max < self.gait_stride_stretch_min
            || self.gait_stride_contact_fraction < 0.0
            || self.gait_stride_contact_fraction > 1.0
            || self.gait_stride_sanity_multiple <= 0.0
            || self.jump_shot_prep_seconds < 0.0
            || self.jump_shot_exec_seconds < 0.0
            || self.jump_shot_follow_seconds < 0.0
            || self.pass_prep_seconds < 0.0
            || self.pass_exec_seconds < 0.0
            || self.pass_follow_seconds < 0.0
            || self.rebound_prep_seconds < 0.0
            || self.rebound_exec_seconds < 0.0
            || self.rebound_follow_seconds < 0.0
            || self.rebound_crash_offense_count == 0
            || self.rebound_boxout_defense_count == 0
        {
            return Err(
                "stamina, rebound, and action policy contains an invalid value".to_string(),
            );
        }
        self.modulation.validate()?;
        self.rotation.validate()?;
        self.decision.validate()?;
        Ok(())
    }
}
