use crate::court::CourtGeometry;
use crate::resolve::ResolveConfig;
use serde::{Deserialize, Serialize};

/// Match rules and timing policy shared by the simulation subsystems.
///
/// The engine consumes this value rather than embedding competition-specific
/// numbers in phase transitions, constraints, or ball execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct GameRules {
    pub tick_seconds: f32,
    /// 联赛规则档案（M10）：计时结构、进攻时钟、犯规政策、几何。
    /// 切换联赛 = 替换该档案，引擎零联赛分支（architecture.md 6.3）。
    pub league: crate::league::LeagueProfile,
    /// Estimated possessions per period used by abbreviated simulations.
    pub estimated_possessions_per_period: u32,
    pub inbound_seconds: f32,
    pub inbound_setup_seconds: f32,
    pub backcourt_seconds: f32,
    pub period_break_seconds: f32,
    pub tactical_initiation_seconds: f32,
    pub decision_interval_seconds: f32,
    pub free_throw_interval_seconds: f32,
    pub pass_speed_ftps: f32,
    pub inbound_pass_speed_ftps: f32,
    pub shot_speed_ftps: f32,
    pub min_pass_duration_seconds: f32,
    pub max_pass_duration_seconds: f32,
    pub min_shot_duration_seconds: f32,
    pub max_shot_duration_seconds: f32,
    pub shot_peak_base_ft: f32,
    pub shot_peak_distance_factor: f32,
    pub chest_height_ft: f32,
    pub rim_height_ft: f32,
    pub pass_peak_ft: f32,
    pub ball_arc_multiplier: f32,
    pub rebound_short_min_ft: f32,
    pub rebound_short_max_ft: f32,
    pub rebound_long_min_ft: f32,
    pub rebound_long_max_ft: f32,
    pub rebound_angle_range_radians: f32,
    pub rebound_flight_base_seconds: f32,
    pub rebound_flight_distance_factor: f32,
    pub rebound_distance_scale_ft: f32,
    pub shot_clock_urgency_seconds: f32,
    /// Contest intensity to field-goal percentage conversion slope.
    pub shot_contest_sensitivity: f32,
    /// Shared semantic interpretation policy; no action-specific constants belong in systems.
    #[serde(default)]
    pub semantics: SemanticRules,
    /// 裁决参数嵌套组（M7 并轨）：概率与权重的唯一规则通道。
    #[serde(default)]
    pub resolve: ResolveConfig,
    /// Distance from the rim below which a shot is treated as a rim attempt.
    pub rim_shot_distance_ft: f32,
    /// Maximum accepted distance from the court boundary for an inbound release.
    pub inbound_boundary_tolerance_ft: f32,
    pub inbound_release_depth_ft: f32,
    /// Horizontal distance from the hoop center to the free throw line.
    pub free_throw_distance_ft: f32,
    /// Clamps applied to any final field-goal probability.
    pub shot_pct_floor: f32,
    pub shot_pct_ceiling: f32,
    pub tip_off_duration_seconds: f32,
    /// Ball presentation policy while a player is dribbling.
    pub ball_bounce_base_ft: f32,
    pub ball_bounce_amplitude_ft: f32,
    pub ball_bounce_frequency_hz: f32,
    /// Court dimensions and hoop locations used by physics, tactics, and officiating.
    pub court: CourtGeometry,
    pub player_radius_ft: f32,
    /// Minimum center-to-center distance maintained by kinematic movement.
    pub min_player_separation_ft: f32,
    /// Numerical cushion used by endpoint projection before protocol rounding.
    #[serde(default = "default_separation_safety_margin_ft")]
    pub separation_safety_margin_ft: f32,
    pub max_player_speed_ftps: f32,
    pub max_player_accel_ftps2: f32,
    pub turnaround_min_decel_seconds: f32,
    pub player_linear_damping: f32,
    pub contact_margin_ft: f32,
    pub ball_max_speed_ftps: f32,
    pub ball_velocity_retention: f32,
    pub defender_reach_ft: f32,
    pub pass_corridor_radius_ft: f32,
    /// Reference clearance used to normalize pass-lane risk scores.
    pub pass_lane_clearance_reference_ft: f32,
    pub teammate_density_radius_ft: f32,
    pub teammate_density_capacity: f32,
    pub court_side_margin_ratio: f32,
    pub open_shot_distance_ft: f32,
    /// Distance normalization reference for shot utility scoring.
    pub shot_distance_reference_ft: f32,
    pub rebound_outlet_fallback_distance_ft: f32,
    /// Minimum rebound arc above the contact height.
    pub rebound_min_arc_ft: f32,
    pub ball_holder_offset_ft: f32,
    pub ball_holder_height_ft: f32,
    /// 属性响应曲线下限（映射层 capability 的规则参数，attributes.md T2）。
    pub attribute_response_floor: f32,
    /// 球与持球者允许的最大距离（L1 BALL_WITH_HOLDER 限值，经 FrameRules 下发）。
    pub invariant_holder_leash_ft: f32,
    /// L1 速度不变量的数值容差（PLAYER_SPEED / BALL_SPEED）。
    pub invariant_speed_tolerance_ftps: f32,
    /// 球高度上限（L1 BALL_HEIGHT_BOUNDS）。
    pub ball_z_max_ft: f32,
    /// Player stamina model parameters, expressed in normalized stamina units.
    pub stamina_sprint_speed_ftps: f32,
    pub stamina_recovery_speed_ftps: f32,
    pub stamina_drain_per_second: f32,
    pub stamina_recovery_per_second: f32,
    pub stamina_floor: f32,
    pub stamina_exhausted_threshold: f32,
    pub jump_shot_prep_seconds: f32,
    pub jump_shot_exec_seconds: f32,
    pub jump_shot_follow_seconds: f32,
    pub pass_prep_seconds: f32,
    pub pass_exec_seconds: f32,
    pub pass_follow_seconds: f32,
    pub rebound_prep_seconds: f32,
    pub rebound_exec_seconds: f32,
    pub rebound_follow_seconds: f32,
    pub rebound_peak_ft: f32,
    /// Tactical movement policy shared by every built-in scheme.
    #[serde(default)]
    pub tactics: TacticalRules,
    /// Match-level parameters for coach and player psychological modulation.
    #[serde(default)]
    pub modulation: ModulationRules,
    /// Decision utility and sampling policy used by every decision system.
    #[serde(default)]
    pub decision: DecisionRules,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionRules {
    pub shoot_base: f32,
    pub pass_base: f32,
    pub dwell_base: f32,
    /// Strength of stamina's influence on action utility.
    pub stamina_sensitivity: f32,
    /// Softmax temperature; lower values make choices more deterministic.
    pub temperature: f32,
    /// Receiver lead time used when creating pass targets.
    pub pass_lead_time_seconds: f32,
    /// Utility penalty applied to risk reported by constraints.
    pub risk_aversion: f32,
    /// Influence of player tendencies on utility.
    pub tendency_weight: f32,
    /// Influence of team style traits on utility.
    pub team_style_weight: f32,
    /// 三分投篮效用折损系数（反映三分球相对近距离攻框的期望难度）。
    pub three_point_utility_multiplier: f32,
    /// 突破攻框基础效用权重。
    pub drive_base: f32,
    /// 最大持球组织衰减比例（随着进攻时间消耗，Dwell 价值衰减的最大幅度）。
    pub dwell_decay_max: f32,
    /// 进攻迫近时受干扰惩罚的保底系数。
    pub contested_patience_floor: f32,
    /// 24 秒倒计时迫近时的投篮效用加成。
    pub urgency_shoot_boost: f32,
    /// 24 秒倒计时迫近时的突破效用加成。
    pub urgency_drive_boost: f32,
    /// 24 秒倒计时迫近时的传球效用惩罚。
    pub urgency_pass_penalty: f32,
    /// 24 秒倒计时迫近时的持球组织惩罚。
    pub urgency_dwell_penalty: f32,
}

impl Default for DecisionRules {
    fn default() -> Self {
        Self {
            shoot_base: 0.60,
            pass_base: 0.82,
            dwell_base: 0.72,
            stamina_sensitivity: 0.5,
            temperature: 0.30,
            pass_lead_time_seconds: 0.65,
            risk_aversion: 0.8,
            tendency_weight: 0.35,
            team_style_weight: 0.25,
            three_point_utility_multiplier: 0.68,
            drive_base: 0.85,
            dwell_decay_max: 0.38,
            contested_patience_floor: 0.25,
            urgency_shoot_boost: 0.25,
            urgency_drive_boost: 0.10,
            urgency_pass_penalty: 0.10,
            urgency_dwell_penalty: 0.20,
        }
    }
}

impl DecisionRules {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.shoot_base,
            self.pass_base,
            self.dwell_base,
            self.stamina_sensitivity,
            self.temperature,
            self.pass_lead_time_seconds,
            self.risk_aversion,
            self.tendency_weight,
            self.team_style_weight,
            self.three_point_utility_multiplier,
            self.drive_base,
            self.dwell_decay_max,
            self.contested_patience_floor,
            self.urgency_shoot_boost,
            self.urgency_drive_boost,
            self.urgency_pass_penalty,
            self.urgency_dwell_penalty,
        ];
        if values.iter().any(|value| !value.is_finite())
            || self.shoot_base < 0.0
            || self.pass_base < 0.0
            || self.dwell_base < 0.0
            || !(0.0..=1.0).contains(&self.stamina_sensitivity)
            || self.temperature <= 0.0
            || self.pass_lead_time_seconds < 0.0
            || self.risk_aversion < 0.0
            || self.tendency_weight < 0.0
            || self.team_style_weight < 0.0
        {
            return Err("decision policy contains an invalid value".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModulationRules {
    pub hot_hand_makes: u32,
    pub frustrated_misses: u32,
    pub frustrated_turnovers: u32,
    pub hot_hand_bias: f32,
    pub clutch_bias: f32,
    pub frustrated_bias: f32,
    pub exhausted_bias: f32,
    pub late_game_period: u32,
    pub late_game_seconds: f32,
    pub trailing_score_margin: i32,
    pub leading_score_margin: i32,
    pub trailing_pace_factor: f32,
    pub trailing_three_point_bias: f32,
    pub trailing_defense_aggression: f32,
    pub leading_pace_factor: f32,
    pub leading_three_point_bias: f32,
    pub leading_defense_aggression: f32,
    pub clutch_period: u32,
    pub clutch_time_remaining: f32,
    pub clutch_score_margin: i32,
}

impl Default for ModulationRules {
    fn default() -> Self {
        Self {
            hot_hand_makes: 2,
            frustrated_misses: 3,
            frustrated_turnovers: 2,
            hot_hand_bias: 0.10,
            clutch_bias: 0.08,
            frustrated_bias: -0.05,
            exhausted_bias: -0.12,
            late_game_period: 4,
            late_game_seconds: 120.0,
            trailing_score_margin: 6,
            leading_score_margin: 6,
            trailing_pace_factor: 1.35,
            trailing_three_point_bias: 1.6,
            trailing_defense_aggression: 1.4,
            leading_pace_factor: 0.75,
            leading_three_point_bias: 0.8,
            leading_defense_aggression: 1.0,
            clutch_period: 4,
            clutch_time_remaining: 120.0,
            clutch_score_margin: 5,
        }
    }
}

impl ModulationRules {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.hot_hand_bias,
            self.clutch_bias,
            self.frustrated_bias,
            self.exhausted_bias,
            self.late_game_seconds,
            self.trailing_pace_factor,
            self.trailing_three_point_bias,
            self.trailing_defense_aggression,
            self.leading_pace_factor,
            self.leading_three_point_bias,
            self.leading_defense_aggression,
            self.clutch_time_remaining,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err("modulation policy contains a non-finite value".to_string());
        }
        if self.hot_hand_makes == 0
            || self.frustrated_misses == 0
            || self.frustrated_turnovers == 0
            || self.late_game_period == 0
            || self.clutch_period == 0
            || self.late_game_seconds < 0.0
            || self.clutch_time_remaining < 0.0
            || self.trailing_score_margin < 0
            || self.leading_score_margin < 0
            || self.clutch_score_margin < 0
            || self.trailing_pace_factor < 0.0
            || self.trailing_three_point_bias < 0.0
            || self.trailing_defense_aggression < 0.0
            || self.leading_pace_factor < 0.0
            || self.leading_three_point_bias < 0.0
            || self.leading_defense_aggression < 0.0
        {
            return Err("modulation policy contains an invalid value".to_string());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SemanticRules {
    pub contact_minor_speed_ratio: f32,
    pub contact_positional_speed_ratio: f32,
    pub contact_foul_candidate_speed_ratio: f32,
    pub screen_stationary_speed_ratio: f32,
    pub spacing_corner_weight: f32,
    pub spacing_weak_side_weight: f32,
    pub spacing_lane_weight: f32,
    pub spacing_paint_penalty: f32,
    pub opponent_density_capacity: f32,
    pub pass_base_probability: f32,
    pub pass_openness_weight: f32,
    pub pass_lane_risk_weight: f32,
    pub density_distance_weight: f32,
    pub minimum_entity_distance_ft: f32,
    /// Perimeter windows used to classify the ball side and its opposite side.
    pub perimeter_side_margin_ratio: f32,
    pub perimeter_depth_ratio: f32,
    pub perimeter_density_capacity: f32,
    pub strong_side_ball_weight: f32,
    pub strong_side_defender_weight: f32,
    pub weak_side_defender_weight: f32,
    pub crowded_zone_threshold: f32,
    pub dense_zone_threshold: f32,
    pub crowded_zone_utility_penalty: f32,
    pub crowded_zone_risk: f32,
    pub dense_zone_utility_penalty: f32,
    pub dense_zone_risk: f32,
    pub heavily_contested_threshold: f32,
    pub heavily_contested_utility_penalty: f32,
    pub heavily_contested_risk: f32,
    pub contested_threshold: f32,
    pub contested_utility_penalty: f32,
    pub contested_risk: f32,
    pub open_shot_contest_threshold: f32,
    pub contest_dist_weight: f32,
    pub contest_speed_weight: f32,
    pub contest_speed_factor_cap: f32,
}

impl Default for SemanticRules {
    fn default() -> Self {
        Self {
            contact_minor_speed_ratio: 0.25,
            contact_positional_speed_ratio: 0.50,
            contact_foul_candidate_speed_ratio: 0.70,
            screen_stationary_speed_ratio: 0.20,
            spacing_corner_weight: 0.30,
            spacing_weak_side_weight: 0.30,
            spacing_lane_weight: 0.40,
            spacing_paint_penalty: 0.50,
            opponent_density_capacity: 3.0,
            pass_base_probability: 0.50,
            pass_openness_weight: 0.40,
            pass_lane_risk_weight: 0.35,
            density_distance_weight: 0.50,
            minimum_entity_distance_ft: 0.001,
            perimeter_side_margin_ratio: 0.32,
            perimeter_depth_ratio: 0.22,
            perimeter_density_capacity: 2.0,
            strong_side_ball_weight: 0.65,
            strong_side_defender_weight: 0.35,
            weak_side_defender_weight: 0.65,
            crowded_zone_threshold: 0.45,
            dense_zone_threshold: 0.25,
            crowded_zone_utility_penalty: 0.18,
            crowded_zone_risk: 0.10,
            dense_zone_utility_penalty: 0.08,
            dense_zone_risk: 0.05,
            heavily_contested_threshold: 0.75,
            heavily_contested_utility_penalty: 0.30,
            heavily_contested_risk: 0.25,
            contested_threshold: 0.50,
            contested_utility_penalty: 0.15,
            contested_risk: 0.12,
            open_shot_contest_threshold: 0.35,
            contest_dist_weight: 0.8,
            contest_speed_weight: 1.0,
            contest_speed_factor_cap: 0.4,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TacticalRules {
    pub initiation_distance_ratio: f32,
    pub action_duration_seconds: f32,
    pub drive_distance_ratio: f32,
    pub screen_distance_ratio: f32,
    pub defensive_gap_ft: f32,
    pub help_sag_ratio: f32,
    pub transition_speed_ratio: f32,
    pub carrier_speed_ratio: f32,
    pub screener_speed_ratio: f32,
    pub support_speed_ratio: f32,
    pub defender_speed_ratio: f32,
}

impl Default for TacticalRules {
    fn default() -> Self {
        Self {
            initiation_distance_ratio: 0.30,
            action_duration_seconds: 7.5,
            drive_distance_ratio: 0.15,
            screen_distance_ratio: 0.272,
            defensive_gap_ft: 4.0,
            help_sag_ratio: 0.35,
            transition_speed_ratio: 0.91,
            carrier_speed_ratio: 0.73,
            screener_speed_ratio: 0.64,
            support_speed_ratio: 0.45,
            defender_speed_ratio: 0.73,
        }
    }
}

fn default_separation_safety_margin_ft() -> f32 {
    0.05
}

impl Default for GameRules {
    fn default() -> Self {
        Self {
            tick_seconds: 0.04,
            league: crate::league::LeagueProfile::nba(),
            estimated_possessions_per_period: 35,
            inbound_seconds: 5.0,
            inbound_setup_seconds: 2.2,
            backcourt_seconds: 8.0,
            period_break_seconds: 15.0,
            tactical_initiation_seconds: 5.2,
            decision_interval_seconds: 1.45,
            free_throw_interval_seconds: 0.8,
            pass_speed_ftps: 32.0,
            inbound_pass_speed_ftps: 30.0,
            shot_speed_ftps: 26.0,
            min_pass_duration_seconds: 0.45,
            max_pass_duration_seconds: 1.4,
            min_shot_duration_seconds: 0.95,
            max_shot_duration_seconds: 1.45,
            decision: DecisionRules::default(),
            shot_peak_base_ft: 14.0,
            shot_peak_distance_factor: 0.25,
            chest_height_ft: 4.0,
            rim_height_ft: 10.0,
            pass_peak_ft: 4.0,
            ball_arc_multiplier: 4.0,
            rebound_short_min_ft: 3.0,
            rebound_short_max_ft: 9.0,
            rebound_long_min_ft: 8.0,
            rebound_long_max_ft: 18.0,
            rebound_angle_range_radians: 1.0,
            rebound_flight_base_seconds: 0.8,
            rebound_flight_distance_factor: 0.4,
            rebound_distance_scale_ft: 15.0,
            shot_clock_urgency_seconds: 5.0,
            shot_contest_sensitivity: 0.22,
            semantics: SemanticRules::default(),
            resolve: crate::resolve::ResolveConfig::default(),
            rim_shot_distance_ft: 8.0,
            inbound_boundary_tolerance_ft: 4.0,
            inbound_release_depth_ft: 3.0,
            free_throw_distance_ft: 13.75,
            shot_pct_floor: 0.10,
            shot_pct_ceiling: 0.85,
            tip_off_duration_seconds: 0.0,
            ball_bounce_base_ft: 3.25,
            ball_bounce_amplitude_ft: 0.35,
            ball_bounce_frequency_hz: 2.2,
            court: CourtGeometry::default(),
            player_radius_ft: 1.8,
            min_player_separation_ft: 3.6,
            separation_safety_margin_ft: default_separation_safety_margin_ft(),
            max_player_speed_ftps: 22.0,
            max_player_accel_ftps2: 35.0,
            turnaround_min_decel_seconds: 0.12,
            player_linear_damping: 4.0,
            contact_margin_ft: 0.6,
            ball_max_speed_ftps: 85.0,
            ball_velocity_retention: 0.85,
            defender_reach_ft: 1.2,
            pass_corridor_radius_ft: 1.0,
            pass_lane_clearance_reference_ft: 8.0,
            teammate_density_radius_ft: 8.0,
            teammate_density_capacity: 4.0,
            court_side_margin_ratio: 0.16,
            open_shot_distance_ft: 4.5,
            shot_distance_reference_ft: 47.0,
            rebound_outlet_fallback_distance_ft: 10.0,
            rebound_min_arc_ft: 0.5,
            ball_holder_offset_ft: 0.8,
            ball_holder_height_ft: 4.0,
            attribute_response_floor: 0.5,
            invariant_holder_leash_ft: 3.0,
            invariant_speed_tolerance_ftps: 1.5,
            ball_z_max_ft: 35.0,
            stamina_sprint_speed_ftps: 15.0,
            stamina_recovery_speed_ftps: 6.0,
            stamina_drain_per_second: 0.015,
            stamina_recovery_per_second: 0.02,
            stamina_floor: 0.2,
            stamina_exhausted_threshold: 0.35,
            jump_shot_prep_seconds: 0.28,
            jump_shot_exec_seconds: 0.22,
            jump_shot_follow_seconds: 0.35,
            pass_prep_seconds: 0.16,
            pass_exec_seconds: 0.12,
            pass_follow_seconds: 0.16,
            rebound_prep_seconds: 0.20,
            rebound_exec_seconds: 0.30,
            rebound_follow_seconds: 0.30,
            rebound_peak_ft: 11.5,
            tactics: TacticalRules::default(),
            modulation: ModulationRules::default(),
        }
    }
}

impl GameRules {
    /// 以指定联赛档案构造规则（多联赛唯一入口，宪章 C3）。
    pub fn with_league(league: crate::league::LeagueProfile) -> Self {
        Self {
            league,
            ..Default::default()
        }
    }

    pub fn period_duration(&self, period: u32) -> f32 {
        let league = &self.league;
        if period > league.regulation_periods {
            league.overtime_duration_seconds
        } else {
            league.period_duration_seconds
        }
    }

    pub fn pass_duration(&self, distance_ft: f32, inbound: bool) -> f32 {
        let speed = if inbound {
            self.inbound_pass_speed_ftps
        } else {
            self.pass_speed_ftps
        };
        (distance_ft / speed.max(f32::EPSILON)).clamp(
            self.min_pass_duration_seconds,
            self.max_pass_duration_seconds,
        )
    }

    pub fn inbound_expired(&self, elapsed: f32) -> bool {
        elapsed >= self.inbound_seconds
    }

    pub fn period_expired(&self, clock: f32) -> bool {
        clock <= 0.0
    }

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
            self.inbound_seconds,
            self.inbound_setup_seconds,
            self.backcourt_seconds,
            self.period_break_seconds,
            self.tactical_initiation_seconds,
            self.decision_interval_seconds,
            self.free_throw_interval_seconds,
            self.pass_speed_ftps,
            self.inbound_pass_speed_ftps,
            self.shot_speed_ftps,
            self.min_pass_duration_seconds,
            self.max_pass_duration_seconds,
            self.min_shot_duration_seconds,
            self.max_shot_duration_seconds,
            self.shot_peak_base_ft,
            self.shot_peak_distance_factor,
            self.chest_height_ft,
            self.rim_height_ft,
            self.pass_peak_ft,
            self.ball_arc_multiplier,
            self.rebound_short_min_ft,
            self.rebound_short_max_ft,
            self.rebound_long_min_ft,
            self.rebound_long_max_ft,
            self.rebound_angle_range_radians,
            self.rebound_flight_base_seconds,
            self.rebound_flight_distance_factor,
            self.rebound_distance_scale_ft,
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
            self.turnaround_min_decel_seconds,
            self.player_linear_damping,
            self.ball_max_speed_ftps,
            self.ball_velocity_retention,
            self.defender_reach_ft,
            self.pass_corridor_radius_ft,
            self.pass_lane_clearance_reference_ft,
            self.rebound_outlet_fallback_distance_ft,
            self.rebound_min_arc_ft,
            self.teammate_density_radius_ft,
            self.teammate_density_capacity,
            self.court_side_margin_ratio,
            self.league.three_point_distance_ft,
            self.ball_holder_offset_ft,
            self.ball_holder_height_ft,
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
            self.rebound_peak_ft,
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
            || self.inbound_setup_seconds < 0.0
            || self.backcourt_seconds <= 0.0
            || self.period_break_seconds < 0.0
            || self.tactical_initiation_seconds < 0.0
            || self.decision_interval_seconds <= 0.0
            || self.free_throw_interval_seconds <= 0.0
            || self.league.regulation_periods == 0
            || self.estimated_possessions_per_period == 0
        {
            return Err("clock and period policy contains an invalid value".to_string());
        }
        if self.min_pass_duration_seconds <= 0.0
            || self.max_pass_duration_seconds < self.min_pass_duration_seconds
            || self.min_shot_duration_seconds <= 0.0
            || self.max_shot_duration_seconds < self.min_shot_duration_seconds
            || self.pass_speed_ftps <= 0.0
            || self.inbound_pass_speed_ftps <= 0.0
            || self.shot_speed_ftps <= 0.0
            || self.ball_max_speed_ftps <= 0.0
            || self.pass_peak_ft < 0.0
            || self.ball_arc_multiplier < 0.0
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
            || self.player_linear_damping < 0.0
            || !(0.0..=1.0).contains(&self.ball_velocity_retention)
            || self.defender_reach_ft < 0.0
            || self.pass_corridor_radius_ft < 0.0
            || self.pass_lane_clearance_reference_ft <= 0.0
            || self.rebound_outlet_fallback_distance_ft < 0.0
            || self.rebound_min_arc_ft < 0.0
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
        if !(0.0..=1.0).contains(&self.tactics.initiation_distance_ratio)
            || self.tactics.action_duration_seconds <= 0.0
            || !(0.0..=1.0).contains(&self.tactics.drive_distance_ratio)
            || !(0.0..=1.0).contains(&self.tactics.screen_distance_ratio)
            || self.tactics.defensive_gap_ft < 0.0
            || !(0.0..=1.0).contains(&self.tactics.help_sag_ratio)
            || !(0.0..=1.0).contains(&self.tactics.transition_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.carrier_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.screener_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.support_speed_ratio)
            || !(0.0..=1.0).contains(&self.tactics.defender_speed_ratio)
        {
            return Err("tactical movement policy contains an invalid value".to_string());
        }
        if self.rebound_short_min_ft < 0.0
            || self.rebound_short_max_ft < self.rebound_short_min_ft
            || self.rebound_long_min_ft < 0.0
            || self.rebound_long_max_ft < self.rebound_long_min_ft
            || self.rebound_angle_range_radians < 0.0
            || self.rebound_flight_base_seconds <= 0.0
            || self.rebound_flight_distance_factor < 0.0
            || self.rebound_distance_scale_ft <= 0.0
            || self.inbound_boundary_tolerance_ft < 0.0
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
            || self.jump_shot_prep_seconds < 0.0
            || self.jump_shot_exec_seconds < 0.0
            || self.jump_shot_follow_seconds < 0.0
            || self.pass_prep_seconds < 0.0
            || self.pass_exec_seconds < 0.0
            || self.pass_follow_seconds < 0.0
            || self.rebound_prep_seconds < 0.0
            || self.rebound_exec_seconds < 0.0
            || self.rebound_follow_seconds < 0.0
            || self.rebound_peak_ft < self.chest_height_ft
        {
            return Err(
                "stamina, rebound, and action policy contains an invalid value".to_string(),
            );
        }
        self.modulation.validate()?;
        self.decision.validate()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::GameRules;

    #[test]
    fn defaults_are_valid() {
        assert!(GameRules::default().validate().is_ok());
    }

    #[test]
    fn rejects_invalid_decision_policy() {
        let mut rules = GameRules::default();
        rules.decision.temperature = 0.0;
        assert!(rules.validate().is_err());
        rules.decision.temperature = 0.2;
        rules.decision.pass_lead_time_seconds = -0.1;
        assert!(rules.validate().is_err());
    }

    #[test]
    fn decision_policy_round_trips_through_json_defaults() {
        let rules = GameRules::default();
        let value = serde_json::to_value(&rules).expect("rules serialize");
        let decoded: GameRules = serde_json::from_value(value).expect("rules deserialize");
        assert_eq!(decoded.decision.temperature, rules.decision.temperature);
    }

    #[test]
    fn rejects_invalid_numeric_envelopes() {
        let rules = GameRules {
            ball_max_speed_ftps: 0.0,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());

        let rules = GameRules {
            court: crate::court::CourtGeometry {
                hoop_right_x_ft: GameRules::default().court.width_ft + 1.0,
                ..GameRules::default().court
            },
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn rejects_invalid_ball_and_rebound_policies() {
        let rules = GameRules {
            ball_velocity_retention: 1.1,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());

        let rules = GameRules {
            rebound_min_arc_ft: -0.1,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn rejects_non_finite_separation_margin() {
        let rules = GameRules {
            separation_safety_margin_ft: f32::NAN,
            ..GameRules::default()
        };
        assert!(rules.validate().is_err());
    }

    #[test]
    fn rejects_invalid_semantic_weight() {
        let mut rules = GameRules::default();
        rules.semantics.spacing_lane_weight = -0.1;
        assert!(rules.validate().is_err());
    }
}

#[cfg(test)]
mod modulation_tests {
    use super::{GameRules, ModulationRules};

    #[test]
    fn modulation_defaults_are_part_of_valid_rules() {
        assert!(GameRules::default().validate().is_ok());
    }

    #[test]
    fn modulation_rejects_zero_thresholds_and_negative_bias_policy() {
        let policy = ModulationRules {
            hot_hand_makes: 0,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
        let policy = ModulationRules {
            trailing_pace_factor: -0.1,
            ..Default::default()
        };
        assert!(policy.validate().is_err());
    }
}
