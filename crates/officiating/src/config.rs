use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveConfig {
    pub foundation_version: String,
    pub model: String,
    pub base_rates: BaseRates,
    pub shot_type_rates: ShotTypeRates,
    pub shot_type_block_bias: ShotTypeBlockBias,
    pub bonus_rule: BonusRule,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseRates {
    pub pass_success: f32,
    pub handoff_success: f32,
    pub drive_success: f32,
    pub shot_make_2pt: f32,
    pub shot_make_3pt: f32,
    pub ft_make: f32,
    pub steal_attempt_success: f32,
    pub foul_on_drive_rate: f32,
    pub offensive_rebound_rate: f32,
    pub block_rate: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotTypeRates {
    pub catch_shoot_2pt: f32,
    pub catch_shoot_3pt: f32,
    pub pull_up_2pt: f32,
    pub pull_up_3pt: f32,
    pub post_2pt: f32,
    pub drive_finish_2pt: f32,
    pub other_2pt: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShotTypeBlockBias {
    pub drive_finish: f32,
    pub post: f32,
    pub catch_shoot: f32,
    pub pull_up: f32,
    pub other: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BonusRule {
    pub team_fouls_per_period_threshold: u32,
}

impl Default for ResolveConfig {
    fn default() -> Self {
        Self {
            foundation_version: "0.15.0".to_string(),
            model: "homogeneous_v0_1".to_string(),
            base_rates: BaseRates {
                pass_success: 0.94,
                handoff_success: 0.97,
                drive_success: 0.78,
                shot_make_2pt: 0.565,
                shot_make_3pt: 0.34,
                ft_make: 0.77,
                steal_attempt_success: 0.005,
                foul_on_drive_rate: 0.10,
                offensive_rebound_rate: 0.26,
                block_rate: 0.05,
            },
            shot_type_rates: ShotTypeRates {
                catch_shoot_2pt: 0.46,
                catch_shoot_3pt: 0.33,
                pull_up_2pt: 0.40,
                pull_up_3pt: 0.27,
                post_2pt: 0.45,
                drive_finish_2pt: 0.60,
                other_2pt: 0.42,
            },
            shot_type_block_bias: ShotTypeBlockBias {
                drive_finish: 1.4,
                post: 1.2,
                catch_shoot: 0.7,
                pull_up: 0.8,
                other: 1.0,
            },
            bonus_rule: BonusRule {
                team_fouls_per_period_threshold: 5,
            },
        }
    }
}
