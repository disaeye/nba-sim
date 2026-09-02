use crate::court::CourtGeometry;
use serde::{Deserialize, Serialize};

/// Static player capabilities consumed by decision, movement, and officiating systems.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerAttributes {
    pub speed: f32,
    pub acceleration: f32,
    pub agility: f32,
    pub strength: f32,
    pub vertical: f32,
    pub stamina: f32,
    pub ball_handling: f32,
    pub passing: f32,
    pub shooting_close: f32,
    pub shooting_mid: f32,
    pub shooting_three: f32,
    pub free_throw: f32,
    pub finishing: f32,
    pub defense_perimeter: f32,
    pub defense_interior: f32,
    pub steal: f32,
    pub block: f32,
    pub offensive_rebound: f32,
    pub defensive_rebound: f32,
    pub decision_iq: f32,
    pub off_ball_sense: f32,
}

impl Default for PlayerAttributes {
    fn default() -> Self {
        Self {
            speed: 0.5,
            acceleration: 0.5,
            agility: 0.5,
            strength: 0.5,
            vertical: 0.5,
            stamina: 0.5,
            ball_handling: 0.5,
            passing: 0.5,
            shooting_close: 0.5,
            shooting_mid: 0.5,
            shooting_three: 0.5,
            free_throw: 0.5,
            finishing: 0.5,
            defense_perimeter: 0.5,
            defense_interior: 0.5,
            steal: 0.5,
            block: 0.5,
            offensive_rebound: 0.5,
            defensive_rebound: 0.5,
            decision_iq: 0.5,
            off_ball_sense: 0.5,
        }
    }
}

impl PlayerAttributes {
    #[inline]
    pub fn defense_on_ball(&self) -> f32 {
        self.defense_perimeter
    }
    #[inline]
    pub fn defense_help(&self) -> f32 {
        self.defense_interior
    }
    #[inline]
    pub fn rebounding(&self) -> f32 {
        (self.offensive_rebound + self.defensive_rebound) * 0.5
    }
    #[inline]
    pub fn basketball_iq(&self) -> f32 {
        self.decision_iq
    }
    #[inline]
    pub fn positioning(&self) -> f32 {
        self.off_ball_sense
    }
}
impl PlayerAttributes {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.speed,
            self.acceleration,
            self.agility,
            self.strength,
            self.vertical,
            self.stamina,
            self.ball_handling,
            self.passing,
            self.shooting_close,
            self.shooting_mid,
            self.shooting_three,
            self.free_throw,
            self.finishing,
            self.defense_perimeter,
            self.defense_interior,
            self.steal,
            self.block,
            self.offensive_rebound,
            self.defensive_rebound,
            self.decision_iq,
            self.off_ball_sense,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("player attributes must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}
impl PlayerTendencies {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.shoot_frequency,
            self.drive_frequency,
            self.pass_frequency,
            self.cut_frequency,
            self.screen_frequency,
            self.offensive_rebound_frequency,
            self.risk_tolerance,
            self.transition_sprint,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("player tendencies must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}

impl TeamTraits {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            self.pace,
            self.three_point_emphasis,
            self.rim_pressure,
            self.defense_aggression,
            self.rebound_emphasis,
        ];
        if values
            .iter()
            .any(|value| !value.is_finite() || !(0.0..=1.0).contains(value))
        {
            return Err("team traits must be finite and within 0..=1".to_string());
        }
        Ok(())
    }
}

/// Decision preferences are separate from ability so the same skill can produce
/// different styles without branching on a roster-specific player id.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlayerTendencies {
    pub shoot_frequency: f32,
    pub drive_frequency: f32,
    pub pass_frequency: f32,
    pub cut_frequency: f32,
    pub screen_frequency: f32,
    pub offensive_rebound_frequency: f32,
    pub risk_tolerance: f32,
    pub transition_sprint: f32,
}

impl Default for PlayerTendencies {
    fn default() -> Self {
        Self {
            shoot_frequency: 0.5,
            drive_frequency: 0.5,
            pass_frequency: 0.5,
            cut_frequency: 0.5,
            screen_frequency: 0.5,
            offensive_rebound_frequency: 0.5,
            risk_tolerance: 0.5,
            transition_sprint: 0.5,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayerRole {
    PrimaryCreator,
    SecondaryCreator,
    Shooter,
    Cutter,
    Roller,
    Screener,
    Spacer,
    RimProtector,
    Rebounder,
    Defender,
    TransitionFinisher,
    PostScorer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerData {
    pub id: String,
    pub name: String,
    pub jersey: String,
    pub team_id: String,
    pub height_cm: u16,
    pub weight_kg: u16,
    pub age: u8,
    pub attributes: PlayerAttributes,
    pub roles: Vec<PlayerRole>,
    pub tendencies: PlayerTendencies,
    /// Initial court position in the engine's feet coordinate system.
    pub initial_position_ft: (f32, f32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TeamTraits {
    pub pace: f32,
    pub three_point_emphasis: f32,
    pub rim_pressure: f32,
    pub defense_aggression: f32,
    pub rebound_emphasis: f32,
}

impl Default for TeamTraits {
    fn default() -> Self {
        Self {
            pace: 0.5,
            three_point_emphasis: 0.5,
            rim_pressure: 0.5,
            defense_aggression: 0.5,
            rebound_emphasis: 0.5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamData {
    pub id: String,
    pub name: String,
    pub short_name: String,
    pub players: Vec<PlayerData>,
    pub default_offense_tactic: String,
    pub default_defense_tactic: String,
    pub team_traits: TeamTraits,
}

impl TeamData {
    pub fn builtin_home() -> Self {
        Self::builtin_home_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_home_with_geometry(geometry: CourtGeometry) -> Self {
        Self {
            id: "north_city_hawks".to_string(),
            name: "North City Hawks".to_string(),
            short_name: "Hawks".to_string(),
            players: builtin_players("H", geometry),
            default_offense_tactic: "off_horns_pnr".to_string(),
            default_defense_tactic: "def_drop_coverage".to_string(),
            team_traits: TeamTraits {
                pace: 0.62,
                three_point_emphasis: 0.58,
                rim_pressure: 0.64,
                defense_aggression: 0.48,
                rebound_emphasis: 0.55,
            },
        }
    }

    pub fn builtin_away() -> Self {
        Self::builtin_away_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_away_with_geometry(geometry: CourtGeometry) -> Self {
        Self {
            id: "south_bay_mariners".to_string(),
            name: "South Bay Mariners".to_string(),
            short_name: "Mariners".to_string(),
            players: builtin_players("A", geometry),
            default_offense_tactic: "off_motion_spacing".to_string(),
            default_defense_tactic: "def_man_conservative".to_string(),
            team_traits: TeamTraits {
                pace: 0.48,
                three_point_emphasis: 0.64,
                rim_pressure: 0.50,
                defense_aggression: 0.56,
                rebound_emphasis: 0.61,
            },
        }
    }

    pub fn builtin_pair() -> [Self; 2] {
        Self::builtin_pair_with_geometry(CourtGeometry::default())
    }

    pub fn builtin_pair_with_geometry(geometry: CourtGeometry) -> [Self; 2] {
        [
            Self::builtin_home_with_geometry(geometry),
            Self::builtin_away_with_geometry(geometry),
        ]
    }
}

fn builtin_players(prefix: &str, geometry: CourtGeometry) -> Vec<PlayerData> {
    let home_positions = [
        (geometry.width_ft * 0.298, geometry.hoop_y_ft),
        (geometry.width_ft * 0.372, geometry.height_ft * 0.20),
        (geometry.width_ft * 0.372, geometry.height_ft * 0.80),
        (geometry.width_ft * 0.234, geometry.height_ft * 0.32),
        (geometry.width_ft * 0.160, geometry.hoop_y_ft),
        (geometry.width_ft * 0.213, geometry.height_ft * 0.14),
        (geometry.width_ft * 0.213, geometry.height_ft * 0.86),
        (geometry.width_ft * 0.128, geometry.height_ft * 0.18),
    ];
    let home_names = [
        "Darius Vale",
        "Malik Rowan",
        "Andre Mercer",
        "Caleb North",
        "Jonas Reed",
        "Jordan Pike",
        "Aaron Wells",
        "Rudy Moss",
    ];
    let away_names = [
        "Luka Maren",
        "Kellan Price",
        "Scott Rowan",
        "Drake Ellis",
        "Anton Vale",
        "Tyler Quinn",
        "Mika Stone",
        "Niko Voss",
    ];
    let home_jerseys = ["0", "7", "4", "8", "9", "11", "24", "35"];
    let away_jerseys = ["23", "3", "15", "1", "28", "12", "6", "41"];
    let is_home = prefix == "H";
    let team_id = if is_home {
        "north_city_hawks"
    } else {
        "south_bay_mariners"
    };
    home_positions
        .into_iter()
        .enumerate()
        .map(|(index, (x, y))| {
            let initial_position_ft = if index < 5 {
                if is_home {
                    (x, y)
                } else {
                    (geometry.width_ft - x, y)
                }
            } else {
                // 替补球员（6/7/8 号）严格位于场外替补席
                let bench_offset_x = (index - 5) as f32 * 6.0;
                if is_home {
                    (geometry.width_ft * 0.20 + bench_offset_x, -4.0)
                } else {
                    (geometry.width_ft * 0.80 - bench_offset_x, geometry.height_ft + 4.0)
                }
            };
            PlayerData {
                id: format!("{}_{}", prefix, index + 1),
                name: (if is_home {
                    home_names[index]
                } else {
                    away_names[index]
                })
                .to_string(),
                jersey: (if is_home {
                    home_jerseys[index]
                } else {
                    away_jerseys[index]
                })
                .to_string(),
                team_id: team_id.to_string(),
                height_cm: player_height_cm(index),
                weight_kg: player_weight_kg(index),
                age: 22 + (index as u8 % 10),
                attributes: builtin_attributes(index),
                roles: builtin_roles(index),
                tendencies: builtin_tendencies(index),
                initial_position_ft,
            }
        })
        .collect()
}

fn builtin_attributes(index: usize) -> PlayerAttributes {
    let mut attributes = PlayerAttributes::default();
    match index {
        0 => {
            attributes.free_throw = 0.84;
            attributes.speed = 0.88;
            attributes.acceleration = 0.86;
            attributes.ball_handling = 0.90;
            attributes.passing = 0.86;
            attributes.shooting_mid = 0.78;
            attributes.shooting_three = 0.76;
            attributes.finishing = 0.80;
            attributes.decision_iq = 0.88;
        }
        1 => {
            attributes.free_throw = 0.88;
            attributes.speed = 0.76;
            attributes.acceleration = 0.72;
            attributes.shooting_three = 0.91;
            attributes.shooting_mid = 0.82;
            attributes.off_ball_sense = 0.84;
        }
        2 => {
            attributes.free_throw = 0.78;
            attributes.speed = 0.80;
            attributes.acceleration = 0.78;
            attributes.passing = 0.78;
            attributes.defense_perimeter = 0.86;
            attributes.defense_interior = 0.80;
            attributes.decision_iq = 0.82;
        }
        3 => {
            attributes.free_throw = 0.72;
            attributes.strength = 0.78;
            attributes.shooting_three = 0.72;
            attributes.shooting_mid = 0.76;
            attributes.off_ball_sense = 0.78;
            attributes.defensive_rebound = 0.72;
            attributes.offensive_rebound = 0.68;
        }
        4 => {
            attributes.free_throw = 0.55;
            attributes.strength = 0.92;
            attributes.stamina = 0.86;
            attributes.finishing = 0.84;
            attributes.defensive_rebound = 0.92;
            attributes.offensive_rebound = 0.90;
            attributes.off_ball_sense = 0.88;
        }
        5 => {
            attributes.free_throw = 0.80;
            attributes.speed = 0.82;
            attributes.acceleration = 0.80;
            attributes.ball_handling = 0.77;
            attributes.shooting_three = 0.79;
            attributes.passing = 0.70;
        }
        6 => {
            attributes.free_throw = 0.86;
            attributes.speed = 0.90;
            attributes.acceleration = 0.89;
            attributes.agility = 0.88;
            attributes.defense_perimeter = 0.79;
            attributes.finishing = 0.78;
        }
        _ => {
            attributes.free_throw = 0.52;
            attributes.strength = 0.94;
            attributes.stamina = 0.88;
            attributes.block = 0.94;
            attributes.defensive_rebound = 0.94;
            attributes.offensive_rebound = 0.92;
            attributes.defense_interior = 0.92;
        }
    }
    attributes
}

fn builtin_roles(index: usize) -> Vec<PlayerRole> {
    match index {
        0 => vec![PlayerRole::PrimaryCreator, PlayerRole::Shooter],
        1 => vec![PlayerRole::Shooter, PlayerRole::Spacer],
        2 => vec![PlayerRole::SecondaryCreator, PlayerRole::Defender],
        3 => vec![PlayerRole::Screener, PlayerRole::Shooter],
        4 => vec![
            PlayerRole::Screener,
            PlayerRole::Roller,
            PlayerRole::Rebounder,
        ],
        5 => vec![PlayerRole::Shooter, PlayerRole::SecondaryCreator],
        6 => vec![
            PlayerRole::Cutter,
            PlayerRole::Defender,
            PlayerRole::TransitionFinisher,
        ],
        _ => vec![PlayerRole::RimProtector, PlayerRole::Rebounder],
    }
}

fn builtin_tendencies(index: usize) -> PlayerTendencies {
    let mut tendencies = PlayerTendencies::default();
    match index {
        0 => {
            tendencies.shoot_frequency = 0.68;
            tendencies.drive_frequency = 0.78;
            tendencies.pass_frequency = 0.76;
            tendencies.risk_tolerance = 0.64;
        }
        1 => {
            tendencies.shoot_frequency = 0.86;
            tendencies.pass_frequency = 0.42;
            tendencies.cut_frequency = 0.72;
        }
        2 => {
            tendencies.pass_frequency = 0.70;
            tendencies.cut_frequency = 0.66;
            tendencies.transition_sprint = 0.70;
        }
        3 | 4 => {
            tendencies.screen_frequency = 0.82;
            tendencies.offensive_rebound_frequency = 0.78;
        }
        5 => {
            tendencies.shoot_frequency = 0.74;
            tendencies.drive_frequency = 0.61;
        }
        6 => {
            tendencies.cut_frequency = 0.84;
            tendencies.transition_sprint = 0.90;
        }
        _ => {
            tendencies.offensive_rebound_frequency = 0.82;
            tendencies.transition_sprint = 0.42;
        }
    }
    tendencies
}

fn player_height_cm(index: usize) -> u16 {
    [193, 190, 198, 203, 211, 191, 201, 216][index]
}

fn player_weight_kg(index: usize) -> u16 {
    [90, 88, 98, 104, 116, 86, 102, 122][index]
}
