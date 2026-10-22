//! 世界状态与决策视图。
//!
//! 决策层只读 [`DecisionState`]；它由世界状态派生，不含动作历史。这样
//! 「掩护是否造成错位」「突破是否过掉人」必须以状态字段的形式存在，否则
//! 下一次决策看不到它们。

use glam::Vec2;
use nba_domain::PlayerData;

use crate::action::DefenderRoute;
pub use crate::action::TeamSide;
use crate::params::PolicyTable;

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
    /// 犯满离场：置位后不再上场。
    pub disqualified: bool,
    pub slot: Slot,
    pub target: Vec2,
    pub home_spot: Vec2,
    /// 本次上场的时刻：两次换人之间必须隔够 `min_rest_seconds`。
    pub entered_at: f64,
    /// 下场后到什么时候才能再上：休息不满的人不能换回来。
    pub resting_until: f64,
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
            disqualified: false,
            slot,
            target: pos,
            home_spot: pos,
            entered_at: f64::default(),
            resting_until: f64::default(),
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

    /// 移动速度：属性给基础值，体能按数据表里的地板与跨度缩放。
    pub fn effective_speed(&self, policy: &PolicyTable) -> f32 {
        crate::physical::Physical::of(self, policy).speed_ftps
    }

    /// 加速度能力决定多快到达目标速度，直接影响推进与回合时长。
    pub fn effective_acceleration(&self, policy: &PolicyTable) -> f32 {
        crate::physical::Physical::of(self, policy).accel_ftps2
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
        /// 最后一次罚球后球权是否仍属罚球方（最后时段的无球犯规是一罚一掷）。
        retain_possession: bool,
    },
    DeadBall {
        team: TeamSide,
        ready_at: f64,
    },
    /// 暂停：比赛钟与进攻钟都停，暂停结束时叫暂停方继续持有球权。
    Timeout {
        team: TeamSide,
        resume_at: f64,
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

/// 持球状态：运球中或已收球。收球后只允许终结（投篮/传球）或两步
/// 内的调整；再运球是二次运球，收球后超步幅位移是走步。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DribbleState {
    #[default]
    Dribbling,
    Gathered,
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
    /// 回合开始时的比赛钟读数。
    pub start_game_clock: f64,
    pub last_cut_at: Option<f64>,
    pub last_offensive_rebound_at: Option<f64>,
    pub last_drive_at: Option<f64>,
    pub last_post_at: Option<f64>,
    pub last_drive_actor: Option<usize>,
    pub last_post_actor: Option<usize>,
    pub last_cut_actor: Option<usize>,
    /// 回合开始到首个动作起手的秒数。
    pub first_action_seconds: Option<f64>,
    /// 持球状态与收球位置。
    pub dribble_state: DribbleState,
    pub gather_pos: Option<Vec2>,
    /// 最近一次掩护结算出的防守人路线，供下一次决策读取。
    pub last_screen: Option<ScreenRead>,
    /// 对位表：`matchup[防守人在场序号] = 所对位的进攻人 handle`，未分配时为 `usize::MAX`。
    /// 只在掩护换防或突破得手时重算，其余时间保持不变。
    pub matchup: [usize; MAX_ON_COURT],
    /// 当前生效的选板窗口，未激活时为 `None`。
    pub play: Option<crate::tactics::PlayWindow>,
    /// 本回合是否已经叫过战术：每个回合只叫一次，之后进自由进攻。
    pub play_called: bool,
}

impl PossessionState {
    pub fn new(
        team: TeamSide,
        id: u64,
        kind: PossessionKind,
        now: f64,
        shot_clock: f64,
        game_clock: f64,
    ) -> Self {
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
            start_game_clock: game_clock,
            last_cut_at: None,
            last_offensive_rebound_at: None,
            last_drive_at: None,
            last_post_at: None,
            last_drive_actor: None,
            last_post_actor: None,
            last_cut_actor: None,
            first_action_seconds: None,
            dribble_state: DribbleState::Dribbling,
            gather_pos: None,
            last_screen: None,
            matchup: [usize::MAX; MAX_ON_COURT],
            play: None,
            play_called: false,
        }
    }
}

/// 掩护结算出的防守人路线及其有效期。
///
/// 决策层看不到这个结构就直接是战术盲人：防守人钻掩护下沿还是换防，
/// 决定了持球人该干拔还是突破。
#[derive(Debug, Clone, Copy)]
pub struct ScreenRead {
    pub route: DefenderRoute,
    pub at: f64,
    pub screener: usize,
    pub separation_ft: f32,
}

/// 逐次出手的过程观测。
#[derive(Debug, Clone, Copy)]
pub struct ShotSample {
    pub shot_clock: f64,
    /// 出手时的比赛时间与节次：分层诊断需要按末段窗口切样本。
    pub game_clock: f64,
    pub period: u32,
    pub contest: f32,
    /// 出手瞬间最近防守人的距离（英尺）。
    pub defender_ft: f32,
    pub zone: u8,
    /// 模型对这次出手的命中概率（含封盖与犯规的边际）。
    pub p_make: f32,
    /// 这次出手是否被吹犯规、是否被封盖（用于核对边际守恒）。
    pub fouled: bool,
    pub blocked: bool,
    /// 抽样时实际使用的三个概率：犯规、封盖、条件化后的命中率。
    pub p_foul: f32,
    pub p_block: f32,
    pub p_make_cond: f32,
    pub made: bool,
    pub points: u8,
}

/// 逐次传球的观测，用于乒乓传球与传导质量判定。
#[derive(Debug, Clone, Copy)]
pub struct PassSample {
    pub possession: u64,
    pub from: usize,
    pub to: usize,
    pub from_pos: Vec2,
    pub to_pos: Vec2,
    pub at: f64,
}

/// 单个候选动作的审计记录。
#[derive(Debug, Clone, Copy)]
pub struct CandidateTrace {
    pub kind: crate::action::ActionKind,
    pub target: Option<usize>,
    pub to_ft: Vec2,
    pub utility: crate::utility::Utility,
    /// 带 γ 的总效用。
    pub total: f32,
    /// 玻尔兹曼概率（全部候选合计为 1）。
    pub probability: f32,
}

/// 一次决策的因果快照：候选集、各自的效用分解与概率。
#[derive(Debug, Clone)]
pub struct DecisionTraceSample {
    pub actor: usize,
    pub beta: f32,
    pub gamma: f32,
    /// 按概率降序排列的全部候选。
    pub candidates: Vec<CandidateTrace>,
    /// 实际被抽中的候选下标。
    pub chosen: usize,
}

impl DecisionTraceSample {
    /// Top-N 候选（N 大于候选数时返回全部）。
    pub fn top(&self, count: usize) -> &[CandidateTrace] {
        &self.candidates[..count.min(self.candidates.len())]
    }
}

/// 逐次决策的观测：掩护路线、所选动作族与效用的完整分解。
#[derive(Debug, Clone, Copy)]
pub struct DecisionSample {
    pub route: Option<DefenderRoute>,
    pub family: u8,
    pub shot_clock: f64,
    /// 距最近一次掩护结算的秒数；大于一个决策步就不是「下一决策步」。
    pub since_screen: f64,
    /// 持球人的三分能力，用于限定路线响应的适用人群。
    pub shooter_three: f32,
    /// 选中动作的效用分解：期望得分的七个成因、方差、时钟成本、犯规成本、战术偏置。
    pub utility: crate::utility::Utility,
    /// 当时的逆温度 β。
    pub beta: f32,
    /// 当时的方差偏好 γ。
    pub gamma: f32,
    /// 当时的候选数量。
    pub candidates: u8,
    /// 指向 `World::decision_traces` 的审计快照下标。
    pub trace: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct DefenderView {
    pub index: usize,
    pub distance_ft: f32,
    pub closing_speed_ftps: f32,
    pub between_ball_and_hoop: bool,
    pub beaten: bool,
    pub route: Option<DefenderRoute>,
    /// 摸高超出基准的部分（英尺），与结算层同一个映射。
    pub reach_excess_ft: f32,
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

/// 比赛处境：杠杆、时间稀缺与方差偏好。
///
/// 三个量都是连续特征，没有「第四节」或「剩 N 秒」这种阶跃开关：
/// 第三节末落后 15 分与第四节剩 2 分钟落后 6 分在数学上是同一件事，
/// 只是 τ 与 ω 不同。
#[derive(Debug, Clone, Copy, Default)]
pub struct Leverage {
    /// τ = 分差 / 剩余回合数（持球方视角，落后为负）。
    pub score_pressure: f32,
    /// ω = exp(−剩余时间 / 稀缺尺度)：终场前接近 1，垃圾时间接近 0。
    pub urgency: f32,
    /// γ：落后为正（偏好高方差），领先为负（压低方差）。
    pub variance_appetite: f32,
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
    /// 持球人与首要防守人的突破技能差（未截断）。
    pub skill_edge: f32,
    pub mismatch: f32,
    pub stamina: f32,
    pub possession_kind: PossessionKind,
    pub elapsed_in_possession: f64,
    /// 本次决策时的比赛处境。
    pub leverage: Leverage,
    /// 本方当前的势头（0 = 冷却，越大越热）。
    pub momentum: f32,
}

impl DecisionState {
    pub fn distance_to_hoop(&self) -> f32 {
        (self.actor_pos - self.hoop).length()
    }

    pub fn openness(&self) -> f32 {
        match self.primary_defender {
            Some(defender) => {
                contest_from_defender(
                    defender.distance_ft,
                    defender.between_ball_and_hoop,
                    defender.reach_excess_ft,
                )
            }
            None => 1.0,
        }
    }

    pub fn shot_clock_pressure(&self) -> f32 {
        (((12.0 - self.shot_clock) / 12.0) as f32).clamp(0.0, 1.0)
    }
}

/// 干扰强度：防守人越近、越处在球与篮筐之间、摸得越高，干扰越高。
///
/// `reach_excess_ft` 来自 [`crate::physical::Physical`]：摸高超出基准的
/// 部分直接抵掉距离，所以臂展/弹跳是通过物理量而不是属性直接进公式的。
pub fn contest_from_defender(distance_ft: f32, between: bool, reach_excess_ft: f32) -> f32 {
    let gap = distance_ft - reach_excess_ft;
    let base = ((6.0 - gap) / 5.0).clamp(0.0, 1.0);
    if between {
        base
    } else {
        base * 0.55
    }
}

pub fn nearest_opponent(players: &[PlayerState], actor: usize) -> Option<&PlayerState> {
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
/// 走廊无阻挡时的返回值（英尺）：远大于球场宽度。
const NO_OBSTRUCTION_FT: f32 = 99.0;
/// 走廊长度的退化阈值（英尺）：两端重合时方向向量无定义。
const MIN_LANE_LENGTH_FT: f32 = 1e-4;

pub fn lane_clearance(players: &[PlayerState], from: Vec2, to: Vec2, offense: TeamSide) -> f32 {
    let segment = to - from;
    let length = segment.length();
    if length < MIN_LANE_LENGTH_FT {
        return NO_OBSTRUCTION_FT;
    }
    let dir = segment / length;
    let mut minimum = NO_OBSTRUCTION_FT;
    for player in players.iter().filter(|p| p.on_court && p.team != offense) {
        let relative = player.pos - from;
        let along = relative.dot(dir).clamp(0.0, length);
        let lateral = (relative - dir * along).length();
        minimum = minimum.min(lateral);
    }
    minimum
}

/// 传球走廊的空隙：只算两端之间的防守人。
///
/// 站在传球人身后或接球人身后的防守人不在走廊里，拦不到球；
/// 按纵向投影区间筛掉它们，而不是把它们按两端点投影后当成堵住走廊。
pub fn pass_lane_clearance(players: &[PlayerState], from: Vec2, to: Vec2, offense: TeamSide) -> f32 {
    let segment = to - from;
    let length = segment.length();
    if length < MIN_LANE_LENGTH_FT {
        return NO_OBSTRUCTION_FT;
    }
    let dir = segment / length;
    let mut minimum = NO_OBSTRUCTION_FT;
    for player in players.iter().filter(|p| p.on_court && p.team != offense) {
        let relative = player.pos - from;
        let along = relative.dot(dir) / length;
        if along <= f32::default() {
            continue;
        }
        let lateral = (relative - dir * (along * length)).length();
        minimum = minimum.min(lateral);
    }
    minimum
}

/// 同时在场人数上限：名册八人，场上五人。
pub const MAX_ON_COURT: usize = 5;

/// 在场球员句柄的定长集合。每 tick 的防守目标求解与箱体计时都读它，
/// 用定长数组避免逐 tick 堆分配。
#[derive(Debug, Clone, Copy)]
pub struct OnCourt {
    handles: [usize; MAX_ON_COURT],
    len: usize,
}

impl OnCourt {
    pub fn new() -> Self {
        Self {
            handles: [0; MAX_ON_COURT],
            len: 0,
        }
    }

    pub fn push(&mut self, handle: usize) {
        assert!(
            self.len < MAX_ON_COURT,
            "on-court set exceeds {MAX_ON_COURT} players"
        );
        self.handles[self.len] = handle;
        self.len += 1;
    }

    pub fn as_slice(&self) -> &[usize] {
        &self.handles[..self.len]
    }

    pub fn as_mut_slice(&mut self) -> &mut [usize] {
        &mut self.handles[..self.len]
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for OnCourt {
    fn default() -> Self {
        Self::new()
    }
}

impl IntoIterator for OnCourt {
    type Item = usize;
    type IntoIter = std::iter::Take<std::array::IntoIter<usize, MAX_ON_COURT>>;

    fn into_iter(self) -> Self::IntoIter {
        self.handles.into_iter().take(self.len)
    }
}
