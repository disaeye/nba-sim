//! 动作规格表：把 111 个动作条目接到生产路径上。
//!
//! 方案 §6：每个动作条目有种类、技术细分、执行人、对象、准备窗口、执行
//! 窗口、恢复窗口、控制要求、运动约束和取消条件；技术细分参与可执行性、
//! 时长、球和身体路线、防守反应、失误风险、体能与接触条件。
//!
//! 这张表是目录与运行时之间的唯一接口：每一条都写明它的生产候选来源、
//! 生产结算器与对手响应。渲染器从表里读配方，验证器从表里读事件，守卫
//! 从表里读「这一条有没有实现」。

use nba_domain::action_spec::{ActionFamily, ActionRole, ActionSpec, ActionSpecTable};

/// 动作动画目录：动作条目有没有阶段窗口由它给出。
///
/// 窗口不是手工标注的事实：目录里的阶段列表是渲染配方的一部分，规格
/// 表读它，因此两者不可能对不上。
const ANIMATION_CATALOG: &str = include_str!("../../../data/rules/action_animation_catalog.json");

/// 动画目录里的一个动作条目。
#[derive(Debug, Clone, serde::Deserialize)]
struct AnimationEntry {
    name: String,
    kind: String,
    variant: String,
    role: String,
    phases: Vec<String>,
}

/// 动画目录里的一个动作族。
#[derive(Debug, Clone, serde::Deserialize)]
struct AnimationFamily {
    id: String,
    actions: Vec<AnimationEntry>,
}

/// 动画目录的顶层结构。
#[derive(Debug, Clone, serde::Deserialize)]
struct AnimationCatalog {
    families: Vec<AnimationFamily>,
}

/// 动画目录读出来的表：按 (族, 名字) 查。
fn animation_catalog() -> Vec<(String, AnimationEntry)> {
    let catalog: AnimationCatalog =
        serde_json::from_str(ANIMATION_CATALOG).expect("action animation catalog parses");
    catalog
        .families
        .into_iter()
        .flat_map(|family| {
            let family_id = family.id;
            family
                .actions
                .into_iter()
                .map(move |entry| (family_id.clone(), entry))
        })
        .collect()
}

/// 持球动作的候选来源与结算器。
const DRIBBLE_SOURCE: &str = "crates/sim/src/policy.rs::hold_candidate";
const DRIBBLE_RESOLVER: &str = "crates/sim/src/resolve/mod.rs::resolve_dribble";
const DRIVE_SOURCE: &str = "crates/sim/src/policy.rs::drive_candidate";
const DRIVE_RESOLVER: &str = "crates/sim/src/resolve/drive.rs::resolve";
const FINISH_SOURCE: &str = "crates/sim/src/policy.rs::finish_candidate";
const FINISH_RESOLVER: &str = "crates/sim/src/resolve/drive.rs::resolve_finish";
const SHOT_SOURCE: &str = "crates/sim/src/policy.rs::shot_candidate";
const SHOT_RESOLVER: &str = "crates/sim/src/resolve/shot.rs::resolve";
const POST_SOURCE: &str = "crates/sim/src/policy.rs::post_candidate";
const POST_RESOLVER: &str = "crates/sim/src/resolve/drive.rs::resolve_post";
const PASS_SOURCE: &str = "crates/sim/src/policy.rs::pass_candidate";
const PASS_RESOLVER: &str = "crates/sim/src/resolve/pass.rs::resolve";
const FAKE_SOURCE: &str = "crates/sim/src/policy.rs::fake_candidate";
const FAKE_RESOLVER: &str = "crates/sim/src/resolve/fake.rs::resolve";
const PROTECT_SOURCE: &str = "crates/sim/src/policy.rs::protect_candidate";
const PROTECT_RESOLVER: &str = "crates/sim/src/resolve/protect.rs::resolve";
const SCREEN_SOURCE: &str = "crates/sim/src/policy.rs::screen_candidate";
const SCREEN_RESOLVER: &str = "crates/sim/src/resolve/screen.rs::resolve";
const CUT_SOURCE: &str = "crates/sim/src/policy.rs::cut_candidate";
const CUT_RESOLVER: &str = "crates/sim/src/resolve/offball.rs::cut_route";
const RELOCATE_SOURCE: &str = "crates/sim/src/policy.rs::off_ball_decision";
const RELOCATE_RESOLVER: &str = "crates/sim/src/resolve/mod.rs::resolve";
const ON_BALL_DEFENSE_SOURCE: &str = "crates/sim/src/defense.rs::apply_defense_targets";
const ON_BALL_DEFENSE_RESOLVER: &str = "crates/sim/src/world.rs::advance_defensive_actions";
const OFF_BALL_DEFENSE_SOURCE: &str = "crates/sim/src/assignment.rs::assign_defense_responsibilities";
const OFF_BALL_DEFENSE_RESOLVER: &str = "crates/sim/src/defense.rs::apply_defense_targets";
const CLOSEOUT_SOURCE: &str = "crates/sim/src/world.rs::advance_defensive_actions";
const CLOSEOUT_RESOLVER: &str = "crates/sim/src/world.rs::advance_block_jumps";
const REBOUND_SOURCE: &str = "crates/sim/src/resolve/rebound.rs::resolve";
const REBOUND_RESOLVER: &str = "crates/sim/src/resolve/rebound.rs::resolve";
const TRANSITION_SOURCE: &str = "crates/sim/src/world.rs::start_possession";
const TRANSITION_RESOLVER: &str = "crates/sim/src/resolve/mod.rs::resolve";

/// 动作规格表。
pub fn action_spec_table() -> ActionSpecTable {
    let mut specs: Vec<ActionSpec> = Vec::new();
    let catalog = animation_catalog();
    let mut push = |family: ActionFamily,
                    name: &'static str,
                    kind: &'static str,
                    variant: &'static str,
                    role: ActionRole,
                    candidate_source: &'static str,
                    resolver: &'static str,
                    opponent_response: &'static str,
                    scenario_test: &'static str| {
        // 阶段窗口由动画目录给出：目录里的阶段列表是渲染配方的一部分，
        // 规格表读它，两者不可能对不上。
        let entry = catalog
            .iter()
            .find(|(family_id, entry)| family_id == family.as_str() && entry.name == name)
            .unwrap_or_else(|| panic!("动作规格表有动画目录里没有的条目：{name}"));
        let entry = &entry.1;
        assert_eq!(entry.kind, kind, "动作 {name} 的类别与动画目录不一致");
        assert_eq!(entry.variant, variant, "动作 {name} 的变体与动画目录不一致");
        assert_eq!(entry.role, role.as_str(), "动作 {name} 的角色与动画目录不一致");
        let windowed = entry.phases.iter().any(|phase| phase == "execution");
        specs.push(ActionSpec {
            family,
            name,
            kind,
            variant,
            role,
            windowed,
            candidate_source,
            resolver,
            opponent_response,
            scenario_test,
        });
    };

    // 运球与持球控制：技术细分由防守间隔与突破走廊决定。
    push(ActionFamily::ActionDribble, "原地运球", "dribble", "direct_drive", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人贴身，伸手成功率按球的暴露程度折减", "crates/sim/tests/dribble_technique.rs::direct_drive_keeps_the_geometric_exposure");
    push(ActionFamily::ActionDribble, "推进运球", "dribble", "advance", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人退一步保持间隔", "crates/sim/tests/dribble_technique.rs::direct_drive_keeps_the_geometric_exposure");
    push(ActionFamily::ActionDribble, "体前变向", "dribble", "crossover", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人被换向甩开，伸手窗口缩短", "crates/sim/tests/dribble_technique.rs::crossover_moves_the_holder_and_reduces_the_poke_exposure");
    push(ActionFamily::ActionDribble, "胯下运球", "dribble", "between_the_legs", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人被换向甩开，球暴露时间短", "crates/sim/tests/dribble_technique.rs::crossover_moves_the_holder_and_reduces_the_poke_exposure");
    push(ActionFamily::ActionDribble, "背后运球", "dribble", "behind_the_back", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人被换向甩开，球藏在身体另一侧", "crates/sim/tests/dribble_technique.rs::crossover_moves_the_holder_and_reduces_the_poke_exposure");
    push(ActionFamily::ActionDribble, "转身运球", "dribble", "spin_move", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人被转身甩开，球护在身体侧面", "crates/sim/tests/dribble_technique.rs::crossover_moves_the_holder_and_reduces_the_poke_exposure");
    push(ActionFamily::ActionDribble, "迟疑步", "dribble", "hesitation", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人的伸手时机被冻结", "crates/sim/tests/dribble_technique.rs::hesitation_freezes_the_defender_and_lowers_the_poke_success");
    push(ActionFamily::ActionDribble, "后撤步运球", "dribble", "retreat", ActionRole::Primary, DRIBBLE_SOURCE, DRIBBLE_RESOLVER, "防守人被拉开一个身位", "crates/sim/tests/dribble_technique.rs::crossover_moves_the_holder_and_reduces_the_poke_exposure");

    // 突破：变体由防守人站位与弱侧空位决定。
    push(ActionFamily::ActionDrive, "直接突破", "drive", "straight", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "防守人站定吃犯规或被过掉", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionDrive, "变向突破", "drive", "crossover", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "防守人绕外侧，过掉的机会更大", "crates/sim/tests/action_scenarios.rs::a_charge_requires_a_planted_defender");
    push(ActionFamily::ActionDrive, "转身突破", "drive", "spin", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "防守人被背过身，丢球风险更小", "crates/sim/tests/action_scenarios.rs::a_charge_requires_a_planted_defender");
    push(ActionFamily::ActionDrive, "急停", "drive", "pull_up_stop", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "防守人跟到位上，投篮干扰增加", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionDrive, "后撤步", "drive", "step_back", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "防守人被带出原位，重获空间", "crates/sim/tests/action_scenarios.rs::a_step_back_shot_creates_space_before_the_release");
    push(ActionFamily::ActionDrive, "突破分球", "drive", "drive_kick", ActionRole::Primary, DRIVE_SOURCE, DRIVE_RESOLVER, "弱侧防守人补位，被分球的队友获得空位", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");

    // 收球与终结：出手类型由阶段与距离决定。
    push(ActionFamily::ActionFinish, "普通上篮", "finish", "layup", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人起跳干扰，出手点被压低", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "反手上篮", "finish", "reverse_layup", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人被换向，干扰落空", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "拉杆上篮", "finish", "double_clutch_layup", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人已经下落，第二次打板无人干扰", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "挑篮", "finish", "finger_roll", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人伸手高度够不到球", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "抛投", "finish", "floater", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人出手点够不到，干扰随距离下降", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "扣篮", "finish", "dunk", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "护框人来不及起跳，干扰为零", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "补篮", "finish", "putback", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "防守篮板人必须先卡位，补篮窗口极短", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");
    push(ActionFamily::ActionFinish, "跳步收球", "finish", "pro_hop", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "收球步的脚步接触被记账，脚步用尽即走步", "crates/sim/tests/action_scenarios.rs::a_third_step_after_the_gather_is_a_travel");
    push(ActionFamily::ActionFinish, "欧洲步", "finish", "euro_step", ActionRole::Primary, FINISH_SOURCE, FINISH_RESOLVER, "挡路防守人被绕开，但第二步仍在脚步额度内", "crates/sim/tests/action_scenarios.rs::a_contested_lane_finish_takes_a_euro_step_around_the_defender");

    // 跳投：出手类型由阶段与区域决定。
    push(ActionFamily::ActionShot, "接球就投", "jump_shot", "catch_shoot", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人来不及建立位置，干扰较低", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "急停跳投", "jump_shot", "pull_up", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人扑上来干扰，起跳距离变长", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "后仰跳投", "jump_shot", "fadeaway", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人扑空，干扰按后仰折减", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "后撤步跳投", "jump_shot", "step_back", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人被拉开一个身位，干扰降低", "crates/sim/tests/action_scenarios.rs::a_step_back_shot_creates_space_before_the_release");
    push(ActionFamily::ActionShot, "转身跳投", "jump_shot", "turnaround", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人只能从侧面干扰", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "勾手", "jump_shot", "hook", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人伸手够不到勾手高点", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "三分出手", "jump_shot", "three_point", ActionRole::Primary, SHOT_SOURCE, SHOT_RESOLVER, "防守人扑到三分线，出手距离变长", "crates/sim/tests/shot_scenarios.rs::a_shot_from_inside_the_arc_is_worth_two_points");
    push(ActionFamily::ActionShot, "罚球", "jump_shot", "free_throw", ActionRole::Primary, "crates/sim/src/resolve/freethrow.rs::shoot", "crates/sim/src/resolve/freethrow.rs::finish", "防守人不得干扰，禁区违例另行判定", "crates/sim/tests/free_throw_scenarios.rs::a_player_outside_the_lane_does_not_trigger_a_violation");

    // 背身：步法由防守人站位决定。
    push(ActionFamily::ActionPost, "背身要位", "post_up", "post_up", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人站定守位，错位时进攻方占便宜", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "背身挤打", "post_up", "backdown", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人被力量挤开", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "前转身", "post_up", "front_turn", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人跟到底线一侧", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "后转身", "post_up", "reverse_turn", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人跟到中路一侧", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "撤步转身", "post_up", "drop_step", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人被挤到底线的另一侧", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "上下步", "post_up", "up_and_under", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人被顶到身后", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "勾手", "post_up", "hook", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人伸手够不到勾手高点", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");
    push(ActionFamily::ActionPost, "背身后仰", "post_up", "turnaround_fadeaway", ActionRole::Primary, POST_SOURCE, POST_RESOLVER, "防守人只能从侧面干扰", "crates/sim/tests/action_scenarios.rs::a_stronger_backdown_gets_closer_to_the_rim");

    // 传球：技术细分由路线与接球人状态决定。
    push(ActionFamily::ActionPass, "胸前传球", "pass", "chest", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人伸手截球，完成概率按距离折减", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "击地传球", "pass", "bounce", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人扑抢落点，触地后才反弹", "crates/sim/tests/boundary_scenarios.rs::a_ball_touched_by_the_defence_out_of_bounds_keeps_the_offence_in_possession");
    push(ActionFamily::ActionPass, "过顶传球", "pass", "overhead", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人伸手够不到，传球路线抬高", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "单手推传", "pass", "one_hand", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人追不上球速，但路线难控", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "背后传球", "pass", "behind_back", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人看不到出手点，完成概率下降", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "高吊传球", "pass", "lob", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人站在线上就能碰到，落点固定", "crates/sim/tests/boundary_scenarios.rs::a_ball_touched_by_the_defence_out_of_bounds_keeps_the_offence_in_possession");
    push(ActionFamily::ActionPass, "快攻长传", "pass", "outlet", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人回防追不上，接球人获得快攻", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "手递手传球", "pass", "handoff", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "防守人扑不到，两人必须贴近", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");
    push(ActionFamily::ActionPass, "空中接力传球", "pass", "alley_oop", ActionRole::Primary, PASS_SOURCE, PASS_RESOLVER, "接球人在空中没有落点，防守人只能看着", "crates/sim/tests/pass_technique.rs::each_pass_technique_changes_the_completion_probability");

    // 假动作：骗起防守人，随后他的干扰在失衡窗口内折减。
    push(ActionFamily::ActionFake, "投篮假动作", "fake", "shot_fake", ActionRole::Primary, FAKE_SOURCE, FAKE_RESOLVER, "防守人重心被骗起，干扰折减", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionFake, "传球假动作", "fake", "pass_fake", ActionRole::Primary, FAKE_SOURCE, FAKE_RESOLVER, "防守人扑向假目标，传球线路变宽", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionFake, "试探步", "fake", "jab_step", ActionRole::Primary, FAKE_SOURCE, FAKE_RESOLVER, "防守人重心前倾半步", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionFake, "迟疑步", "fake", "hesitation", ActionRole::Primary, FAKE_SOURCE, FAKE_RESOLVER, "防守人的伸手时机被冻结", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");

    // 护球：窗口内防守人的切球成功率折减。
    push(ActionFamily::ActionBallProtection, "面框护球", "protect", "face_up_protect", ActionRole::Primary, PROTECT_SOURCE, PROTECT_RESOLVER, "防守人伸手够不到球，切球成功率折减", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionBallProtection, "背身护球", "protect", "post_protect", ActionRole::Primary, PROTECT_SOURCE, PROTECT_RESOLVER, "防守人被身体宽度隔开，切球成功率折减", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionBallProtection, "转身护球", "protect", "spin_protect", ActionRole::Primary, PROTECT_SOURCE, PROTECT_RESOLVER, "球被带到远离防守的一侧，切球成功率折减", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");
    push(ActionFamily::ActionBallProtection, "被包夹护球", "protect", "trap_protect", ActionRole::Primary, PROTECT_SOURCE, PROTECT_RESOLVER, "两名防守人夹击，切球成功率折减最大", "crates/sim/tests/action_scenarios.rs::fakes_and_protection_keep_the_pivot_foot_planted");

    // 掩护：建立与利用是不同参与者的动作。
    push(ActionFamily::ActionScreen, "持球掩护", "screen", "on_ball", ActionRole::Screener, SCREEN_SOURCE, SCREEN_RESOLVER, "防守人选择钻下沿或挤过掩护", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionScreen, "无球掩护", "screen", "off_ball", ActionRole::Screener, SCREEN_SOURCE, SCREEN_RESOLVER, "防守人被掩护卡住，切入人获得空位", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionScreen, "顺下", "screen", "roll", ActionRole::Screener, SCREEN_SOURCE, SCREEN_RESOLVER, "防守人跟顺下者，持球人得到空间", "crates/sim/tests/action_scenarios.rs::a_screen_whose_defender_left_the_court_fails_instead_of_crashing");
    push(ActionFamily::ActionScreen, "外弹", "screen", "pop", ActionRole::Screener, SCREEN_SOURCE, SCREEN_RESOLVER, "防守人跟外弹者，掩护人与持球人之间空出一步", "crates/sim/tests/action_scenarios.rs::a_screen_whose_defender_left_the_court_fails_instead_of_crashing");
    push(ActionFamily::ActionScreen, "提前滑出", "screen", "slip", ActionRole::Screener, SCREEN_SOURCE, SCREEN_RESOLVER, "防守人被晃过，掩护人已经不在原位", "crates/sim/tests/action_scenarios.rs::a_screen_whose_defender_left_the_court_fails_instead_of_crashing");
    push(ActionFamily::ActionScreen, "绕掩护接球", "screen", "curl", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人被掩护卡住，接球人获得空位", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionScreen, "手递手接球", "screen", "handoff", ActionRole::CutPartner, PASS_SOURCE, PASS_RESOLVER, "防守人扑不到交接球", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");

    // 空切：每条路线有自己的落点与到达条件。
    push(ActionFamily::ActionCut, "空切", "cut", "basket", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人跟丢，接球人获得篮下空位", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "背切", "cut", "backdoor", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人被卡在身后", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "反跑", "cut", "give_and_go", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人跟着球走，反跑得到空位", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "V 字跑位", "cut", "v_cut", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人被引到球侧", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "上提接球", "cut", "pop_up", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人跟到外线", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "闪切", "cut", "flash", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人换防来不及", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "拉开站位", "cut", "spot_up", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "防守人被带出禁区", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");
    push(ActionFamily::ActionCut, "底角转移", "cut", "corner_relocate", ActionRole::CutPartner, CUT_SOURCE, CUT_RESOLVER, "弱侧防守人轮转补位", "crates/sim/tests/action_scenarios.rs::cut_pass_aims_at_the_cutters_planned_catch_point");

    // 跑位：位置调整，防守人随对位人移动。
    push(ActionFamily::ActionRelocate, "接球点调整", "relocate", "adjust", ActionRole::Primary, RELOCATE_SOURCE, RELOCATE_RESOLVER, "防守人跟到新的接球点", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");
    push(ActionFamily::ActionRelocate, "外线定点", "relocate", "spot_up", ActionRole::Primary, RELOCATE_SOURCE, RELOCATE_RESOLVER, "防守人跟到外线", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");
    push(ActionFamily::ActionRelocate, "无球绕行", "relocate", "flare", ActionRole::Primary, RELOCATE_SOURCE, RELOCATE_RESOLVER, "防守人被绕开", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");
    push(ActionFamily::ActionRelocate, "错位后要位", "relocate", "mismatch_seal", ActionRole::Primary, RELOCATE_SOURCE, RELOCATE_RESOLVER, "错位时防守人贴上来要位", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");

    // 对位防守：每一次站位与赌博都有后果。
    push(ActionFamily::ActionDefenseOnBall, "防守滑步", "on_ball_defense", "slide", ActionRole::OnBallDefender, ON_BALL_DEFENSE_SOURCE, ON_BALL_DEFENSE_RESOLVER, "持球人必须改变方向，突破机会受限", "crates/sim/tests/action_scenarios.rs::a_defender_out_of_position_recovers_to_his_man");
    push(ActionFamily::ActionDefenseOnBall, "贴身施压", "on_ball_defense", "contain", ActionRole::OnBallDefender, ON_BALL_DEFENSE_SOURCE, ON_BALL_DEFENSE_RESOLVER, "持球人过掉时防守人失位", "crates/sim/tests/action_scenarios.rs::a_defender_out_of_position_recovers_to_his_man");
    push(ActionFamily::ActionDefenseOnBall, "追防", "on_ball_defense", "chase", ActionRole::OnBallDefender, ON_BALL_DEFENSE_SOURCE, ON_BALL_DEFENSE_RESOLVER, "持球人甩开时追上，干扰恢复", "crates/sim/tests/action_scenarios.rs::a_defender_out_of_position_recovers_to_his_man");
    push(ActionFamily::ActionDefenseOnBall, "抢断尝试", "on_ball_defense", "steal_attempt", ActionRole::Defender, "crates/sim/src/world.rs::start_defensive_steal", ON_BALL_DEFENSE_RESOLVER, "成功则球权转换，失败则失位", "crates/sim/tests/action_scenarios.rs::a_steal_hands_possession_over_and_starts_a_quick_transition");
    push(ActionFamily::ActionDefenseOnBall, "掏球", "on_ball_defense", "poke", ActionRole::Defender, "crates/sim/src/resolve/poke.rs::poke_success", ON_BALL_DEFENSE_RESOLVER, "成功则球被打掉，护球姿态可以化解", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");
    push(ActionFamily::ActionDefenseOnBall, "造带球撞人", "on_ball_defense", "charge_take", ActionRole::Defender, ON_BALL_DEFENSE_SOURCE, ON_BALL_DEFENSE_RESOLVER, "持球人撞上被判进攻犯规", "crates/sim/tests/action_scenarios.rs::a_charge_requires_a_planted_defender");

    // 无球防守：职责是这一名防守人当前在做的事。
    push(ActionFamily::ActionDefenseOffBall, "拒绝接球", "off_ball_defense", "deny", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "接球人被迫远离传球路线", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "传球路线拦截", "off_ball_defense", "intercept", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "传球路线被切断，完成概率下降", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "协防", "off_ball_defense", "help", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "协防人离开自己的对位人，空位出现", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "补位", "off_ball_defense", "rotate", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "轮转补位的人接管空当", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "回位", "off_ball_defense", "recover", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "回位的人恢复对自己的对位", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "夹击", "off_ball_defense", "double_team", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "持球人被包夹，出手与传球都受限", "crates/sim/tests/action_scenarios.rs::a_trap_raises_poke_risk_and_protection_helps_the_exit");
    push(ActionFamily::ActionDefenseOffBall, "换防", "off_ball_defense", "switch", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "换防后两名防守人对位互换", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "延误", "off_ball_defense", "hedge", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "延误的防守人先碰持球人再回位", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "沉退", "off_ball_defense", "drop", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "沉退的防守人让出外线，赌投篮不进", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "挤过掩护", "off_ball_defense", "go_over", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "持球人被延误一下", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "绕过掩护", "off_ball_defense", "go_under", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "持球人被迫往外线", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");
    push(ActionFamily::ActionDefenseOffBall, "护框", "off_ball_defense", "rim_protect", ActionRole::Defender, OFF_BALL_DEFENSE_SOURCE, OFF_BALL_DEFENSE_RESOLVER, "护框人站在篮筐附近，干扰篮下出手", "crates/sim/tests/screen_route.rs::screen_route_reaches_the_next_decision");

    // 扑防：出手那一刻的对抗。
    push(ActionFamily::ActionCloseout, "扑防干扰", "closeout", "closeout_contest", ActionRole::Defender, CLOSEOUT_SOURCE, CLOSEOUT_RESOLVER, "投篮人被迫提前出手或改方向", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionCloseout, "垂直起跳", "closeout", "verticality", ActionRole::Defender, CLOSEOUT_SOURCE, CLOSEOUT_RESOLVER, "投篮人的出手点被抬高，封盖窗口打开", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionCloseout, "封盖尝试", "closeout", "block_jump", ActionRole::Defender, CLOSEOUT_SOURCE, CLOSEOUT_RESOLVER, "球被拍下则记封盖，落地时不能卡位", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionCloseout, "追身封盖", "closeout", "chase_down_block", ActionRole::Defender, CLOSEOUT_SOURCE, CLOSEOUT_RESOLVER, "落后半个身位的防守人追上并封盖", "crates/sim/tests/action_scenarios.rs::a_defender_in_the_lane_denies_the_pass");
    push(ActionFamily::ActionCloseout, "造犯规接触", "closeout_response", "draw_contact", ActionRole::Shooter, "crates/sim/src/resolve/shot.rs::closeout_response", SHOT_RESOLVER, "防守人身体被送进落地路线，犯规几乎必然发生", "crates/sim/tests/foul_scenarios.rs::a_shooting_foul_on_a_two_point_shot_awards_two_free_throws");
    push(ActionFamily::ActionCloseout, "侧移避让", "closeout_response", "sidestep", ActionRole::Shooter, "crates/sim/src/resolve/shot.rs::closeout_response", SHOT_RESOLVER, "防守人扑空，投篮姿势被拉歪", "crates/sim/tests/foul_scenarios.rs::a_shooting_foul_on_a_two_point_shot_awards_two_free_throws");

    // 篮板：归属只在实际控制成立时确定。
    push(ActionFamily::ActionRebound, "卡位", "rebound", "box_out", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "进攻人被挤在身后", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "起跳争抢", "rebound", "jump_rebound", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "两人同时起跳，归属由身体位置决定", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "控制篮板", "rebound", "secure_rebound", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "控制成立的一方获得球权", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "点球", "rebound", "tap", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "点球未控制，继续争抢", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "点传", "rebound", "tip_pass", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "点传把球拨给队友", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "补篮", "rebound", "tip_in", ActionRole::Rebounder, REBOUND_SOURCE, REBOUND_RESOLVER, "补篮直接进攻，防守人来不及落位", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "地板球争抢", "rebound", "loose_ball_dive", ActionRole::Primary, REBOUND_SOURCE, REBOUND_RESOLVER, "两人都趴下，控制先成立的一方拿到球", "crates/sim/tests/action_scenarios.rs::defensive_control_emits_a_defensive_rebound");
    push(ActionFamily::ActionRebound, "双方同时控制", "rebound", "held_ball", ActionRole::Primary, REBOUND_SOURCE, REBOUND_RESOLVER, "双方同时控制球，裁判抛球重新开始", "crates/sim/tests/action_scenarios.rs::a_tie_for_the_ball_enters_a_held_ball_jump_ball");

    // 转换：球权刚转换时的攻防形态。
    push(ActionFamily::ActionTransition, "快攻推进", "transition", "transition_push", ActionRole::Primary, TRANSITION_SOURCE, TRANSITION_RESOLVER, "退防人来不及落位", "crates/sim/tests/action_scenarios.rs::a_defensive_rebound_advances_past_midcourt_within_the_transition_window");
    push(ActionFamily::ActionTransition, "快攻跟进", "transition", "transition_trail", ActionRole::Primary, TRANSITION_SOURCE, TRANSITION_RESOLVER, "第二波跑动把防守人拉散", "crates/sim/tests/action_scenarios.rs::a_defensive_rebound_advances_past_midcourt_within_the_transition_window");
    push(ActionFamily::ActionTransition, "退防", "transition", "transition_retreat", ActionRole::Defender, TRANSITION_SOURCE, TRANSITION_RESOLVER, "退防人先到位，进攻选择变少", "crates/sim/tests/action_scenarios.rs::a_defensive_rebound_advances_past_midcourt_within_the_transition_window");
    push(ActionFamily::ActionTransition, "快速形成阵地防守", "transition", "transition_wall", ActionRole::Defender, TRANSITION_SOURCE, TRANSITION_RESOLVER, "退防落位后进攻被迫进入阵地", "crates/sim/tests/action_scenarios.rs::a_defensive_rebound_advances_past_midcourt_within_the_transition_window");

    ActionSpecTable::new(specs)
}
