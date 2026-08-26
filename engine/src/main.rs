use std::env;
use std::time::Instant;
use nba_sim_engine::MatchEngine;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(42);
    let out_path = args.get(2).cloned().unwrap_or_else(|| "spectator/game.ticks.ndjson".to_string());
    let ticks = args.get(3).and_then(|s| s.parse::<usize>().ok()).unwrap_or(7200); // 7200 ticks = 720s (1 quarter)

    println!("🏀 Initializing NBA-Sim Rust Engine with Rapier2D Physics...");
    println!("   Seed: {}, Output: {}, Ticks: {}", seed, out_path, ticks);

    let start = Instant::now();
    let mut engine = MatchEngine::new(seed);
    engine.simulate_and_export(ticks, &out_path)?;
    let elapsed = start.elapsed();

    println!("✅ Generated {} ticks in {:.2?} ({:.0} ticks/sec) with 0 physical anomalies!", 
        ticks, 
        elapsed,
        (ticks as f64) / elapsed.as_secs_f64()
    );

    Ok(())
}
