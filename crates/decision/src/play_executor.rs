//! Play 激活期间的纯规则评估与候选族效果汇总。

use nba_domain::play::{DecisionActionFamily, PlayInhibitionMode, PlaySpec, PlayVerb};

use crate::play_selector::{eval_predicate, PlaySelectionContext};

/// 一个动作族当前 Play 产生的效用与抑制效果。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayFamilyEffect {
    pub action_family: DecisionActionFamily,
    pub bonus: f32,
    pub soft_penalty: f32,
    pub hard_inhibited: bool,
}

/// 一条已命中的 Play 规则动作。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayRuleAction {
    pub rule_id: String,
    pub verb: PlayVerb,
    pub slot: String,
}

/// 针对当前激活 Play 与 tick 谓词事实计算出的不可变效果结果。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayExecution {
    play_id: String,
    family_effects: [PlayFamilyEffect; 6],
    matched_rule_actions: Vec<PlayRuleAction>,
}

impl PlayExecution {
    /// 当前激活 Play 的标识。
    pub fn play_id(&self) -> &str {
        &self.play_id
    }

    /// 按 `DecisionActionFamily::ALL` 的稳定顺序返回全部动作族效果。
    pub fn family_effects(&self) -> &[PlayFamilyEffect; 6] {
        &self.family_effects
    }

    /// 读取指定动作族效果。
    pub fn family_effect(&self, action_family: DecisionActionFamily) -> &PlayFamilyEffect {
        self.family_effects
            .iter()
            .find(|effect| effect.action_family == action_family)
            .expect("every DecisionActionFamily must have a PlayFamilyEffect")
    }

    /// 按 PlaySpec 规则声明顺序返回本 tick 已命中的规则动作。
    pub fn matched_rule_actions(&self) -> &[PlayRuleAction] {
        &self.matched_rule_actions
    }
}

/// 对当前激活 Play 按本 tick 谓词事实重新求值。
///
/// 顶层偏好和抑制始终生效；规则动作与规则偏好只在规则谓词全部为真时
/// 生效。偏好和软抑制按动作族累加。非法档案在入口处快速失败。
pub fn evaluate_active_play(spec: &PlaySpec, ctx: &PlaySelectionContext) -> PlayExecution {
    spec.validate()
        .unwrap_or_else(|error| panic!("play `{}` is invalid at executor input: {error}", spec.id));
    // 零值是偏好与软抑制累加的加法单位元。
    let zero = f32::from(0u8);
    let mut family_effects = DecisionActionFamily::ALL.map(|action_family| PlayFamilyEffect {
        action_family,
        bonus: zero,
        soft_penalty: zero,
        hard_inhibited: false,
    });

    for preference in &spec.carrier_preferences {
        assert_effect_value(
            spec,
            "carrier_preferences",
            preference.action_family,
            "bonus",
            preference.bonus,
        );
        let effect = family_effect_mut(&mut family_effects, preference.action_family);
        effect.bonus += preference.bonus;
        assert!(
            effect.bonus.is_finite(),
            "play `{}` bonus sum for family {} is not finite",
            spec.id,
            preference.action_family.as_str()
        );
    }

    for inhibition in &spec.inhibitions {
        let effect = family_effect_mut(&mut family_effects, inhibition.action_family);
        match inhibition.mode {
            PlayInhibitionMode::Hard => effect.hard_inhibited = true,
            PlayInhibitionMode::Soft { penalty } => {
                assert_effect_value(
                    spec,
                    "inhibitions",
                    inhibition.action_family,
                    "penalty",
                    penalty,
                );
                effect.soft_penalty += penalty;
                assert!(
                    effect.soft_penalty.is_finite(),
                    "play `{}` soft penalty sum for family {} is not finite",
                    spec.id,
                    inhibition.action_family.as_str()
                );
            }
        }
    }

    let mut matched_rule_actions = Vec::new();
    for rule in &spec.rules {
        for preference in &rule.carrier_preferences {
            assert_effect_value(
                spec,
                "rules[].carrier_preferences",
                preference.action_family,
                "bonus",
                preference.bonus,
            );
        }
        if rule
            .when
            .iter()
            .all(|predicate| eval_predicate(predicate, ctx))
        {
            matched_rule_actions.push(PlayRuleAction {
                rule_id: rule.id.clone(),
                verb: rule.then.verb,
                slot: rule.then.slot.clone(),
            });
            for preference in &rule.carrier_preferences {
                let effect = family_effect_mut(&mut family_effects, preference.action_family);
                effect.bonus += preference.bonus;
                assert!(
                    effect.bonus.is_finite(),
                    "play `{}` bonus sum for family {} is not finite",
                    spec.id,
                    preference.action_family.as_str()
                );
            }
        }
    }

    PlayExecution {
        play_id: spec.id.clone(),
        family_effects,
        matched_rule_actions,
    }
}

fn family_effect_mut(
    effects: &mut [PlayFamilyEffect; 6],
    action_family: DecisionActionFamily,
) -> &mut PlayFamilyEffect {
    effects
        .iter_mut()
        .find(|effect| effect.action_family == action_family)
        .expect("every DecisionActionFamily must have a PlayFamilyEffect")
}

fn assert_effect_value(
    spec: &PlaySpec,
    section: &str,
    action_family: DecisionActionFamily,
    field: &str,
    value: f32,
) {
    // PlaySpec 将 bonus 与 penalty 的合法下限定义为零。
    assert!(
        value.is_finite() && value >= f32::from(0u8),
        "play `{}` {} family {} has invalid {} value {}",
        spec.id,
        section,
        action_family.as_str(),
        field,
        value
    );
}
