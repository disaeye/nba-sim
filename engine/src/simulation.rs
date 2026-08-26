use glam::Vec2;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use std::fs::File;
use std::io::{BufWriter, Write};

use crate::court::Court;
use crate::movement::{PhysicsWorld, PlayerPhysicsState};
use crate::protocol::{RenderBall, RenderFrame, RenderPlayer, RenderScore, StreamTick};
use crate::tactics::{Phase, Possession, TacticalPlanner};

pub struct MatchEngine {
    physics: PhysicsWorld,
    possession: Possession,
    phase: Phase,
    shot_clock: f32,
    game_clock: f32,
    period: u32,
    home_score: u32,
    away_score: u32,
    ball_pos_ft: Vec2,
    ball_carrier: Option<String>,
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

        Self {
            physics,
            possession: Possession::Home,
            phase: Phase::HalfCourtSet,
            shot_clock: 24.0,
            game_clock: 720.0,
            period: 1,
            home_score: 0,
            away_score: 0,
            ball_pos_ft: Vec2::new(30.0, 15.0),
            ball_carrier: Some("H_1".to_string()),
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

        // 2. High-level Basketball Adjudication & Action State Machine
        if self.shot_clock <= 0.2 {
            // 24s Shot Clock Turnover
            self.possession = match self.possession {
                Possession::Home => Possession::Away,
                Possession::Away => Possession::Home,
            };
            self.shot_clock = 24.0;
            self.ball_carrier = match self.possession {
                Possession::Home => Some("H_1".to_string()),
                Possession::Away => Some("A_1".to_string()),
            };
            event_type = Some("TURNOVER".to_string());
            callout = Some("24秒进攻违例，球权转换！".to_string());
            intensity = Some(0.8);
        } else if frame_idx > 0 && frame_idx % 120 == 0 {
            // Shot Attempt & Resolution
            let is_three = self.rng.gen_bool(0.4);
            let is_made = self.rng.gen_bool(0.48);
            let shooter_id = self.ball_carrier.clone().unwrap_or_else(|| "H_1".to_string());
            let shooter_name = match shooter_id.as_str() {
                "H_1" => "Jayson Tatum",
                "H_2" => "Jaylen Brown",
                "H_3" => "Jrue Holiday",
                "H_4" => "Kristaps Porziņģis",
                "H_5" => "Derrick White",
                "A_1" => "LeBron James",
                "A_2" => "Anthony Davis",
                "A_3" => "Austin Reaves",
                "A_4" => "D'Angelo Russell",
                "A_5" => "Rui Hachimura",
                _ => "Shooter",
            };

            if is_made {
                let pts = if is_three { 3 } else { 2 };
                match self.possession {
                    Possession::Home => self.home_score += pts,
                    Possession::Away => self.away_score += pts,
                }
                event_type = Some("MADE_SHOT".to_string());
                callout = Some(format!("{} 迎着防守干拔跳投命中！(+{}分)", shooter_name, pts));
                intensity = Some(0.95);

                // Switch possession after make
                self.possession = match self.possession {
                    Possession::Home => Possession::Away,
                    Possession::Away => Possession::Home,
                };
                self.shot_clock = 24.0;
                self.ball_carrier = match self.possession {
                    Possession::Home => Some("H_1".to_string()),
                    Possession::Away => Some("A_1".to_string()),
                };
            } else {
                event_type = Some("MISSED_SHOT".to_string());
                callout = Some(format!("{} 出手不中，篮下展开激烈卡位拼抢！", shooter_name));
                intensity = Some(0.75);
                self.shot_clock = (self.shot_clock - 5.0).max(4.0);
            }
        } else if frame_idx > 0 && frame_idx % 45 == 0 {
            // Tactical Pass
            let r = self.rng.gen_range(1..=5);
            self.ball_carrier = match self.possession {
                Possession::Home => Some(format!("H_{}", r)),
                Possession::Away => Some(format!("A_{}", r)),
            };
            event_type = Some("PASS".to_string());
            callout = Some("精准战术导球，迅速转移至空位队友".to_string());
            intensity = Some(0.4);
        }
        self.physics.set_ball_holder(self.ball_carrier.as_deref());

        // 3. Tactical Target Generation
        let (home_targets, away_targets) = TacticalPlanner::plan_targets(
            self.possession,
            self.phase,
            self.ball_pos_ft,
            self.shot_clock,
            &mut self.rng,
        );

        for (i, t) in home_targets.into_iter().enumerate() {
            let pid = format!("H_{}", i + 1);
            let speed = if self.ball_carrier.as_deref() == Some(&pid) { 15.0 } else { 12.0 };
            let action = if self.ball_carrier.as_deref() == Some(&pid) { "DRIBBLE" } else { "MOVE" };
            self.physics.set_player_target(&pid, t, speed, action);
        }

        for (i, t) in away_targets.into_iter().enumerate() {
            let pid = format!("A_{}", i + 1);
            let speed = if self.ball_carrier.as_deref() == Some(&pid) { 15.0 } else { 12.0 };
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

        // 6. Pack StreamTick and Serialize
        let ball_norm = Court::ft_to_norm(self.ball_pos_ft);
        let mut render_players = Vec::with_capacity(10);

        for p in self.physics.get_players().values() {
            let norm_pos = Court::ft_to_norm(p.pos_ft);
            render_players.push(RenderPlayer {
                jersey: p.jersey.clone(),
                team: p.team.clone(),
                x: norm_pos.x,
                y: norm_pos.y,
                zone: "frontcourt".to_string(),
                has_ball: p.has_ball,
                action: p.action.clone(),
                stm: p.stamina,
                stm_max: p.max_stamina,
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
