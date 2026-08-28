use glam::Vec2;
use rand::Rng;
use nba_domain::court::{COURT_HEIGHT_FT, COURT_WIDTH_FT, HOOP_LEFT_FT, HOOP_RIGHT_FT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Possession {
    Home,
    Away,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalSet {
    HighPickAndRoll,
    FiveOutMotion,
    IsolationDrive,
    DriveAndKick,
    PostUp,
    FastBreakTransition,
}

impl TacticalSet {
    pub fn name_zh(&self) -> &'static str {
        match self {
            TacticalSet::HighPickAndRoll => "高位挡拆战术 (High Pick and Roll)",
            TacticalSet::FiveOutMotion => "五外动态进攻 (5-Out Motion)",
            TacticalSet::IsolationDrive => "巨星高位单打 (Isolation Drive)",
            TacticalSet::DriveAndKick => "突分投射体系 (Drive & Kick)",
            TacticalSet::PostUp => "低位背身单打 (Post Up)",
            TacticalSet::FastBreakTransition => "快攻闪击反击 (Fastbreak Transition)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffensiveRole {
    BallHandler,
    Screener,
    RollMan,
    PopMan,
    Cutter,
    SpotUpSpacer,
    PostIsolator,
    DunkerSpot,
    WingShooter,
    CornerShooter,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveRole {
    PrimaryOnBall,
    DropProtector,
    SwitchDefender,
    HelpAndRecover,
    NailDenial,
    WeakSideTag,
    DenyWing,
    HelpCorner,
    RimProtector,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamIntent {
    ExecuteSet,
    EmergencyReset,
    AttackRimNow,
    KickToOpenCorner,
    HuntMismatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubPhase {
    Initiation,
    ActionExecution,
    ShotAttempt,
    FlightAndRebound,
    DeadBallReset,
}

#[derive(Debug, Clone)]
pub struct TargetAssignment {
    pub target_pos: Vec2,
    pub speed: f32,
    pub action: String,
    pub slot: String,
    pub morale: String,
}

pub struct TacticalPlanner;

impl TacticalPlanner {
    pub fn plan_possession_targets(
        tactical_set: TacticalSet,
        sub_phase: SubPhase,
        possession: Possession,
        _ball_pos: Vec2,
        carrier_idx: usize,
        progress_sec: f32,
        _rng: &mut impl Rng,
    ) -> (Vec<TargetAssignment>, Vec<TargetAssignment>) {
        let is_home = possession == Possession::Home;
        let hoop = if is_home { HOOP_RIGHT_FT } else { HOOP_LEFT_FT };
        let base_x = hoop.x;
        let dir = if is_home { -1.0 } else { 1.0 }; // Direction pointing from hoop outwards

        let mut off_targets = Vec::with_capacity(5);
        let mut def_targets = Vec::with_capacity(5);

        match tactical_set {
            TacticalSet::HighPickAndRoll => {
                // PG (0) at top of key (28ft), C (4) comes up to set high screen (25ft)
                // SG (1) and SF (2) space to corners/wings, PF (3) at opposite wing
                let pg_spot = match sub_phase {
                    SubPhase::Initiation => Vec2::new(base_x + dir * 28.0, 25.0),
                    SubPhase::ActionExecution => {
                        // Drive around the screen towards the paint
                        let drive_t = (progress_sec / 3.0).clamp(0.0, 1.0);
                        Vec2::new(base_x + dir * (28.0 - drive_t * 14.0), 25.0 + drive_t * 6.0)
                    }
                    _ => Vec2::new(base_x + dir * 14.0, 30.0),
                };

                let c_spot = match sub_phase {
                    SubPhase::Initiation => {
                        // Move to set screen at top of key
                        Vec2::new(base_x + dir * 25.5, 27.5)
                    }
                    SubPhase::ActionExecution => {
                        // Roll hard to the basket
                        let roll_t = (progress_sec / 3.0).clamp(0.0, 1.0);
                        Vec2::new(base_x + dir * (25.5 - roll_t * 17.0), 27.5 - roll_t * 3.0)
                    }
                    _ => Vec2::new(base_x + dir * 8.0, 24.0),
                };

                let sg_spot = Vec2::new(base_x + dir * 22.0, 8.0);  // Corner spacer
                let sf_spot = Vec2::new(base_x + dir * 22.0, 42.0); // Opposite corner spacer
                let pf_spot = Vec2::new(base_x + dir * 26.0, 12.0); // Wing spacer

                let spots = [pg_spot, sg_spot, sf_spot, pf_spot, c_spot];
                let slots = ["BallHandler", "CornerSpacer", "CornerSpacer", "WingSpacer", "ScreenAndRoll"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier {
                        if sub_phase == SubPhase::ActionExecution { "DRIVE_OFF_SCREEN" } else { "DRIBBLE_TOP" }
                    } else if i == 4 {
                        if sub_phase == SubPhase::Initiation { "SET_HIGH_SCREEN" } else { "ROLL_TO_RIM" }
                    } else {
                        "SPOT_UP_3PT"
                    };

                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: if is_carrier { 16.0 } else if i == 4 { 14.0 } else { 10.0 },
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if is_carrier { "HotHand".to_string() } else { "Normal".to_string() },
                    });
                }
            }

            TacticalSet::FiveOutMotion => {
                // 5-Out perimeter motion with continuous cut & replace
                let phase_shift = progress_sec * 0.8;
                let spots = [
                    Vec2::new(base_x + dir * 28.0, 25.0 + (phase_shift).sin() * 4.0),
                    Vec2::new(base_x + dir * 24.0, 10.0),
                    Vec2::new(base_x + dir * 24.0, 40.0),
                    Vec2::new(base_x + dir * 22.0, 6.0),
                    Vec2::new(base_x + dir * 22.0, 44.0),
                ];
                let slots = ["Playmaker", "WingCutter", "WingCutter", "CornerSpacer", "CornerSpacer"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier { "BALL_MOVEMENT" } else { "PERIMETER_CUT" };
                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: 12.0,
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: "Normal".to_string(),
                    });
                }
            }

            TacticalSet::IsolationDrive => {
                // Creator isolates at wing/top, 4 teammates clear out to weakside perimeter
                let iso_spot = match sub_phase {
                    SubPhase::Initiation => Vec2::new(base_x + dir * 25.0, 16.0),
                    SubPhase::ActionExecution => {
                        let drive_t = (progress_sec / 2.5).clamp(0.0, 1.0);
                        Vec2::new(base_x + dir * (25.0 - drive_t * 16.0), 16.0 + drive_t * 7.0)
                    }
                    _ => Vec2::new(base_x + dir * 9.0, 23.0),
                };

                let spots = [
                    iso_spot,
                    Vec2::new(base_x + dir * 27.0, 38.0),
                    Vec2::new(base_x + dir * 23.0, 45.0),
                    Vec2::new(base_x + dir * 26.0, 28.0),
                    Vec2::new(base_x + dir * 10.0, 44.0),
                ];
                let slots = ["IsoStar", "WeaksideWing", "WeaksideCorner", "TopSpacer", "ShortCorner"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier { "ISOLATION_DRIVE" } else { "SPACING_CLEAROUT" };
                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: if is_carrier { 18.0 } else { 9.0 },
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if is_carrier { "HotHand".to_string() } else { "Normal".to_string() },
                    });
                }
            }

            TacticalSet::DriveAndKick => {
                // Penetrator drives deep into paint, draws help, kicks out to open 3 shooter
                let drive_spot = match sub_phase {
                    SubPhase::Initiation => Vec2::new(base_x + dir * 27.0, 25.0),
                    _ => Vec2::new(base_x + dir * 8.0, 25.0),
                };

                let spots = [
                    drive_spot,
                    Vec2::new(base_x + dir * 23.0, 7.0),  // Kickout target corner 3
                    Vec2::new(base_x + dir * 25.0, 42.0),
                    Vec2::new(base_x + dir * 27.0, 16.0),
                    Vec2::new(base_x + dir * 9.0, 40.0),
                ];
                let slots = ["Penetrator", "CornerSniper", "WingSniper", "TopReset", "DunkerSpot"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier { "COLLAPSE_PAINT" } else if i == 1 { "OPEN_CATCH_SHOOT" } else { "PERIMETER_STATION" };
                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: if is_carrier { 17.0 } else { 11.0 },
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if i == 1 { "HotHand".to_string() } else { "Normal".to_string() },
                    });
                }
            }

            TacticalSet::PostUp => {
                // Center (4) posts up on low block (8ft from hoop), guards space
                let c_post = Vec2::new(base_x + dir * 9.0, 18.0);
                let spots = [
                    Vec2::new(base_x + dir * 26.0, 14.0), // Entry passer
                    Vec2::new(base_x + dir * 24.0, 40.0),
                    Vec2::new(base_x + dir * 26.0, 28.0),
                    Vec2::new(base_x + dir * 22.0, 45.0),
                    c_post,
                ];
                let slots = ["EntryPasser", "WeaksideWing", "TopRelief", "CornerSpacer", "PostMaster"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if i == 4 { "POST_UP_MOVE" } else if is_carrier { "FEED_THE_POST" } else { "SPACE_WEAKSIDE" };
                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: if i == 4 { 13.0 } else { 10.0 },
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if i == 4 { "HotHand".to_string() } else { "Normal".to_string() },
                    });
                }
            }

            TacticalSet::FastBreakTransition => {
                // Sprint down court
                let spots = [
                    Vec2::new(base_x + dir * 8.0, 25.0),  // Rim runner
                    Vec2::new(base_x + dir * 18.0, 8.0),  // Left lane
                    Vec2::new(base_x + dir * 18.0, 42.0), // Right lane
                    Vec2::new(base_x + dir * 25.0, 25.0), // Trailer
                    Vec2::new(base_x + dir * 12.0, 22.0), // Second runner
                ];
                let slots = ["RimRunner", "LeftLaneSprinter", "RightLaneSprinter", "Trailer", "LobThreat"];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    off_targets.push(TargetAssignment {
                        target_pos: spot,
                        speed: 20.0,
                        action: if is_carrier { "FASTBREAK_LAYUP" } else { "TRANSITION_SPRINT" }.to_string(),
                        slot: slot.to_string(),
                        morale: "HotHand".to_string(),
                    });
                }
            }
        }

        // Defensive Elastic Mesh: On-ball contest + Help-side defensive shell
        for (i, off) in off_targets.iter().enumerate() {
            let is_guarding_carrier = i == carrier_idx;
            let to_hoop = hoop - off.target_pos;
            let dist_to_hoop = to_hoop.length();
            let to_hoop_dir = if dist_to_hoop > 0.1 { to_hoop.normalize() } else { Vec2::X };

            let (def_pos, action, slot) = if is_guarding_carrier {
                // On-ball defender stands 4.0ft between ball handler and hoop, arms contested
                let pos = off.target_pos + to_hoop_dir * 4.0;
                (pos, "ON_BALL_CONTEST", "PointDefender")
            } else {
                // Help-side defenders form the defensive shell sagging toward the paint (ball-you-man triangle)
                let sag_distance = (dist_to_hoop * 0.35).clamp(4.5, 9.0);
                let help_pos = off.target_pos + to_hoop_dir * sag_distance;
                (help_pos, "HELP_SIDE_SHELL", "HelpAnchor")
            };

            def_targets.push(TargetAssignment {
                target_pos: Vec2::new(
                    def_pos.x.clamp(2.0, COURT_WIDTH_FT - 2.0),
                    def_pos.y.clamp(2.0, COURT_HEIGHT_FT - 2.0),
                ),
                speed: if is_guarding_carrier { 16.0 } else { 13.0 },
                action: action.to_string(),
                slot: slot.to_string(),
                morale: "Normal".to_string(),
            });
        }

        if is_home {
            (off_targets, def_targets)
        } else {
            (def_targets, off_targets)
        }
    }
}
