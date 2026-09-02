use nba_engine::MatchEngine;

#[test]
fn bisect_fnv() {
    let mut e = MatchEngine::new(42);
    for tick in 0..16 {
        let t = e.step();
        if tick == 14 {
            for p in &t.frame.players {
                if p.id == "H_1" {
                    println!("id={:?} x={:.8} y={:.8} has_ball={} on_court={} act={:?} stm={:.8} foul={}",
                             p.id, p.x, p.y, p.has_ball, p.on_court, p.action, p.stm, p.foul_count);
                }
            }
        }
    }
}
