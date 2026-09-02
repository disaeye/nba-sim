use glam::Vec2;
use nba_decision::tactics::{DefensiveTactic, TacticalSet};
use nba_domain::{GameRules, TeamData};
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
}
impl MatchSetup {
    pub fn builtin(rules: GameRules) -> Self {
        let [home_team, away_team] = TeamData::builtin_pair_with_geometry(rules.court);
        let home_lineup = default_lineup(&home_team);
        let away_lineup = default_lineup(&away_team);
        Self {
            home_team,
            away_team,
            home_lineup,
            away_lineup,
            rules,
            physics_backend: PhysicsBackend::Rapier,
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
        TacticalSet::from_id(&self.home_lineup.offense_tactic).ok_or_else(|| {
            format!(
                "home lineup has unknown offense tactic: {}",
                self.home_lineup.offense_tactic
            )
        })?;
        TacticalSet::from_id(&self.away_lineup.offense_tactic).ok_or_else(|| {
            format!(
                "away lineup has unknown offense tactic: {}",
                self.away_lineup.offense_tactic
            )
        })?;
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

fn default_lineup(team: &TeamData) -> LineupConfig {
    let mut ids = team.players.iter().map(|player| player.id.clone());
    let starters = std::array::from_fn(|_| ids.next().unwrap_or_default());
    let bench = ids.collect();
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
        // 首发 5 人必须在球场界内，替补球员允许位于界外替补席区域
        let is_starter = ids.len() <= 5;
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
