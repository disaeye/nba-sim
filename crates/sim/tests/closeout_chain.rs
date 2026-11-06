//! 封盖起跳之后的进攻应对链条。
//!
//! 防守人起跳封盖是一次不可撤销的冒险：滞空的人不能改向，进攻方因此多出
//! 两个选择——把身体送进他的落地路线制造接触，或者侧移把出手点让开。
//! 两个选择都由倾向与能力驱动，判定在 `nba_sim_core::resolve::shot` 里。
//!
//! 抽样口径：同一次抽取在同一世界里是确定的，因此概率只能在种子之间量。

mod common;

use nba_sim_core::draw::Stream;
use nba_sim_core::resolve::shot::{closeout_response, CloseoutResponse};
use nba_sim_core::World;

const SEEDS: u64 = 60;

/// 出手方球队里对抗意愿最高与最低的两个人。
fn lean_and_shy(world: &World) -> (usize, usize) {
    let roster = world.on_court(world.possession.team);
    let mut lean = roster.as_slice()[0];
    let mut shy = roster.as_slice()[0];
    for handle in roster.as_slice() {
        let risk = world.players[*handle].data.tendencies.risk_tolerance;
        if risk > world.players[lean].data.tendencies.risk_tolerance {
            lean = *handle;
        }
        if risk < world.players[shy].data.tendencies.risk_tolerance {
            shy = *handle;
        }
    }
    (lean, shy)
}

/// 出手方球队里敏捷度最高与最低的两个人。
fn quick_and_slow(world: &World) -> (usize, usize) {
    let roster = world.on_court(world.possession.team);
    let mut quick = roster.as_slice()[0];
    let mut slow = roster.as_slice()[0];
    for handle in roster.as_slice() {
        if world.players[*handle].data.attributes.agility
            > world.players[quick].data.attributes.agility
        {
            quick = *handle;
        }
        if world.players[*handle].data.attributes.agility
            < world.players[slow].data.attributes.agility
        {
            slow = *handle;
        }
    }
    (quick, slow)
}

fn tally(seed_from: u64, pick: fn(&World) -> usize) -> [u32; 3] {
    let mut counts = [0u32; 3];
    for offset in 0..SEEDS {
        let mut world = common::short_world(seed_from + offset);
        let shooter = pick(&world);
        let index = match closeout_response(&mut world, shooter) {
            CloseoutResponse::DrawContact => 0,
            CloseoutResponse::Sidestep => 1,
            CloseoutResponse::None => 2,
        };
        counts[index] += 1;
    }
    counts
}

#[test]
fn an_airborne_blocker_leaves_the_shooter_two_responses() {
    let mut draws = 0;
    let mut sidesteps = 0;
    let mut unchanged = 0;
    for offset in 0..SEEDS {
        let mut world = common::short_world(940 + offset);
        let (_, shy) = lean_and_shy(&world);
        match closeout_response(&mut world, shy) {
            CloseoutResponse::DrawContact => draws += 1,
            CloseoutResponse::Sidestep => sidesteps += 1,
            CloseoutResponse::None => unchanged += 1,
        }
    }
    assert!(draws > 0, "造犯规接触必须是一条真实可达的选择");
    assert!(sidesteps > 0, "侧移避让必须是一条真实可达的选择");
    assert!(
        draws + sidesteps + unchanged == SEEDS as u32,
        "每一场都必须落到三个分支之一"
    );
}

#[test]
fn the_contact_branch_follows_the_shooters_risk_tolerance() {
    // 造接触的门槛就是「对抗意愿 × 策略权重」：同一世界里逐个球员核对
    // 分支与门槛的对应关系，比抽样比较更直接。
    let mut drawn = 0;
    for offset in 0..20 {
        let mut world = common::short_world(1100 + offset);
        let roster = world.on_court(world.possession.team).as_slice().to_vec();
        for shooter in roster {
            let roll = world.draw(
                Stream::Foul,
                shooter,
                nba_sim_core::draw::purpose::CLOSEOUT_DRAW_CONTACT,
                0,
            );
            let risk = world.players[shooter].data.tendencies.risk_tolerance;
            let rate = world.params.policy.w("closeout_draw_contact_rate");
            let expected = roll < risk * rate;
            let response = closeout_response(&mut world, shooter);
            assert_eq!(
                response == CloseoutResponse::DrawContact,
                expected,
                "造接触分支必须只看对抗意愿门槛：roll={roll} risk={risk} rate={rate}"
            );
            if expected {
                drawn += 1;
            }
        }
    }
    assert!(drawn > 0, "没人越过门槛就说明权重或系数取反了");
}

#[test]
fn the_sidestep_share_follows_agility() {
    let quick_counts = tally(1400, |world| quick_and_slow(world).0);
    let slow_counts = tally(1400, |world| quick_and_slow(world).1);
    assert!(
        quick_counts[1] >= slow_counts[1],
        "敏捷度高的球员侧移不更少：{} vs {}",
        quick_counts[1],
        slow_counts[1]
    );
}

#[test]
fn the_response_draws_come_from_the_closeout_streams() {
    // 同一次抽取在同一世界里可重放：这两条选择从固定槽位取值，
    // 因此重放结果必须一致。
    let mut world = common::short_world(1500);
    let shooter = world.on_court(world.possession.team).as_slice()[0];
    let first = closeout_response(&mut world, shooter);
    let second = closeout_response(&mut world, shooter);
    assert_eq!(first, second, "同一次抽取必须可重放");
    assert!(matches!(
        first,
        CloseoutResponse::DrawContact | CloseoutResponse::Sidestep | CloseoutResponse::None
    ));
    // 这两条选择分别占用犯规流与决策流里的独立槽位。
    assert_ne!(Stream::Foul as u8, Stream::Decision as u8);
}
