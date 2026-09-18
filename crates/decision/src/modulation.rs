/// Psychological State Machine of a Player (Phase 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoraleState {
    Normal,
    /// Made multiple consecutive shots: boosts confidence & shot utility.
    /// 由 [`PlayerModulationState::record_shot`] 累积，在
    /// `update_stamina_with_rules` 里比较 `ModulationRules::hot_hand_makes`。
    HotHand,
    Frustrated, // Turnovers/blocked: increases error rate and erratic decision-making
    Exhausted,  // Low stamina: heavily dampens explosive drive & sprint weights
}

#[derive(Debug, Clone)]
pub struct PlayerModulationState {
    pub stamina: f32, // 0.0 to 1.0 (1.0 = fresh)
    pub morale: MoraleState,
    pub consecutive_makes: u32,
    pub consecutive_misses: u32,
    pub turnover_count: u32,
    pub catch_equilibrium: f32, // 1.0 = balanced, decreases on bad pass receptions
}

impl Default for PlayerModulationState {
    fn default() -> Self {
        Self {
            stamina: 1.0,
            morale: MoraleState::Normal,
            consecutive_makes: 0,
            consecutive_misses: 0,
            turnover_count: 0,
            catch_equilibrium: 1.0,
        }
    }
}

impl PlayerModulationState {
    /// Update stamina, reception recovery, and morale from the match policy.
    pub fn update_stamina_with_rules(
        &mut self,
        speed: f32,
        dt: f32,
        rules: &nba_domain::GameRules,
    ) {
        if speed > rules.stamina_sprint_speed_ftps {
            self.stamina =
                (self.stamina - rules.stamina_drain_per_second * dt).max(rules.stamina_floor);
        } else if speed < rules.stamina_recovery_speed_ftps {
            self.stamina = (self.stamina + rules.stamina_recovery_per_second * dt).min(1.0);
        }

        if self.catch_equilibrium < 1.0 {
            let recovery = rules.stamina_recovery_per_second * dt;
            self.catch_equilibrium = (self.catch_equilibrium + recovery).min(1.0);
        }

        let policy = &rules.modulation;
        if self.stamina < rules.stamina_exhausted_threshold {
            self.morale = MoraleState::Exhausted;
        } else if self.consecutive_makes >= policy.hot_hand_makes {
            self.morale = MoraleState::HotHand;
        } else if self.consecutive_misses >= policy.frustrated_misses
            || self.turnover_count >= policy.frustrated_turnovers
        {
            self.morale = MoraleState::Frustrated;
        } else {
            self.morale = MoraleState::Normal;
        }
    }

    /// Compatibility wrapper for callers that use the default policy.
    pub fn update_stamina(&mut self, speed: f32, dt: f32) {
        self.update_stamina_with_rules(speed, dt, &nba_domain::GameRules::default());
    }

    /// Record a shot result to modulate psychological feedback loop.
    pub fn record_shot(&mut self, is_made: bool) {
        if is_made {
            self.consecutive_makes += 1;
            self.consecutive_misses = 0;
        } else {
            self.consecutive_misses += 1;
            self.consecutive_makes = 0;
        }
    }

    /// Apply bad pass reception debuff.
    pub fn apply_reception_debuff(&mut self, pass_quality: f32) {
        self.catch_equilibrium = pass_quality.clamp(0.4, 1.0);
    }
}

/// Coach AI: monitors game state and applies configured macro strategy modulations.
#[derive(Debug, Clone)]
pub struct CoachStrategy {
    pub pace_factor: f32,
    pub three_point_bias: f32,
    pub defense_aggression: f32,
}

impl Default for CoachStrategy {
    fn default() -> Self {
        Self {
            pace_factor: 1.0,
            three_point_bias: 1.0,
            defense_aggression: 1.0,
        }
    }
}

impl CoachStrategy {
    pub fn evaluate(
        score_diff: i32,
        period: u32,
        time_remaining_s: f32,
        rules: &nba_domain::GameRules,
    ) -> Self {
        let mut strategy = Self::default();
        let policy = &rules.modulation;
        if period == policy.late_game_period && time_remaining_s < policy.late_game_seconds {
            if score_diff <= -policy.trailing_score_margin {
                strategy.pace_factor = policy.trailing_pace_factor;
                strategy.three_point_bias = policy.trailing_three_point_bias;
                strategy.defense_aggression = policy.trailing_defense_aggression;
            } else if score_diff >= policy.leading_score_margin {
                strategy.pace_factor = policy.leading_pace_factor;
                strategy.three_point_bias = policy.leading_three_point_bias;
                strategy.defense_aggression = policy.leading_defense_aggression;
            }
        }
        strategy
    }
}
