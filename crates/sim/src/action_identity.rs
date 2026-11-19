//! 动作身份：运行时动作到协议身份的唯一映射。
//!
//! 方案 §10.2：渲染帧里的球员动作状态必须来自权威 `ActionRuntime`、防守
//! 动作运行时或无窗口姿态事实，包含稳定动作身份、动作族、技术变体、
//! 参与者角色、阶段与动作时间区间。
//!
//! 协议身份只有一份定义：动作族、类别、变体与角色都从这里给出，事件
//! 投影、动作规格表与渲染器都读同一份。运行时枚举与目录不一致的地方
//! 在这里暴露，不能各说各话。

use nba_domain::action_spec::{ActionFamily, ActionRole};

use crate::action::{ActionKind, ActionIntentRole, DefensiveActionKind, OnBallPositioning};

/// 一个动作在协议里的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActionIdentity {
    /// 动作族。
    pub family: ActionFamily,
    /// 协议类别。
    pub kind: &'static str,
    /// 技术变体。
    pub variant: &'static str,
    /// 交互中的角色。
    pub role: ActionRole,
    /// 动作是否有阶段窗口。
    pub windowed: bool,
}

/// 持球人动作的身份。
///
/// 身份只由动作种类给出。出手身份里的三分变体需要出手点的几何，因此
/// 它由 `shot_identity` 在拿到世界状态后给出。
pub fn offense_identity(kind: ActionKind) -> ActionIdentity {
    match kind {
        ActionKind::Dribble(move_kind) => ActionIdentity {
            family: ActionFamily::ActionDribble,
            kind: "dribble",
            variant: move_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Drive(drive_kind) => ActionIdentity {
            family: ActionFamily::ActionDrive,
            kind: "drive",
            variant: drive_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Finish(finish_kind) => ActionIdentity {
            family: ActionFamily::ActionFinish,
            kind: "finish",
            variant: finish_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Pass(pass_kind) => ActionIdentity {
            family: ActionFamily::ActionPass,
            kind: "pass",
            variant: pass_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Screen(screen_kind) => ActionIdentity {
            family: ActionFamily::ActionScreen,
            kind: "screen",
            variant: screen_kind.as_str(),
            // 掩护由掩护人执行：他才是这次动作的角色。持球人是点名的人，
            // ���不是掩护人。
            role: ActionRole::Screener,
            windowed: true,
        },
        ActionKind::Cut(cut_kind) => {
            // 绕掩护接球与手递手接球属于掩护族：切入人借掩护人的身体接到
            // 球，两个角色是两个人。其余空切属于切入族。
            let screen_variant = match cut_kind {
                crate::action::CutKind::Curl => Some("curl"),
                crate::action::CutKind::Handoff => Some("handoff"),
                _ => None,
            };
            if let Some(variant) = screen_variant {
                ActionIdentity {
                    family: ActionFamily::ActionScreen,
                    kind: "screen",
                    variant,
                    role: ActionRole::CutPartner,
                    windowed: true,
                }
            } else {
                ActionIdentity {
                    family: ActionFamily::ActionCut,
                    kind: "cut",
                    variant: cut_kind.as_str(),
                    // 空切的执行人就是切入人本身：他在这次进攻里的角色是
                    // 配合持球人的切入方，不是动作的主持球人。
                    role: ActionRole::CutPartner,
                    windowed: true,
                }
            }
        }
        ActionKind::PostUp(step) => ActionIdentity {
            family: ActionFamily::ActionPost,
            kind: "post_up",
            variant: step.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Shoot(shot_kind) => {
            if shot_kind.is_rim_finish() {
                ActionIdentity {
                    family: ActionFamily::ActionFinish,
                    kind: "finish",
                    variant: finish_variant(shot_kind),
                    role: ActionRole::Primary,
                    windowed: true,
                }
            } else {
                ActionIdentity {
                    family: ActionFamily::ActionShot,
                    kind: "jump_shot",
                    variant: shot_variant(shot_kind),
                    role: ActionRole::Primary,
                    windowed: true,
                }
            }
        }
        ActionKind::FreeThrow => ActionIdentity {
            family: ActionFamily::ActionShot,
            kind: "jump_shot",
            variant: "free_throw",
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Relocate(relocate_kind) => ActionIdentity {
            family: ActionFamily::ActionRelocate,
            kind: "relocate",
            variant: relocate_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Fake(fake_kind) => ActionIdentity {
            family: ActionFamily::ActionFake,
            kind: "fake",
            variant: fake_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Protect(protect_kind) => ActionIdentity {
            family: ActionFamily::ActionBallProtection,
            kind: "protect",
            variant: protect_kind.as_str(),
            role: ActionRole::Primary,
            windowed: true,
        },
        ActionKind::Rebound(rebound_kind) => ActionIdentity {
            family: ActionFamily::ActionRebound,
            kind: "rebound",
            variant: rebound_kind.as_str(),
            // 抢篮板的人就是抢篮板的：地板球争抢与双方同时控制时双方
            // 都是争抢方，其余情况下只有抢到球的那个人是抢篮板者。
            role: match rebound_kind {
                crate::action::ReboundKind::LooseBallDive
                | crate::action::ReboundKind::HeldBall => ActionRole::Primary,
                _ => ActionRole::Rebounder,
            },
            windowed: true,
        },
    }
}

/// 防守动作的身份。
pub fn defense_identity(kind: DefensiveActionKind) -> ActionIdentity {
    match kind {
        DefensiveActionKind::CloseoutContest => ActionIdentity {
            family: ActionFamily::ActionCloseout,
            kind: "closeout",
            variant: "closeout_contest",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::BlockJump => ActionIdentity {
            family: ActionFamily::ActionCloseout,
            kind: "closeout",
            variant: "block_jump",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::Verticality => ActionIdentity {
            family: ActionFamily::ActionCloseout,
            kind: "closeout",
            variant: "verticality",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::StealAttempt => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "steal_attempt",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::Poke => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "poke",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::Chase => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "chase",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
        DefensiveActionKind::ChaseDownBlock => ActionIdentity {
            family: ActionFamily::ActionCloseout,
            kind: "closeout",
            variant: "chase_down_block",
            role: ActionRole::Defender,
            windowed: true,
        },
        DefensiveActionKind::Slide => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "slide",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
        DefensiveActionKind::Contain => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "contain",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
        DefensiveActionKind::ChargeTake => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "charge_take",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
    }
}

/// 无窗口防守姿态的身份。
pub fn posture_identity(positioning: OnBallPositioning) -> ActionIdentity {
    match positioning {
        OnBallPositioning::Slide => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "slide",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
        OnBallPositioning::Contain => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "contain",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
        // 站位吃犯规是一次主动冒险，因此角色是普通的防守人，而不是
        // 对位防守人自己选的站位。
        OnBallPositioning::ChargeTake => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "charge_take",
            role: ActionRole::Defender,
            windowed: false,
        },
        // 追防：被甩开之后追上去，角色仍是对位防守人。
        OnBallPositioning::Chase => ActionIdentity {
            family: ActionFamily::ActionDefenseOnBall,
            kind: "on_ball_defense",
            variant: "chase",
            role: ActionRole::OnBallDefender,
            windowed: false,
        },
    }
}

/// 转换姿态的身份。
///
/// 转换是姿态而不是动作：跑者在快攻窗口里仍然可以切入、传球与出手。
pub fn transition_identity(kind: crate::action::TransitionKind) -> ActionIdentity {
    ActionIdentity {
        family: ActionFamily::ActionTransition,
        kind: "transition",
        variant: kind.as_str(),
        role: match kind {
            crate::action::TransitionKind::Retreat | crate::action::TransitionKind::Wall => {
                ActionRole::Defender
            }
            _ => ActionRole::Primary,
        },
        windowed: false,
    }
}

/// 卡位姿态的身份。
pub fn box_out_identity() -> ActionIdentity {
    ActionIdentity {
        family: ActionFamily::ActionRebound,
        kind: "rebound",
        variant: "box_out",
        role: ActionRole::Rebounder,
        windowed: false,
    }
}

/// 无球防守职责的身份。
///
/// 职责不是动作：它没有窗口，但它是这一名防守人当前在做的事。渲染器
/// 需要它来画出「谁在协防、谁在沉退、谁在挤过掩护」。
pub fn off_ball_identity(
    responsibility: crate::state::DefenseResponsibility,
) -> Option<ActionIdentity> {
    use crate::state::DefenseResponsibility;
    let (variant, role) = match responsibility {
        DefenseResponsibility::Deny => ("deny", ActionRole::Defender),
        DefenseResponsibility::Intercept => ("intercept", ActionRole::Defender),
        DefenseResponsibility::Help => ("help", ActionRole::Defender),
        DefenseResponsibility::Rotate => ("rotate", ActionRole::Defender),
        DefenseResponsibility::Recover => ("recover", ActionRole::Defender),
        DefenseResponsibility::DoubleTeam => ("double_team", ActionRole::Defender),
        DefenseResponsibility::Switch => ("switch", ActionRole::Defender),
        DefenseResponsibility::Hedge => ("hedge", ActionRole::Defender),
        DefenseResponsibility::Sink => ("drop", ActionRole::Defender),
        DefenseResponsibility::GoOver => ("go_over", ActionRole::Defender),
        DefenseResponsibility::GoUnder => ("go_under", ActionRole::Defender),
        DefenseResponsibility::RimProtect => ("rim_protect", ActionRole::Defender),
        // 对位、防守篮板与卡位已经有自己的身份，不重复投影。
        DefenseResponsibility::OnBall
        | DefenseResponsibility::BoxOut => return None,
    };
    Some(ActionIdentity {
        family: ActionFamily::ActionDefenseOffBall,
        kind: "off_ball_defense",
        variant,
        role,
        windowed: false,
    })
}

/// 收球终结的协议变体。
///
/// 引擎的出手类型按发起方式区分（突破上篮、空切上篮、跑动上篮），动画
/// 目录按技术区分（普通上篮、反手上篮）。因此这里把发起方式落到技术上：
/// 渲染器播的是技术动作，动作族与类别保持一致。
pub fn finish_variant(kind: crate::action::ShotKind) -> &'static str {
    match kind {
        crate::action::ShotKind::ReverseLayup => "reverse_layup",
        crate::action::ShotKind::DoubleClutchLayup => "double_clutch_layup",
        crate::action::ShotKind::FingerRoll => "finger_roll",
        crate::action::ShotKind::Floater => "floater",
        crate::action::ShotKind::Dunk
        | crate::action::ShotKind::DrivingDunk
        | crate::action::ShotKind::CuttingDunk
        | crate::action::ShotKind::RunningDunk => "dunk",
        crate::action::ShotKind::Putback => "putback",
        crate::action::ShotKind::ProHop => "pro_hop",
        crate::action::ShotKind::EuroStep => "euro_step",
        _ => "layup",
    }
}

/// 跳投的协议变体。
pub fn shot_variant(kind: crate::action::ShotKind) -> &'static str {
    match kind {
        crate::action::ShotKind::CatchShoot => "catch_shoot",
        crate::action::ShotKind::PullUp => "pull_up",
        crate::action::ShotKind::StepBack => "step_back",
        crate::action::ShotKind::Turnaround => "turnaround",
        crate::action::ShotKind::Hook => "hook",
        _ => "catch_shoot",
    }
}

/// 出手动作的协议身份。
///
/// 出手的统计类型与协议变体不是一回事：统计里的“接球投”既可以是两分
/// 也可以是三分，渲染器需要知道这一次是不是远投。因此出手的协议变体
/// 由出手类型与出手点的几何一起给出：出手点在三分线外时，协议变体是
/// 三分出手。
pub fn shot_identity(
    world: &crate::world::World,
    player: usize,
    kind: crate::action::ShotKind,
) -> ActionIdentity {
    let identity = offense_identity(ActionKind::Shoot(kind));
    if identity.family != ActionFamily::ActionShot {
        return identity;
    }
    // 背身后仰之后的跳投是后仰跳投。
    //
    // 出手点在十英尺以外：背身的转身类步法在中距离出手，因此那一记是
    // 后仰；贴身步法在篮下出手，那一记是勾手。出手点因此就是技术本身。
    let team = world.players[player].team;
    let hoop = world.hoop_for(team);
    let release = world.players[player].pos;
    if identity.variant == "turnaround"
        && world.possession.last_post_actor == Some(player)
        && (release - hoop).length() > 9.0
    {
        return ActionIdentity {
            variant: "fadeaway",
            ..identity
        };
    }
    let three = world
        .rules
        .court
        .is_three_point_attempt(
            release,
            world.attacking_right(team),
            world.rules.league.three_point_distance_ft,
            world.rules.league.corner_three_distance_ft,
        );
    if !three {
        return identity;
    }
    ActionIdentity {
        variant: "three_point",
        ..identity
    }
}

/// 交互角色的协议身份。
///
/// 掩护交互里有两个角色：掩护人自己与借掩护的切入人。空切由切入人自己
/// 执行，他的角色是配合持球人的切入方。
pub fn role_identity(role: ActionIntentRole) -> ActionRole {
    match role {
        ActionIntentRole::Primary => ActionRole::Primary,
        ActionIntentRole::ScreenPartner => ActionRole::Screener,
        ActionIntentRole::HandlerPartner => ActionRole::Primary,
        ActionIntentRole::CutPartner => ActionRole::CutPartner,
    }
}
