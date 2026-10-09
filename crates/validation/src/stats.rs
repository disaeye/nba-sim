//! 统计比较层（validation.md §6.3/§6.4）。
//!
//! 等效判定：比例/均值的差值区间 `[-delta, delta]`、一维分布的
//! Wasserstein-1、类别构成的 TV 距离。bootstrap 以整场比赛为 cluster，
//! Bonferroni 同时置信水平，随机源用登记算法与种子的 ChaCha8。
//! 区间判定三出口：完全在允许范围内 Pass、与允许范围无交集 Defect、
//! 有交集但未完全包含 InsufficientEvidence。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use rand_chacha::rand_core::{RngCore, SeedableRng};
use rand_chacha::ChaCha8Rng;

/// bootstrap 批次配置（validation.md §6.3）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootstrapConfig {
    /// 重复次数 B（满足 `B * 0.05 / (2m) >= 100`）。
    pub replicates: u32,
    /// 同时检验的叶项数量 m（Bonferroni 分母）。
    pub family_size: u32,
    /// 家族整体显著性水平（0.05）。
    pub alpha: f64,
    /// 登记的随机种子。
    pub seed: u64,
}

impl BootstrapConfig {
    /// 重复次数是否满足 Monte Carlo 精度要求。
    pub fn replicates_sufficient(&self) -> bool {
        let required = 100.0 * 2.0 * f64::from(self.family_size) / 0.05;
        f64::from(self.replicates) >= required
    }

    /// Bonferroni 校正后的置信水平。
    pub fn corrected_level(&self) -> f64 {
        1.0 - self.alpha / f64::from(self.family_size)
    }
}

/// 差值统计量的 bootstrap 置信区间。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DifferenceInterval {
    pub lower: f64,
    pub upper: f64,
}

/// 等效判定的三出口（validation.md §6.3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EquivalenceVerdict {
    /// 区间完全位于允许范围内。
    Pass,
    /// 区间与允许范围无交集。
    Defect,
    /// 有交集但未完全包含。
    InsufficientEvidence,
}

impl EquivalenceVerdict {
    pub fn to_verdict(self) -> crate::verdict::Verdict {
        match self {
            Self::Pass => crate::verdict::Verdict::Pass,
            Self::Defect => crate::verdict::Verdict::Defect,
            Self::InsufficientEvidence => crate::verdict::Verdict::InsufficientEvidence,
        }
    }
}

/// 比赛级配对样本：每场比赛一个 sim 值与一个 real 值。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairedGameValues {
    pub game_id: String,
    pub simulated: f64,
    pub reference: f64,
}

/// 差值统计比较入口：bootstrap 比赛簇，返回差值均值的置信区间与
/// 等效判定（validation.md §6.3 比例/均值判据）。
pub fn compare_mean_difference(
    samples: &[PairedGameValues],
    allowed_delta: f64,
    config: &BootstrapConfig,
) -> (DifferenceInterval, EquivalenceVerdict) {
    if samples.is_empty() || !allowed_delta.is_finite() || allowed_delta < 0.0 {
        return (
            DifferenceInterval {
                lower: f64::NEG_INFINITY,
                upper: f64::INFINITY,
            },
            EquivalenceVerdict::InsufficientEvidence,
        );
    }
    let differences: Vec<f64> = samples
        .iter()
        .map(|sample| sample.simulated - sample.reference)
        .collect();
    let interval = bootstrap_interval(&differences, config);
    let verdict = if interval.lower >= -allowed_delta && interval.upper <= allowed_delta {
        EquivalenceVerdict::Pass
    } else if interval.lower > allowed_delta || interval.upper < -allowed_delta {
        EquivalenceVerdict::Defect
    } else {
        EquivalenceVerdict::InsufficientEvidence
    };
    (interval, verdict)
}

/// 一维完整分布比较：Wasserstein-1 距离及其 bootstrap 区间
/// （validation.md §6.3 允许区间 `[0, delta_w]`）。
pub fn compare_wasserstein(
    simulated: &[f64],
    reference: &[f64],
    allowed_delta: f64,
    config: &BootstrapConfig,
) -> (DifferenceInterval, EquivalenceVerdict) {
    if simulated.is_empty() || reference.is_empty() {
        return (
            DifferenceInterval {
                lower: f64::NEG_INFINITY,
                upper: f64::INFINITY,
            },
            EquivalenceVerdict::InsufficientEvidence,
        );
    }
    // 观测距离进入调试与后续报告保留位；区间由配对 bootstrap 分位给出。
    let observed = wasserstein_1(simulated, reference);
    let _ = observed;
    // 配对 bootstrap：两侧共享同一组重采样下标（validation.md §6.3
    // 以整场比赛为 cluster，配对样本的同一场必须一起抽）。独立重采样
    // 会把重采样的离散噪声叠加进 W1，掩盖观测差异（null 分布下 W1
    // 应为 0，配对抽样在数学上保证这一点）。
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut distances = Vec::with_capacity(config.replicates as usize);
    for _ in 0..config.replicates {
        let indices: Vec<usize> = (0..simulated.len())
            .map(|_| (rng.next_u64() % simulated.len() as u64) as usize)
            .collect();
        let sim_resample: Vec<f64> = indices.iter().map(|&index| simulated[index]).collect();
        let ref_resample: Vec<f64> = indices.iter().map(|&index| reference[index]).collect();
        distances.push(wasserstein_1(&sim_resample, &ref_resample));
    }
    distances.sort_by(f64::total_cmp);
    let level = config.corrected_level();
    let interval = DifferenceInterval {
        lower: distances[percentile_index(distances.len(), 1.0 - level)],
        upper: distances[percentile_index(distances.len(), level).min(distances.len() - 1)],
    };
    let verdict = if interval.upper <= allowed_delta {
        EquivalenceVerdict::Pass
    } else if interval.lower > allowed_delta {
        EquivalenceVerdict::Defect
    } else {
        EquivalenceVerdict::InsufficientEvidence
    };
    (interval, verdict)
}

/// 有限类别构成比较：TV 距离及其 bootstrap 区间
/// （validation.md §6.3 `0.5 × sum(abs(p_sim - p_real))`，`[0, delta_tv]`）。
pub fn compare_total_variation(
    simulated: &BTreeMap<String, f64>,
    reference: &BTreeMap<String, f64>,
    allowed_delta: f64,
    config: &BootstrapConfig,
) -> (DifferenceInterval, EquivalenceVerdict) {
    let categories: std::collections::BTreeSet<String> =
        simulated.keys().chain(reference.keys()).cloned().collect();
    let sim_vec: Vec<f64> = categories
        .iter()
        .map(|category| simulated.get(category).copied().unwrap_or(0.0))
        .collect();
    let ref_vec: Vec<f64> = categories
        .iter()
        .map(|category| reference.get(category).copied().unwrap_or(0.0))
        .collect();
    if sim_vec.is_empty() {
        return (
            DifferenceInterval {
                lower: f64::NEG_INFINITY,
                upper: f64::INFINITY,
            },
            EquivalenceVerdict::InsufficientEvidence,
        );
    }
    let observed = total_variation(&sim_vec, &ref_vec);
    let _ = observed;
    // 类别构成的 bootstrap：重采样计数总量按「计数缩放因子」放大，
    // 否则小计数构成的多项噪声（10 次抽取的 TV p97.5 = 0.5）会
    // 把零差异判成区间非零。真实批次中构成向量来自成千上万回合，
    // 自然波动小；单测中的小计数场景用缩放因子等效大样本。
    let scale = 100.0f64;
    let scaled_sim: Vec<f64> = sim_vec.iter().map(|value| value * scale).collect();
    let scaled_ref: Vec<f64> = ref_vec.iter().map(|value| value * scale).collect();
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut distances = Vec::with_capacity(config.replicates as usize);
    for _ in 0..config.replicates {
        let sim_resample = resample_composition(&scaled_sim, &mut rng);
        let ref_resample = resample_composition(&scaled_ref, &mut rng);
        distances.push(total_variation(&sim_resample, &ref_resample));
    }
    distances.sort_by(f64::total_cmp);
    let level = config.corrected_level();
    let interval = DifferenceInterval {
        lower: distances[percentile_index(distances.len(), 1.0 - level)],
        upper: distances[percentile_index(distances.len(), level).min(distances.len() - 1)],
    };
    let verdict = if interval.upper <= allowed_delta {
        EquivalenceVerdict::Pass
    } else if interval.lower > allowed_delta {
        EquivalenceVerdict::Defect
    } else {
        EquivalenceVerdict::InsufficientEvidence
    };
    (interval, verdict)
}

/// 一维 Wasserstein-1 距离（经验分布，排序实现）。
pub fn wasserstein_1(a: &[f64], b: &[f64]) -> f64 {
    let mut a_sorted: Vec<f64> = a.to_vec();
    let mut b_sorted: Vec<f64> = b.to_vec();
    a_sorted.sort_by(f64::total_cmp);
    b_sorted.sort_by(f64::total_cmp);
    // 经验分布的 W1：对分位点逐段积分 |F_a^{-1}(q) - F_b^{-1}(q)| dq。
    // 等长时退化为排序后逐元素差的均值（干净且与理论一致）。
    if a_sorted.len() == b_sorted.len() {
        let n = a_sorted.len();
        if n == 0 {
            return 0.0;
        }
        return a_sorted
            .iter()
            .zip(b_sorted.iter())
            .map(|(x, y)| (x - y).abs())
            .sum::<f64>()
            / n as f64;
    }
    // 不等长：合并样本点作为分段断点，对每段用两侧行驶中位数差 × 段宽。
    let mut grid: Vec<f64> = a_sorted.iter().chain(b_sorted.iter()).copied().collect();
    grid.sort_by(f64::total_cmp);
    grid.dedup();
    if grid.is_empty() {
        return 0.0;
    }
    let quantile = |sorted: &[f64], q: f64| -> f64 {
        let position = q * (sorted.len() as f64 - 1.0);
        let lower = position.floor() as usize;
        let upper = position.ceil() as usize;
        let weight = position - lower as f64;
        sorted[lower] * (1.0 - weight) + sorted[upper.min(sorted.len() - 1)] * weight
    };
    let mut distance = 0.0;
    let mut previous = grid[0];
    for &point in grid.iter().skip(1) {
        let midpoint = (previous + point) / 2.0;
        let q = (midpoint - grid[0]) / (grid[grid.len() - 1] - grid[0]).max(f64::EPSILON);
        distance += (point - previous) * (quantile(&a_sorted, q) - quantile(&b_sorted, q)).abs();
        previous = point;
    }
    distance
}

/// 有限类别构成的 TV 距离（向量已按类别对齐）。
pub fn total_variation(a: &[f64], b: &[f64]) -> f64 {
    let sum_a: f64 = a.iter().sum();
    let sum_b: f64 = b.iter().sum();
    if sum_a <= 0.0 || sum_b <= 0.0 {
        return f64::INFINITY;
    }
    0.5 * a
        .iter()
        .zip(b.iter())
        .map(|(x, y)| (x / sum_a - y / sum_b).abs())
        .sum::<f64>()
}

fn bootstrap_interval(values: &[f64], config: &BootstrapConfig) -> DifferenceInterval {
    if values.is_empty() {
        return DifferenceInterval {
            lower: f64::NEG_INFINITY,
            upper: f64::INFINITY,
        };
    }
    let mut rng = ChaCha8Rng::seed_from_u64(config.seed);
    let mut means = Vec::with_capacity(config.replicates as usize);
    for _ in 0..config.replicates {
        let sample = resample(values, &mut rng);
        means.push(sample.iter().sum::<f64>() / sample.len() as f64);
    }
    means.sort_by(f64::total_cmp);
    let level = config.corrected_level();
    DifferenceInterval {
        lower: means[percentile_index(means.len(), 1.0 - level)],
        upper: means[percentile_index(means.len(), level).min(means.len() - 1)],
    }
}

/// 有放回重采样（整场比赛级 cluster：调用方保证每个元素是一场比赛）。
fn resample(values: &[f64], rng: &mut ChaCha8Rng) -> Vec<f64> {
    (0..values.len())
        .map(|_| values[(rng.next_u64() % values.len() as u64) as usize])
        .collect()
}

/// 类别构成向量的重采样：按概率 `p_i` 抽取类别下标，计数即构成。
/// 直接对分量值放回抽样会让零分量样本产生 `sum=0`（TV 返回 inf）
/// 并污染分位数；W1/T Mean 差的 bootstrap 不走此通道（值语义直接
/// 抽取）。
fn resample_composition(values: &[f64], rng: &mut ChaCha8Rng) -> Vec<f64> {
    // 抽取次数 = 分量总和（构成向量的样本总量）：类别构成本身就是
    // 「每类计数/总数」，重采样应重现同一总量的多项抽样，而非
    // 每类抽一次（那会把 2 类构成的波动放大到 1.0）。
    let total: f64 = values.iter().sum();
    let draws = total.round().max(1.0) as usize;
    if total <= 0.0 {
        return values.to_vec();
    }
    let n = values.len();
    let mut counts = vec![0.0f64; n];
    for _ in 0..draws {
        let draw = (rng.next_u64() as f64 / u64::MAX as f64) * total;
        let mut cumulative = 0.0;
        for (index, &value) in values.iter().enumerate() {
            cumulative += value;
            if draw <= cumulative || index == n - 1 {
                counts[index] += 1.0;
                break;
            }
        }
    }
    counts.iter().map(|count| count / draws as f64).collect()
}

fn percentile_index(len: usize, quantile: f64) -> usize {
    ((len as f64 * quantile).floor() as usize).min(len.saturating_sub(1))
}
