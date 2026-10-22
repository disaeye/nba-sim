//! 犯规计账：哪些犯规计入个人累计、哪些计入球队单节 bonus 计数。
//!
//! 这两件事在规则上是分开的：进攻犯规与裁判技术犯规都计入个人犯规累计
//! （个人 6 犯仍然离场），但不计入球队单节犯规，因此不会把对手送进
//! bonus。把两者合成一个计数器，会让一个进攻犯规白送对手两次罚球。

use crate::event::FoulKind;

/// 该犯规是否计入个人犯规累计（满 `max_personal_fouls` 离场）。
///
/// 技术犯规不计入个人犯规累计，按技术犯规的独立规则处理。
pub fn counts_toward_personal_limit(kind: FoulKind) -> bool {
    !matches!(kind, FoulKind::Technical)
}

/// 该犯规是否计入球队单节犯规（即 bonus 计数）。
///
/// 进攻犯规与技术犯规都不计入：前者是进攻方的失误，后者不改变球权。
pub fn counts_toward_team_period_fouls(kind: FoulKind) -> bool {
    !matches!(kind, FoulKind::Offensive | FoulKind::Technical)
}

/// 该犯规是否让对手获得罚球机会（bonus 之外的直接判罚）。
///
/// 投篮犯规按出手区域给 2 或 3 罚，这一项由出手方决定，这里只回答
/// 「是否走上罚球线」。
pub fn awards_free_throws(kind: FoulKind) -> bool {
    matches!(kind, FoulKind::Shooting | FoulKind::Flagrant)
}
