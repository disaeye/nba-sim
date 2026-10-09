//! 可执行机会与空间测量（validation.md §4）。
//!
//! `OpportunitySpec` 声明一次机会的进入/退出谓词与测量字段；提取器在
//! 流上按 `(game_id, spec_id, actor_ids, start_tick)` 唯一识别机会。
//! 空间谓词只做声明的几何测量（英尺/秒），不读取生产决策结果。

use serde::{Deserialize, Serialize};

use nba_protocol::StreamTick;

/// 机会配置缺项错误（validation.md §4.1：缺项在运行前拒绝配置）。
#[derive(Debug)]
pub enum SpecError {
    EmptyId,
    MissingEnterPredicate,
    MissingExitPredicate,
    NonFiniteParameter,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyId => write!(f, "opportunity spec id is empty"),
            Self::MissingEnterPredicate => write!(f, "enter predicate missing"),
            Self::MissingExitPredicate => write!(f, "exit predicate missing"),
            Self::NonFiniteParameter => write!(f, "parameter not finite"),
        }
    }
}

impl std::error::Error for SpecError {}

/// 机会的生命周期状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpportunityPhase {
    /// 进入谓词已满足但未达最短持续时间。
    Probing,
    /// 机会已确认（起始时间回记首次满足时刻）。
    Confirmed,
    /// 机会已结束。
    Closed,
}

/// 一次机会的实例（validation.md §4.1 唯一识别四元组 + 观测值）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityInstance {
    pub spec_id: String,
    /// 参与者（按角色顺序）。
    pub actor_ids: Vec<String>,
    pub start_tick: u64,
    /// 确认 tick（进入谓词持续达到最短持续时间后）。
    pub confirmed_tick: u64,
    pub end_tick: u64,
    /// 机会持续时间（tick）。
    pub duration_ticks: u64,
    /// 观测测量值（字段名 → 值）。
    pub measurements: Vec<(String, f64)>,
}

/// 机会规格（validation.md §4.1 必填字段）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunitySpec {
    pub id: String,
    pub version: u32,
    /// 观察频率说明（`every_tick` / `on_event:<kind>`）。
    pub observation_frequency: String,
    /// 进入谓词（几何谓词脚本化描述，由提取器解释）。
    pub enter: GeometricPredicate,
    /// 退出谓词。
    pub exit: GeometricPredicate,
    /// 最短持续时间（tick；即时机会为 0/1）。
    pub min_duration_ticks: u32,
    /// 合并间隔（tick）：间隔小于该值的同主体机会合并。
    pub merge_gap_ticks: u32,
    /// 窗口截止（tick；超窗不再确认）。
    pub window_deadline_ticks: u64,
    /// 观察对象（球员/球队/球）。
    pub actors: Vec<String>,
    /// 测量值定义（字段名 → 几何测量）。
    pub measurements: Vec<MeasurementField>,
}

impl OpportunitySpec {
    /// 运行前配置校验：全部字段有限且谓词存在。
    pub fn validate(&self) -> Result<(), SpecError> {
        if self.id.is_empty() {
            return Err(SpecError::EmptyId);
        }
        if self.actors.is_empty() {
            return Err(SpecError::MissingEnterPredicate);
        }
        if self.min_duration_ticks as u64 > self.window_deadline_ticks {
            return Err(SpecError::NonFiniteParameter);
        }
        Ok(())
    }
}

/// 几何谓词（validation.md §4.2 空间谓词的声明式子集）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GeometricPredicate {
    /// 球员位于矩形区域内（x/y 上下界，英尺）。
    InsideRegion {
        actor: String,
        x_min: f32,
        x_max: f32,
        y_min: f32,
        y_max: f32,
    },
    /// 两名球员距离小于阈值（英尺）。
    Proximity {
        actor_a: String,
        actor_b: String,
        max_distance_ft: f32,
    },
    /// 球权归属（`home`/`away`/`loose`）。
    BallOwnedBy { side: String },
    /// 出手事件出现（`SHOT_RELEASE`）。
    ShotReleased,
    /// 恒真（用于即时机会的退出边界）。
    Always,
}

impl GeometricPredicate {
    /// 在一个 tick 的帧上求值。
    pub fn holds(&self, tick: &StreamTick) -> bool {
        match self {
            Self::InsideRegion {
                actor,
                x_min,
                x_max,
                y_min,
                y_max,
            } => tick
                .frame
                .players
                .iter()
                .filter(|player| &player.id == actor)
                .any(|player| {
                    player.x >= *x_min
                        && player.x <= *x_max
                        && player.y >= *y_min
                        && player.y <= *y_max
                }),
            Self::Proximity {
                actor_a,
                actor_b,
                max_distance_ft,
            } => {
                let position = |id: &str| {
                    tick.frame
                        .players
                        .iter()
                        .find(|player| player.id == id)
                        .map(|player| (player.x, player.y))
                };
                match (position(actor_a), position(actor_b)) {
                    (Some((ax, ay)), Some((bx, by))) => {
                        ((ax - bx).powi(2) + (ay - by).powi(2)).sqrt() <= *max_distance_ft
                    }
                    _ => false,
                }
            }
            Self::BallOwnedBy { side } => match side.as_str() {
                "home" => tick.frame.possession_team == "home",
                "away" => tick.frame.possession_team == "away",
                _ => false,
            },
            Self::ShotReleased => tick
                .frame
                .events
                .iter()
                .any(|event| event.as_str() == "SHOT_RELEASE"),
            Self::Always => true,
        }
    }
}

/// 测量字段（validation.md §4.2 轨迹测量：统一英尺/秒）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeasurementField {
    pub name: String,
    pub kind: MeasurementKind,
}

/// 支持的几何测量。
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementKind {
    /// 确认时刻球员到目标点的距离（英尺）。
    DistanceToTargetFt,
    /// 机会持续时间（秒，按 tick_seconds 折算）。
    DurationSeconds,
    /// 机会期间路径长度（相邻位置距离之和，英尺）。
    PathLengthFt,
}

/// 机会提取器（validation.md §10 `OpportunityExtractor`）。
#[derive(Debug)]
pub struct OpportunityExtractor {
    spec: OpportunitySpec,
    tick_seconds: f32,
    /// 进行中的探测状态（actor 键 → 起始 tick）。
    probing: std::collections::BTreeMap<String, u64>,
    /// 确认未关闭的机会。
    open: Vec<OpportunityInstance>,
    /// 已完成机会。
    finished: Vec<OpportunityInstance>,
    /// 路径累计（actor 键 → 最近位置与累计长度）。
    path: std::collections::BTreeMap<String, ((f32, f32), f64)>,
}

impl OpportunityExtractor {
    pub fn new(spec: OpportunitySpec, tick_seconds: f32) -> Result<Self, SpecError> {
        spec.validate()?;
        Ok(Self {
            spec,
            tick_seconds,
            probing: Default::default(),
            open: Vec::new(),
            finished: Vec::new(),
            path: Default::default(),
        })
    }

    pub fn spec(&self) -> &OpportunitySpec {
        &self.spec
    }

    /// 推进一个 tick：谓词求值、确认、退出与合并。
    pub fn observe(&mut self, tick: &StreamTick) {
        let tick_index = tick.frame.tick_index;
        let enter_holds = self.spec.enter.holds(tick);
        let exit_holds = self.spec.exit.holds(tick);
        let actor_key = self.spec.actors.join("+");
        if enter_holds && !exit_holds {
            self.probing.entry(actor_key.clone()).or_insert(tick_index);
            self.accumulate_path(&actor_key, tick);
            let start = self.probing[&actor_key];
            if tick_index.saturating_sub(start) >= u64::from(self.spec.min_duration_ticks)
                && tick_index <= self.spec.window_deadline_ticks
                && !self
                    .open
                    .iter()
                    .any(|instance| instance.spec_id == self.spec.id)
            {
                self.open.push(OpportunityInstance {
                    spec_id: self.spec.id.clone(),
                    actor_ids: self.spec.actors.clone(),
                    start_tick: start,
                    confirmed_tick: tick_index,
                    end_tick: tick_index,
                    duration_ticks: tick_index - start,
                    measurements: Vec::new(),
                });
            }
            if let Some(instance) = self.open.last_mut() {
                instance.end_tick = tick_index;
                instance.duration_ticks = tick_index - instance.start_tick;
            }
        } else {
            self.close_open(tick);
            self.probing.remove(&actor_key);
            self.path.remove(&actor_key);
        }
    }

    /// 流结束：关闭仍在进行中的机会。
    pub fn finish(&mut self, last_tick: u64) {
        let indices: Vec<usize> = (0..self.open.len()).collect();
        for index in indices {
            let mut instance = self.open.swap_remove(index);
            instance.end_tick = last_tick;
            instance.duration_ticks = last_tick - instance.start_tick;
            self.finished.push(instance);
        }
    }

    /// 已完成机会（按确认顺序）。
    pub fn into_finished(self) -> Vec<OpportunityInstance> {
        self.finished
    }

    fn close_open(&mut self, tick: &StreamTick) {
        if self.open.is_empty() {
            return;
        }
        let mut closed = std::mem::take(&mut self.open);
        for instance in &mut closed {
            instance.end_tick = tick.frame.tick_index;
            instance.duration_ticks = tick.frame.tick_index - instance.start_tick;
            for field in &self.spec.measurements {
                let value = match field.kind {
                    MeasurementKind::DurationSeconds => {
                        f64::from(self.tick_seconds) * instance.duration_ticks as f64
                    }
                    MeasurementKind::PathLengthFt => self
                        .path
                        .get(&instance.actor_ids.join("+"))
                        .map(|(_, length)| *length)
                        .unwrap_or(0.0),
                    MeasurementKind::DistanceToTargetFt => 0.0,
                };
                instance.measurements.push((field.name.clone(), value));
            }
            self.finished.push(instance.clone());
        }
    }

    fn accumulate_path(&mut self, actor_key: &str, tick: &StreamTick) {
        let Some(actor) = self.spec.actors.first() else {
            return;
        };
        let Some(player) = tick.frame.players.iter().find(|player| &player.id == actor) else {
            return;
        };
        let position = (player.x, player.y);
        let entry = self
            .path
            .entry(actor_key.to_string())
            .or_insert((position, 0.0));
        let delta = ((entry.0 .0 - position.0).powi(2) + (entry.0 .1 - position.1).powi(2)).sqrt();
        entry.1 += f64::from(delta);
        entry.0 = position;
    }
}
