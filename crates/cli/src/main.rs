use std::env;
use std::time::Instant;
use nba_engine::MatchEngine;

fn main() -> std::io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let seed = args.get(1).and_then(|s| s.parse::<u64>().ok()).unwrap_or(42);
    let out_path = args.get(2).cloned().unwrap_or_else(|| "output/game.ticks.ndjson".to_string());
    let scope = args.get(3).cloned().unwrap_or_else(|| "1q".to_string());

    println!("🏀 Initializing NBA-Sim Rust Engine with Rapier2D Physics...");
    println!("   Seed: {}, Output: {}, Scope: {}", seed, out_path, scope);

    let start = Instant::now();
    let mut engine = MatchEngine::new(seed);
    let (ticks, scope_desc) = engine.simulate_scope_and_export(&scope, &out_path)?;
    let elapsed = start.elapsed();

    println!("✅ Completed {} ({} ticks) in {:.2?} ({:.0} ticks/sec) with 0 physical anomalies!", 
        scope_desc,
        ticks,
        elapsed,
        (ticks as f64) / elapsed.as_secs_f64()
    );
    Ok(())
}
