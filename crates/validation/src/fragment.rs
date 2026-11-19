//! 使用生产比赛状态执行有界片段，并记录可定位的事实。

use nba_domain::league::LeagueId;
use nba_protocol::frame::{FrameEvent, StreamTick};
use nba_sim_core::action::Fact;
use nba_sim_core::world::TICK_SECONDS;
use nba_sim_core::{MatchManifest, MatchSetup, RuntimeParams, World};
use serde::{Deserialize, Serialize};

use crate::core_validation::check_ledger_scoped;

const DEFAULT_FRAGMENT_MICROS: u64 = 30 * nba_domain::rules::MICROS_PER_SECOND;
pub const DEFAULT_FRAGMENT_TICKS: u32 = (DEFAULT_FRAGMENT_MICROS / nba_domain::rules::TICK_MICROS) as u32;
const MAX_FRAGMENT_TICKS: u32 = 2_400;
const VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scenario {
    OpeningControl,
    FirstPossession,
    HalfcourtScreen,
}

impl Scenario {
    pub const ALL: [Self; 2] = [Self::OpeningControl, Self::FirstPossession];

    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "opening-control" => Ok(Self::OpeningControl),
            "first-possession" => Ok(Self::FirstPossession),
            "halfcourt-screen" => Ok(Self::HalfcourtScreen),
            _ => Err(format!("unknown scenario `{value}`")),
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::OpeningControl => "opening-control",
            Self::FirstPossession => "first-possession",
            Self::HalfcourtScreen => "halfcourt-screen",
        }
    }

    fn starts(self, world: &World) -> bool {
        match self {
            Self::OpeningControl => world.clock.tick() == 0,
            Self::FirstPossession => first_live_control(world),
            Self::HalfcourtScreen => world.live_play() && world.holder().is_some()
                && world.interactions.values().any(|interaction| {
                    matches!(interaction.intent.kind, nba_sim_core::ActionKind::Screen(_))
                        && (world.holder() == Some(interaction.intent.actor)
                            || world.holder() == interaction.intent.target)
                }),
        }
    }

    fn ends(self, world: &World, possession_id: u64) -> bool {
        match self {
            Self::OpeningControl => first_live_control(world),
            Self::FirstPossession | Self::HalfcourtScreen => world.possession_log.last()
                .is_some_and(|possession| possession.id == possession_id),
        }
    }
}

fn first_live_control(world: &World) -> bool {
    world.live_play() && world.possession.id == 1 && world.holder().is_some()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FragmentInput {
    pub version: u32,
    pub manifest: MatchManifest,
    pub scenario: Scenario,
    pub max_ticks: u32,
}

impl FragmentInput {
    pub fn builtin(league: LeagueId, seed: u64, scenario: Scenario, params: &RuntimeParams) -> Self {
        Self {
            version: VERSION,
            manifest: MatchManifest::new(
                MatchSetup::builtin(nba_domain::GameRules::for_league(league)),
                seed,
                &params.calibration_version,
            ),
            scenario,
            max_ticks: DEFAULT_FRAGMENT_TICKS,
        }
    }

    fn validate(&self, params: &RuntimeParams) -> Result<(), String> {
        if self.version != VERSION || !(1..=MAX_FRAGMENT_TICKS).contains(&self.max_ticks) {
            return Err(format!("invalid fragment version or tick budget: {self:?}"));
        }
        self.manifest.check_against(params)?;
        self.manifest.setup.validate()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Finding {
    pub check: &'static str,
    pub tick: u64,
    pub events: Vec<u64>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct FragmentReport {
    pub input: FragmentInput,
    pub start_tick: u64,
    pub end_tick: u64,
    pub possession_id: u64,
    pub advanced_ticks: u64,
    pub events: Vec<FrameEvent>,
    pub decisions: Vec<nba_sim_core::state::DecisionRecord>,
    pub shooting_windows: Vec<crate::opportunities::ShootingWindow>,
    pub screen: Option<crate::halfcourt::ScreenSequence>,
    pub findings: Vec<Finding>,
}

impl FragmentReport {
    pub fn passed(&self) -> bool {
        self.findings.is_empty()
    }
}

#[derive(Debug, Serialize)]
pub struct FragmentRun {
    pub report: FragmentReport,
    pub frames: Vec<StreamTick>,
}

/// 保存生产世界及本场景使用的输入。恢复从当前位置继续。
#[derive(Clone)]
pub struct FragmentCheckpoint {
    input: FragmentInput,
    world: World,
}

pub fn capture(input: FragmentInput, params: &RuntimeParams) -> Result<FragmentCheckpoint, String> {
    input.validate(params)?;
    let mut world = World::new(input.manifest.setup.clone(), params.clone(), input.manifest.seed);
    while !input.scenario.starts(&world) {
        advance(&input, &mut world, "start")?;
    }
    Ok(FragmentCheckpoint { input, world })
}

fn advance(input: &FragmentInput, world: &mut World, boundary: &str) -> Result<(), String> {
    if world.finished || world.clock.tick() >= u64::from(input.max_ticks) {
        return Err(format!(
            "scenario {} seed {} did not reach {boundary} at tick {} (budget {})",
            input.scenario.name(), input.manifest.seed, world.clock.tick(), input.max_ticks
        ));
    }
    world.step(TICK_SECONDS);
    Ok(())
}

pub fn run(input: FragmentInput, params: &RuntimeParams) -> Result<FragmentRun, String> {
    capture(input, params)?.resume()
}

impl FragmentCheckpoint {
    pub fn tick(&self) -> u64 {
        self.world.clock.tick()
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        nba_sim_core::checkpoint::save(&self.world, path)
    }

    pub fn load(input: FragmentInput, path: &std::path::Path, params: &RuntimeParams) -> Result<Self, String> {
        input.validate(params)?;
        let world = nba_sim_core::checkpoint::load(path, params)?;
        let world_setup = serde_json::to_vec(&world.setup).map_err(|error| error.to_string())?;
        let input_setup = serde_json::to_vec(&input.manifest.setup).map_err(|error| error.to_string())?;
        if world.seed != input.manifest.seed || world_setup != input_setup
            || !input.scenario.starts(&world) || world.clock.tick() >= u64::from(input.max_ticks)
        {
            return Err("checkpoint does not match the fragment start".to_string());
        }
        Ok(Self { input, world })
    }

    pub fn resume(self) -> Result<FragmentRun, String> {
        let Self { input, mut world } = self;
        let start_tick = world.clock.tick();
        let possession_id = world.possession.id;
        let first_event = world.events.len();
        let frame_event_start = world.events.iter().position(|event| event.tick_index == start_tick)
            .unwrap_or(first_event);
        let first_violation = world.violations.len();
        let mut frames = vec![world.stream_tick()];
        while !input.scenario.ends(&world, possession_id) {
            advance(&input, &mut world, "end")?;
            frames.push(world.stream_tick());
        }
        let end_tick = world.clock.tick();
        let report = world.report_at_boundary();
        let events: Vec<_> = report.events[frame_event_start..]
            .iter()
            .map(|event| world.project_frame_event(event))
            .collect();
        let mut findings = check_frames(&frames);
        let terminal_event = world.events[first_event..].iter().find(|event| match input.scenario {
            Scenario::OpeningControl => matches!(event.fact, Fact::JumpBall { .. }),
            Scenario::FirstPossession | Scenario::HalfcourtScreen => matches!(event.fact, Fact::PossessionEnded { possession_id: id, .. } if id == possession_id),
        });
        if terminal_event.is_none() {
            findings.push(Finding { check: "terminal_event", tick: end_tick, events: Vec::new(), detail: "达到结束状态但缺少对应的结算事件".to_string() });
        }
        let ledger = check_ledger_scoped(input.manifest.seed, &report, false);
        if !ledger.passed() {
            findings.push(Finding { check: "independent_ledger", tick: end_tick, events: events.iter().rev().take(1).map(|event| event.event_id).collect(), detail: ledger.detail });
        }
        for detail in &report.violations[first_violation..] {
            findings.push(Finding { check: "engine_violation", tick: end_tick, events: Vec::new(), detail: detail.clone() });
        }
        let shooting_windows = crate::opportunities::shooting_windows(
            &frames,
            crate::opportunities::Measurement {
                contested_radius_ft: world.params.policy.g("contest_proximity_ft"),
                release_seconds: 0.32,
            },
        )?;
        let decisions: Vec<_> = report.decision_records.iter().filter(|decision| decision.tick >= start_tick).cloned().collect();
        let screen = if input.scenario == Scenario::HalfcourtScreen {
            match crate::halfcourt::screen_sequence(&frames, &events, &decisions) {
                Ok(sequence) => Some(sequence),
                Err(finding) => {
                    findings.push(finding);
                    None
                }
            }
        } else {
            None
        };
        Ok(FragmentRun { report: FragmentReport { input, start_tick, end_tick, possession_id, advanced_ticks: end_tick, events, decisions, shooting_windows, screen, findings }, frames })
    }
}

/// 检查帧连续性与控球投影，缺失区间不用于推断比赛行为。
pub fn check_frames(frames: &[StreamTick]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for pair in frames.windows(2) {
        let before = &pair[0].frame;
        let after = &pair[1].frame;
        if before.tick_index.checked_add(1) != Some(after.tick_index) {
            findings.push(Finding {
                check: "frame_sequence",
                tick: after.tick_index,
                events: after.event_log.iter().map(|event| event.event_id).collect(),
                detail: format!("帧序号 {} 之后得到 {}，无法检查中间过程", before.tick_index, after.tick_index),
            });
        }
    }
    for tick in frames {
        let frame = &tick.frame;
        let mut flagged = frame.players.iter().filter(|player| player.has_ball);
        let first = flagged.next();
        let valid = flagged.next().is_none()
            && first.map(|player| player.id.as_str()) == frame.ball.holder_id.as_deref();
        if !valid {
            findings.push(Finding {
                check: "holder_projection",
                tick: frame.tick_index,
                events: frame.event_log.iter().map(|event| event.event_id).collect(),
                detail: format!("球状态的持球人 {:?} 与球员持球标记不一致", frame.ball.holder_id),
            });
        }
    }
    findings
}
