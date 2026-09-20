//! 接球人感知与领传几何：传球落点的提前量、接球人的自身预判与制动接近。
//!
//! 依据 `gap.md` §7.1 的层 A（P-1 有限信息）原则：接球人只能依据**可观察**
//! 信息预判球的落点，不得直读传球人冻结的意图；预判可能错，因此他可能接不到。

use glam::Vec2;
use nba_physics::ballistics::BallTrajectoryKind;

use super::MatchEngine;

impl MatchEngine {
    /// 领传点：按接球人的**当前速度**外推一个飞行期内的可达点（round-7）。
    ///
    /// ## 为什么需要它
    ///
    /// 传球 release 时冻结的 `to_pos` 是「球将到达的位置」。若直接取接球人
    /// **释放时刻**的位置，而接球人在飞行期间仍在跑动，他永远不会恰好停在
    /// 那个点——实测越位 5–12 ft。
    ///
    /// 决策侧传球已通过决策层给出带提前量的 `to_pos`；outlet 一传漏了这一步，
    /// 成为 `PASS_CORRIDOR_REACHABLE` 的最大单一来源。
    ///
    /// ## 模型
    ///
    /// ```text
    /// flight = pass_duration(distance)          // 与弹道使用同一时长函数
    /// lead   = receiver_velocity × flight × lead_gain
    /// to_pos = receiver_pos + lead              // 封顶到 lead_max_ft
    /// ```
    ///
    /// 不做二次迭代（先用初速估时长，再用新距离重算）：一阶近似已足够，
    /// 且迭代会使事实依赖收敛路径，不利于可复现性。
    pub(crate) fn lead_receiver_position(
        &self,
        receiver_id: &str,
        receiver_pos: Vec2,
        distance_hint: f32,
        inbound: bool,
    ) -> Vec2 {
        let Some(p) = self.systems.physics.get_player(receiver_id) else {
            return receiver_pos;
        };
        let speed = p.vel_ft.length();
        if speed <= f32::EPSILON {
            return receiver_pos;
        }
        let flight = self.config.rules.pass_duration(distance_hint, inbound);
        let gain = self.config.rules.tactics.pass_lead_gain;
        let max_lead = self.config.rules.tactics.pass_lead_max_ft;
        let lead = p.vel_ft * flight * gain;
        let lead = if lead.length() > max_lead {
            lead.normalize_or_zero() * max_lead
        } else {
            lead
        };
        self.config
            .rules
            .court
            .clamp_playable(receiver_pos + lead, self.config.rules.player_radius_ft)
    }

    /// 接球人向冻结点收敛的目标与速度（round-6 审计修复）。
    ///
    /// ## 缺陷（修复前）
    ///
    /// 接球人被硬编码为 20 ft/s 全速冲向 `frozen_to_pos`，且**没有减速模型**。
    /// 传球飞行时长受 `max_pass_duration_seconds`（默认 1.4s）封顶，但 40–60 ft
    /// 的跨场 outlet 实际只需 0.5–0.7s。于是接球人在被拉长的飞行期内持续全速
    /// 前进，实测**越过**冻结点 5–12.5 ft（越位方向几乎垂直于传球线）。
    ///
    /// 后果：`PASS_CORRIDOR_REACHABLE` 在 8 seed 下报 8–17 条 Hard defect。
    ///
    /// ## 模型
    ///
    /// 用「制动距离」反解允许速度：
    ///
    /// ```text
    /// v_allow = sqrt(2 · a_max · max(d_remaining - margin, 0))
    /// ```
    ///
    /// 即剩余距离越短，允许速度越低（接近冻结点时自然减速）；同时在远距离
    /// 封顶到 `receive_approach_speed_ratio × max_player_speed`，避免用超过
    /// 人体上限的速度冲向终点。
    ///
    /// 这不是「为了通过评判而调参」：它补的是**缺失的物理约束**——此前模型
    /// 允许球员以 20 ft/s 穿过目标点而不减速，违反 `gap.md §12.1`
    /// 「加速、制动、变向受属性与规则上限约束」。
    pub(crate) fn receive_approach(&self, frozen_to_pos: Vec2, player_id: &str) -> (Vec2, f32) {
        let Some(p) = self.systems.physics.get_player(player_id) else {
            return (frozen_to_pos, self.config.rules.max_player_speed_ftps * 0.5);
        };
        let to_target = frozen_to_pos - p.pos_ft;
        let d = to_target.length();
        let accel = self.config.rules.max_player_accel_ftps2.max(f32::EPSILON);
        let current_speed = p.vel_ft.length();

        // ## 制动距离必须自洽（round-8 审计修复）
        //
        // `receive_stop_margin_ft` 是一个**固定**裕量，但所需的制动距离是
        // `v²/(2a)` —— 随接近速度增大。实测：
        //   19.8 ft/s -> 需 5.60 ft；12.0 -> 2.06 ft；5.5 -> 0.43 ft
        // 当裕量（2.5 ft）小于所需制动距离时，「已到位则停下」的分支永远
        // 不可达：接球人一边被 `sqrt(2as)` 减速、一边因为 `d > margin` 继续
        // 被推着走，结果在球到达时已越过 5–12 ft。
        //
        // 修正：用**物理所需的制动距离**取代固定裕量，并取两者较大值
        // （保留一个下限，避免低速时数值抖动）。
        let brake_dist = current_speed * current_speed / (2.0 * accel);
        let stop_margin = self
            .config
            .rules
            .tactics
            .receive_stop_margin_ft
            .max(brake_dist);

        // 已进入制动距离内：站住等球（真实接球动作的语义）。
        if d <= stop_margin {
            return (p.pos_ft, 0.0);
        }

        let remaining = (d - stop_margin).max(0.0);
        // 制动距离反解：v = sqrt(2·a·s)。
        let v_allow = (2.0 * accel * remaining).sqrt();
        let v_cap = self.config.rules.max_player_speed_ftps
            * self.config.rules.tactics.receive_approach_speed_ratio;
        let v_min = self.config.rules.max_player_speed_ftps
            * self.config.rules.tactics.receive_min_approach_speed_ratio;
        let speed = v_allow.min(v_cap).max(v_min);

        // 目标点提前 stop_margin：即使在离散 tick 下也不会越过冻结点。
        let aim = frozen_to_pos - to_target.normalize() * stop_margin;
        (aim, speed)
    }

    /// 由一次失败的传球构造松球（层 A 失败，P-1）。
    ///
    /// 初速必须服从 `ball_max_speed_ftps`（留安全余量），而不是
    /// `segment / duration`（那会产生 49 ft/s 的初速，松球随即飞出边线）。
    pub(crate) fn loose_ball_from(&self, pos: Vec2, segment: Vec2) -> BallTrajectoryKind {
        let dir = segment.normalize_or_zero();
        let cap = (self.config.rules.ball_max_speed_ftps
            - self.config.rules.invariant_speed_tolerance_ftps)
            .max(self.config.rules.invariant_speed_tolerance_ftps);
        BallTrajectoryKind::LooseBall {
            pos,
            vel: dir * cap,
            z: self.ball.ball_pos_3d.1,
            vel_z: 0.0,
            last_touch_team: self.flow.possession,
        }
    }

    /// 接球人对"球会到哪里"的**自身估计**（层 A，P-1）。
    ///
    /// ## 为什么不能用 `frozen_to_pos`
    ///
    /// `frozen_to_pos` 是**传球人的意图**。接球人若直读它，就获得了全知视角，
    /// 必然到位——这违反真实性：真实比赛里接球人只能根据**可观察到的**信息
    /// 预判（球的来向与速度、传球人的动作、自己的位置与速度），预判可能错。
    ///
    /// ## 估计模型
    ///
    /// ```text
    /// 观测点  = 上一 tick 的球位置（感知延迟，不是瞬时真值）
    /// 球速估计 = 球的当前速度（接球人看不到 flight_duration）
    /// 到达时间 = |观测点 − 自己| / max(球速估计, 下限)      ← 他自己的估算
    /// 落点估计 = 观测点 + 球向 × 球速 × 到达时间
    ///            + 自己的速度 × 到达时间 × 预判增益
    ///            + 噪声(off_ball_sense 越低越大)
    /// ```
    ///
    /// 噪声是**确定性伪随机**（由 tick + 球员 id 哈希派生），不是真随机：
    /// 保证可复现（charter C4），同时使不同球员/回合的预判偏差不同。
    ///
    /// 参数全部走规则通道（`receive_estimate_noise_ft` 等），无内联行为常数。
    pub(crate) fn estimate_receiver_landing(
        &mut self,
        receiver_id: &str,
        frozen_to_pos: Vec2,
    ) -> Vec2 {
        let Some(receiver) = self.systems.physics.get_player(receiver_id) else {
            return frozen_to_pos;
        };
        // 规则开关：噪声为 0 时退化为"精确知道落点"（旧行为，仅用于对照）。
        let noise_cap =
            nba_domain::receive_estimate_noise(&self.config.rules, &receiver.attributes);
        if noise_cap <= f32::EPSILON {
            return frozen_to_pos;
        }

        // 感知延迟：用球**上一 tick** 的位置作为观测点。
        // 引擎在决策前已将本 tick 的 ball_pos_3d 推进，故这里近似为"当前可见位置"。
        let observed_ball = self.ball.ball_pos_3d.0;
        let self_pos = receiver.pos_ft;
        let to_ball = observed_ball - self_pos;
        let dist = to_ball.length();
        if dist <= f32::EPSILON {
            return frozen_to_pos;
        }

        // ## 稳定估计（round-10 修正两次）
        //
        // 球员在球出手后形成一个**预判**，之后只按观察力做小幅修正；
        // 不会每 tick 整体重算（那会因球的逼近导致目标塌缩与方向翻转，
        // 实测接球人距球从 6.8 ft 恶化到 12.8 ft）。
        //
        // **第二次修正**：初版用 `ball_vel × pass_duration(dist)` 做外推，
        // 但 `ball_vel` 是**实际**飞行速度（`seg/duration`，可达 50 ft/s），
        // 而 `pass_duration` 按**名义**球速（32 ft/s）给出时长——两者混用
        // 导致 10 ft 传球被外推 22.5 ft（冲过头），层 A 随即失败。
        //
        // 正确的初判：球能走多远，受**剩余飞行距离**约束，不得超出
        // 「球到接球人的距离」。即接球人认为球最多飞到他自己所在处；
        // 提前量来自他看见球的运动方向，而不是把他自己再外推一次。
        let ball_vel = self.ball_velocity_estimate();
        let ball_speed = ball_vel.length();
        let remaining = dist; // 球到接球人的距离 = 它最多还能前进的量
        let initial = if ball_speed > f32::EPSILON {
            let travel = remaining.min(ball_speed * self.config.rules.tick_seconds * dist.max(1.0));
            observed_ball + ball_vel.normalize_or_zero() * travel
        } else {
            observed_ball
        };

        // 取出或建立本回合的稳定估计。
        let prev = match &self.ball.receiver_estimate {
            Some((id, p)) if id == receiver_id => Some(*p),
            _ => None,
        };
        let sense = receiver.attributes.off_ball_sense.clamp(0.0, 1.0);

        // 观察修正：向"球的实际位置 + 其运动方向上的有限外推"按观察力加权。
        // 观察力强 → 快速跟上球的真实轨迹；观察力弱 → 停在最初预判上。
        let observe_target = initial;
        let base = prev.unwrap_or(initial);
        let blended = base + (observe_target - base) * sense;

        // 预判噪声：off_ball_sense 越低残留越大；方向由确定性哈希决定。
        // round-19 修复：`(1−sense)` 此前被应用两次（capability 里一次、
        // 这里一次），实际噪声 = noise_ft×(1−sense)²，低观察力球员的
        // 噪声被意外压缩。恢复设计意图：线性 (1−sense)。
        let residual_noise = noise_cap;
        let noise = self.deterministic_estimate_offset(receiver_id) * residual_noise;
        let est = self
            .config
            .rules
            .court
            .clamp_playable(blended + noise, self.config.rules.player_radius_ft);
        // 保存稳定估计（跨 tick 复用）。
        self.ball.receiver_estimate = Some((receiver_id.to_string(), est));
        est
    }

    /// 球的瞬时速度估计（层 A 用）。
    ///
    /// 接球人能看到球在动，但看不到传球人冻结的 `duration`。这里用
    /// **上一 tick 与本 tick 的球位置差**给出方向与量级；无运动时返回零。
    pub(crate) fn ball_velocity_estimate(&self) -> Vec2 {
        // ## P-1 修复（round-19）：观测差分，不读传球人的冻结意图
        //
        // 原实现直读球态的 `to_pos`/`from_pos`/`duration`——传球人的私有
        // 意图（全知泄漏）。其文档注释声称"用上一 tick 与本 tick 的球位置
        // 差"，注释与实现不一致（契约-代码漂移）。
        //
        // 接球人可观测的是**球的运动本身**：位置差分给出方向与速度，
        // 信息量与冻结向量等价（球匀速直线飞行），但来源合法。
        // `prev_observed_ball_pos` 由引擎每 tick 记录（感知延迟一步）。
        if let Some(prev) = self.ball.prev_observed_ball_pos {
            let dt = self.config.rules.tick_seconds.max(f32::EPSILON);
            let delta = (self.ball.ball_pos_3d.0 - prev) / dt;
            // 速度量级钳制在球的物理上限内（观测噪声保护）。
            let cap = self.config.rules.ball_max_speed_ftps;
            if delta.length() > cap {
                delta.normalize_or_zero() * cap
            } else {
                delta
            }
        } else {
            Vec2::ZERO
        }
    }

    /// 确定性偏差向量（单位长度内），由 tick 与球员 id 派生。
    ///
    /// 不是真随机：同一 tick + 同一球员必得同一值（charter C4 可复现）。
    /// 用简单 FNV 混合：避免引入新 RNG 流而改变既有随机序列。
    pub(crate) fn deterministic_estimate_offset(&self, player_id: &str) -> Vec2 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in player_id.as_bytes() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x100_0000_01b3);
        }
        h ^= self.clock.tick_index;
        h = h.wrapping_mul(0x100_0000_01b3);
        // 映射到 [-1, 1] 的二维单位向量（用两个 16 位切片）。
        let q = self
            .config
            .rules
            .estimate_offset_quantization
            .max(f32::EPSILON);
        let a = ((h & 0xFFFF) as f32 / q) - 1.0;
        let b = (((h >> 16) & 0xFFFF) as f32 / q) - 1.0;
        Vec2::new(a, b).normalize_or_zero()
    }
}
