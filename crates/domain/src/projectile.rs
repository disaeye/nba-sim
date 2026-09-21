//! 重力抛体纯数学（第一步飞行抛体化）。
//!
//! 传球、投篮、篮板飞行的垂直分量全部由这一条抛物线产生：
//! `z(t) = z0 + vz0·t − g/2·t²`，`g = GameRules::ball_gravity_ftps2`
//! （真实值 32.17 ft/s²）。水平方向保持恒速直线（篮球场尺度内空气
//! 阻力可忽略）。住在 domain 层使 `GameRules::pass_duration` /
//! `BallisticsEngine::shot_duration` 共用同一实现，两处不会漂移。

/// 一个已解出 vz0 的抛体垂直参数。
#[derive(Debug, Clone, Copy)]
pub struct ProjectileArc {
    pub vz0: f32,
}

impl ProjectileArc {
    /// 由两端高度与时长反解 vz0：`vz0 = (z1 − z0 + g/2·T²) / T`。
    /// 这是唯一使 `z(0)=z0`、`z(T)=z1` 同时成立的重力抛体。
    pub fn solve(z0: f32, z1: f32, duration_s: f32, g: f32) -> Self {
        let t = duration_s.max(f32::EPSILON);
        ProjectileArc {
            vz0: (z1 - z0 + 0.5 * g * t * t) / t,
        }
    }

    /// 由请求弧顶闭式解出对称抛体时长：
    /// `T = √(2(peak−z0)/g) + √(2(peak−z1)/g)`（升段 + 降段）。
    /// 这是「请求弧顶」唯一能同时满足两端高度的时长（抛体对称性，无需迭代）。
    pub fn time_for_peak(z0: f32, z1: f32, peak_z: f32, g: f32) -> f32 {
        let rise = (peak_z - z0).max(0.0);
        let fall = (peak_z - z1).max(0.0);
        let g_safe = g.max(f32::EPSILON);
        ((2.0 * rise / g_safe).sqrt() + (2.0 * fall / g_safe).sqrt()).max(f32::EPSILON)
    }

    /// 沿抛物线采样高度。
    pub fn z_at(&self, t: f32, z0: f32, g: f32) -> f32 {
        z0 + self.vz0 * t - 0.5 * g * t * t
    }

    /// 抛物线自然顶点高度（vz0 ≤ 0 时为起点高度）。
    pub fn peak_height(&self, z0: f32, g: f32) -> f32 {
        let g_safe = g.max(f32::EPSILON);
        if self.vz0 <= 0.0 {
            z0
        } else {
            z0 + self.vz0 * self.vz0 / (2.0 * g_safe)
        }
    }
}
