/// Psychological State Machine of a Player (Phase 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoraleState {
    Normal,
    HotHand,     // Made multiple consecutive shots: boosts confidence & shot utility
    Frustrated,  // Turnovers/blocked: increases error rate and erratic decision-making
    Exhausted,   // Low stamina: heavily dampens explosive drive & sprint weights
    Clutch,      // High focus in tight 4th quarter moments
}

#[derive(Debug, Clone)]
pub struct PlayerModulationState {
    pub stamina: f32,          // 0.0 to 1.0 (1.0 = fresh)
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
    /// Update stamina based on current movement speed.
    pub fn update_stamina(&mut self, speed: f32, dt: f32) {
        if speed > 15.0 {
            // Sprint drain
            self.stamina = (self.stamina - 0.015 * dt).max(0.2);
        } else if speed < 6.0 {
            // Recovery while jogging / standing
            self.stamina = (self.stamina + 0.02 * dt).min(1.0);
        }

        // Catch equilibrium restores over time
        if self.catch_equilibrium < 1.0 {
            self.catch_equilibrium = (self.catch_equilibrium + 0.8 * dt).min(1.0);
        }

        // Evaluate Morale State
        if self.stamina < 0.35 {
            self.morale = MoraleState::Exhausted;
        } else if self.consecutive_makes >= 2 {
            self.morale = MoraleState::HotHand;
        } else if self.consecutive_misses >= 3 || self.turnover_count >= 2 {
            self.morale = MoraleState::Frustrated;
        } else {
            self.morale = MoraleState::Normal;
        }
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

/// Coach AI: Monitors game state and applies macro strategy modulations.
#[derive(Debug, Clone)]
pub struct CoachStrategy {
    pub pace_factor: f32,         // 1.0 = normal, 1.3 = fast break push, 0.8 = slow down
    pub three_point_bias: f32,    // 1.0 = normal, 1.5 = trailing by 10 in 4th quarter
    pub defense_aggression: f32,  // 1.0 = base, 1.4 = full court press / blitz
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
    pub fn evaluate(score_diff: i32, period: u32, time_remaining_s: f32) -> Self {
        let mut strat = CoachStrategy::default();
        if period == 4 && time_remaining_s < 120.0 {
            if score_diff < -6 {
                // Trailing late: shoot more 3s and push pace
                strat.pace_factor = 1.35;
                strat.three_point_bias = 1.6;
                strat.defense_aggression = 1.4;
            } else if score_diff > 6 {
                // Leading late: slow down and control clock
                strat.pace_factor = 0.75;
                strat.three_point_bias = 0.8;
                strat.defense_aggression = 1.0;
            }
        }
        strat
    }
}
