use glam::Vec2;
use nba_domain::{GameRules, Possession, SubPhase};

use crate::potential_field::EmergentDefenseTarget;
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TacticalSet {
    HighPickAndRoll,
    SpainPickAndRoll,
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
            "off_spain_pnr" => Some(Self::SpainPickAndRoll),
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
            Self::SpainPickAndRoll => "off_spain_pnr",
            Self::FiveOutMotion => "off_motion_spacing",
            Self::IsolationDrive => "off_delay_attack",
            Self::DriveAndKick => "off_drag_screen",
            Self::PostUp => "off_post_split",
            Self::FastBreakTransition => "off_transition_push",
        }
    }

    pub fn name_zh(&self) -> &'static str {
        match self {
            TacticalSet::HighPickAndRoll => "牛角高位挡拆体系 (Horns Pick-and-Roll)",
            TacticalSet::SpainPickAndRoll => "西班牙双掩护体系 (Spain Pick-and-Roll)",
            TacticalSet::FiveOutMotion => "五外动态进攻体系 (5-Out Motion)",
            TacticalSet::IsolationDrive => "高位单打体系 (Isolation Drive)",
            TacticalSet::DriveAndKick => "突分投射体系 (Drive & Kick)",
            TacticalSet::PostUp => "低位背身策应体系 (Post Up & Split)",
            TacticalSet::FastBreakTransition => "快攻闪击转换体系 (Fastbreak Transition)",
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
    /// Solver output exists only when this target came from the potential-field branch.
    pub potential_field: Option<EmergentDefenseTarget>,
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
    /// 档案槽位的标准权重：按该档案的**持球槽位**能力需求打分。
    ///
    /// 处理球人的身份与「谁填哪个槽位」必须用同一套能力口径，否则会出现
    /// 「slot fill 认为是持球人、实际选出的持球人不是」。持球槽位由档案声明
    /// （`SlotBehaviour::DribbleTop`），因此权重也取自该槽位的 `requirements`。
    pub fn handler_score(
        spec: &nba_domain::TacticalSetSpec,
        attributes: &nba_domain::PlayerAttributes,
    ) -> f32 {
        let fitness = nba_domain::PlayerSlotFitness {
            player_id: String::new(),
            attributes: attributes.clone(),
        };
        spec.slots
            .iter()
            .find(|slot| slot.behaviour == nba_domain::SlotBehaviour::DribbleTop)
            .map(|slot| {
                slot.requirements
                    .iter()
                    .map(|requirement| {
                        requirement.weight * fitness.attribute(requirement.attribute)
                    })
                    .sum()
            })
            .unwrap_or(0.0)
    }

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
            false,
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
        drive_active: bool,
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
            drive_active,
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

    /// D5.1b：能力适配的 slot fill（tactics.md §3 规定）。
    ///
    /// 输入：档案槽位 + 在场球员的能力画像；输出：`slots[i] -> player_id` 的
    /// 确定性匹配，或 `FitError`（无人可满足最低要求）。
    ///
    /// 算法（确定性、可复现）：
    /// 1. 计算每个 (槽位, 球员) 的适配分：按槽位语义加权相关能力；
    /// 2. 按**稀缺性**降序处理槽位（候选人少者优先），避免被通用球员抢占；
    /// 3. 每个槽位取当前剩余球员中最高分者；分数相同时按 player_id 排序决胜。
    ///
    /// 评分只使用档案声明的能力需求（不引入全局 IQ 乘数，也不按角色字符串分支）。
    ///
    /// ## 为什么不再读 `rules`（rules.md C1）
    ///
    /// 过去每类槽位的能力权重写在 `TacticalRules` 的 `slot_*_weight` 字段里，
    /// 而“哪类槽位看哪几维能力”写在代码的 `role.contains("playmaker")` 分支里。
    /// 后者是行为硬编码（charter C1：一切影响行为的判断必须来自数据通道）。
    /// 现在权重与维度都由 `data/tactics/*.json` 的 `requirements` 声明，
    /// `rules` 参数因此不再被读取。
    pub fn fill_slots(
        spec: &nba_domain::TacticalSetSpec,
        players: &[nba_domain::data::PlayerSlotFitness],
        _rules: &GameRules,
    ) -> Result<Vec<Option<String>>, String> {
        if players.is_empty() {
            return Err("slot fill requires at least one available player".to_string());
        }
        if spec.slots.is_empty() {
            return Err("tactical spec declares no slots".to_string());
        }
        let score =
            |slot: &nba_domain::TacticalSlotSpec, p: &nba_domain::data::PlayerSlotFitness| -> f32 {
                slot.requirements
                    .iter()
                    .map(|requirement| requirement.weight * p.attribute(requirement.attribute))
                    .sum()
            };

        // 稀缺性：候选人数（分数显著高于 0 的球员数）升序 → 先处理难填的槽位。
        let mut order: Vec<usize> = (0..spec.slots.len()).collect();
        let candidate_count = |i: usize| -> usize {
            players
                .iter()
                .filter(|p| score(&spec.slots[i], p) > 0.01)
                .count()
        };
        order.sort_by(|&a, &b| candidate_count(a).cmp(&candidate_count(b)).then(a.cmp(&b)));

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
                    return Err(format!("no available player fits slot `{}`", slot.id));
                }
            }
        }
        Ok(assignment)
    }

    /// D5.1b：返回与槽位顺序一致的球员 id 列表（供 bind_targets 使用）。
    ///
    /// 失败时回退到 roster 顺序，保证引擎不会因档案/人员不匹配而停摆；
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
                    (
                        roster_order.to_vec(),
                        Some("partial slot assignment".to_string()),
                    )
                }
            }
            Err(e) => (roster_order.to_vec(), Some(e)),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn plan_possession_targets_with_geometry(
        _tactical_set: TacticalSet,
        _sub_phase: SubPhase,
        possession: Possession,
        _ball_pos: Vec2,
        carrier_idx: usize,
        progress_sec: f32,
        _rng: &mut impl Rng,
        rules: &GameRules,
        live_off_positions: Option<&[Vec2]>,
        drive_active: bool,
    ) -> (Vec<TargetAssignment>, Vec<TargetAssignment>) {
        let court = rules.court;
        let policy = &rules.tactics;
        let is_home = possession == Possession::Home;
        let hoop = court.hoop_pos(is_home);
        let base_x = hoop.x;
        let dir = if is_home { -1.0 } else { 1.0 };
        let mid_y = court.hoop_y_ft;
        let _side_margin = court.height_ft * rules.court_side_margin_ratio;
        let _baseline_offset = court.width_ft * policy.initiation_distance_ratio;
        let _screen_offset = court.width_ft * policy.screen_distance_ratio;
        let drive_offset = court.width_ft * policy.drive_distance_ratio;
        let _action_t = (progress_sec / policy.action_duration_seconds).clamp(0.0, 1.0);
        let speed = |ratio: f32| rules.max_player_speed_ftps * ratio;

        let mut off_targets = Vec::with_capacity(5);
        let mut def_targets = Vec::with_capacity(5);
        // 旧 `TacticalSet` 几何分支已退役（evidence/problem.md §26）。
        //
        // 那 6 个分支为每种旧战术枚举硬编码一套槽位几何，但引擎随后用
        // **档案路径**（`plan_offense_from_spec`，见 `match_engine.rs`）
        // 覆盖进攻侧，使这些位置成为每 tick 计算、每 tick 丢弃的无效计算。
        //
        // 三组对照实验（8 seed full）证明删除是行为中性的：清空进攻输出、
        // 或把全部进攻位置中性化后，逐 seed 全字段比对差异均为 0。
        // 防守目标由 `live_off_positions`（实时位置）驱动；进攻槽位位置仅在
        // 调用方传 `None` 时兜底，而引擎路径总是传 `Some`。
        //
        // 这里只保留防守循环所需的槽位**数量**与占位点；真正的进攻几何
        // 由版本化战术档案负责（`docs/tactics.md` §2）。
        for slot in [
            "BallHandler",
            "CornerSpacer",
            "WingSpacer",
            "ScreenAndRoll",
            "Spacer",
        ] {
            off_targets.push(TargetAssignment {
                player_id: None,
                target_pos: court.clamp_playable(
                    Vec2::new(base_x + dir * drive_offset, mid_y),
                    rules.player_radius_ft,
                ),
                speed: speed(policy.carrier_speed_ratio),
                action: "Spot".to_string(),
                slot: slot.to_string(),
                morale: "Normal".to_string(),
                potential_field: None,
            });
        }
        let carrier_pos = live_off_positions
            .and_then(|positions| positions.get(carrier_idx).copied())
            .or_else(|| off_targets.get(carrier_idx).map(|t| t.target_pos))
            .unwrap_or(hoop);
        let screen_rules = policy.defense.screen_defense;
        let field_rules = policy.defense.potential_field;
        let screener_idx = off_targets
            .iter()
            .position(|t| t.slot == "ScreenAndRoll")
            .unwrap_or(3);
        let screener_pos = live_off_positions
            .and_then(|positions| positions.get(screener_idx).copied())
            .or_else(|| off_targets.get(screener_idx).map(|t| t.target_pos))
            .unwrap_or(carrier_pos);
        let dist_to_screener = (carrier_pos - screener_pos).length();
        let is_screening_action = dist_to_screener < field_rules.screen_detection_radius_ft
            && screener_idx != carrier_idx;
        let is_hedge_scheme = screen_rules.hedge_distance_ft > 0.0;
        let is_drop_scheme = screen_rules.drop_depth_ft > 0.0;
        let should_switch = is_screening_action
            && !is_hedge_scheme
            && !is_drop_scheme
            && (policy.defense.switch_aggressiveness > field_rules.switch_high_threshold
                || (policy.defense.switch_aggressiveness > field_rules.switch_low_threshold
                    && dist_to_screener <= screen_rules.switch_trigger_distance_ft));

        // 连续多体势能场求解器 (ADR-012 空间动力学与涌现模型)。
        //
        // 系数取自**当前生效的防守方案**（`rules.tactics.defense` 由
        // `sync_team_tactics` 按防守方档案写入），因此势能场随防守方案变化，
        // 而不是每次都用同一份默认值。
        let potential_solver = crate::potential_field::DefensePotentialFieldSolver::new(
            rules.tactics.defense.potential_field,
        );
        let off_coords: Vec<Vec2> = (0..5)
            .map(|idx| {
                live_off_positions
                    .and_then(|positions| positions.get(idx).copied())
                    .or_else(|| off_targets.get(idx).map(|t| t.target_pos))
                    .unwrap_or(carrier_pos)
            })
            .collect();

        for (i, off) in off_targets.iter().enumerate() {
            let is_guarding_carrier = i == carrier_idx;
            let is_guarding_screener = i == screener_idx;
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
            let (def_pos, action, slot, potential_field) =
                if should_switch && (is_guarding_carrier || is_guarding_screener) {
                    if is_guarding_carrier {
                        let to_screener_hoop = (hoop - screener_pos).normalize_or_zero();
                        (
                            screener_pos + to_screener_hoop * field_rules.switch_anchor_gap_ft,
                            "SWITCH_ASSIGNMENT",
                            "SwitchAnchor",
                            None,
                        )
                    } else {
                        let to_carrier_hoop = (hoop - carrier_pos).normalize_or_zero();
                        (
                            carrier_pos + to_carrier_hoop * field_rules.switch_screener_gap_ft,
                            "SWITCH_ASSIGNMENT",
                            "SwitchDefender",
                            None,
                        )
                    }
                } else if is_guarding_screener
                    && is_screening_action
                    && screen_rules.drop_depth_ft > 0.0
                {
                    let to_hoop_from_screen = (hoop - screener_pos).normalize_or_zero();
                    (
                        screener_pos + to_hoop_from_screen * screen_rules.drop_depth_ft,
                        "DROP_CONTAIN",
                        "DropAnchor",
                        None,
                    )
                } else if is_guarding_screener
                    && is_screening_action
                    && screen_rules.hedge_distance_ft > 0.0
                {
                    let to_carrier_from_screen = (carrier_pos - screener_pos).normalize_or_zero();
                    (
                        screener_pos + to_carrier_from_screen * screen_rules.hedge_distance_ft,
                        "HEDGE_AND_RECOVER",
                        "HedgeDefender",
                        None,
                    )
                } else if is_guarding_carrier {
                    // 领防人：建立紧逼与滑步阻截线 (Pursuit Contest)。
                    // round-6：间隔经防守方案倍率调制（迫使贴防或退到纵深）。
                    let gap = (policy.defensive_gap_ft * policy.defense.on_ball_gap_multiplier)
                        .min(dist_to_hoop * field_rules.on_ball_gap_hoop_ratio)
                        .max(field_rules.on_ball_gap_min_ft);
                    (
                        off_pos + to_hoop_dir * gap,
                        "ON_BALL_CONTEST",
                        "PointDefender",
                        None,
                    )
                } else {
                    // 弱侧协防人与轮转体系：完全由连续多体势能场梯度与能量极小值求解驱动
                    // 绝不依赖硬编码 if-else 判定，自然涌现出 ROTATE_RIM_HELP、X_OUT_CLOSEOUT 或 HELP_SIDE_SHELL
                    // 体能入口暂传满体能（行为中性）；引擎接线（消费真实体能）在 #18 落地。
                    let emergent = potential_solver.solve_equilibrium(
                        carrier_pos,
                        hoop,
                        &off_coords,
                        i,
                        carrier_idx,
                        rules,
                        1.0,
                        drive_active,
                    );
                    (
                        emergent.target_pos,
                        emergent.action,
                        emergent.slot,
                        Some(emergent),
                    )
                };
            def_targets.push(TargetAssignment {
                player_id: None,
                target_pos: court.clamp_playable(def_pos, rules.player_radius_ft),
                speed: speed(policy.defender_speed_ratio),
                action: action.to_string(),
                slot: slot.to_string(),
                morale: "Normal".to_string(),
                potential_field,
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
        // 若目标在球员半径之外（例如 y=2.5 而可站立下限是 1.8，
        // 或 y=0 直接压在边线上），物理层会每 tick 把球员夹回边界——
        // 球员被**永久固定在边界**，且反复产生 `BoundaryCross` 边界事实，
        // 对持球人即被判成出界失误（实测每场 42 次虚假
        // `TURNOVER:OUT_OF_BOUNDS`）。
        //
        // 这里把目标 clamp 到「含球员半径的可站立区域」，使目标可达。
        let margin = rules.player_radius_ft;
        let x = x.clamp(margin, court.width_ft - margin);
        let y = slot.base_offset_y.clamp(margin, court.height_ft - margin);
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
                let initiating = sub_phase == SubPhase::Initiation;
                // 持球人在执行阶段向篮筐压迫；其余槽位按档案声明的行为移动。
                let (target_pos, speed_ratio, action) = match slot.behaviour {
                    nba_domain::SlotBehaviour::DribbleTop => {
                        let pressed = if initiating {
                            base
                        } else {
                            base + (hoop - base) * (action_t * policy.drive_distance_ratio)
                        };
                        (
                            pressed,
                            policy.carrier_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                    nba_domain::SlotBehaviour::HighScreenRoll => {
                        let rolled = if initiating {
                            base
                        } else {
                            base + (hoop - base) * action_t * 0.5
                        };
                        (
                            rolled,
                            policy.screener_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                    nba_domain::SlotBehaviour::SpotUp => (
                        base,
                        policy.support_speed_ratio,
                        slot.behaviour.action_label(initiating),
                    ),
                    nba_domain::SlotBehaviour::PerimeterRelocate => {
                        // 翼位球员在弧顶与内线之间做纵向 relocate，制造切入时机。
                        let drift = Vec2::new(0.0, dir * action_t * 6.0);
                        (
                            base + drift,
                            policy.support_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                    nba_domain::SlotBehaviour::BackdoorCut => {
                        // G6a 链 1：弱侧背切。执行期从翼位沿「篮筐方向」切入，
                        // 切入深度随进攻进度推进（action_t 0..1），目标是篮下
                        // 接球攻框位置；发起期保持原翼位站位（与 SpotUp 同）。
                        let cut = (hoop - base) * action_t * policy.backdoor_cut_depth_ratio;
                        (
                            base + cut,
                            policy.carrier_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                    nba_domain::SlotBehaviour::DipToRim => {
                        // G6a 链 4：下沉禁区。底角/翼位球员沿「篮筐方向」下沉
                        // 到篮下边缘争抢内线落位，为持球突破提供传球终点与
                        // 篮板位置；发起期保持原站位拉开空间。
                        let dip = (hoop - base) * action_t * policy.dip_to_rim_depth_ratio;
                        (
                            base + dip,
                            policy.support_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                    nba_domain::SlotBehaviour::BackScreenPop => {
                        // 西班牙背掩护外弹：发起期在罚球线中路架设背掩护，
                        // 执行期反向弹向弧顶三分线外大空位。
                        let popped = if initiating {
                            base
                        } else {
                            let pop_dir = (base - hoop).normalize_or_zero();
                            base + pop_dir * (action_t * 15.0)
                        };
                        (
                            popped,
                            policy.carrier_speed_ratio,
                            slot.behaviour.action_label(initiating),
                        )
                    }
                };
                let _ = is_carrier;
                TargetAssignment {
                    player_id: None,
                    target_pos: court.clamp_playable(target_pos, rules.player_radius_ft),
                    speed: speed(speed_ratio),
                    action: action.to_string(),
                    slot: slot.id.clone(),
                    morale: "Normal".to_string(),
                    potential_field: None,
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
                false,
            );
            assert_eq!(home.len(), 5);
            assert_eq!(away.len(), 5);
        }
    }
}
