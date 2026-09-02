use crate::rules::GameRules;
use serde::{Deserialize, Serialize};

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
