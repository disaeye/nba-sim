use glam::Vec2;
use nba_domain::{GameRules, Possession, SubPhase};
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalSet {
    HighPickAndRoll,
    FiveOutMotion,
    IsolationDrive,
    DriveAndKick,
    PostUp,
    FastBreakTransition,
}

/// 防守战术策略体系（tactics.md §2.2 防守覆盖模型）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveScheme {
    /// 人盯人防守伴随弱侧协防刷（Man-to-Man Shell）。
    ManToManShell,
    /// 2-3 联防（Two-Three Zone）：双后卫高位封锁，三锋线保护油漆区与底角。
    TwoThreeZone,
    /// 全员无限换防（Switch-All Aggressive）：防守人紧贴并切断传球线路。
    SwitchAllAggressive,
}

impl DefensiveScheme {
    pub fn id(self) -> &'static str {
        match self {
            Self::ManToManShell => "def_man_shell",
            Self::TwoThreeZone => "def_23_zone",
            Self::SwitchAllAggressive => "def_switch_all",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::ManToManShell => "人盯人协防体系 (Man-to-Man Shell)",
            Self::TwoThreeZone => "2-3 区域联防体系 (2-3 Zone)",
            Self::SwitchAllAggressive => "无限换防体系 (Switch-All Aggressive)",
        }
    }
}

impl TacticalSet {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "off_horns_pnr" => Some(Self::HighPickAndRoll),
            "off_motion_spacing" => Some(Self::FiveOutMotion),
            "off_transition_push" => Some(Self::FastBreakTransition),
            "off_delay_attack" => Some(Self::IsolationDrive),
            "off_post_split" => Some(Self::PostUp),
            "off_drag_screen" => Some(Self::DriveAndKick),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::HighPickAndRoll => "off_horns_pnr",
            Self::FiveOutMotion => "off_motion_spacing",
            Self::IsolationDrive => "off_delay_attack",
            Self::DriveAndKick => "off_drag_screen",
            Self::PostUp => "off_post_split",
            Self::FastBreakTransition => "off_transition_push",
        }
    }

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

/// Defensive scheme selected by the pre-game setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DefensiveTactic {
    ManConservative,
    ManPressure,
    SwitchHeavy,
    DropCoverage,
    HedgeRecover,
    Zone23,
}

impl DefensiveTactic {
    pub fn from_id(id: &str) -> Option<Self> {
        match id {
            "def_man_conservative" => Some(Self::ManConservative),
            "def_man_pressure" => Some(Self::ManPressure),
            "def_switch_heavy" => Some(Self::SwitchHeavy),
            "def_drop_coverage" => Some(Self::DropCoverage),
            "def_hedge_recover" => Some(Self::HedgeRecover),
            "def_zone_23" => Some(Self::Zone23),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::ManConservative => "def_man_conservative",
            Self::ManPressure => "def_man_pressure",
            Self::SwitchHeavy => "def_switch_heavy",
            Self::DropCoverage => "def_drop_coverage",
            Self::HedgeRecover => "def_hedge_recover",
            Self::Zone23 => "def_zone_23",
        }
    }

    pub fn name_zh(self) -> &'static str {
        match self {
            Self::ManConservative => "保守人盯人 (Conservative Man)",
            Self::ManPressure => "压迫人盯人 (Pressure Man)",
            Self::SwitchHeavy => "大量换防 (Switch Heavy)",
            Self::DropCoverage => "沉退防守 (Drop Coverage)",
            Self::HedgeRecover => "延误回位 (Hedge & Recover)",
            Self::Zone23 => "2-3 联防 (2-3 Zone)",
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

#[derive(Debug, Clone)]
pub struct TargetAssignment {
    pub player_id: Option<String>,
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
        ball_pos: Vec2,
        carrier_idx: usize,
        progress_sec: f32,
        rng: &mut impl Rng,
    ) -> (Vec<TargetAssignment>, Vec<TargetAssignment>) {
        Self::plan_possession_targets_with_geometry(
            tactical_set,
            sub_phase,
            possession,
            ball_pos,
            carrier_idx,
            progress_sec,
            rng,
            &GameRules::default(),
        )
    }
    #[allow(clippy::too_many_arguments)]
    pub fn plan_possession_targets_with_rules(
        tactical_set: TacticalSet,
        sub_phase: SubPhase,
        possession: Possession,
        ball_pos: Vec2,
        carrier_idx: usize,
        progress_sec: f32,
        rng: &mut impl Rng,
        rules: &GameRules,
    ) -> (Vec<TargetAssignment>, Vec<TargetAssignment>) {
        Self::plan_possession_targets_with_geometry(
            tactical_set,
            sub_phase,
            possession,
            ball_pos,
            carrier_idx,
            progress_sec,
            rng,
            rules,
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn plan_possession_targets_with_geometry(
        tactical_set: TacticalSet,
        sub_phase: SubPhase,
        possession: Possession,
        _ball_pos: Vec2,
        carrier_idx: usize,
        progress_sec: f32,
        _rng: &mut impl Rng,
        rules: &GameRules,
    ) -> (Vec<TargetAssignment>, Vec<TargetAssignment>) {
        let court = rules.court;
        let policy = &rules.tactics;
        let is_home = possession == Possession::Home;
        let hoop = court.hoop_pos(is_home);
        let base_x = hoop.x;
        let dir = if is_home { -1.0 } else { 1.0 };
        let mid_y = court.hoop_y_ft;
        let side_margin = court.height_ft * rules.court_side_margin_ratio;
        let baseline_offset = court.width_ft * policy.initiation_distance_ratio;
        let screen_offset = court.width_ft * policy.screen_distance_ratio;
        let drive_offset = court.width_ft * policy.drive_distance_ratio;
        let action_t = (progress_sec / policy.action_duration_seconds).clamp(0.0, 1.0);
        let speed = |ratio: f32| rules.max_player_speed_ftps * ratio;

        let mut off_targets = Vec::with_capacity(5);
        let mut def_targets = Vec::with_capacity(5);
        match tactical_set {
            TacticalSet::HighPickAndRoll => {
                let pg_spot = match sub_phase {
                    SubPhase::Initiation => Vec2::new(base_x + dir * baseline_offset, mid_y),
                    SubPhase::ActionExecution => Vec2::new(
                        base_x + dir * (baseline_offset - action_t * drive_offset),
                        mid_y + action_t * side_margin,
                    ),
                    _ => Vec2::new(base_x + dir * drive_offset, mid_y + side_margin * 0.6),
                };

                let c_spot = match sub_phase {
                    SubPhase::Initiation => {
                        Vec2::new(base_x + dir * screen_offset, mid_y + side_margin * 0.3)
                    }
                    SubPhase::ActionExecution => Vec2::new(
                        base_x + dir * (screen_offset - drive_offset),
                        mid_y - action_t * side_margin * 0.3,
                    ),
                    _ => Vec2::new(base_x + dir * drive_offset * 0.5, mid_y),
                };

                let sg_spot = Vec2::new(base_x + dir * screen_offset * 0.86, side_margin);
                let sf_spot = Vec2::new(
                    base_x + dir * screen_offset * 0.86,
                    court.height_ft - side_margin,
                );
                let pf_spot = Vec2::new(base_x + dir * screen_offset, side_margin * 1.5);

                let spots = [pg_spot, sg_spot, sf_spot, pf_spot, c_spot];
                let slots = [
                    "BallHandler",
                    "CornerSpacer",
                    "CornerSpacer",
                    "WingSpacer",
                    "ScreenAndRoll",
                ];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier {
                        if sub_phase == SubPhase::ActionExecution {
                            "DRIVE_OFF_SCREEN"
                        } else {
                            "DRIBBLE_TOP"
                        }
                    } else if i == 4 {
                        if sub_phase == SubPhase::Initiation {
                            "SET_HIGH_SCREEN"
                        } else {
                            "ROLL_TO_RIM"
                        }
                    } else {
                        "SPOT_UP_3PT"
                    };

                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: if is_carrier {
                            speed(policy.carrier_speed_ratio)
                        } else if i == 4 {
                            speed(policy.screener_speed_ratio)
                        } else {
                            speed(policy.support_speed_ratio)
                        },
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if is_carrier {
                            "HotHand".to_string()
                        } else {
                            "Normal".to_string()
                        },
                    });
                }
            }

            TacticalSet::FiveOutMotion => {
                // 5-Out perimeter motion with continuous cut & replace
                let phase_shift = progress_sec * 0.8;
                let spots = [
                    Vec2::new(
                        base_x + dir * baseline_offset,
                        mid_y + phase_shift.sin() * side_margin * 0.5,
                    ),
                    Vec2::new(base_x + dir * screen_offset * 0.86, side_margin * 1.25),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.86,
                        court.height_ft - side_margin * 1.25,
                    ),
                    Vec2::new(base_x + dir * screen_offset, side_margin),
                    Vec2::new(base_x + dir * screen_offset, court.height_ft - side_margin),
                ];
                let slots = [
                    "Playmaker",
                    "WingCutter",
                    "WingCutter",
                    "CornerSpacer",
                    "CornerSpacer",
                ];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier {
                        "BALL_MOVEMENT"
                    } else {
                        "PERIMETER_CUT"
                    };
                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: speed(if is_carrier {
                            policy.carrier_speed_ratio
                        } else {
                            policy.support_speed_ratio
                        }),
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: "Normal".to_string(),
                    });
                }
            }

            TacticalSet::IsolationDrive => {
                let iso_spot = match sub_phase {
                    SubPhase::Initiation => {
                        Vec2::new(base_x + dir * baseline_offset * 0.83, side_margin * 2.0)
                    }
                    SubPhase::ActionExecution => Vec2::new(
                        base_x + dir * (baseline_offset * 0.83 - action_t * drive_offset),
                        side_margin * 2.0 + action_t * side_margin * 0.9,
                    ),
                    _ => Vec2::new(base_x + dir * drive_offset * 0.6, mid_y),
                };
                let spots = [
                    iso_spot,
                    Vec2::new(base_x + dir * screen_offset, court.height_ft - side_margin),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.8,
                        court.height_ft - side_margin * 0.55,
                    ),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.95,
                        mid_y + side_margin * 0.4,
                    ),
                    Vec2::new(
                        base_x + dir * drive_offset * 0.6,
                        court.height_ft - side_margin * 0.75,
                    ),
                ];
                let slots = [
                    "IsoStar",
                    "WeaksideWing",
                    "WeaksideCorner",
                    "TopSpacer",
                    "ShortCorner",
                ];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier {
                        "ISOLATION_DRIVE"
                    } else {
                        "SPACING_CLEAROUT"
                    };
                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: speed(if is_carrier {
                            policy.carrier_speed_ratio
                        } else {
                            policy.support_speed_ratio
                        }),
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if is_carrier {
                            "HotHand".to_string()
                        } else {
                            "Normal".to_string()
                        },
                    });
                }
            }

            TacticalSet::DriveAndKick => {
                let drive_spot = match sub_phase {
                    SubPhase::Initiation => Vec2::new(base_x + dir * baseline_offset * 0.96, mid_y),
                    _ => Vec2::new(base_x + dir * drive_offset * 0.53, mid_y),
                };
                let spots = [
                    drive_spot,
                    Vec2::new(base_x + dir * screen_offset * 0.8, side_margin * 0.9),
                    Vec2::new(
                        base_x + dir * screen_offset,
                        court.height_ft - side_margin * 0.9,
                    ),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.95,
                        mid_y - side_margin * 1.1,
                    ),
                    Vec2::new(
                        base_x + dir * drive_offset * 0.6,
                        court.height_ft - side_margin,
                    ),
                ];
                let slots = [
                    "Penetrator",
                    "CornerSniper",
                    "WingSniper",
                    "TopReset",
                    "DunkerSpot",
                ];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if is_carrier {
                        "COLLAPSE_PAINT"
                    } else if i == 1 {
                        "OPEN_CATCH_SHOOT"
                    } else {
                        "PERIMETER_STATION"
                    };
                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: speed(if is_carrier {
                            policy.carrier_speed_ratio
                        } else {
                            policy.support_speed_ratio
                        }),
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if i == 1 {
                            "HotHand".to_string()
                        } else {
                            "Normal".to_string()
                        },
                    });
                }
            }

            TacticalSet::PostUp => {
                let c_post = Vec2::new(base_x + dir * drive_offset * 0.6, side_margin * 1.8);
                let spots = [
                    Vec2::new(base_x + dir * baseline_offset * 0.86, side_margin),
                    Vec2::new(base_x + dir * screen_offset, court.height_ft - side_margin),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.95,
                        mid_y + side_margin * 0.6,
                    ),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.8,
                        court.height_ft - side_margin * 0.6,
                    ),
                    c_post,
                ];
                let slots = [
                    "EntryPasser",
                    "WeaksideWing",
                    "TopRelief",
                    "CornerSpacer",
                    "PostMaster",
                ];
                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    let action = if i == 4 {
                        "POST_UP_MOVE"
                    } else if is_carrier {
                        "FEED_THE_POST"
                    } else {
                        "SPACE_WEAKSIDE"
                    };
                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: speed(if i == 4 {
                            policy.screener_speed_ratio
                        } else {
                            policy.support_speed_ratio
                        }),
                        action: action.to_string(),
                        slot: slot.to_string(),
                        morale: if i == 4 {
                            "HotHand".to_string()
                        } else {
                            "Normal".to_string()
                        },
                    });
                }
            }

            TacticalSet::FastBreakTransition => {
                let spots = [
                    Vec2::new(base_x + dir * drive_offset * 0.53, mid_y),
                    Vec2::new(base_x + dir * screen_offset * 0.65, side_margin),
                    Vec2::new(
                        base_x + dir * screen_offset * 0.65,
                        court.height_ft - side_margin,
                    ),
                    Vec2::new(base_x + dir * baseline_offset * 0.83, mid_y),
                    Vec2::new(base_x + dir * drive_offset * 0.8, mid_y - side_margin * 0.3),
                ];
                let slots = [
                    "RimRunner",
                    "LeftLaneSprinter",
                    "RightLaneSprinter",
                    "Trailer",
                    "LobThreat",
                ];

                for (i, (&spot, &slot)) in spots.iter().zip(slots.iter()).enumerate() {
                    let is_carrier = i == carrier_idx;
                    off_targets.push(TargetAssignment {
                        player_id: None,
                        target_pos: court.clamp_playable(spot, rules.player_radius_ft),
                        speed: speed(policy.transition_speed_ratio),
                        action: if is_carrier {
                            "FASTBREAK_LAYUP"
                        } else {
                            "TRANSITION_SPRINT"
                        }
                        .to_string(),
                        slot: slot.to_string(),
                        morale: "HotHand".to_string(),
                    });
                }
            }
        }

        for (i, off) in off_targets.iter().enumerate() {
            let is_guarding_carrier = i == carrier_idx;
            let to_hoop = hoop - off.target_pos;
            let dist_to_hoop = to_hoop.length();
            let to_hoop_dir = if dist_to_hoop > 0.1 {
                to_hoop.normalize()
            } else {
                Vec2::X
            };
            let (def_pos, action, slot) = if is_guarding_carrier {
                (
                    off.target_pos + to_hoop_dir * policy.defensive_gap_ft,
                    "ON_BALL_CONTEST",
                    "PointDefender",
                )
            } else {
                (
                    off.target_pos + to_hoop_dir * (dist_to_hoop * policy.help_sag_ratio),
                    "HELP_SIDE_SHELL",
                    "HelpAnchor",
                )
            };
            def_targets.push(TargetAssignment {
                player_id: None,
                target_pos: court.clamp_playable(def_pos, rules.player_radius_ft),
                speed: speed(policy.defender_speed_ratio),
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

    /// Assign generated targets to the ordered roster supplied by the caller.
    pub fn bind_targets(targets: &mut [TargetAssignment], roster_ids: &[String]) {
        for (target, player_id) in targets.iter_mut().zip(roster_ids.iter()) {
            target.player_id = Some(player_id.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::StdRng;

    #[test]
    fn tactical_sets_roundtrip_ids_and_names() {
        let sets = [
            TacticalSet::HighPickAndRoll,
            TacticalSet::FiveOutMotion,
            TacticalSet::IsolationDrive,
            TacticalSet::DriveAndKick,
            TacticalSet::PostUp,
            TacticalSet::FastBreakTransition,
        ];
        for set in sets {
            let id = set.id();
            assert_eq!(TacticalSet::from_id(id), Some(set));
            assert!(!set.name_zh().is_empty());
        }
    }

    #[test]
    fn defensive_schemes_have_valid_ids_and_display_names() {
        let schemes = [
            DefensiveScheme::ManToManShell,
            DefensiveScheme::TwoThreeZone,
            DefensiveScheme::SwitchAllAggressive,
        ];
        for s in schemes {
            assert!(!s.id().is_empty());
            assert!(!s.display_name().is_empty());
        }
    }

    #[test]
    fn plan_possession_targets_generates_five_on_five_slots() {
        let mut rng = StdRng::seed_from_u64(42);
        let rules = GameRules::default();
        for set in [
            TacticalSet::HighPickAndRoll,
            TacticalSet::FiveOutMotion,
            TacticalSet::IsolationDrive,
            TacticalSet::DriveAndKick,
            TacticalSet::PostUp,
            TacticalSet::FastBreakTransition,
        ] {
            let (home, away) = TacticalPlanner::plan_possession_targets_with_rules(
                set,
                SubPhase::Initiation,
                Possession::Home,
                Vec2::ZERO,
                0,
                0.0,
                &mut rng,
                &rules,
            );
            assert_eq!(home.len(), 5);
            assert_eq!(away.len(), 5);
        }
    }
}
