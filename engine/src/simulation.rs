use glam::Vec2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::court::Court;
use crate::movement::{PhysicsWorld, PlayerPhysicsState};
use crate::protocol::{DecisionTrace, RenderBall, RenderFrame, RenderPlayer, RenderScore, StreamTick};
use crate::tactics::{MatchStageType, PlayerProfile, Possession, TacticalPlanner};

pub struct MatchEngine {
    physics: PhysicsWorld,
    possession: Possession,
    stage_id: usize,
    stage_type: MatchStageType,
    stage_title: String,
    stage_elapsed: f32,
    stage_duration: f32,
    shot_clock: f32,
    game_clock: f32,
    period: u32,
    home_score: u32,
    away_score: u32,
    ball_pos_ft: Vec2,
    ball_carrier: Option<String>,
    home_profiles: Vec<PlayerProfile>,
    away_profiles: Vec<PlayerProfile>,
    frame_count: usize,
    rng: StdRng,
}

impl MatchEngine {
    pub fn new(seed: u64) -> Self {
        let mut physics = PhysicsWorld::new();
        let rng = StdRng::seed_from_u64(seed);

        // Register Home Lineup
        let home_jerseys = ["0", "7", "4", "8", "9"];
        for (i, jersey) in home_jerseys.iter().enumerate() {
            physics.register_player(PlayerPhysicsState {
                id: format!("H_{}", i + 1),
                jersey: jersey.to_string(),
                team: "home".to_string(),
                pos_ft: Vec2::new(30.0 + (i as f32) * 5.0, 15.0 + (i as f32) * 4.0),
                vel_ft: Vec2::ZERO,
                target_pos_ft: Vec2::new(30.0, 25.0),
                target_speed_ftps: 12.0,
                has_ball: i == 0,
                action: "INIT".to_string(),
                stamina: 100.0,
                max_stamina: 100.0,
            });
        }

        // Register Away Lineup
        let away_jerseys = ["23", "3", "15", "1", "28"];
        for (i, jersey) in away_jerseys.iter().enumerate() {
            physics.register_player(PlayerPhysicsState {
                id: format!("A_{}", i + 1),
                jersey: jersey.to_string(),
                team: "away".to_string(),
                pos_ft: Vec2::new(60.0 - (i as f32) * 5.0, 15.0 + (i as f32) * 4.0),
                vel_ft: Vec2::ZERO,
                target_pos_ft: Vec2::new(60.0, 25.0),
                target_speed_ftps: 12.0,
                has_ball: false,
                action: "INIT".to_string(),
                stamina: 100.0,
                max_stamina: 100.0,
            });
        }

        let home_profiles = vec![
            PlayerProfile { name: "Jayson Tatum".to_string(), jersey: "0".to_string(), shot_range_max_ft: 29.0, catch_and_shoot_fg: 0.42, vision_fov_degrees: 180.0, shoot_tendency: 0.75, pass_first_tendency: 0.35, drive_tendency: 0.80 },
            PlayerProfile { name: "Jaylen Brown".to_string(), jersey: "7".to_string(), shot_range_max_ft: 28.0, catch_and_shoot_fg: 0.40, vision_fov_degrees: 150.0, shoot_tendency: 0.70, pass_first_tendency: 0.30, drive_tendency: 0.85 },
            PlayerProfile { name: "Jrue Holiday".to_string(), jersey: "4".to_string(), shot_range_max_ft: 26.0, catch_and_shoot_fg: 0.44, vision_fov_degrees: 200.0, shoot_tendency: 0.40, pass_first_tendency: 0.80, drive_tendency: 0.50 },
            PlayerProfile { name: "Kristaps Porziņģis".to_string(), jersey: "8".to_string(), shot_range_max_ft: 28.0, catch_and_shoot_fg: 0.41, vision_fov_degrees: 140.0, shoot_tendency: 0.65, pass_first_tendency: 0.20, drive_tendency: 0.40 },
            PlayerProfile { name: "Derrick White".to_string(), jersey: "9".to_string(), shot_range_max_ft: 27.0, catch_and_shoot_fg: 0.43, vision_fov_degrees: 190.0, shoot_tendency: 0.50, pass_first_tendency: 0.70, drive_tendency: 0.60 },
        ];

        let away_profiles = vec![
            PlayerProfile { name: "LeBron James".to_string(), jersey: "23".to_string(), shot_range_max_ft: 28.0, catch_and_shoot_fg: 0.41, vision_fov_degrees: 230.0, shoot_tendency: 0.70, pass_first_tendency: 0.75, drive_tendency: 0.85 },
            PlayerProfile { name: "Anthony Davis".to_string(), jersey: "3".to_string(), shot_range_max_ft: 20.0, catch_and_shoot_fg: 0.33, vision_fov_degrees: 160.0, shoot_tendency: 0.75, pass_first_tendency: 0.25, drive_tendency: 0.70 },
            PlayerProfile { name: "Austin Reaves".to_string(), jersey: "15".to_string(), shot_range_max_ft: 27.0, catch_and_shoot_fg: 0.40, vision_fov_degrees: 175.0, shoot_tendency: 0.55, pass_first_tendency: 0.65, drive_tendency: 0.60 },
            PlayerProfile { name: "D'Angelo Russell".to_string(), jersey: "1".to_string(), shot_range_max_ft: 29.0, catch_and_shoot_fg: 0.42, vision_fov_degrees: 180.0, shoot_tendency: 0.65, pass_first_tendency: 0.60, drive_tendency: 0.50 },
            PlayerProfile { name: "Rui Hachimura".to_string(), jersey: "28".to_string(), shot_range_max_ft: 26.0, catch_and_shoot_fg: 0.43, vision_fov_degrees: 130.0, shoot_tendency: 0.50, pass_first_tendency: 0.20, drive_tendency: 0.55 },
        ];

        Self {
            physics,
            possession: Possession::Home,
            stage_id: 1,
            stage_type: MatchStageType::HighPickAndRoll,
            stage_title: "塔图姆弧顶发起高位挡拆".to_string(),
            stage_elapsed: 0.0,
            stage_duration: 6.0,
            shot_clock: 24.0,
            game_clock: 720.0,
            period: 1,
            home_score: 0,
            away_score: 0,
            ball_pos_ft: Vec2::new(30.0, 25.0),
            ball_carrier: Some("H_1".to_string()),
            home_profiles,
            away_profiles,
            frame_count: 0,
            rng,
        }
    }

    /// Step simulation by 1 tick (0.1s) and return StreamTick
    pub fn step(&mut self) -> StreamTick {
        let dt = 0.1;
        let frame_idx = self.frame_count;
        self.frame_count += 1;

        // 1. Advance Game Clock & Shot Clock
        self.game_clock = (self.game_clock - dt).max(0.0);
        self.shot_clock = (self.shot_clock - dt).max(0.0);

        let mut event_type: Option<String> = None;
        let mut callout: Option<String> = None;
        let mut intensity: Option<f32> = None;

        let mut decision_trace: Option<DecisionTrace> = None;

        // 2. Stage Progression & State Switching
        self.stage_elapsed += dt;
        if self.stage_elapsed >= self.stage_duration {
            self.stage_elapsed = 0.0;
            self.stage_id += 1;
            
            // 阶段轮转调度 (Stage Sequencer)
            let (next_stage, next_dur, next_title) = match self.stage_type {
                MatchStageType::JumpBall => (MatchStageType::HighPickAndRoll, 7.0, "塔图姆弧顶发起高位挡拆".to_string()),
                MatchStageType::HighPickAndRoll => (MatchStageType::DriveAndKick, 5.5, "突分战术：分球底角大空位".to_string()),
                MatchStageType::DriveAndKick => (MatchStageType::PostBasketInbound, 4.0, "进球死球：底线发球全队推进".to_string()),
                MatchStageType::PostBasketInbound => (MatchStageType::IsolationDrive, 6.0, "詹姆斯强侧单打突破突破造犯规".to_string()),
                MatchStageType::IsolationDrive => (MatchStageType::FreeThrow, 5.0, "裁判鸣哨执行罚球序列".to_string()),
                MatchStageType::FreeThrow => (MatchStageType::Timeout, 4.0, "教练席请求战术暂停与人员调整".to_string()),
                MatchStageType::SidelineInbound => (MatchStageType::HighPickAndRoll, 6.5, "霍勒迪二次组织高位挡拆".to_string()),
                MatchStageType::FiveOutMotion => (MatchStageType::DriveAndKick, 5.5, "五外突分底角射手".to_string()),
                MatchStageType::FastBreakTransition => (MatchStageType::IsolationDrive, 4.5, "快攻前场单打攻筐".to_string()),
                MatchStageType::Timeout => (MatchStageType::SidelineInbound, 4.0, "暂停结束边线发球".to_string()),
            };
            self.stage_type = next_stage;
            self.stage_duration = next_dur;
            self.stage_title = next_title.clone();
            
            event_type = Some("STAGE_CHANGE".to_string());
            callout = Some(format!("▶ 进入阶段 #{}: {}", self.stage_id, next_title));
            intensity = Some(0.6);
        }

        // 3. 运行微观决策函数 (Decision Function Call in Current Stage)
        if frame_idx % 15 == 0 { // 每 1.5 秒进行一次微观决策评估
            let is_home = self.possession == Possession::Home;
            let active_profiles = if is_home { &self.home_profiles } else { &self.away_profiles };
            let carrier_id = self.ball_carrier.clone().unwrap_or_else(|| (if is_home { "H_1" } else { "A_1" }).to_string());
            let carrier_idx = carrier_id.split('_').nth(1).and_then(|s| s.parse::<usize>().ok()).unwrap_or(1) - 1;
            let profile = &active_profiles[carrier_idx.min(4)];
            
            let hoop_ft = Court::hoop_pos(is_home);
            let defender_id = if is_home { format!("A_{}", carrier_idx + 1) } else { format!("H_{}", carrier_idx + 1) };
            let defender_pos = self.physics.get_player(&defender_id).map(|p| p.pos_ft).unwrap_or(hoop_ft);
            
            let mut teammates: Vec<(String, Vec2)> = Vec::new();
            for i in 0..5 {
                let t_id = if is_home { format!("H_{}", i + 1) } else { format!("A_{}", i + 1) };
                if t_id != carrier_id {
                    if let Some(p) = self.physics.get_player(&t_id) {
                        let jersey = if is_home { &self.home_profiles[i].jersey } else { &self.away_profiles[i].jersey };
                        teammates.push((jersey.clone(), p.pos_ft));
                    }
                }
            }
            
            let mut opponents: Vec<Vec2> = Vec::new();
            for i in 0..5 {
                let o_id = if is_home { format!("A_{}", i + 1) } else { format!("H_{}", i + 1) };
                if let Some(p) = self.physics.get_player(&o_id) {
                    opponents.push(p.pos_ft);
                }
            }

            let decision = TacticalPlanner::evaluate_ballhandler_decision(
                profile,
                self.ball_pos_ft,
                hoop_ft,
                defender_pos,
                &teammates,
                &opponents,
            );

            decision_trace = Some(DecisionTrace {
                actor_jersey: profile.jersey.clone(),
                action: decision.action.clone(),
                reason: decision.reason.clone(),
                target_jersey: decision.target_jersey.clone(),
                shot_openness: Some(decision.shot_openness),
                pass_openness: Some(decision.pass_openness),
                drive_lane_space: Some(decision.drive_lane_space),
            });

            if decision.action == "PASS" {
                if let Some(target_j) = decision.target_jersey {
                    for i in 0..5 {
                        let j = if is_home { &self.home_profiles[i].jersey } else { &self.away_profiles[i].jersey };
                        if j == &target_j {
                            self.ball_carrier = Some(if is_home { format!("H_{}", i + 1) } else { format!("A_{}", i + 1) });
                            event_type = Some("PASS".to_string());
                            callout = Some(format!("{} 果断分球给空位队友 #{}！", profile.name, target_j));
                            intensity = Some(0.65);
                            break;
                        }
                    }
                }
            } else if decision.action == "SHOT" && frame_idx % 60 == 0 {
                let is_made = self.rng.gen_bool(profile.catch_and_shoot_fg as f64);
                if is_made {
                    if is_home { self.home_score += 2; } else { self.away_score += 2; }
                    event_type = Some("MADE_SHOT".to_string());
                    callout = Some(format!("{} 抓住出手窗口干拔命中！(+2分)", profile.name));
                    intensity = Some(0.95);
                } else {
                    event_type = Some("MISSED_SHOT".to_string());
                    callout = Some(format!("{} 强行出手弹框而出，拼抢篮板！", profile.name));
                    intensity = Some(0.7);
                }
            }
        }
        // Extract current positions for dynamic speed planning
        let mut curr_positions = [Vec2::ZERO; 10];
        for i in 0..5 {
            let h_id = format!("H_{}", i + 1);
            let a_id = format!("A_{}", i + 1);
            if let Some(p) = self.physics.get_player(&h_id) {
                curr_positions[i] = p.pos_ft;
            }
            if let Some(p) = self.physics.get_player(&a_id) {
                curr_positions[5 + i] = p.pos_ft;
            }
        }

        // 3. Tactical Target & Adaptive Dynamic Speed Generation
        let (home_targets, away_targets) = TacticalPlanner::plan_segment(
            self.possession,
            self.stage_type,
            self.stage_elapsed,
            self.stage_duration,
            &curr_positions,
            &mut self.rng,
        );

        for (i, (t, speed)) in home_targets.into_iter().enumerate() {
            let pid = format!("H_{}", i + 1);
            let action = if self.ball_carrier.as_deref() == Some(&pid) { "DRIBBLE" } else { "MOVE" };
            self.physics.set_player_target(&pid, t, speed, action);
        }

        for (i, (t, speed)) in away_targets.into_iter().enumerate() {
            let pid = format!("A_{}", i + 1);
            let action = if self.ball_carrier.as_deref() == Some(&pid) { "DRIBBLE" } else { "DEFEND" };
            self.physics.set_player_target(&pid, t, speed, action);
        }

        // 4. Step Physics Pipeline (Rapier2D Multi-body, CCD, Non-overlapping Collision)
        self.physics.step(dt);

        // 5. Update Ball Position (attach to carrier or trajectory)
        if let Some(ref carrier_id) = self.ball_carrier {
            if let Some(p) = self.physics.get_player(carrier_id) {
                self.ball_pos_ft = p.pos_ft;
            }
        }

        self.physics.set_ball_holder(self.ball_carrier.as_deref());
        // 6. Pack StreamTick and Serialize
        let ball_norm = Court::ft_to_norm(self.ball_pos_ft);
        let mut render_players = Vec::with_capacity(10);

        for p in self.physics.get_players().values() {
            let norm_pos = Court::ft_to_norm(p.pos_ft);
            render_players.push(RenderPlayer {
                id: p.id.clone(),
                jersey: p.jersey.clone(),
                team: p.team.clone(),
                x: norm_pos.x,
                y: norm_pos.y,
                vx: p.vel_ft.x,
                vy: p.vel_ft.y,
                action: p.action.clone(),
                zone: Some("frontcourt".to_string()),
                task: Some(p.action.clone()),
                stm: Some(p.stamina),
                stm_max: Some(p.max_stamina),
            });
        }
        // Sort players deterministically by jersey
        render_players.sort_by(|a, b| a.jersey.cmp(&b.jersey));

        let frame = RenderFrame {
            t: (frame_idx as f32) * dt,
            t_game: (self.game_clock * 10.0).round() / 10.0,
            shot_clock: (self.shot_clock * 10.0).round() / 10.0,
            period: self.period,
            phase: "HALF_COURT".to_string(),
            score: RenderScore {
                home: self.home_score,
                away: self.away_score,
            },
            players: render_players,
            ball: RenderBall {
                x: ball_norm.x,
                y: ball_norm.y,
                z: Some(0.0),
                status: "HELD".to_string(),
                holder_id: self.ball_carrier.clone(),
            },
            event_type,
            callout,
            intensity,
            stage_id: Some(self.stage_id),
            stage_type: Some(format!("{:?}", self.stage_type)),
            stage_title: Some(self.stage_title.clone()),
            decision_trace,
        };

        StreamTick {
            frame,
            game_clock: (self.game_clock * 10.0).round() / 10.0,
            keyframe_index: None,
        }
    }

    pub fn simulate_and_export(&mut self, total_ticks: usize, out_path: &str) -> std::io::Result<()> {
        let file = File::create(out_path)?;
        let mut writer = BufWriter::new(file);

        for _ in 0..total_ticks {
            let tick = self.step();
            serde_json::to_writer(&mut writer, &tick)?;
            writer.write_all(b"\n")?;
        }

        writer.flush()?;
        Ok(())
    }
}
