// 检查: 联防球员arrived抖动期间,是position动还是target动
// 从snapshot无法直接看target,但arrived由dist(pos, target)<=eps决定
// 如果arrived抖动但position几乎不动 → target在pos附近来回
// 检查local-react的normalizeHelpLabels: 它把help>18ft改为tag!
// 联防的翼位(rim 20-27ft)全是help/tag — normalize会改它们的label!
// 而label改变 → 速度profile改变 → 目标保持但动作变 → arrived不变(位置)
// 但normalizeHelpLabels调用setTarget(pose, pose.targetX, pose.targetY, ...) — 不改target!
console.log('分析: normalizeHelpLabels只改label不改target');
// 真正嫌疑: applyOffenseSpacing — 它对FLEXIBLE_SPACING_ACTIONS(space/weak_side/cut/relocate)的队友
// 在<6ft时nudge target! 联防球员被nudge!
// 还有: sim-tick每tick的planSpatialTargets(713行) — 只在activeBallAction===null时运行
// 联防时activeBallAction通常为null → 每tick重算zone target
// 但zone target现在已量化(固定) — 除非handlerPose的top guard在动!
// zoneSlots[0] = handlerPose! top guard target = handler位置(每帧动)
// 而slot分配是固定的: defensePlayers.map((d, i) => slot[i])
// 如果i=0的球员是top guard,他追handler — 正确
// 但其他球员的slot固定 — 应该能到达!
