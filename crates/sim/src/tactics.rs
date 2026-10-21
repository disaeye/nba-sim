use std::collections::BTreeMap;
use std::path::Path;

use nba_domain::play::{DecisionActionFamily, PlayCourtSide, PlayInhibitionMode, PlayPredicate, PlaySpec};
use nba_domain::tactics::TacticalSetSpec;
use rand::Rng;

use crate::action::Fact;
use crate::state::PossessionKind;
use crate::world::World;

/// 剧本档案清单：名称与内嵌文本成对出现，浏览器目标同样可用。
pub const PLAY_SOURCES: &[(&str, &str)] = &[
    (
        "corner_backdoor_v1",
        include_str!("../../../data/tactics/plays/corner_backdoor_v1.json"),
    ),
    (
        "delay_dho_handoff_v1",
        include_str!("../../../data/tactics/plays/delay_dho_handoff_v1.json"),
    ),
    (
        "drag_screen_drive_kick_v1",
        include_str!("../../../data/tactics/plays/drag_screen_drive_kick_v1.json"),
    ),
    (
        "high_pnr_roll_v1",
        include_str!("../../../data/tactics/plays/high_pnr_roll_v1.json"),
    ),
    (
        "horns_flare_pop_v1",
        include_str!("../../../data/tactics/plays/horns_flare_pop_v1.json"),
    ),
    (
        "post_split_cut_v1",
        include_str!("../../../data/tactics/plays/post_split_cut_v1.json"),
    ),
    (
        "spain_pnr_stack_v1",
        include_str!("../../../data/tactics/plays/spain_pnr_stack_v1.json"),
    ),
    (
        "transition_rim_runner_v1",
        include_str!("../../../data/tactics/plays/transition_rim_runner_v1.json"),
    ),
    (
        "weak_side_lift_v1",
        include_str!("../../../data/tactics/plays/weak_side_lift_v1.json"),
    ),
];

/// 体系档案清单。
pub const SYSTEM_SOURCES: &[(&str, &str)] = &[
    (
        "delay_attack",
        include_str!("../../../data/tactics/delay_attack.json"),
    ),
    (
        "drag_screen",
        include_str!("../../../data/tactics/drag_screen.json"),
    ),
    (
        "five_out_motion",
        include_str!("../../../data/tactics/five_out_motion.json"),
    ),
    (
        "high_pick_and_roll",
        include_str!("../../../data/tactics/high_pick_and_roll.json"),
    ),
    (
        "post_split",
        include_str!("../../../data/tactics/post_split.json"),
    ),
    (
        "spain_pick_and_roll",
        include_str!("../../../data/tactics/spain_pick_and_roll.json"),
    ),
    (
        "transition_push",
        include_str!("../../../data/tactics/transition_push.json"),
    ),
];

/// 选板档案与体系档案的合集。
#[derive(Debug, Clone)]
pub struct TacticsTable {
    pub plays: Vec<PlaySpec>,
    pub systems: BTreeMap<String, TacticalSetSpec>,
}

impl TacticsTable {
    pub fn embedded() -> Self {
        let plays = PLAY_SOURCES
            .iter()
            .map(|(_, text)| PlaySpec::from_json(text).expect("embedded play spec must parse"))
            .collect();
        let systems = SYSTEM_SOURCES
            .iter()
            .map(|(_, text)| {
                let spec: TacticalSetSpec =
                    serde_json::from_str(text).expect("embedded system spec must parse");
                (spec.id.clone(), spec)
            })
            .collect();
        let table = Self { plays, systems };
        table.validate().expect("embedded tactics must validate");
        table
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        // 校准目录是 `data/calibration`，战术档案与它平级在 `data/tactics`。
        let data_dir = dir.parent().unwrap_or(dir);
        let plays_dir = data_dir.join("tactics/plays");
        let mut plays = Vec::new();
        for (name, _) in PLAY_SOURCES {
            let path = plays_dir.join(format!("{name}.json"));
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("play spec {} unreadable: {error}", path.display()))?;
            plays.push(PlaySpec::from_json(&text).map_err(|error| {
                format!("play spec {} invalid: {error:?}", path.display())
            })?);
        }
        let tactics_dir = data_dir.join("tactics");
        let mut systems = BTreeMap::new();
        for (name, _) in SYSTEM_SOURCES {
            let path = tactics_dir.join(format!("{name}.json"));
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("system spec {} unreadable: {error}", path.display()))?;
            let spec: TacticalSetSpec = serde_json::from_str(&text)
                .map_err(|error| format!("system spec {} invalid: {error}", path.display()))?;
            systems.insert(spec.id.clone(), spec);
        }
        let table = Self { plays, systems };
        table.validate()?;
        Ok(table)
    }

    /// 校验档案互相自洽：剧本被至少一个体系引用、引用的剧本存在、
    /// 规则目标槽位在被引用的体系里存在。
    pub fn validate(&self) -> Result<(), String> {
        let mut seen = BTreeMap::new();
        for play in &self.plays {
            if seen.insert(play.id.clone(), ()).is_some() {
                return Err(format!("duplicate play id {}", play.id));
            }
        }
        for system in self.systems.values() {
            for id in &system.play_ids {
                if !seen.contains_key(id) {
                    return Err(format!(
                        "system {} references unknown play {id}",
                        system.id
                    ));
                }
            }
        }
        for play in &self.plays {
            let hosts: Vec<&TacticalSetSpec> = self
                .systems
                .values()
                .filter(|system| system.play_ids.iter().any(|id| *id == play.id))
                .collect();
            if hosts.is_empty() {
                return Err(format!("play {} is not referenced by any system", play.id));
            }
            for rule in &play.rules {
                for system in &hosts {
                    if !system.slots.iter().any(|slot| slot.id == rule.then.slot) {
                        return Err(format!(
                            "play {} rule {} targets slot {} which system {} does not declare",
                            play.id, rule.id, rule.then.slot, system.id
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn play(&self, id: &str) -> Option<&PlaySpec> {
        self.plays.iter().find(|play| play.id == id)
    }
}

/// 当前生效的选板窗口。
#[derive(Debug, Clone, Copy)]
pub struct PlayWindow {
    pub play: usize,
    pub ends_at: f64,
}

/// 谓词求值。词汇表外的谓词在解析期就被拒绝，这里只需覆盖全部变体。
pub fn trigger_matches(world: &World, predicates: &[PlayPredicate]) -> bool {
    predicates.iter().all(|predicate| match predicate {
        PlayPredicate::CarrierPossed => world.holder().is_some(),
        PlayPredicate::HalfcourtPossession => {
            world.possession.kind == PossessionKind::Live
                && world.now() - world.possession.started_at
                    >= world.params.policy.g("transition_window_seconds") as f64
        }
        PlayPredicate::TransitionPossession => {
            world.possession.kind == PossessionKind::Live
                && world.now() - world.possession.started_at
                    < world.params.policy.g("transition_window_seconds") as f64
        }
        PlayPredicate::ShotClockUrgent { threshold_s } => {
            world.clock.shot_clock <= *threshold_s as f64
        }
        PlayPredicate::BeyondCircleFt { r } => world
            .holder()
            .map(|handle| {
                (world.players[handle].pos - world.hoop_for(world.possession.team)).length()
                    > *r
            })
            .unwrap_or(false),
        PlayPredicate::ScreenEstablished => world
            .possession
            .last_screen
            .map(|read| world.now() - read.at < world.params.policy.g("screen_read_window_seconds") as f64)
            .unwrap_or(false),
        PlayPredicate::CornerOccupied { side } => {
            let hoop = world.hoop_for(world.possession.team);
            let reach = world.rules.league.three_point_distance_ft;
            world
                .on_court(world.possession.team)
                .as_slice()
                .iter()
                .any(|handle| {
                    let pos = world.players[*handle].pos;
                    let side_matches = match side {
                        PlayCourtSide::Left => pos.y < hoop.y,
                        PlayCourtSide::Right => pos.y > hoop.y,
                    };
                    side_matches && (pos - hoop).length() <= reach
                })
        }
        PlayPredicate::HelpShadingOff | PlayPredicate::WeakSideVacated => {
            let hoop = world.hoop_for(world.possession.team);
            let radius = world.rules.tactics.defense.potential_field.rim_help_radius_ft;
            world
                .on_court(world.possession.team.other())
                .as_slice()
                .iter()
                .filter(|handle| Some(**handle) != world.possession.matchup[0].into())
                .any(|handle| (world.players[*handle].pos - hoop).length() < radius)
        }
    })
}

/// 推进选板窗口：到期关闭，空窗时尝试激活。
pub fn update_play_window(world: &mut World) {
    if let Some(window) = world.possession.play {
        if world.now() >= window.ends_at {
            let id = world.params.tactics.plays[window.play].id.clone();
            world.possession.play = None;
            world.emit(Fact::PlayWindowEnded {
                play: id,
                possession: world.possession.id,
            });
        }
        return;
    }
    if world.possession.kind != PossessionKind::Live {
        return;
    }
    if world.possession.play_called {
        return;
    }
    for (index, play) in world.params.tactics.plays.iter().enumerate() {
        let ready = world
            .play_cooldowns
            .get(&index)
            .map(|ready_at| world.now() >= *ready_at)
            .unwrap_or(true);
        if !ready {
            continue;
        }
        for trigger in &play.triggers {
            if !trigger_matches(world, &trigger.when) {
                continue;
            }
            // 叫战术是教练组的取舍：不是每个阵地回合都跑套路，剩下的回合打自由进攻。
            let call_rate = world.params.policy.w("play_call_rate");
            if world.rng.gen::<f32>() >= call_rate {
                world.possession.play_called = true;
                return;
            }
            let ends_at = world.now() + trigger.window_seconds as f64;
            world.possession.play = Some(PlayWindow {
                play: index,
                ends_at,
            });
            world.possession.play_called = true;
            world
                .play_cooldowns
                .insert(index, ends_at + trigger.cooldown_seconds as f64);
            let id = play.id.clone();
            world.emit(Fact::PlayActivated {
                play: id,
                possession: world.possession.id,
            });
            return;
        }
    }
}

/// 回合结束时关闭未到期的窗口，保证激活与结束一一对应。
pub fn close_play_window(world: &mut World) {
    if let Some(window) = world.possession.play {
        let id = world.params.tactics.plays[window.play].id.clone();
        world.possession.play = None;
        world.emit(Fact::PlayWindowEnded {
            play: id,
            possession: world.possession.id,
        });
    }
}

/// 当前窗口对应的剧本，未激活时为 `None`。
pub fn active_play<'a>(world: &'a World) -> Option<&'a PlaySpec> {
    world
        .possession
        .play
        .map(|window| &world.params.tactics.plays[window.play])
}

/// 把档案族映射到 `ActionKind::family()` 的编码。
///
/// `TripleThreatJab` 在动作链里没有对应动作，返回 `None` 表示该声明不生效。
fn family_code(family: DecisionActionFamily) -> Option<usize> {
    match family {
        DecisionActionFamily::Shoot => Some(0),
        DecisionActionFamily::Drive => Some(1),
        DecisionActionFamily::Pass => Some(2),
        DecisionActionFamily::PostUp => Some(5),
        DecisionActionFamily::Dwell => Some(6),
        DecisionActionFamily::TripleThreatJab => None,
    }
}

/// 把选板档案的抑制与偏好应用到候选集。
///
/// 硬抑制把整个族从候选集里移除，软抑制扣分，持球人偏好加分；
/// 规则级偏好只在规则的 `when` 全真时生效。
pub fn apply_play_effects(
    world: &World,
    candidates: &mut Vec<(crate::action::ActionKind, f32, Option<usize>, glam::Vec2)>,
) {
    let Some(play) = active_play(world) else {
        return;
    };
    let mut bonus = [f32::default(); 9];
    let mut hard = [false; 9];
    for inhibition in &play.inhibitions {
        let Some(code) = family_code(inhibition.action_family) else {
            continue;
        };
        match inhibition.mode {
            PlayInhibitionMode::Hard => hard[code] = true,
            PlayInhibitionMode::Soft { penalty } => bonus[code] -= penalty,
        }
    }
    for preference in &play.carrier_preferences {
        if let Some(code) = family_code(preference.action_family) {
            bonus[code] += preference.bonus;
        }
    }
    for rule in &play.rules {
        if !trigger_matches(world, &rule.when) {
            continue;
        }
        for preference in &rule.carrier_preferences {
            if let Some(code) = family_code(preference.action_family) {
                bonus[code] += preference.bonus;
            }
        }
    }
    candidates.retain(|(kind, _, _, _)| !hard[kind.family() as usize]);
    assert!(
        !candidates.is_empty(),
        "选板的硬抑制不能清空候选集：{}",
        play.id
    );
    for (kind, score, _, _) in candidates.iter_mut() {
        *score = (*score + bonus[kind.family() as usize]).max(f32::MIN_POSITIVE);
    }
}
