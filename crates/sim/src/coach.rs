//! 跨回合教练调整：从可观察的回合事实积累证据，在合法窗口改变安排。
//!
//! 证据只记录两类：对手反复借同一机制得到机会，以及对手靠偶然高难度
//! 命中得分。前者改变防守安排，后者保持原状。证据带时效，逐回合衰减，
//! 阵容或比分变化不会自动被当成同一种机制的重复。

use crate::action::TeamSide;
use crate::action::Fact;
use crate::world::World;

/// 一个回合里可被教练观察到的机会事实。
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct PossessionSignal {
    /// 对手借掩护形成了实际掩护路线。
    pub screen_used: bool,
    /// 对手在本次回合出手。
    pub shot: bool,
    /// 出手距离：高难度命中不视为机制被反复打穿。
    pub shot_distance_ft: f32,
    /// 防守干扰强度。
    pub contest: f32,
    /// 本次得分。
    pub points: u32,
}

/// 单队跨回合证据。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TeamEvidence {
    /// 仍在时效内的同机制机会次数。
    pub repeated_opportunities: u32,
    /// 仍在时效内的高难度命中次数。
    pub difficult_makes: u32,
    /// 最近一次相关回合的仿真时刻。
    pub last_signal_at: f64,
}

/// 一次调整决定。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Adjustment {
    /// 证据不足，保持原安排。
    Keep,
    /// 对手反复借掩护得到机会，改用换防方案。
    SwitchHeavy,
    /// 对手反复借掩护得到机会，改用沉退护框方案。
    DropCoverage,
}

/// 在回合边界登记一份可观察信号。
pub fn observe_possession(
    world: &mut World,
    defense: TeamSide,
    signal: PossessionSignal,
) {
    if !signal.screen_used || !signal.shot {
        return;
    }
    let index = World::team_index(defense);
    let now = world.now();
    let window = f64::from(world.params.policy.g("coach_evidence_window_seconds"));
    let difficult_ft = world.params.policy.g("coach_difficult_shot_ft");
    let contest_gate = world.params.policy.w("coach_contested_threshold");
    let evidence = &mut world.coach_evidence[index];
    if now - evidence.last_signal_at > window {
        evidence.repeated_opportunities = 0;
        evidence.difficult_makes = 0;
    }
    evidence.last_signal_at = now;
    let difficult = difficult_ft <= signal.shot_distance_ft && contest_gate <= signal.contest;
    if signal.points > 0 && difficult {
        evidence.difficult_makes = evidence.difficult_makes.saturating_add(1);
    } else if signal.points > 0 {
        evidence.repeated_opportunities = evidence.repeated_opportunities.saturating_add(1);
    }
}

/// 在合法窗口给出调整。合法窗口是死球或回合刚开始的短暂时间。
pub fn adjust(world: &mut World, defense: TeamSide) -> Adjustment {
    let index = World::team_index(defense);
    let evidence = &world.coach_evidence[index];
    let threshold = world.params.policy.w("coach_adjust_threshold");
    if evidence.repeated_opportunities < threshold as u32 {
        return Adjustment::Keep;
    }
    // 同样的机会若主要靠高难度命中转成得分，证据不足：不据此改防守。
    if evidence.difficult_makes >= evidence.repeated_opportunities {
        return Adjustment::Keep;
    }
    let current = world.coach_schemes[index].clone();
    let evidence_repeated = evidence.repeated_opportunities;
    let evidence_difficult = evidence.difficult_makes;
    let target = if current == "def_switch_heavy" {
        "def_drop_coverage"
    } else {
        "def_switch_heavy"
    };
    let Some(rules) = nba_domain::DefenseRules::for_scheme(target) else {
        return Adjustment::Keep;
    };
    world.defense_schemes[index] = rules;
    world.coach_schemes[index] = target.to_string();
    let decision = if target == "def_switch_heavy" {
        Adjustment::SwitchHeavy
    } else {
        Adjustment::DropCoverage
    };
    world.emit(Fact::CoachAdjustment {
        team: defense,
        evidence_repeated,
        evidence_difficult,
        target: target.to_string(),
    });
    world.coach_evidence[index] = TeamEvidence {
        last_signal_at: world.now(),
        ..TeamEvidence::default()
    };
    decision
}
