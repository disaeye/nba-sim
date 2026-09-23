use glam::Vec2;
use nba_decision::tactics::DefensiveTactic;
use nba_domain::{GameRules, PlaySpec, TacticalSetSpec, TeamData};
use nba_physics::PhysicsBackend;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// A complete, serializable pre-game lineup selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineupConfig {
    pub starters: [String; 5],
    #[serde(default)]
    pub bench: Vec<String>,
    pub offense_tactic: String,
    pub defense_tactic: String,
}

/// Immutable inputs used to create an authoritative match state.
///
/// The engine keeps the setup alongside the running state so replay and UI
/// clients can identify the roster and policies that produced a stream.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchSetup {
    pub home_team: TeamData,
    pub away_team: TeamData,
    pub home_lineup: LineupConfig,
    pub away_lineup: LineupConfig,
    #[serde(default)]
    pub rules: GameRules,
    #[serde(default)]
    pub physics_backend: PhysicsBackend,
    #[serde(default)]
    pub home_playbook: Vec<PlaySpec>,
    #[serde(default)]
    pub away_playbook: Vec<PlaySpec>,
}
impl MatchSetup {
    pub fn builtin(rules: GameRules) -> Self {
        let [home_team, away_team] = TeamData::builtin_pair_with_geometry(rules.court);
        let home_lineup = default_lineup(&home_team);
        let away_lineup = default_lineup(&away_team);
        let home_spec = TacticalSetSpec::builtin(&home_lineup.offense_tactic)
            .expect("内建主队进攻战术必须存在");
        let away_spec = TacticalSetSpec::builtin(&away_lineup.offense_tactic)
            .expect("内建客队进攻战术必须存在");
        let plays = builtin_play_specs();
        let home_playbook = select_compatible_plays(&plays, &home_spec);
        let away_playbook = select_compatible_plays(&plays, &away_spec);
        Self {
            home_team,
            away_team,
            home_lineup,
            away_lineup,
            rules,
            physics_backend: PhysicsBackend::Rapier,
            home_playbook,
            away_playbook,
        }
    }

    /// Validates all cross-object references before any runtime body is built.
    pub fn validate(&self) -> Result<(), String> {
        self.rules.validate()?;
        if self.home_team.id.is_empty()
            || self.away_team.id.is_empty()
            || self.home_team.id == self.away_team.id
        {
            return Err("match teams must have distinct non-empty ids".to_string());
        }
        validate_team(&self.home_team, self.rules.court)?;
        validate_team(&self.away_team, self.rules.court)?;
        validate_lineup(&self.home_team, &self.home_lineup, "home")?;
        validate_lineup(&self.away_team, &self.away_lineup, "away")?;
        let home_spec =
            TacticalSetSpec::builtin(&self.home_lineup.offense_tactic).ok_or_else(|| {
                format!(
                    "home lineup has unknown offense tactic profile: {}",
                    self.home_lineup.offense_tactic
                )
            })?;
        let away_spec =
            TacticalSetSpec::builtin(&self.away_lineup.offense_tactic).ok_or_else(|| {
                format!(
                    "away lineup has unknown offense tactic profile: {}",
                    self.away_lineup.offense_tactic
                )
            })?;
        validate_playbook(&self.home_playbook, &home_spec, "home")?;
        validate_playbook(&self.away_playbook, &away_spec, "away")?;
        DefensiveTactic::from_id(&self.home_lineup.defense_tactic).ok_or_else(|| {
            format!(
                "home lineup has unknown defense tactic: {}",
                self.home_lineup.defense_tactic
            )
        })?;
        DefensiveTactic::from_id(&self.away_lineup.defense_tactic).ok_or_else(|| {
            format!(
                "away lineup has unknown defense tactic: {}",
                self.away_lineup.defense_tactic
            )
        })?;
        Ok(())
    }
}

fn builtin_play_specs() -> Vec<PlaySpec> {
    const PLAY_JSON: [&str; 3] = [
        include_str!("../../../data/tactics/plays/weak_side_lift_v1.json"),
        include_str!("../../../data/tactics/plays/high_pnr_roll_v1.json"),
        include_str!("../../../data/tactics/plays/corner_backdoor_v1.json"),
    ];
    PLAY_JSON
        .into_iter()
        .map(|json| PlaySpec::from_json(json).expect("内建 PlaySpec 必须合法"))
        .collect()
}

fn select_compatible_plays(plays: &[PlaySpec], tactic: &TacticalSetSpec) -> Vec<PlaySpec> {
    let slots: HashSet<&str> = tactic.slots.iter().map(|slot| slot.id.as_str()).collect();
    plays
        .iter()
        .filter(|play| {
            play.rules
                .iter()
                .all(|rule| slots.contains(rule.then.slot.as_str()))
        })
        .cloned()
        .collect()
}

fn validate_playbook(
    plays: &[PlaySpec],
    tactic: &TacticalSetSpec,
    side: &str,
) -> Result<(), String> {
    let slots: HashSet<&str> = tactic.slots.iter().map(|slot| slot.id.as_str()).collect();
    let mut play_ids = HashSet::new();
    for play in plays {
        play.validate()
            .map_err(|error| format!("{side} play `{}` is invalid: {error}", play.id))?;
        if !play_ids.insert(play.id.as_str()) {
            return Err(format!(
                "{side} playbook has duplicate play id `{}`",
                play.id
            ));
        }
        for rule in &play.rules {
            if !slots.contains(rule.then.slot.as_str()) {
                return Err(format!(
                    "{side} play `{}` rule `{}` references unknown slot `{}`",
                    play.id, rule.id, rule.then.slot
                ));
            }
        }
    }
    Ok(())
}

fn default_lineup(team: &TeamData) -> LineupConfig {
    // 首发由档案的 `starter` 标记决定，**不取数组前 5 个**（round-10 Step4a）。
    // 后者使「顺序即身份」：轮转名册数组就会改变首发阵容。
    let mut starters: Vec<String> = team
        .players
        .iter()
        .filter(|p| p.starter)
        .map(|p| p.id.clone())
        .collect();
    // 兜底：档案未标记任何首发时，按 id 字典序取 5 人（确定性，
    // 且不依赖数组顺序），而不是取前 5。
    if starters.is_empty() {
        let mut ids: Vec<String> = team.players.iter().map(|p| p.id.clone()).collect();
        ids.sort();
        starters = ids.into_iter().take(5).collect();
    }
    let starters: [String; 5] =
        std::array::from_fn(|i| starters.get(i).cloned().unwrap_or_default());
    let starter_set: std::collections::HashSet<&String> = starters.iter().collect();
    let bench = team
        .players
        .iter()
        .map(|p| p.id.clone())
        .filter(|id| !starter_set.contains(id))
        .collect();
    LineupConfig {
        starters,
        bench,
        offense_tactic: team.default_offense_tactic.clone(),
        defense_tactic: team.default_defense_tactic.clone(),
    }
}

fn validate_team(team: &TeamData, geometry: nba_domain::CourtGeometry) -> Result<(), String> {
    if team.id.trim().is_empty() || team.name.trim().is_empty() {
        return Err(format!(
            "team {} must have non-empty identity fields",
            team.id
        ));
    }
    team.team_traits.validate()?;
    let mut ids = HashSet::new();
    for player in &team.players {
        if player.id.is_empty() || !ids.insert(player.id.clone()) {
            return Err(format!(
                "team {} contains a duplicate or empty player id",
                team.id
            ));
        }
        if player.team_id != team.id {
            return Err(format!(
                "player {} does not belong to team {}",
                player.id, team.id
            ));
        }
        player
            .attributes
            .validate()
            .map_err(|error| format!("player {} has invalid attributes: {}", player.id, error))?;
        player
            .tendencies
            .validate()
            .map_err(|error| format!("player {} has invalid tendencies: {}", player.id, error))?;
        if player.name.trim().is_empty() || player.jersey.trim().is_empty() {
            return Err(format!(
                "player {} must have non-empty display data",
                player.id
            ));
        }
        let (x, y) = player.initial_position_ft;
        if !x.is_finite() || !y.is_finite() {
            return Err(format!(
                "player {} has a non-finite initial position",
                player.id
            ));
        }
        // 首发必须在球场界内，替补允许位于界外替补席区域。
        //
        // round-10 Step4a：改用档案的 `starter` 标记，而**不是**数组位置。
        // 原实现 `ids.len() <= 5` 有两重问题：
        //   (a) `ids` 是用于查重的 HashSet，其 len 是"已插入数量"而非索引，
        //       语义上完全不是"前 5 个"；
        //   (b) 即便改成索引，也仍是"顺序即身份"——轮转名册数组就会把
        //       替补当成首发去校验场地边界。
        let is_starter = player.starter;
        let tolerance = 0.0;
        if is_starter && !geometry.contains(Vec2::new(x, y), tolerance) {
            return Err(format!(
                "starter {} starts outside the court geometry",
                player.id
            ));
        }
    }
    if team.players.len() < 5 {
        return Err(format!(
            "team {} must provide at least five players",
            team.id
        ));
    }
    Ok(())
}

fn validate_lineup(team: &TeamData, lineup: &LineupConfig, side: &str) -> Result<(), String> {
    if lineup.starters.iter().any(String::is_empty) {
        return Err(format!("{side} lineup must contain five starters"));
    }
    if lineup.offense_tactic.trim().is_empty() || lineup.defense_tactic.trim().is_empty() {
        return Err(format!(
            "{side} lineup must select offense and defense tactics"
        ));
    }
    let roster: HashSet<&str> = team
        .players
        .iter()
        .map(|player| player.id.as_str())
        .collect();
    let mut selected = HashSet::new();
    for starter in &lineup.starters {
        if !roster.contains(starter.as_str()) || !selected.insert(starter.as_str()) {
            return Err(format!(
                "{side} lineup contains an invalid or duplicate starter"
            ));
        }
    }
    for bench in &lineup.bench {
        if !roster.contains(bench.as_str()) || !selected.insert(bench.as_str()) {
            return Err(format!(
                "{side} lineup contains an invalid or duplicate bench player"
            ));
        }
    }
    Ok(())
}
