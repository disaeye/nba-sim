//! 球员级箱体：由动作链直接累加，不做事后推断。
//!
//! 助攻由「传球 → 接球人命中」的因果链产生：传球时记录候选，接球人在
//! 助攻窗口内完成命中则记给传球人。

use crate::action::TeamSide;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct PlayerLine {
    pub id: String,
    pub name: String,
    pub jersey: String,
    pub team: Option<TeamSide>,
    pub seconds: f64,
    pub points: u32,
    pub fgm: u32,
    pub fga: u32,
    pub tpm: u32,
    pub tpa: u32,
    pub ftm: u32,
    pub fta: u32,
    pub oreb: u32,
    pub dreb: u32,
    pub assists: u32,
    pub steals: u32,
    pub blocks: u32,
    pub turnovers: u32,
    pub fouls: u32,
    pub plus_minus: i32,
    pub touches: u32,
}

impl PlayerLine {
    pub fn new(id: &str, name: &str, jersey: &str, team: TeamSide) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            jersey: jersey.to_string(),
            team: Some(team),
            ..Default::default()
        }
    }

    pub fn minutes(&self) -> f64 {
        self.seconds / 60.0
    }

    pub fn fg_pct(&self) -> f32 {
        if self.fga == 0 {
            0.0
        } else {
            self.fgm as f32 / self.fga as f32
        }
    }

    pub fn tp_pct(&self) -> f32 {
        if self.tpa == 0 {
            0.0
        } else {
            self.tpm as f32 / self.tpa as f32
        }
    }

    pub fn ft_pct(&self) -> f32 {
        if self.fta == 0 {
            0.0
        } else {
            self.ftm as f32 / self.fta as f32
        }
    }

    pub fn rebounds(&self) -> u32 {
        self.oreb + self.dreb
    }

    pub fn true_shooting(&self) -> f32 {
        let denominator = 2.0 * (self.fga as f32 + 0.44 * self.fta as f32);
        if denominator == 0.0 {
            0.0
        } else {
            self.points as f32 / denominator
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TeamLine {
    pub points: u32,
    pub fgm: u32,
    pub fga: u32,
    pub tpm: u32,
    pub tpa: u32,
    pub ftm: u32,
    pub fta: u32,
    pub oreb: u32,
    pub dreb: u32,
    pub assists: u32,
    pub steals: u32,
    pub blocks: u32,
    pub turnovers: u32,
    pub fouls: u32,
}

impl TeamLine {
    pub fn add(&mut self, line: &PlayerLine) {
        self.points += line.points;
        self.fgm += line.fgm;
        self.fga += line.fga;
        self.tpm += line.tpm;
        self.tpa += line.tpa;
        self.ftm += line.ftm;
        self.fta += line.fta;
        self.oreb += line.oreb;
        self.dreb += line.dreb;
        self.assists += line.assists;
        self.steals += line.steals;
        self.blocks += line.blocks;
        self.turnovers += line.turnovers;
        self.fouls += line.fouls;
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct BoxScore {
    pub home: Vec<PlayerLine>,
    pub away: Vec<PlayerLine>,
}

impl BoxScore {
    pub fn team_totals(&self, team: TeamSide) -> TeamLine {
        let lines = match team {
            TeamSide::Home => &self.home,
            TeamSide::Away => &self.away,
        };
        let mut totals = TeamLine::default();
        for line in lines {
            totals.add(line);
        }
        totals
    }
}
