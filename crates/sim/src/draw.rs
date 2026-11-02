//! 按地址抽样的随机源。
//!
//! 每个抽样由 `(base_seed, 流名, 回合, tick, 实体, 用途, 序号)` 唯一决定，
//! 不依赖调用顺序。反事实推演把 `base_seed` 换成 `rollout_seed`，同一地址
//! 在各分支取到同一个值；分叉后的世界不需要复制可变的随机流状态。

/// 具名随机流。每个流对应一类选择，便于审计与分离。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Aim,
    Pass,
    Reaction,
    Foul,
    Rebound,
    Decision,
    FreeThrow,
    Play,
}

impl Stream {
    fn tag(self) -> u64 {
        match self {
            Self::Aim => 0x01,
            Self::Pass => 0x02,
            Self::Reaction => 0x03,
            Self::Foul => 0x04,
            Self::Rebound => 0x05,
            Self::Decision => 0x06,
            Self::FreeThrow => 0x07,
            Self::Play => 0x08,
        }
    }
}

/// 抽样地址。`ordinal` 是语义序号，例如罚球第几次、候选动作下标、
/// 同一 tick 的第几个接触候选；它不是循环计数器。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawKey {
    pub stream: Stream,
    pub possession: u64,
    pub tick: u64,
    pub entity: u32,
    pub purpose: u16,
    pub ordinal: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct DrawSource {
    pub base_seed: u64,
}

impl DrawSource {
    pub fn new(base_seed: u64) -> Self {
        Self { base_seed }
    }

    /// 返回 `[0, 1)` 的浮点抽样。
    pub fn f32(&self, key: DrawKey) -> f32 {
        let bits = self.u64(key) >> 40;
        bits as f32 / ((1u32 << 24) as f32)
    }

    /// 返回一个 64 位无符号抽样；供整数选择与测试使用。
    pub fn u64(&self, key: DrawKey) -> u64 {
        let mut hash = self.base_seed;
        hash = mix(hash ^ key.stream.tag());
        hash = mix(hash ^ key.possession);
        hash = mix(hash ^ key.tick);
        hash = mix(hash ^ u64::from(key.entity));
        hash = mix(hash ^ u64::from(key.purpose));
        hash = mix(hash ^ u64::from(key.ordinal));
        hash
    }
}

fn mix(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

/// 用途编号。数值是协议的一部分，改动需要同步工件。
pub mod purpose {
    pub const ACTION_CHOICE: u16 = 1;
    pub const SHOT_MAKE: u16 = 2;
    pub const SHOT_BLOCK: u16 = 3;
    pub const SHOT_BLOCK_FOUL: u16 = 4;
    pub const SHOOTING_FOUL: u16 = 5;
    pub const AND_ONE: u16 = 6;
    pub const DRIVE_TURNOVER: u16 = 7;
    pub const STEAL_CREDIT: u16 = 8;
    pub const REACH_IN_FOUL: u16 = 9;
    pub const DRIVE_BEAT: u16 = 10;
    pub const FINISH_DISTANCE: u16 = 11;
    pub const FINISH_STEP: u16 = 12;
    pub const POST_TARGET: u16 = 13;
    pub const PASS_INTERCEPT: u16 = 14;
    pub const BAD_PASS: u16 = 15;
    pub const CUT_COMPLETION: u16 = 16;
    pub const LANE_DEFENDER: u16 = 17;
    pub const SCREEN_ROUTE: u16 = 18;
    pub const MOVING_SCREEN: u16 = 19;
    pub const REBOUND_TEAM: u16 = 20;
    pub const REBOUND_LANDING: u16 = 21;
    pub const REBOUND_PICK: u16 = 22;
    pub const FREE_THROW_MAKE: u16 = 23;
    pub const PLAY_CALL: u16 = 24;
    pub const OPENING_TIP: u16 = 25;
    pub const OFF_BALL_HOLD: u16 = 26;
    pub const DRIVE_CHARGE: u16 = 27;
    pub const GATHER_TRAVEL: u16 = 28;
    pub const SCREEN_FOUL: u16 = 29;
}
