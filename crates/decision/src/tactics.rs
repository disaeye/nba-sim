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
impl TargetAssignment {
    pub fn off_ball_kind(&self) -> Option<nba_domain::action_window::OffBallActionKind> {
        match self.action.as_str() {
            "SET_HIGH_SCREEN" => Some(nba_domain::action_window::OffBallActionKind::SetScreen),
            "ROLL_TO_RIM" => Some(nba_domain::action_window::OffBallActionKind::RollToRim),
            "SPOT_UP_3PT" => Some(nba_domain::action_window::OffBallActionKind::SpotUpRelocate),
            "PERIMETER_CUT" => Some(nba_domain::action_window::OffBallActionKind::VCut),
            _ => None,
        }
    }
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
            None,
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
        live_off_positions: Option<&[Vec2]>,
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
            live_off_positions,
        )
    }
    /// D5.1b：以战术档案（slot 元数据）为准生成进攻目标。
    ///
    /// 这是取代「全局 ratio 推所有槽位」的正式路径：档案里声明的
    /// `base_offset_x/y` 决定每个槽位的真实距离，因此底角（8–10 ft）、
    /// 内线（18 ft）、弧顶（28–29 ft）会同时存在，而不是全队挤在弧顶。
    ///
    /// `carrier_slot_index` 由调用方通过能力适配的 slot fill 给出
    /// （见 [`Self::select_carrier_slot`]）；此处不做球员选择。
    #[allow(clippy::too_many_arguments)]
    pub fn plan_offense_from_spec(
        spec: &nba_domain::TacticalSetSpec,
        sub_phase: SubPhase,
        possession: Possession,
        carrier_slot_index: usize,
        progress_sec: f32,
        rules: &GameRules,
    ) -> Vec<TargetAssignment> {
        let court = rules.court;
        let policy = &rules.tactics;
        let is_home = possession == Possession::Home;
        let hoop = court.hoop_pos(is_home);
        let dir = if is_home { -1.0 } else { 1.0 };
        let action_t = (progress_sec / policy.action_duration_seconds).clamp(0.0, 1.0);
        Self::spec_offense_targets(
            spec,
            is_home,
            court,
            policy,
            carrier_slot_index,
            sub_phase,
            action_t,
            hoop,
            dir,
            rules,
        )
    }

    /// D5.1b：能力适配的 slot fill（tactics.md §3 契约）。
    ///
    /// 输入：档案槽位 + 在场球员的能力画像；输出：`slots[i] -> player_id` 的
    /// 确定性匹配，或 `FitError`（无人可满足最低要求）。
    ///
    /// 算法（确定性、可复现）：
    /// 1. 计算每个 (槽位, 球员) 的适配分：按槽位语义加权相关能力；
    /// 2. 按**稀缺性**降序处理槽位（候选人少者优先），避免被通用球员抢占；
    /// 3. 每个槽位取当前剩余球员中最高分者；分数相同时按 player_id 排序决胜。
    ///
    /// 评分只使用与槽位职责相关的能力（不引入全局 IQ 乘数）。
    pub fn fill_slots(
        spec: &nba_domain::TacticalSetSpec,
        players: &[nba_domain::data::PlayerSlotFitness],
        rules: &GameRules,
    ) -> Result<Vec<Option<String>>, String> {
        if players.is_empty() {
            return Err("slot fill requires at least one available player".to_string());
        }
        if spec.slots.is_empty() {
            return Err("tactical spec declares no slots".to_string());
        }
        let policy = &rules.tactics;
        let score = |slot: &nba_domain::TacticalSlotSpec, p: &nba_domain::data::PlayerSlotFitness| -> f32 {
            let role = slot.role.to_ascii_lowercase();
            if role.contains("playmaker") || role.contains("handler") {
                p.ball_handling * policy.slot_handler_ball_handling_weight
                    + p.decision_iq * policy.slot_handler_decision_iq_weight
            } else if slot.is_screener {
                p.strength * policy.slot_screener_strength_weight
                    + p.finishing * policy.slot_screener_finishing_weight
            } else if slot.is_corner_spacer {
                p.shooting_three * policy.slot_corner_three_weight
                    + p.off_ball_sense * policy.slot_corner_off_ball_weight
            } else if slot.is_wing_relocate {
                p.shooting_mid * policy.slot_wing_mid_weight
                    + p.off_ball_sense * policy.slot_wing_off_ball_weight
            } else {
                p.decision_iq * policy.slot_generic_decision_weight
                    + p.off_ball_sense * policy.slot_generic_off_ball_weight
            }
        };

        // 稀缺性：候选人数（分数显著高于 0 的球员数）升序 → 先处理难填的槽位。
        let mut order: Vec<usize> = (0..spec.slots.len()).collect();
        let candidate_count = |i: usize| -> usize {
            players
                .iter()
                .filter(|p| score(&spec.slots[i], p) > 0.01)
                .count()
        };
        order.sort_by(|&a, &b| {
            candidate_count(a)
                .cmp(&candidate_count(b))
                .then(a.cmp(&b))
        });

        let mut taken = vec![false; players.len()];
        let mut assignment: Vec<Option<String>> = vec![None; spec.slots.len()];
        for &slot_idx in &order {
            let slot = &spec.slots[slot_idx];
            let mut best: Option<(f32, usize)> = None;
            for (pi, p) in players.iter().enumerate() {
                if taken[pi] {
                    continue;
                }
                let s = score(slot, p);
                match best {
                    Some((bs, _)) if s <= bs => {}
                    _ => best = Some((s, pi)),
                }
            }
            match best {
                Some((_, pi)) => {
                    taken[pi] = true;
                    assignment[slot_idx] = Some(players[pi].player_id.clone());
                }
                None => {
                    return Err(format!(
                        "no available player fits slot `{}`",
                        slot.role
                    ));
                }
            }
        }
        Ok(assignment)
    }

    /// D5.1b：返回与槽位顺序一致的球员 id 列表（供 bind_targets 使用）。
    ///
    /// 失败时回退到 roster 顺序，保证引擎不会因档案/人员不匹配而死锁；
    /// 回退是显式的，调用方可据 `Ok/Err` 记录缺口。
    pub fn fill_slots_or_roster_order(
        spec: &nba_domain::TacticalSetSpec,
        players: &[nba_domain::data::PlayerSlotFitness],
        roster_order: &[String],
        rules: &GameRules,
    ) -> (Vec<String>, Option<String>) {
        match Self::fill_slots(spec, players, rules) {
            Ok(assignment) => {
                let ids: Vec<String> = assignment.into_iter().flatten().collect();
                if ids.len() == spec.slots.len() {
                    (ids, None)
                } else {
                    (roster_order.to_vec(), Some("partial slot assignment".to_string()))
                }
            }
            Err(e) => (roster_order.to_vec(), Some(e)),
        }
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
        live_off_positions: Option<&[Vec2]>,
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
                    _ => Vec2::new(base_x + dir * drive_offset * 0.5, mid_y),
                };
                let off_carrier_pos = live_off_positions
                    .and_then(|p| p.get(carrier_idx).copied())
                    .unwrap_or(pg_spot);
                let drive_penetration = ((off_carrier_pos.x - hoop.x).abs() < 24.0) as u32 as f32;
                let corner_lift = dir * drive_penetration * 4.0;
                let sg_spot = Vec2::new(
                    base_x + dir * screen_offset * 0.86 + corner_lift,
                    side_margin,
                );
                let sf_spot = Vec2::new(
                    base_x + dir * screen_offset * 0.86 + corner_lift,
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
        let carrier_pos = live_off_positions
            .and_then(|positions| positions.get(carrier_idx).copied())
            .or_else(|| off_targets.get(carrier_idx).map(|t| t.target_pos))
            .unwrap_or(hoop);
        for (i, off) in off_targets.iter().enumerate() {
            let is_guarding_carrier = i == carrier_idx;
            let off_pos = live_off_positions
                .and_then(|positions| positions.get(i).copied())
                .unwrap_or(off.target_pos);
            let to_hoop = hoop - off_pos;
            let dist_to_hoop = to_hoop.length();
            let to_hoop_dir = if dist_to_hoop > 0.1 {
                to_hoop.normalize()
            } else {
                Vec2::X
            };
            let (def_pos, action, slot) = if is_guarding_carrier {
                // 领防人：建立紧逼与滑步阻截线 (Pursuit Contest)
                let gap = policy.defensive_gap_ft.min(dist_to_hoop * 0.4).max(2.5);
                (
                    off_pos + to_hoop_dir * gap,
                    "ON_BALL_CONTEST",
                    "PointDefender",
                )
            } else {
                // 弱侧协防人：真实球-人-筐三角 (Ball-Man-Basket Defensive Triangle)
                // 防守人目标点位于对位人与篮筐、持球人位置的外心，绝不盲目扎堆禁区中心
                let to_carrier = carrier_pos - off_pos;
                let to_carrier_dir = if to_carrier.length() > 0.1 {
                    to_carrier.normalize()
                } else {
                    Vec2::ZERO
                };
                // 综合人-筐方向与人-球方向，保持在传球拦截视野与回防扑防（Closeout）边界
                let bisector_dir = (to_hoop_dir * 0.7 + to_carrier_dir * 0.3).normalize_or_zero();
                let effective_sag = (dist_to_hoop * policy.help_sag_ratio)
                    .min(policy.defensive_gap_ft * 2.0)
                    .max(rules.player_radius_ft * 2.0);
                (
                    off_pos + bisector_dir * effective_sag,
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

    /// D5.1b：把战术档案声明的槽位转换为场上世界坐标（gap.md §10.3）。
    ///
    /// 坐标语义（与 `data/tactics/*.json` 一致）：
    /// - `base_offset_x`：距**进攻底线**的距离（home 攻右篮，故 x = width - offset）；
    /// - `base_offset_y`：绝对 y（0 = 一侧边线，height = 另一侧）。
    ///
    /// 关键修复动机：此前所有槽位都由全局 ratio 推得，导致**全队都在弧顶
    /// 三分线外**（实测进攻方 90% 球员距篮筐 > 23.75 ft），中距离出手根本
    /// 没有机会产生（2PA 均值 14.5，真实 ~55）。档案里已声明了正确且多样化
    /// 的距离（底角 8–10 ft、内线 18 ft、弧顶 28–29 ft），但从未被消费。
    pub fn spec_slot_world_pos(
        slot: &nba_domain::TacticalSlotSpec,
        is_home: bool,
        court: nba_domain::court::CourtGeometry,
        rules: &GameRules,
    ) -> Vec2 {
        let x = if is_home {
            court.width_ft - slot.base_offset_x
        } else {
            slot.base_offset_x
        };
        // 第一性原理：槽位目标必须是**球员能真实站立**的位置。
        //
        // 若目标落在球员半径之外（例如 y=2.5 而可站立下限是 1.8，
        // 或 y=0 直接压在边线上），物理层会每 tick 把球员夹回边界——
        // 球员被**永久钉在边界**，且反复产生 `BoundaryCross` 边界事实，
        // 对持球人即被判成出界失误（实测每场 42 次虚假
        // `TURNOVER:OUT_OF_BOUNDS`）。
        //
        // 这里把目标 clamp 到「含球员半径的可站立区域」，使目标可达。
        let margin = rules.player_radius_ft;
        let x = x.clamp(margin, court.width_ft - margin);
        let y = slot
            .base_offset_y
            .clamp(margin, court.height_ft - margin);
        Vec2::new(x, y)
    }

    /// 按档案槽位生成进攻目标（顺序 = 档案声明顺序）。
    ///
    /// 每个槽位必须恰好得到一个球员；调用方负责把返回的 targets 绑定到
    /// 经能力适配的球员（D5.1b 的 slot fill）。
    #[allow(clippy::too_many_arguments)]
    fn spec_offense_targets(
        spec: &nba_domain::TacticalSetSpec,
        is_home: bool,
        court: nba_domain::court::CourtGeometry,
        policy: &nba_domain::rules::TacticalRules,
        carrier_slot_index: usize,
        sub_phase: SubPhase,
        action_t: f32,
        hoop: Vec2,
        dir: f32,
        rules: &GameRules,
    ) -> Vec<TargetAssignment> {
        let speed = |ratio: f32| rules.max_player_speed_ftps * ratio;
        spec.slots
            .iter()
            .enumerate()
            .map(|(i, slot)| {
                let base = Self::spec_slot_world_pos(slot, is_home, court, rules);
                let is_carrier = i == carrier_slot_index;
                // 持球人在执行阶段向篮筐压迫；其余槽位保持在档案声明的位置，
                // 以保证中距离/底角/内线距离真实存在。
                let (target_pos, speed_ratio, action) = if is_carrier {
                    let pressed = match sub_phase {
                        SubPhase::Initiation => base,
                        _ => base + (hoop - base) * (action_t * policy.drive_distance_ratio),
                    };
                    (
                        pressed,
                        policy.carrier_speed_ratio,
                        if sub_phase == SubPhase::Initiation {
                            "DRIBBLE_TOP"
                        } else {
                            "DRIVE_OFF_SCREEN"
                        },
                    )
                } else if slot.is_screener {
                    let rolled = match sub_phase {
                        SubPhase::Initiation => base,
                        _ => base + (hoop - base) * action_t * 0.5,
                    };
                    (
                        rolled,
                        policy.screener_speed_ratio,
                        if sub_phase == SubPhase::Initiation {
                            "SET_HIGH_SCREEN"
                        } else {
                            "ROLL_TO_RIM"
                        },
                    )
                } else if slot.is_corner_spacer {
                    (base, policy.support_speed_ratio, "SPOT_UP_3PT")
                } else if slot.is_wing_relocate {
                    // 翼位球员在弧顶与内线之间做纵向 relocate，制造切入时机。
                    let drift = Vec2::new(0.0, dir * action_t * 6.0);
                    (base + drift, policy.support_speed_ratio, "PERIMETER_CUT")
                } else {
                    (base, policy.support_speed_ratio, "SPOT_UP_3PT")
                };
                TargetAssignment {
                    player_id: None,
                    target_pos: court.clamp_playable(target_pos, rules.player_radius_ft),
                    speed: speed(speed_ratio),
                    action: action.to_string(),
                    slot: slot.role.clone(),
                    morale: "Normal".to_string(),
                }
            })
            .collect()
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
    use rand::rngs::StdRng;
    use rand::SeedableRng;

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
                None,
            );
            assert_eq!(home.len(), 5);
            assert_eq!(away.len(), 5);
        }
    }
}
