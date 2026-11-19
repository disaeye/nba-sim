//! 从比赛帧重建窗口的开闭时刻，供逐段复核。

use glam::Vec2;
use nba_protocol::StreamTick;
use serde::Serialize;

#[derive(Debug, Clone, Copy)]
pub struct Measurement {
    pub contested_radius_ft: f32,
    pub release_seconds: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Closure {
    DefenderArrived,
    TimeExpired,
    ControlChanged,
    PlayStopped,
    EvidenceEnded,
}

#[derive(Debug, Serialize)]
pub struct ShootingWindow {
    pub rule: &'static str,
    pub player_id: String,
    pub possession: u32,
    pub opened_at: u64,
    pub closed_at: u64,
    pub closure: Closure,
    pub start_gap_ft: f32,
    pub end_gap_ft: Option<f32>,
    pub initial_margin_seconds: f32,
    pub final_margin_seconds: Option<f32>,
}

pub fn shooting_windows(frames: &[StreamTick], measurement: Measurement) -> Result<Vec<ShootingWindow>, String> {
    if !measurement.contested_radius_ft.is_finite() || measurement.contested_radius_ft <= f32::default()
        || !measurement.release_seconds.is_finite() || measurement.release_seconds <= f32::default()
    {
        return Err("contest radius and release time must be finite and positive".to_string());
    }
    let mut windows = Vec::new();
    let mut active: Option<ShootingWindow> = None;
    let mut previous = None;
    for tick in frames {
        let frame = &tick.frame;
        if previous.is_some_and(|before: u64| before.checked_add(1) != Some(frame.tick_index)) {
            return Err(format!("missing frames before tick {}", frame.tick_index));
        }
        previous = Some(frame.tick_index);
        if frame.players.is_empty() {
            return Err(format!("player geometry missing at tick {}", frame.tick_index));
        }
        if frame.players.iter().filter(|player| player.on_court).any(|player| {
            !player.x.is_finite() || !player.y.is_finite() || player.team.is_empty()
        }) {
            return Err(format!("invalid player geometry at tick {}", frame.tick_index));
        }
        let live = frame.phase == "Live"
            && frame.possession_id > 0
            && tick.game_clock > f32::default()
            && frame.shot_clock > f32::default();
        let holder = match frame.ball.holder_id.as_ref() {
            Some(id) => Some(frame.players.iter().find(|player| &player.id == id && player.on_court)
                .ok_or_else(|| format!("holder geometry missing at tick {}", frame.tick_index))?),
            None => None,
        };
        let geometry = Vec2::new(frame.rules.court_width_ft, frame.rules.court_height_ft);
        let tick_seconds = frame.rules.tick_seconds;
        let speed = frame.rules.max_player_speed_ftps;
        if !geometry.is_finite() || geometry.min_element() <= f32::default()
            || !tick_seconds.is_finite() || tick_seconds <= f32::default()
            || !speed.is_finite() || speed <= f32::default()
        {
            return Err(format!("court geometry or movement limits missing at tick {}", frame.tick_index));
        }
        let nearest = holder.map(|player| {
            let point = Vec2::new(player.x, player.y);
            frame.players.iter().filter(|defender| defender.on_court && defender.team != player.team)
                .map(|defender| (geometry * (Vec2::new(defender.x, defender.y) - point)).length())
                .min_by(f32::total_cmp)
        });
        let gap = nearest.flatten();
        if holder.is_some() && gap.is_none() {
            return Err(format!("defender geometry missing at tick {}", frame.tick_index));
        }
        let margin = gap.map(|distance| {
            (distance - measurement.contested_radius_ft).max(f32::default()) / speed - measurement.release_seconds
        });
        let enough_time = tick.game_clock >= measurement.release_seconds && frame.shot_clock >= measurement.release_seconds;
        if let Some(mut current) = active.take() {
            let same = holder.is_some_and(|player| player.id == current.player_id) && frame.possession_id == current.possession;
            if live && same && enough_time && margin.is_some_and(|value| value > f32::default()) {
                active = Some(current);
            } else {
                current.closed_at = frame.tick_index;
                current.closure = if !live { Closure::PlayStopped } else if !same { Closure::ControlChanged } else if !enough_time { Closure::TimeExpired } else { Closure::DefenderArrived };
                current.end_gap_ft = gap.filter(|_| same);
                current.final_margin_seconds = margin.filter(|_| same);
                windows.push(current);
            }
        }
        if live && enough_time && active.is_none() {
            if let (Some(holder), Some(gap), Some(margin)) = (holder, gap, margin.filter(|value| *value > f32::default())) {
                active = Some(ShootingWindow { rule: "release-arrival-lower-bound-v1", player_id: holder.id.clone(), possession: frame.possession_id, opened_at: frame.tick_index, closed_at: frame.tick_index, closure: Closure::EvidenceEnded, start_gap_ft: gap, end_gap_ft: None, initial_margin_seconds: margin, final_margin_seconds: None });
            }
        }
    }
    if let (Some(mut current), Some(last)) = (active, previous) {
        current.closed_at = last;
        windows.push(current);
    }
    Ok(windows)
}
