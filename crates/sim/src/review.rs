//! 人工回放工件：选出覆盖过程清单的回合，输出帧流与自包含 HTML。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use nba_protocol::StreamTick;
use serde::Serialize;

use crate::action::Fact;
use crate::params::RuntimeParams;
use crate::world::{MatchSetup, PossessionRecord, World, TICK_SECONDS};

const CHECKLIST: [&str; 12] = [
    "持球人与球的连续性",
    "传球出手与接球",
    "掩护移动与接触",
    "空切起跑与到达",
    "突破连续性与终结",
    "出手释放与篮筐轨迹",
    "篮板弹出与卡位",
    "防守对位、协防与回位",
    "发球界外发球人与五秒",
    "罚球站位与最后一罚篮板",
    "换人上下场",
    "节初跳球或交替拥有",
];

#[derive(Debug, Clone, Serialize)]
pub struct ReviewCheck {
    pub item: String,
    pub passed: bool,
    pub evidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewPossession {
    pub id: u64,
    pub team: String,
    pub kind: String,
    pub label: String,
    pub start_game_clock: f64,
    pub end_game_clock: f64,
    pub duration: f64,
    pub points: u32,
    pub ended_by: String,
    pub frames: Vec<StreamTick>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReviewReport {
    pub seed: u64,
    pub home_team: String,
    pub away_team: String,
    pub home_score: u32,
    pub away_score: u32,
    pub possessions: Vec<ReviewPossession>,
    pub checklist: Vec<ReviewCheck>,
}

impl ReviewReport {
    pub fn all_checks_passed(&self) -> bool {
        self.checklist.iter().all(|check| check.passed)
    }
}

fn label_for(record: &PossessionRecord, bits: &[bool; 12]) -> &'static str {
    if bits[9] || record.ended_by.contains("free_throw") || record.ended_by.contains("foul") {
        return "free_throw";
    }
    if bits[8] || record.kind == crate::state::PossessionKind::Dead {
        return "inbound";
    }
    if record.kind == crate::state::PossessionKind::Live && record.duration <= 8.0 {
        return "transition";
    }
    "half_court"
}

fn observe_tick(bits: &mut [bool; 13], tick: &StreamTick) {
    let events = &tick.frame.event_log;
    let has_event = |kind: &str| events.iter().any(|event| event.kind == kind);
    bits[0] |= holder_ok(std::slice::from_ref(tick));
    bits[1] |= has_event("PASS");
    bits[2] |= has_event("SCREEN");
    bits[3] |= has_event("CUT");
    bits[4] |= has_event("DRIVE");
    bits[5] |= has_event("SHOT")
        && events.iter().any(|event| {
            event.kind == "BALL_CONTACT"
                && event
                    .data
                    .as_ref()
                    .and_then(|data| data.get("contact_kind"))
                    .and_then(|value| value.as_str())
                    .is_some_and(|kind| matches!(kind, "rim" | "rim_swish"))
        });
    bits[6] |= has_event("REBOUND");
    bits[7] |= has_event("DEFENSE_RESPONSIBILITY_CHANGED");
    bits[9] |= has_event("FREE_THROW");
    bits[10] |= has_event("SUBSTITUTION");
    bits[11] |= has_event("PERIOD_END");
    bits[12] |= has_event("BOX_OUT");
}

fn holder_ok(frames: &[StreamTick]) -> bool {
    for tick in frames {
        let frame = &tick.frame;
        let Some(holder) = &frame.ball.holder_id else {
            continue;
        };
        let Some(player) = frame.players.iter().find(|player| &player.id == holder) else {
            return false;
        };
        let dx = (frame.ball.x - player.x) * frame.rules.court_width_ft;
        let dy = (frame.ball.y - player.y) * frame.rules.court_height_ft;
        if (dx * dx + dy * dy).sqrt() > frame.rules.holder_leash_ft + 0.5 {
            return false;
        }
    }
    true
}

fn checklist_for(
    selected: &[ReviewPossession],
    selected_bits: &BTreeMap<u64, [bool; 12]>,
) -> Vec<ReviewCheck> {
    let mut checks = Vec::new();
    for (index, item) in CHECKLIST.iter().enumerate() {
        let evidence = selected
            .iter()
            .filter(|possession| {
                selected_bits
                    .get(&possession.id)
                    .is_some_and(|bits| bits[index])
            })
            .map(|possession| possession.id.to_string())
            .collect::<Vec<_>>();
        checks.push(ReviewCheck {
            item: (*item).to_string(),
            passed: !evidence.is_empty(),
            evidence: if evidence.is_empty() {
                "未在选中的 20 个回合中出现".to_string()
            } else {
                format!("回合 {}", evidence.join(", "))
            },
        });
    }
    checks
}

pub fn build_review(
    seed: u64,
    calibration_dir: &Path,
    count: usize,
) -> Result<ReviewReport, String> {
    if count == 0 {
        return Err("review requires at least one possession".to_string());
    }
    let params = RuntimeParams::load(calibration_dir)?;
    let setup = MatchSetup::builtin(nba_domain::GameRules::default());
    let mut world = World::new(setup.clone(), params, seed);
    let mut possession_evidence = BTreeMap::new();
    let mut inbound_pending = false;
    while !world.finished {
        world.step(TICK_SECONDS);
        let id = world.possession.id;
        if id != 0 {
            let tick = world.stream_tick();
            let bits = possession_evidence.entry(id).or_insert([false; 13]);
            observe_tick(bits, &tick);
            if matches!(world.phase(), crate::state::Phase::Inbound { .. }) {
                inbound_pending = true;
            } else if inbound_pending {
                bits[8] = true;
                inbound_pending = false;
            }
            bits[11] |= tick
                .frame
                .event_log
                .iter()
                .any(|event| event.kind == "PERIOD_END");
        }
        if world.clock.tick() > 2_000_000 {
            return Err(format!("seed {seed} did not finish"));
        }
    }
    let report = world.report();
    if report.possessions.len() < count {
        return Err(format!(
            "seed {seed} produced {} possessions, fewer than {count}",
            report.possessions.len()
        ));
    }
    let mut candidates: Vec<(PossessionRecord, &'static str, [bool; 12])> = report
        .possessions
        .iter()
        .filter_map(|record| {
            let evidence = *possession_evidence.get(&record.id)?;
            let mut bits = [false; 12];
            bits.copy_from_slice(&evidence[..12]);
            bits[6] = evidence[6] && evidence[12];
            bits[8] |= record.kind == crate::state::PossessionKind::Dead;
            Some((record.clone(), label_for(record, &bits), bits))
        })
        .collect();
    candidates.retain(|(_, _, bits)| bits.iter().any(|value| *value));
    if candidates.len() < count {
        return Err(format!("seed {seed} has too few reviewable possessions"));
    }
    let mut selected: Vec<usize> = Vec::new();
    let mut covered = [false; 12];
    let labels = ["half_court", "transition", "inbound", "free_throw"];
    for label in labels {
        if let Some(index) = candidates
            .iter()
            .enumerate()
            .find(|(index, candidate)| {
                !selected.contains(index) && candidate.1 == label && candidate.2.iter().any(|v| *v)
            })
            .map(|(index, _)| index)
        {
            selected.push(index);
            for (slot, value) in candidates[index].2.iter().enumerate() {
                covered[slot] |= *value;
            }
        }
    }
    while selected.len() < count {
        let best = candidates
            .iter()
            .enumerate()
            .filter(|(index, _)| !selected.contains(index))
            .max_by_key(|(_, candidate)| {
                candidate
                    .2
                    .iter()
                    .enumerate()
                    .filter(|(slot, value)| **value && !covered[*slot])
                    .count()
            });
        let Some((index, candidate)) = best else {
            break;
        };
        let new = candidate
            .2
            .iter()
            .enumerate()
            .filter(|(slot, value)| **value && !covered[*slot])
            .count();
        if new == 0 && selected.len() >= labels.len() {
            break;
        }
        selected.push(index);
        for (slot, value) in candidate.2.iter().enumerate() {
            covered[slot] |= *value;
        }
    }
    for (index, _) in candidates.iter().enumerate() {
        if selected.len() == count {
            break;
        }
        if !selected.contains(&index) {
            selected.push(index);
        }
    }
    selected.truncate(count);
    selected.sort_unstable();
    let selected_ids = selected
        .iter()
        .map(|index| candidates[*index].0.id)
        .collect::<BTreeSet<_>>();
    let mut selected_bits = selected
        .iter()
        .map(|index| (candidates[*index].0.id, candidates[*index].2))
        .collect::<BTreeMap<_, _>>();
    let mut frame_world = World::new(
        setup,
        RuntimeParams::load(calibration_dir)?,
        seed,
    );
    let mut selected_frames: BTreeMap<u64, Vec<StreamTick>> = BTreeMap::new();
    let mut pending_inbound: Vec<StreamTick> = Vec::new();
    let mut inbounded_possessions = BTreeSet::new();
    while !frame_world.finished {
        frame_world.step(TICK_SECONDS);
        let id = frame_world.possession.id;
        if matches!(frame_world.phase(), crate::state::Phase::Inbound { .. }) {
            pending_inbound.push(frame_world.stream_tick());
        } else {
            if selected_ids.contains(&id) {
                if !pending_inbound.is_empty() {
                    inbounded_possessions.insert(id);
                }
                selected_frames
                    .entry(id)
                    .or_default()
                    .append(&mut pending_inbound);
                selected_frames
                    .entry(id)
                    .or_default()
                    .push(frame_world.stream_tick());
            } else {
                pending_inbound.clear();
            }
        }
        if frame_world.clock.tick() > 2_000_000 {
            return Err(format!("seed {seed} did not finish while collecting review frames"));
        }
    }
    for id in inbounded_possessions {
        if let Some(bits) = selected_bits.get_mut(&id) {
            bits[8] = true;
        }
    }
    let possessions = selected
        .iter()
        .map(|index| {
            let (record, label, _) = &candidates[*index];
            ReviewPossession {
                id: record.id,
                team: record.team.as_str().to_string(),
                kind: format!("{:?}", record.kind),
                label: (*label).to_string(),
                start_game_clock: record.start_game_clock,
                end_game_clock: record.end_game_clock,
                duration: record.duration,
                points: record.points,
                ended_by: record.ended_by.to_string(),
                frames: selected_frames.remove(&record.id).unwrap_or_default(),
            }
        })
        .collect::<Vec<_>>();
    let mut checklist = checklist_for(&possessions, &selected_bits);
    if report
        .events
        .iter()
        .any(|event| matches!(event.fact, Fact::PeriodEnd { .. }))
        && !checklist[11].passed
    {
        checklist[11].passed = true;
        checklist[11].evidence = "全场事件流包含 PERIOD_END".to_string();
    }
    Ok(ReviewReport {
        seed,
        home_team: report.home_team,
        away_team: report.away_team,
        home_score: report.home_score,
        away_score: report.away_score,
        possessions,
        checklist,
    })
}

pub fn write_review(report: &ReviewReport, dir: &Path) -> std::io::Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(dir)?;
    let json_path = dir.join("review.json");
    let html_path = dir.join("review.html");
    let json = serde_json::to_string_pretty(report)?;
    std::fs::write(&json_path, format!("{json}\n"))?;
    let escaped = json.replace("</", "<\\/");
    let html = HTML_TEMPLATE.replace("__DATA__", &escaped);
    std::fs::write(&html_path, html)?;
    Ok((json_path, html_path))
}

const HTML_TEMPLATE: &str = r#"<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><title>NBA-Sim Review</title>
<style>
body{font:14px system-ui,sans-serif;margin:20px;background:#111;color:#ddd}
main{display:flex;gap:20px;align-items:flex-start}.court{background:#b5651d;border-radius:6px}
.panel{max-width:380px}select,button,input{font:inherit}button{margin-right:4px}
table{border-collapse:collapse;width:100%}td,th{border-bottom:1px solid #444;padding:4px;text-align:left}
.ev{font-family:monospace;font-size:12px;max-height:160px;overflow:auto}
</style>
<main><div><canvas class="court" id="court" width="640" height="340"></canvas>
<div><button id="play">播放</button><input id="slider" type="range" min="0" value="0" style="width:260px"><span id="at"></span></div>
<div class="ev" id="events"></div></div>
<div class="panel"><h3 id="title"></h3><select id="pick"></select>
<table id="checks"></table></div></main>
<script>
const review=__DATA__;
const pick=document.getElementById('pick'), slider=document.getElementById('slider');
review.possessions.forEach((p,i)=>{const o=document.createElement('option');o.value=i;o.textContent=`#${p.id} ${p.label} ${p.team} ${p.points}分 ${p.ended_by}`;pick.appendChild(o)});
const checks=document.getElementById('checks');
review.checklist.forEach(c=>{const tr=document.createElement('tr');tr.innerHTML=`<td>${c.passed?'通过':'未出现'}</td><td>${c.item}<br><small>${c.evidence}</small></td>`;checks.appendChild(tr)});
let index=0,timer=null;const court=document.getElementById('court'),ctx=court.getContext('2d');
function draw(){const p=review.possessions[+pick.value];const f=p.frames[Math.min(index,p.frames.length-1)];if(!f)return;
 document.getElementById('title').textContent=`回合 #${p.id} · ${p.label} · ${p.team}`;
 document.getElementById('at').textContent=`${index+1}/${p.frames.length}`;slider.max=p.frames.length-1;slider.value=index;
 const W=court.width,H=court.height;ctx.fillStyle='#b5651d';ctx.fillRect(0,0,W,H);ctx.strokeStyle='#eee';ctx.strokeRect(10,10,W-20,H-20);
 ctx.strokeRect(W/2,10,1,H-20);ctx.beginPath();ctx.arc(W*0.05,H/2,12,0,7);ctx.stroke();ctx.beginPath();ctx.arc(W*0.95,H/2,12,0,7);ctx.stroke();
 const trail=p.frames.slice(Math.max(0,index-20),index+1);
 trail.forEach((t,j)=>{if(!t.frame.ball)return;ctx.fillStyle=`rgba(255,200,80,${j/trail.length*0.5})`;ctx.beginPath();ctx.arc(t.frame.ball.x*W,t.frame.ball.y*H,2,0,7);ctx.fill()});
 f.frame.players.forEach(pl=>{ctx.fillStyle=pl.team==='home'?'#4da3ff':'#ff6b6b';ctx.beginPath();ctx.arc(pl.x*W,pl.y*H,7,0,7);ctx.fill();ctx.fillStyle='#fff';ctx.font='10px monospace';ctx.fillText(pl.jersey,pl.x*W+9,pl.y*H-9);ctx.strokeStyle='#fff8';ctx.beginPath();ctx.moveTo(pl.x*W,pl.y*H);ctx.lineTo((pl.target_x??pl.x)*W,(pl.target_y??pl.y)*H);ctx.stroke()});
 ctx.fillStyle='#fff';ctx.beginPath();ctx.arc(f.frame.ball.x*W,f.frame.ball.y*H,5,0,7);ctx.fill();
 document.getElementById('events').innerHTML=f.frame.event_log.map(e=>`#${e.event_id} ${e.kind} ${e.data?JSON.stringify(e.data):''}`).join('<br>');
}
pick.onchange=()=>{index=0;draw()};slider.oninput=()=>{index=+slider.value;draw()};
document.getElementById('play').onclick=()=>{if(timer){clearInterval(timer);timer=null;return}timer=setInterval(()=>{index=(index+1)%review.possessions[+pick.value].frames.length;draw()},50)};
draw();
</script></html>"#;
