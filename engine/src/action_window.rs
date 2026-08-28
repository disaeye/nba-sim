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
    pub fn new_jump_shot(player_id: &str, start_time: f32) -> Self {
        let prep_dur = 0.28;
        let exec_dur = 0.22;
        let follow_dur = 0.35;
        Self {
            player_id: player_id.to_string(),
            action_type: ActionType::JumpShot,
            phase: ActionPhase::Preparation,
            start_time,
            prep_duration: prep_dur,
            exec_duration: exec_dur,
            follow_duration: follow_dur,
            lock_kinematics: true,
            interference_start: start_time + prep_dur * 0.5,
            interference_end: start_time + prep_dur + exec_dur,
        }
    }

    pub fn new_pass(player_id: &str, start_time: f32) -> Self {
        let prep_dur = 0.16;
        let exec_dur = 0.12;
        let follow_dur = 0.16;
        Self {
            player_id: player_id.to_string(),
            action_type: ActionType::PassRelease,
            phase: ActionPhase::Preparation,
            start_time,
            prep_duration: prep_dur,
            exec_duration: exec_dur,
            follow_duration: follow_dur,
            lock_kinematics: false,
            interference_start: start_time,
            interference_end: start_time + prep_dur + exec_dur,
        }
    }

    pub fn new_rebound_jump(player_id: &str, start_time: f32) -> Self {
        let prep_dur = 0.20;
        let exec_dur = 0.30;
        let follow_dur = 0.30;
        Self {
            player_id: player_id.to_string(),
            action_type: ActionType::ReboundJump,
            phase: ActionPhase::Preparation,
            start_time,
            prep_duration: prep_dur,
            exec_duration: exec_dur,
            follow_duration: follow_dur,
            lock_kinematics: true,
            interference_start: start_time + prep_dur,
            interference_end: start_time + prep_dur + exec_dur,
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
        let total_duration = self.prep_duration + self.exec_duration + self.follow_duration;
        current_time >= self.start_time + total_duration
    }

    pub fn is_in_interference_window(&self, current_time: f32) -> bool {
        current_time >= self.interference_start && current_time <= self.interference_end
    }
}
