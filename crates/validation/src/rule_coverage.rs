//! 场景-规则映射登记（validation.md §9.2 场景矩阵的规则域对照）。
//!
//! 目的：回答「每条主要篮球规则是否有专门测试局面」——22 个必需场景
//! 与规则域的对应表；缺口的场景（未建）如实登记为 `missing`，与
//! R5 违规程序队列（走步/二次运球/干扰球/贴身五秒/背筐五秒）衔接。
//!
//! 覆盖三态：
//! - `covered`：有专门场景（场景矩阵内的 SC-*）；
//! - `sampling_only`：无专门场景，依赖采样比赛自然触发（不保证命中）；
//! - `missing`：引擎侧规则程序未实现（R5 队列），场景无从建立。

use serde::Serialize;

/// 规则域覆盖状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleCoverage {
    /// 有专门场景。
    Covered,
    /// 无专门场景，依赖采样比赛自然触发。
    SamplingOnly,
    /// 引擎规则程序未实现，场景无从建立。
    Missing,
}

/// 一条规则域的映射记录。
#[derive(Debug, Clone, Serialize)]
pub struct RuleDomainEntry {
    /// 规则域标识（稳定英文 id）。
    pub rule_id: &'static str,
    /// 规则域描述（中文，术语保留英文）。
    pub description: &'static str,
    /// 对应场景（有则列 id；无则空）。
    pub scenarios: &'static [&'static str],
    /// 采样证据键（流内事件种类/变体名；覆盖报告用它查采样触发）。
    pub sampling_keys: &'static [&'static str],
    pub coverage: RuleCoverage,
    /// 缺口说明（sampling_only / missing 时必填）。
    pub gap_note: &'static str,
}

const fn covered(
    rule_id: &'static str,
    description: &'static str,
    scenarios: &'static [&'static str],
    sampling_keys: &'static [&'static str],
) -> RuleDomainEntry {
    RuleDomainEntry {
        rule_id,
        description,
        scenarios,
        sampling_keys,
        coverage: RuleCoverage::Covered,
        gap_note: "",
    }
}

const fn sampling_only(
    rule_id: &'static str,
    description: &'static str,
    gap_note: &'static str,
) -> RuleDomainEntry {
    RuleDomainEntry {
        rule_id,
        description,
        scenarios: &[],
        sampling_keys: &[],
        coverage: RuleCoverage::SamplingOnly,
        gap_note,
    }
}

const fn missing(
    rule_id: &'static str,
    description: &'static str,
    gap_note: &'static str,
) -> RuleDomainEntry {
    RuleDomainEntry {
        rule_id,
        description,
        scenarios: &[],
        sampling_keys: &[],
        coverage: RuleCoverage::Missing,
        gap_note,
    }
}

/// 场景-规则映射登记（主表）。
pub fn rule_domain_registry() -> Vec<RuleDomainEntry> {
    vec![
        covered("shot_open", "空位出手与防守距离对照", &["SC-SHOT-OPEN"], &["SHOT_RELEASE", "SHOT_ATTEMPT_RESOLVED"]),
        covered("shot_block", "封盖（离手前/后）与球权后续", &["SC-BLOCK-BEFORE", "SC-BLOCK-AFTER"], &["BLOCKED_SHOT"]),
        covered("airball", "三不沾与触筐状态时钟处理", &["SC-AIRBALL"], &["SHOT_ATTEMPT_RESOLVED"]),
        covered("rebound_possession", "篮板控制与前场续回合", &["SC-REBOUND"], &["REBOUND"]),
        covered("pass_identity", "传球身份配对与中断保留", &["SC-PASS-REPEAT", "SC-PASS-WHISTLE"], &["PASS", "PASS_RECEIVED", "PASS_TIPPED"]),
        covered("drive_resolution", "突破通路/阻断/协防结算", &["SC-DRIVE"], &["DRIVE_INITIATED", "DRIVE_REACHED"]),
        covered("shooting_foul", "投篮犯规的得分×罚则组合", &["SC-SHOOTING-FOUL"], &["shooting"]),
        covered("technical_foul", "技术犯规与原球权保持", &["SC-TECHNICAL"], &["technical"]),
        covered("free_throw_program", "罚球程序（非末罚/末罚/保留球权）", &["SC-FT-PROGRAM"], &["FREE_THROW"]),
        covered("shot_clock", "进攻时钟边界与重置", &["SC-CLOCK"], &["CLOCK_CHANGED"]),
        covered("buzzer_beater", "压哨球与终场结算顺序", &["SC-BUZZER"], &["PERIOD_SETTLEMENT_COMPLETED"]),
        covered("team_turnover", "球队责任违例的记账归属", &["SC-TEAM-TOV"], &["TURNOVER"]),
        covered("substitution", "换人窗口与资格判断", &["SC-SUBSTITUTION"], &["SUBSTITUTION"]),
        covered("movement_tasks", "移动意图/轨迹/到位结算", &["SC-MOVEMENT"], &["MOVEMENT_INTENT", "MOVEMENT_OUTCOME"]),
        covered("screen_legality", "掩护合法性与接触分类", &["SC-SCREEN"], &["SCREEN_CONTACT"]),
        covered("defense_coverage", "防守责任移交与覆盖策略", &["SC-COVERAGE", "SC-ZONE"], &["DEFENSE_RESPONSIBILITY_CHANGED", "RESPONSIBILITY_STARTED"]),
        covered("play_lifecycle", "Play 窗口/竞争/冷却", &["SC-PLAY-END", "SC-PLAY-COMPETE"], &["PLAY_ACTIVATED", "PLAY_WINDOW_ENDED"]),
        covered("transition", "转换进攻的跨回合因果", &["SC-TRANSITION"], &["TRANSITION_STARTED"]),
        sampling_only(
            "travel",
            "走步（带球走）",
            "引擎侧已有判罚链（收球事实 + 接地脚步 + 两步预算，专项测试验证）；无专门场景登记，采样比赛中行为合法时无哨可采",
        ),
        sampling_only(
            "double_dribble",
            "二次运球",
            "引擎侧已有判罚链（收球后再突破即违例）；无专门场景登记，采样比赛中行为合法时无哨可采",
        ),
        sampling_only(
            "goaltending",
            "干扰球（干扰/ goaltending）",
            "下落段干扰判定未实现（R5 队列）；SC-BLOCK-AFTER 覆盖合法封盖侧",
        ),
        sampling_only(
            "closely_guarded",
            "贴身五秒",
            "引擎侧违规程序未实现（R5 队列）",
        ),
        sampling_only(
            "backcourt",
            "球回后场",
            "无专门场景；采样比赛中偶发（VIOLATION 事件含 backcourt 变体时可见）",
        ),
        sampling_only(
            "jump_ball",
            "争球与交替 possession arrow",
            "NBA 规则跳球只在开局（联赛档案 use_alternate_possession_arrow=false）；
             采样覆盖开局，节间与争球重跳无专门场景",
        ),
        sampling_only(
            "flagrant_foul",
            "恶意犯规（两档）与额外罚则",
            "无专门场景；FoulKind::Flagrant 在引擎存在但采样触发率极低",
        ),
        sampling_only(
            "offensive_charge",
            "带球撞人与合法阻挡的区分",
            "无专门场景；采样中 Offensive Charge 变体可见但条件覆盖不定",
        ),
        missing(
            "out_of_bounds_throw_in",
            "界外发球程序（最近点/底线/边线细分）",
            "发球基线在引擎为简化程序；细分规则程序未实现（R5 邻接队列）",
        ),
    ]
}

/// 覆盖汇总（coverage 工件用）：三态计数与缺口清单。
#[derive(Debug, Serialize)]
pub struct RuleCoverageSummary {
    pub total_domains: u32,
    pub covered: u32,
    pub sampling_only: u32,
    pub missing: u32,
    /// 未覆盖域（sampling_only + missing）的 id 与缺口说明。
    pub gaps: Vec<GapEntry>,
}

/// 缺口条目。
#[derive(Debug, Serialize)]
pub struct GapEntry {
    pub rule_id: &'static str,
    pub coverage: RuleCoverage,
    pub gap_note: &'static str,
}

/// 生成覆盖汇总。
pub fn rule_coverage_summary() -> RuleCoverageSummary {
    let registry = rule_domain_registry();
    let mut covered = 0u32;
    let mut sampling = 0u32;
    let mut missing_count = 0u32;
    let mut gaps = Vec::new();
    for entry in &registry {
        match entry.coverage {
            RuleCoverage::Covered => covered += 1,
            RuleCoverage::SamplingOnly => {
                sampling += 1;
                gaps.push(GapEntry {
                    rule_id: entry.rule_id,
                    coverage: entry.coverage,
                    gap_note: entry.gap_note,
                });
            }
            RuleCoverage::Missing => {
                missing_count += 1;
                gaps.push(GapEntry {
                    rule_id: entry.rule_id,
                    coverage: entry.coverage,
                    gap_note: entry.gap_note,
                });
            }
        }
    }
    RuleCoverageSummary {
        total_domains: registry.len() as u32,
        covered,
        sampling_only: sampling,
        missing: missing_count,
        gaps,
    }
}
