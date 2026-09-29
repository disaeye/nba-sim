use crate::rules::GameRules;
use serde::{Deserialize, Serialize};

/// 持球姿态：接球后的一次显式技术选择。
///
/// 面框（FaceUp）是三威胁姿态，适合外线持球与突破/拔起投篮；
/// 背身（BackToBasket）背对篮筐要位，适合低位大个与力量错位。
/// 姿态由决策层在新持球确立时评估，期间保持，失去球权即重置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BallOrientation {
    FaceUp,
    BackToBasket,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionPhase {
    Preparation,   // Wind-up / setup (can be interrupted or contested)
    Execution,     // Release / jump apex (blockable/stealable keyframe)
    FollowThrough, // Landing / recovery (kinematically locked, vulnerable to fouls)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionType {
    JumpShot,
    Layup,
    Dunk,
    PassRelease,
    ScreenSet,
    CloseoutContest,
    ReboundJump,
    ScreenRoll,
    ScreenPop,
    Cut,
    CutBackdoor,
    BoxOut,
    Putback,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DribbleMoveKind {
    DirectDrive,
    Crossover,
    BetweenTheLegs,
    BehindTheBack,
    SpinMove,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum JumperKind {
    CatchAndShoot,
    PullUp,
    StepBack,
    TurnaroundFadeaway,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RimFinishKind {
    Layup,
    Dunk,
    Floater,
    ReverseLayup,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OffBallActionKind {
    SetScreen,
    RollToRim,
    PopToThree,
    SlipScreen,
    BackdoorCut,
    VCut,
    FlashToNail,
    SpotUpRelocate,
    DribbleHandOffReceive,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PostMoveKind {
    DropStep,
    UpAndUnder,
    HookShot,
    Fadeaway,
    Backdown,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionTimeWindow {
    pub player_id: String,
    pub action_type: ActionType,
    pub phase: ActionPhase,
    pub start_time: f32,
    pub prep_duration: f32,
    pub exec_duration: f32,
    pub follow_duration: f32,
    pub lock_kinematics: bool,
    pub interference_start: f32,
    pub interference_end: f32,
}

impl ActionTimeWindow {
    pub fn new_jump_shot(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::JumpShot,
            start_time,
            rules.jump_shot_prep_seconds,
            rules.jump_shot_exec_seconds,
            rules.jump_shot_follow_seconds,
            true,
            rules.jump_shot_prep_seconds * 0.5,
        )
    }

    pub fn new_pass(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::PassRelease,
            start_time,
            rules.pass_prep_seconds,
            rules.pass_exec_seconds,
            rules.pass_follow_seconds,
            false,
            0.0,
        )
    }

    pub fn new_rebound_jump(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::ReboundJump,
            start_time,
            rules.rebound_prep_seconds,
            rules.rebound_exec_seconds,
            rules.rebound_follow_seconds,
            true,
            rules.rebound_prep_seconds,
        )
    }

    pub fn new_layup(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::Layup,
            start_time,
            rules.layup_prep_seconds,
            rules.layup_exec_seconds,
            rules.layup_follow_seconds,
            true,
            rules.layup_prep_seconds * 0.5,
        )
    }

    pub fn new_dunk(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::Dunk,
            start_time,
            rules.dunk_prep_seconds,
            rules.dunk_exec_seconds,
            rules.dunk_follow_seconds,
            true,
            rules.dunk_prep_seconds * 0.5,
        )
    }

    pub fn new_screen_set(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::ScreenSet,
            start_time,
            rules.screen_prep_seconds,
            rules.screen_exec_seconds,
            rules.screen_follow_seconds,
            true,
            0.0,
        )
    }

    pub fn new_closeout_contest(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::CloseoutContest,
            start_time,
            rules.contest_prep_seconds,
            rules.contest_exec_seconds,
            rules.contest_follow_seconds,
            false,
            f32::from(0u8),
        )
    }

    pub fn new_screen_roll(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::ScreenRoll,
            start_time,
            rules.screen_prep_seconds,
            rules.screen_exec_seconds,
            rules.screen_follow_seconds,
            false,
            f32::from(0u8),
        )
    }

    pub fn new_screen_pop(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::ScreenPop,
            start_time,
            rules.screen_prep_seconds,
            rules.screen_exec_seconds,
            rules.screen_follow_seconds,
            false,
            f32::from(0u8),
        )
    }

    pub fn new_cut_backdoor(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::CutBackdoor,
            start_time,
            rules.pass_prep_seconds,
            rules.tactics.drive_min_duration_seconds,
            rules.pass_follow_seconds,
            false,
            f32::from(0u8),
        )
    }

    pub fn new_cut(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::Cut,
            start_time,
            rules.pass_prep_seconds,
            rules.tactics.drive_min_duration_seconds,
            rules.pass_follow_seconds,
            false,
            f32::from(0u8),
        )
    }

    pub fn new_box_out(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::BoxOut,
            start_time,
            rules.rebound_prep_seconds,
            rules.rebound_exec_seconds,
            rules.rebound_follow_seconds,
            true,
            rules.rebound_prep_seconds,
        )
    }

    pub fn new_putback(player_id: &str, start_time: f32, rules: &GameRules) -> Self {
        Self::new(
            player_id,
            ActionType::Putback,
            start_time,
            rules.layup_prep_seconds,
            rules.layup_exec_seconds,
            rules.layup_follow_seconds,
            true,
            f32::from(0u8),
        )
    }
    #[allow(clippy::too_many_arguments)]
    fn new(
        player_id: &str,
        action_type: ActionType,
        start_time: f32,
        prep_duration: f32,
        exec_duration: f32,
        follow_duration: f32,
        lock_kinematics: bool,
        interference_offset: f32,
    ) -> Self {
        Self {
            player_id: player_id.to_string(),
            action_type,
            phase: ActionPhase::Preparation,
            start_time,
            prep_duration,
            exec_duration,
            follow_duration,
            lock_kinematics,
            interference_start: start_time + interference_offset,
            interference_end: start_time + prep_duration + exec_duration,
        }
    }

    pub fn update(&mut self, current_time: f32) -> ActionPhase {
        let elapsed = current_time - self.start_time;
        if elapsed < self.prep_duration {
            self.phase = ActionPhase::Preparation;
        } else if elapsed < self.prep_duration + self.exec_duration {
            self.phase = ActionPhase::Execution;
        } else {
            self.phase = ActionPhase::FollowThrough;
        }
        self.phase
    }

    pub fn is_finished(&self, current_time: f32) -> bool {
        current_time
            >= self.start_time + self.prep_duration + self.exec_duration + self.follow_duration
    }

    pub fn is_in_interference_window(&self, current_time: f32) -> bool {
        current_time >= self.interference_start && current_time <= self.interference_end
    }
}
