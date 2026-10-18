//! 世界状态与决策视图。
//!
//! 决策层只读 [`DecisionState`]；它由世界状态派生，不含动作历史。这样
//! 「掩护是否造成错位」「突破是否过掉人」必须以状态字段的形式存在，否则
//! 下一次决策看不到它们。

use glam::Vec2;
use nba_domain::PlayerData;

use crate::action::DefenderRoute;
pub use crate::action::TeamSide;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Handler,
    Wing,
    Corner,
    Top,
    Post,
    DunkerSpot,
}

impl Slot {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Handler => "handler",
            Self::Wing => "wing",
            Self::Corner => "corner",
            Self::Top => "top",
            Self::Post => "post",
            Self::DunkerSpot => "dunker",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PlayerState {
    pub id: String,
    pub name: String,
    pub jersey: String,
    pub team: TeamSide,
    pub data: PlayerData,
    pub pos: Vec2,
    pub vel: Vec2,
    pub facing: Vec2,
    pub on_court: bool,
    pub stamina: f32,
    pub fouls: u8,
    pub slot: Slot,
    pub target: Vec2,
    pub home_spot: Vec2,
}

impl PlayerState {
    pub fn new(data: PlayerData, team: TeamSide, slot: Slot, pos: Vec2) -> Self {
        Self {
            id: data.id.clone(),
            name: data.name.clone(),
            jersey: data.jersey.clone(),
            team,
            data,
            pos,
            vel: Vec2::ZERO,
            facing: Vec2::X,
            on_court: false,
            stamina: 1.0,
            fouls: 0,
            slot,
            target: pos,
            home_spot: pos,
        }
    }

    pub fn attribute(&self, name: &str) -> f32 {
        let a = &self.data.attributes;
        match name {
            "speed" => a.speed,
            "acceleration" => a.acceleration,
            "agility" => a.agility,
            "strength" => a.strength,
            "vertical" => a.vertical,
            "stamina" => a.stamina,
            "ball_handling" => a.ball_handling,
            "passing" => a.passing,
            "shooting_close" => a.shooting_close,
            "shooting_near" => a.shooting_near,
            "shooting_mid" => a.shooting_mid,
            "shooting_three" => a.shooting_three,
            "free_throw" => a.free_throw,
            "finishing" => a.finishing,
            "defense_perimeter" => a.defense_perimeter,
            "defense_interior" => a.defense_interior,
            "steal" => a.steal,
            "block" => a.block,
            "offensive_rebound" => a.offensive_rebound,
            "defensive_rebound" => a.defensive_rebound,
            "decision_iq" => a.decision_iq,
            "off_ball_sense" => a.off_ball_sense,
            other => panic!("unknown attribute `{other}`"),
        }
    }

    pub fn shooting_attribute(&self, zone: &str) -> f32 {
        match zone {
            "rim" => self.data.attributes.shooting_close,
            "near" => self.data.attributes.shooting_near,
            "mid" => self.data.attributes.shooting_mid,
            "three" => self.data.attributes.shooting_three,
            other => panic!("unknown zone `{other}`"),
        }
    }

    pub fn effective_speed(&self) -> f32 {
        let base = 14.0 + 8.0 * self.data.attributes.speed;
        (base * (0.85 + 0.15 * self.stamina)).min(22.0)
    }

    /// 加速度能力决定多快到达目标速度，直接影响推进与回合时长。
    pub fn effective_acceleration(&self) -> f32 {
        (18.0 + 22.0 * self.data.attributes.acceleration).min(40.0)
    }
}

#[derive(Debug, Clone)]
pub struct Clock {
    pub period: u32,
    pub game_clock: f64,
    pub shot_clock: f64,
    pub elapsed: f64,
    pub tick: u64,
    pub period_seconds: f64,
}

impl Clock {
    pub fn new(period_seconds: f64, shot_clock: f64) -> Self {
        Self {
            period: 1,
            game_clock: period_seconds,
            shot_clock,
            elapsed: 0.0,
            tick: 0,
            period_seconds,
        }
    }
}

#[derive(Debug, Clone)]
pub enum BallState {
    Held(usize),
    Pass {
        from: usize,
        to: usize,
        from_pos: Vec2,
        to_pos: Vec2,
        start: f64,
        duration: f64,
        completed: bool,
    },
    Shot {
        shooter: usize,
        from_pos: Vec2,
        to_pos: Vec2,
        start: f64,
        duration: f64,
        made: bool,
    },
    Loose {
        pos: Vec2,
        vel: Vec2,
    },
}

#[derive(Debug, Clone)]
pub enum Phase {
    Inbound {
        team: TeamSide,
        ready_at: f64,
    },
    Live,
    PassFlight {
        resolve_at: f64,
    },
    ShotFlight {
        resolve_at: f64,
    },
    FreeThrows {
        shooter: usize,
        total: u8,
        remaining: u8,
        made: u8,
        next_at: f64,
        and_one: bool,
    },
    DeadBall {
        team: TeamSide,
        ready_at: f64,
    },
    PeriodBreak {
        resume_at: f64,
    },
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PossessionKind {
    Dead,
    Live,
}

#[derive(Debug, Clone)]
pub struct PossessionState {
    pub team: TeamSide,
    pub id: u64,
    pub kind: PossessionKind,
    pub started_at: f64,
    pub passes: u32,
    pub screens: u32,
    pub drives: u32,
    pub had_shot: bool,
    pub had_offensive_rebound: bool,
    pub last_pass_at: Option<f64>,
    pub last_pass_from: Option<usize>,
    pub assist_candidate: Option<(usize, f64)>,
    pub shot_clock_start: f64,
    pub last_cut_at: Option<f64>,
    pub last_offensive_rebound_at: Option<f64>,
    pub last_drive_at: Option<f64>,
    pub last_post_at: Option<f64>,
    pub last_drive_actor: Option<usize>,
    pub last_post_actor: Option<usize>,
    pub last_cut_actor: Option<usize>,
}

impl PossessionState {
    pub fn new(team: TeamSide, id: u64, kind: PossessionKind, now: f64, shot_clock: f64) -> Self {
        Self {
            team,
            id,
            kind,
            started_at: now,
            passes: 0,
            screens: 0,
            drives: 0,
            had_shot: false,
            had_offensive_rebound: false,
            last_pass_at: None,
            last_pass_from: None,
            assist_candidate: None,
            shot_clock_start: shot_clock,
            last_cut_at: None,
            last_offensive_rebound_at: None,
            last_drive_at: None,
            last_post_at: None,
            last_drive_actor: None,
            last_post_actor: None,
            last_cut_actor: None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct DefenderView {
    pub index: usize,
    pub distance_ft: f32,
    pub closing_speed_ftps: f32,
    pub between_ball_and_hoop: bool,
    pub beaten: bool,
    pub route: Option<DefenderRoute>,
}

#[derive(Debug, Clone, Copy)]
pub struct HelpView {
    pub distance_ft: f32,
    pub arrival_seconds: f32,
}

#[derive(Debug, Clone, Copy)]
pub struct TeammateView {
    pub index: usize,
    pub pos: Vec2,
    pub openness: f32,
    pub defender_distance_ft: f32,
    pub lane_clearance: f32,
    pub distance_to_hoop: f32,
}

#[derive(Debug, Clone)]
pub struct DecisionState {
    pub actor: usize,
    pub actor_pos: Vec2,
    pub hoop: Vec2,
    pub attacking_right: bool,
    pub clock: f64,
    pub shot_clock: f64,
    pub margin: i32,
    pub period: u32,
    pub primary_defender: Option<DefenderView>,
    pub help: Option<HelpView>,
    pub teammates: Vec<TeammateView>,
    pub lane_clearance: f32,
    pub mismatch: f32,
    pub stamina: f32,
    pub possession_kind: PossessionKind,
    pub elapsed_in_possession: f64,
}

impl DecisionState {
    pub fn distance_to_hoop(&self) -> f32 {
        (self.actor_pos - self.hoop).length()
    }

    pub fn openness(&self) -> f32 {
        match self.primary_defender {
            Some(defender) => contest_from_defender(defender.distance_ft, defender.between_ball_and_hoop),
            None => 1.0,
        }
    }

    pub fn shot_clock_pressure(&self) -> f32 {
        (((12.0 - self.shot_clock) / 12.0) as f32).clamp(0.0, 1.0)
    }

    pub fn early_clock_slack(&self) -> f32 {
        (((self.shot_clock - 12.0) / 12.0) as f32).clamp(0.0, 1.0)
    }
}

/// 干扰强度：防守人越近、越处在球与篮筐之间，干扰越高。
pub fn contest_from_defender(distance_ft: f32, between: bool) -> f32 {
    let base = ((6.0 - distance_ft) / 5.0).clamp(0.0, 1.0);
    if between {
        base
    } else {
        base * 0.55
    }
}

pub fn nearest_opponent<'a>(
    players: &'a [PlayerState],
    actor: usize,
) -> Option<&'a PlayerState> {
    let team = players[actor].team;
    let pos = players[actor].pos;
    players
        .iter()
        .filter(|p| p.on_court && p.team != team)
        .min_by(|a, b| {
            let da = (a.pos - pos).length_squared();
            let db = (b.pos - pos).length_squared();
            da.partial_cmp(&db).expect("distances are finite")
        })
}

pub fn distance_between(players: &[PlayerState], a: usize, b: usize) -> f32 {
    (players[a].pos - players[b].pos).length()
}

pub fn is_between(point: Vec2, a: Vec2, b: Vec2) -> bool {
    let ab = b - a;
    let length = ab.length();
    if length < 1e-4 {
        return false;
    }
    let dir = ab / length;
    let ap = point - a;
    let along = ap.dot(dir);
    if along <= 0.0 || along >= length {
        return false;
    }
    let lateral = (ap - dir * along).length();
    lateral < 2.2
}

/// 传球线路净空：线路上防守人离传球直线的最小距离。
pub fn lane_clearance(players: &[PlayerState], from: Vec2, to: Vec2, offense: TeamSide) -> f32 {
    let segment = to - from;
    let length = segment.length();
    if length < 1e-4 {
        return 99.0;
    }
    let dir = segment / length;
    let mut minimum = 99.0_f32;
    for player in players.iter().filter(|p| p.on_court && p.team != offense) {
        let relative = player.pos - from;
        let along = relative.dot(dir).clamp(0.0, length);
        let lateral = (relative - dir * along).length();
        minimum = minimum.min(lateral);
    }
    minimum
}
