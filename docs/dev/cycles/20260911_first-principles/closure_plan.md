# 闭环修复方案 · 让「正确的结论只能由机器算出」

> 文档类型：已结束周期的实施记录与历史证据。
> 历史属性：非当前状态来源；当前状态见 `docs/dev/status.md`，当前任务见 `docs/dev/current/plan.md`。
> 上游：`charter.md` §8（成功判据）、`gap.md` §18.6（硬门）、`protocol.md` §2.1（M 验收）。
> 记录纪律：本文件保留当时的命令、结果和判断，不把历史结论升级为当前事实。
> **Round 序列**：本文档记录 Round-6~9；Round-10~17 见 `docs/dev/cycles/20260911_first-principles/pass_and_identity_fix.md`。

---

## 0. 诊断：漂移只发生在「散文关节」上

### 0.1 现象

同一作者、同一仓库、同一时间段，交付物按形态分成了两个成功率：

| 交付物形态 | 实例 | 结果 |
| --- | --- | --- |
| 机器可判定 | D0 账本检查器、D1 四态 `Verdict`、D4 事件 ID + World 私有化守卫、4 个 Python 守卫 | ✅ 全部真实存在并生效 |
| 需要被相信的散文 | `docs/dev/cycles/20260901_historical/status_snapshot.md §11` 的 12 个阶段函数、M7「常数 ≤20」、M9「行为级扰动链」 | ❌ 见下 |

### 0.2 铁证（可重跑）

```bash
# M4 声称的 12 个阶段函数在 git 全历史中只出现在 status.md
git grep -n "phase_clock_advance" 4594a2d
#   → 4594a2d:docs/dev/status.md:406   （crates/ 下 0 命中）

# 实际 step_inner 行数
python3 - <<'PY'
import re
src=open('crates/engine/src/match_engine.rs').read().split('\n')
i=[n for n,l in enumerate(src) if re.match(r'\s+fn step_inner\(&mut self\)',l)][0]
print("step_inner starts at line", i+1)   # → 1108；函数体 1844 行
PY

# M7 目标 ≤20，实测
python3 scripts/check_inline_constants.py | head -1   # → 1703
```

### 0.3 第一性原理

> **可信度 = 独立第三方重算结论的成本。**
>
> 成本低 → 说谎立刻被抓 → 声明可信。
> 成本高 → 声明无意义（无论真假）。

由此得到唯一元原则：

> **每个设计决策都要用「别人证明我错了有多难」来评估。若很难，该声明本身就没有价值。**

四层方案全部是这条原则的推论。

---

## 1. L0 · 尺子必须独立于被测物

### 1.1 已记录的两个事故

| # | 事故 | 证据 |
|---|---|---|
| 1 | `scripts/inline_constant_budget.json` 与源码**同一 commit** 创建（`f0d0944`，纯新增 70 行）。基线 == 当时实测值 → 只能检测"新增"，永不推动"收编" | `git log --oneline -- scripts/inline_constant_budget.json` 只有 1 条 |
| 2 | 棘轮可被**同一个 PR**无声放宽：把某文件预算上调 10 → 守卫仍绿 | 实测 `✅ passed, EXIT=0` |

### 1.2 已落地：阈值完整性守卫

新增 `scripts/check_threshold_integrity.py`（含 5 条负面对照）。

规则：**同一提交不得同时修改「判定基准」与「被测源码」**，除非提交信息含
`Threshold-Change: <理由>` 尾注。

```bash
python3 scripts/check_threshold_integrity.py --self-test   # 5 条对照片全部通过
python3 scripts/check_threshold_integrity.py --range f0d0944~1..f0d0944
#   → ❌ COUPLED CHANGE（对真实历史事故的裁决正确）
```

这不是禁止修改基准——校准本来就要改基准。它把"改基准"从**隐形的、免费的**
动作，变成**显式的、需要署名的**动作。

### 1.3 待落地（L0 剩余）

1. `status.md` 的「现状」段改为**脚本生成**：读测试退出码 + 四项守卫输出 +
   固定种子矩阵的 `attribution_report.json`。散文只允许写原因分析/假设/教训
   （那才是 §39 撤回两个假设这类高价值内容），**禁止写数字**。
2. 里程碑判定式化：`protocol.md §2.1 M4` 的「每个 Phase 有独立单元测试」
   改为可判定式 `crates/engine/tests/phase_*.rs` 数量 ≥ 13 —— 今天就会红，
   **这才是它应有的状态**。

---

## 2. L1 · 门必须接线（已落地）

### 2.1 缺陷

`AttributionReport.hard_gate_failed` 自 D1.2 起就正确计算，但**无任何调用点消费**：
单场 / batch / evaluate 三条路径都只打印 `⛔ HARD gate FAILED` 然后正常返回。

```bash
# 修复前
nba-sim --seeds 0..1 batch full --out ci.jsonl
#   → Realism Index: 0.000 (61 defects)，hard_gate_failed=true，EXIT=0
#   → CI 的 stage-gate job 是一条永远为绿的假门
```

### 2.2 修复（已落地）

新增 `enforce_hard_gate(&report, context)`，接入**全部三条**返回路径；
`AttributionReport` 新增 `hard_defect_count` / `soft_defect_count`，报告不再
把 Hard+Soft 合计写成「Hard defects」（那会高估严重度，与被修的病同构）。

```bash
nba-sim --seeds 0..7 batch full --out s.jsonl
#   → ⛔ HARD gate FAILED [batch]: 16 Hard + 141 Soft defects (11574 judgments)
#   → EXIT=1  ✅
```

---

## 3. L2a · 代码缺陷（已修 2 项）

### 3.1 `TURNOVER_ATTRIBUTION` 98 条 Hard —— 归因被覆盖

**根因**（`start_out_of_bounds_transition`）：松球出界时无条件以
`TurnoverViolation` 结算，**覆盖**了球出界前已确定的失误原因（掉球/点掉/抢断）。
且 `OUT_OF_BOUNDS` 在事件流中出现 **0 次** —— 出界这一事实在因果账本里不存在。

**为什么 D0.1 没抓到**：D0.1 只校验"归因字段非空"，而错配的归因**字段是满的**。
正确口径是**类别一致**。

**已修**：保留既有 `pending_loose_ball_terminal`；新增
`GameEvent::BallOutOfBounds` 事实。

**回归测试**（`crates/engine/tests/attribution_integrity.rs`，先红后绿）：

```bash
cargo test --release -p nba-engine --test attribution_integrity -- --nocapture
# 修复前: 8 seed 共 108 条 mismatch，2 tests FAILED
# 修复后: 2 tests ok
```

### 3.2 `POSSESSION_DURATION_BOUNDS` —— 上界隐含「每回合最多 1 个进攻篮板」

**根因**：上界硬编码为 `24 + 14 + 2 = 40`（`_rebounds` 字段声明了但从未写入）。
实测 seed 1 possession 148 有 **3 个进攻篮板、43.6s**（其中一次 3 个 ORB 的窗口
实际活球仅 3.5s、墙钟却 51.6s）——**合法**回合被判 Hard。

**已修**：按本回合实际进攻篮板数动态推导；数值来源遵守 `gap.md §8.2` 单一事实源
（进攻时钟取自帧携带的 `FrameRules`，此前 `rules_cache` 声明却未消费；
重置量与容差进入 fixture 数据契约）。

**过程教训（自我否证，如实记录）**：我第一次把上界改成
`base + reset×ORB` 得到 405 条（更糟），第二次用 `base+reset+tol` 得到 351 条。
两次都是**收紧**了原本宽松的经验包络。正确做法是**保持对 ORB≤1 的原包络不变、
只按额外 ORB 放宽** → 11 条降到 1 条。教训：改判定式时必须先证明新式对
**已知通过样本**仍然通过。

---

## 4. L2b · 缺失机制：防守执行器（未落地，最高杠杆）

### 4.1 决定性证据（独立复现）

6 种防守方案各跑一场全场模拟，对**行为字段**（排除展示字符串）计算哈希：

```text
def_man_conservative   ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
def_man_pressure       ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
def_switch_heavy       ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
def_drop_coverage      ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
def_hedge_recover      ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
def_zone_23            ticks=84029 score=96081 hash=0x3ba37e7fa9e5ec7d
```

**逐字节相同，包括 2-3 联防。** 代码层印证：`home/away_defensive_tactic`
全仓库仅 6 处引用 = 2 处声明 + 2 处赋值 + 2 处 `name_zh()`（生成展示字符串）。

进攻侧同样：6 个已声明战术中 **4 个 panic**（`TacticalSetSpec::builtin` 只认 2 个）。

### 4.2 为什么这是最高杠杆

三个看似独立的症状是同一根因：

| 症状 | 引用 |
| --- | --- |
| 防守战术零影响 | `impact_assessment.md §1` |
| 48% 回合零传球；每回合 1.61 次（真实 3.5） | `docs/dev/cycles/20260911_first-principles/status_history.md §39.4` |
| 41.6% 回合以违例终结 | `impact_assessment.md §2` |

`§39.4` 已给出精确表述：**传球不改变后续出手的价值期望**。原因是防守方
**不改变责任分配**（`plan_possession_targets` 只做对位跟随；`DefensiveTactic`
只用于生成展示字符串）。进攻在跟一个不会轮转的障碍物场打球 → 传球不能制造
空位 → 传球无收益 → 理性持球人不传。

**修好防守轮转，三个指标同时改善；不修，全部调不动**（`§39.3` 已用实验证明：
`dwell_base` 砍到 1/4 只提升 8% 传球）。

### 4.3 性质：不是从零实现，是接线

`crates/decision/src/defense.rs` **已存在**：12 个候选动作
（`SwitchAssignment`/`DropAndContain`/`HedgeAndRecover`/`NavigateScreenOver`…）、
`evaluate_gamble_interception`、`evaluate_rim_help_vs_shooter`、风险/收益权衡。
但 `DefensiveContext` **全 crate 无构造点、无调用者**。

### 4.4 验收门（可判定）

```bash
# 门：不同防守方案必须产生不同的行为哈希与不同的回合终端分布
# 当前：完全相同 → 红
# 修复后：逐字段比对产生差异（注意：不能用哈希差异作为唯一证据，
#         展示字符串也会让哈希不同——见 impact_assessment.md §3）
```

---

## 5. L2c · 参数校准（未落地，必须在 L2b 之后）

当前所有进攻参数都是在「防守不响应」的世界里拟合的。**换掉防守后全部失效**，
因此校准必须排在 L2b 之后。

在此之前，`nba.v2.json` 的 composition bands 给出了客观靶子（`provenance: prior`，
项目自述"禁止以模拟输出反标定"）：

| 指标 | 8 seed 实测 | 带 | |
| --- | ---: | ---: | --- |
| 3PA 占比 | 0.506 | [0.30, 0.45] | ❌ |
| 2PA 占比 | 0.494 | [0.55, 0.70] | ❌ |
| 2P% | 0.596 | [0.48, 0.58] | ❌ |
| FT 率 | 0.119 | [0.20, 0.35] | ❌ |
| 回合/48min | 230 | [185, 220] | ❌ |
| 3P% | 0.365 | [0.30, 0.40] | ✅ |

---

## 6. 不在本方案内（明确不做）

1. **不重冻黄金哈希**。`golden_hash.rs` 有 **19 个版本号缺失**（v11–14、v30–34），
   每次标注"设计内行为修复"。在单一 RNG + `std::mem::replace` save/restore 结构下，
   **无法区分"授权修复"与"RNG 序列错位"**。应先按 `gap.md §17.1` 拆命名流。
2. **不用"N/20 seed 零违规"作为验收**。那只覆盖 L1：8 场 full 的 L1 是 0，
   同时 L2 是 118 条 Hard。此措辞会误导读者认为比赛是干净的。
3. **不先拆 `step_inner`（1844 行）**。它是行为中性重构，必须最后做、用 L0+L1 验证。
4. **不修改 `scripts/inline_constant_budget.json` 的既有预算值**。
   本轮的常数变化来自真实收编（1706 → 1703）。

---

## 7. 本轮已落地的全部变更与证据

| 文件 | 变更 | 证据 |
| --- | --- | --- |
| `scripts/check_threshold_integrity.py` | 新增阈值完整性守卫 + 5 条对照片 | `--self-test` PASS；对 `f0d0944` 正确判红 |
| `crates/domain/src/event.rs` | 新增 `GameEvent::BallOutOfBounds` | `OUT_OF_BOUNDS` 事件 0 → 15/场 |
| `crates/engine/src/match_engine.rs` | 出界不覆盖既有失误原因；`PeriodEnd` 节末显式结算 | `TURNOVER_ATTRIBUTION` 98 → **0**；跨节回合 duration 40.1s → 22.3s |
| `crates/evaluator/src/lib.rs` | 时长上界按实际 ORB 推导；`hard/soft_defect_count` | 时长 Hard 11 → 1 |
| `crates/evaluator/src/fixture.rs` + 3 个 fixture JSON | 时钟政策进入数据契约（`gap.md §15.5`） | 不再硬编码 |
| `crates/cli/src/main.rs` | `enforce_hard_gate` 接入 3 条路径；`pct()` 单一换算点 | `batch`/`single`/`evaluate` EXIT=1 |
| `crates/engine/tests/attribution_integrity.rs` | 2 条先红后绿回归测试 | 108 mismatch → `2 passed` |

### 7.1 8 seed full 前后对比（CI 自己的命令）

```bash
nba-sim --seeds 0..7 batch full --out s.jsonl
```

| | Hard 总数 | 构成 | 退出码 |
| --- | ---: | --- | --- |
| 修复前 | **118** | `TURNOVER_ATTRIBUTION` 98 + `PASS_CORRIDOR_REACHABLE` 12 + `POSSESSION_DURATION_BOUNDS` 8 | **0** ❌ |
| 修复后 | **16** | `PASS_CORRIDOR_REACHABLE` 12 + `TURNOVER_ACTOR_CONSISTENCY` 3 + `POSSESSION_DURATION_BOUNDS` 1 | **1** ✅ |

### 7.2 全量测试与守卫

```text
cargo test --release --workspace   → 42 个测试二进制，0 failed
check_inline_constants.py          → 1703（↓3），passed
check_world_privacy.py             → passed（57 真值字段全私有）
check_docs.py                      → 历史版本的引用守卫记录
check_threshold_integrity.py       → --self-test PASS
check_disk_budget.py --report      → 余量 4.4 GiB ≥ 3.0
```

---

## 8. 下一步顺序（依赖驱动）

```text
L0 剩余（status 生成 + 里程碑判定式化）   ← 否则无法判断是否在赢
   ↓
L2b 防守执行器接线                        ← 唯一模型级杠杆，会改变一切统计
   ↓
L2c 参数校准（走 JSON override A/B + fixture 证据包）
   ↓
L2 结构性重构（step_inner 拆分、RNG 命名流）
```

**为什么 L2b 不能在 L2c 之后**：当前进攻参数是在防守不响应的世界里拟合的。

**为什么 L0 在最前**：没有独立尺子，后面每一步都无法验收——这正是本项目
M4「~90%」得以写成的原因。

---

## 9. Round-6 追加：`PASS_CORRIDOR_REACHABLE` 12 → 8，并区分两类成因

### 9.1 已修：评判器按 id 配对（12 条中的 4 条为误报）

**根因**：`PASS_CORRIDOR_REACHABLE` 按 `receiver_id` 字符串配对 release 与
reception。当同一接球人在一个回合内多次接球、且其中一次传球被点掉/掉落
（不产生 `PASS_RECEIVED`）时，会把**更早的释放**与**更晚的接球**凑成一对。

实测 seed 1 full（按 id 配对 vs 按顺序配对）：

```text
按 id 配对：    4 条 Hard，其中 3 条是错配
   rel_t=266  rec_t=292  gap=1.0s  A_2->A_1  dist=55.3ft
   rel_t=1236 rec_t=1259 gap=0.9s  A_1->A_2  dist=20.2ft
   rel_t=7484 rec_t=7513 gap=1.2s  A_2->A_1  dist=52.9ft
   rel_t=7592 rec_t=7599 gap=0.3s  A_1->A_4  dist= 5.1ft   ← 唯一真实
按顺序配对：    1 条
```

`gap` 只有 0.3–1.2s，却算出 52–55 ft 的距离——这是错配的决定性特征。

**已修**：`PassRelease` / `pass_received` 携带 `sequence`；新增
`pass_terminations`（点掉/掉落按 sequence 标记），配对规则改为
「取 sequence 更小、receiver 相同、未被终结、未被占用的最近释放」。
8 seed 的 `PASS_CORRIDOR_REACHABLE`：**12 → 8**。

### 9.2 未修：8 条真实缺陷——接球点越过冻结点

剩余 8 条经逐条插桩确认为**真实引擎缺陷**，不是误报：

```text
seed 3  rel=704  rec=708  H_3->H_1  dist=9.4ft
    from=(10.8, 36.8) to=(56.3, 24.8)   recv_pos=(65.4, 22.9)   |recv-to|=9.37ft
seed 3  rel=1996 rec=2004 A_1->A_5  dist=5.3ft
    from=(97.1, 25.0) to=(43.0, 11.5)   recv_pos=(38.7,  8.4)   |recv-to|=5.29ft
seed 3  rel=6093 rec=6098 A_1->A_2  dist=12.4ft
    from=(97.0, 25.0) to=(33.4, 22.2)   recv_pos=(21.1, 23.9)   |recv-to|=12.43ft
```

**签名一致**：接球人沿传球方向**越过**冻结点 5–12 ft，且 `dist == |recv_pos − to_pos|`
（即球最终贴在球员身上，而球员没停在冻结点）。

**机制**（`match_engine.rs:1959-1995`）：`receiver_ready` 的判定是
「接球人距冻结点 ≤ `invariant_holder_leash_ft`」。若接球人**在 tau≥1.0 的那一
tick 恰好处于该 leash 内但已冲过冻结点**，引擎会立即发 `PassReceived`，
并把**球的实际位置**（贴在球员身上）写入 fact —— 此时球已不在冻结点。

**这不是评判器问题**：引擎 emit 的 `PassReceived.position` 忠实反映了球的
位置；问题是**接球判定允许球员冲过冻结点**，使「传球终点」这一冻结事实
与实际接球位置脱节。gap.md §9.5 要求 release 时冻结
`from_pos / frozen_to_pos / release_tick`，接球人应「向 frozen_to_pos 收敛」——
当前实现只保证「收敛到 leash 内」，未保证「不越过」。

**修复方向**（未实施，需先补回归测试）：

1. 接球判定改为「到达冻结点附近**且**在飞行末端」（增加时间维度约束），或
2. 接球人运动目标在传球飞行期间锁定为 `frozen_to_pos`，到达即减速 —— 依赖
   `ActionTimeWindow` 与战术目标覆盖的交互，属 D5 范围；
3. 若选择「球收敛到人」（当前 `ControlTransfer` 路径），则 `PassReceived.position`
   必须同时携带 `frozen_to_pos`，让评判器能区分「终点事实」与「接球事实」。

**处置**：登记为 D5 的前置项；`gap.md §9.5` 已明文禁止「引擎用接球人当前位置
采样 / 事件保存旧目标点 / 评判器再用第三个位置解释」三者并存——当前正是这种状态。

---

## 10. Round-7：L2b 防守执行器接线（已落地）

### 10.1 缺陷（复现证据）

`DefensiveTactic` 对比赛结果**零影响**。全仓库仅 6 处引用 = 2 处字段声明 +
2 处赋值 + 2 处 `name_zh()`（生成展示字符串）。新增
`crates/engine/tests/defense_effect.rs` 作为门，**先红**：

```text
def_man_conservative   mean_def_dist_to_hoop= 60.54ft  TO=112 score=767
def_zone_23            mean_def_dist_to_hoop= 60.54ft  TO=112 score=767
def_drop_coverage      mean_def_dist_to_hoop= 60.54ft  TO=112 score=767
  → defensive scheme must change defensive geometry: max spread 0.000 ft. FAILED
```

（首版测试还有我自己的度量 bug：把防守方保护的篮筐取反了，导致三种方案都
读出 ~61 ft。正确口径是 `attacking_right = (possession == Home)`，
即 home 攻右篮。修正后基线为 29.94/29.22/28.60 ft。）

### 10.2 修复

1. **`data/defense/schemes.json`**（新数据档案）：六个方案的
   `sag_multiplier` / `on_ball_gap_multiplier` / `help_priority` /
   `switch_aggressiveness`，外加 `help_blend` 基准权重。
2. **`DefenseRules`**（`domain/rules.rs`）：`for_scheme(id)` 从数据档案查表，
   `Default` **从同一数据源派生**中性档案（消除重复定义）。
   未知 id 返回 `None` —— 不静默回退。
3. **`sync_team_tactics`**：把**防守方**的方案经规则通道写入
   `rules.tactics.defense`（charter C1/C3：数据通道，非代码分支）。
4. **`plan_possession_targets_with_geometry`**：领防人间隔与弱侧协防方向
   消费 `DefenseRules`，`help_priority == 0.5` 时**逐位复原**历史公式
   `to_hoop*0.7 + to_carrier*0.3`。

### 10.3 中性性证明（黄金哈希 v49 的授权依据）

把双方防守方案都置为中性档案（`def_man_conservative`）后，seed42×2000 的哈希为

```text
NEUTRAL hash = 0x97c7de28a95cb3d5   (historical v48 = 0x97c7de28a95cb3d5)
```

**逐位相同**。因此 v49 的漂移**不是** RNG 错位或未授权行为变化，而是
「默认阵容使用的 `drop_coverage` / `man_conservative` 开始真实生效」。

### 10.4 顺带修复：接球减速模型（黄金哈希 v50）

**缺陷**：接球人被硬编码为 20 ft/s 全速冲向 `frozen_to_pos`，**无减速模型**。
传球飞行时长受 `max_pass_duration_seconds`（1.4s）封顶，但 40–60 ft 的跨场
outlet 实际只需 0.5–0.7s，接球人在被拉长的飞行期内持续全速前进，实测
**越过**冻结点 5–12.5 ft。后果：`PASS_CORRIDOR_REACHABLE` 报 8–17 条 Hard。
这违反 `gap.md §12.1`「加速、制动、变向受属性与规则上限约束」。

**修复**：新增 `receive_approach()`，按制动距离反解允许速度

```text
v_allow = sqrt(2 · a_max · max(d_remaining - margin, 0))    # 受倍率封顶
```

并把瞄准点提前 `receive_stop_margin_ft`；同时把 `PassReceived.position` 在
`Pass` 分支固定为 `to_pos`（终点事实），`ControlTransfer` 分支保留球的
实际位置（接球事实）——两者语义分离，消除 `gap.md §9.5` 禁止的「三个位置并存」。

**效果**：`PASS_CORRIDOR_REACHABLE` 8 seed **17 → 10**；L1 violations 仍 0。

### 10.5 三指标联动验证（§4.2 的预测）

`docs/dev/cycles/20260911_first-principles/status_history.md §39.4` 预测「修好防守轮转，三个指标同时改善」。实测：

| 指标 | 修复前 | 修复后 | 方向 |
| --- | ---: | ---: | --- |
| `RHYTHM_DURATION`（soft） | 80 | **55 → 70** | 改善 |
| `PASS_CORRIDOR_REACHABLE`（hard） | 8 | **17 → 10** | 中间回归后净改善 |
| 防守方案是否因果 | **否**（spread 0.000ft） | **是**（TO 126 vs 113；score 781 vs 814） | 质变 |

**预测部分成立**：防守方案从「装饰」变成「因果」是质变，节奏类缺陷改善；
但**传球组织度（1.61 vs 3.5）与违例终结未改善**——说明 `§39.4` 的
「传球不改变出手价值期望」还缺另一半：防守方**轮转后重新分配责任**
（closeout / help / recover），而本轮只做了「站位倾斜」，没有做「责任重分配」。

### 10.6 仍未完成（诚实登记）

1. `PASS_CORRIDOR_REACHABLE` 剩 10 条：全部是 `(97.0, 25.0)` 基准发球点的
   outlet 长传，接球人仍越位 5–12 ft。减速模型把越位量从 8–17 降到 5–12，
   但**未消除**。根因在「传球飞行时长（1.4s 上限）与接球人跑动距离不匹配」，
   需要让 `pass_duration` 与接球人的合理制动距离互相约束，属独立一轮工作。
2. 防守的**责任重分配**（switch/drop/hedge 的执行链）未做：本轮只有站位倾斜。
   `decision/src/defense.rs` 的 12 个候选动作仍无构造点。
3. `TURNOVER_ACTOR_CONSISTENCY` 3 条、`POSSESSION_DURATION_BOUNDS` 1 条未修。

### 10.7 守卫交互（值得记录）

本轮 `check_inline_constants.py` 三次变红，三次都是**它对了**：

| 次 | 触发 | 处理 |
| --- | --- | --- |
| 1 | 六方案参数写成代码常量（+26 floats） | 移到 `data/defense/schemes.json`；`tactic_ids` 6→0 |
| 2 | `DefenseRules::default()` 重复定义 JSON 里的值 | 改为从同一数据源派生 |
| 3 | 制动模型的 4 个字面量 | 移入规则通道（`help_blend` / `receive_*`） |

最终棘轮更新（`rules.rs` 447→450、`match_engine.rs` 187→188、
`tactics.rs` 69→71）是**新增规则通道参数本身**，属合法增长；
且本次改动同时修改了基准与源码，被 `check_threshold_integrity.py` 正确判红——
按该守卫的设计，这需要显式署名（`Threshold-Change:` 尾注）才能落地。

---

## 11. Round-8：领传实验（两个假设被否证，指标持平）

### 11.1 起点

`PASS_CORRIDOR_REACHABLE` 在 round-7 后剩 10 条。插桩发现全部集中在两类：

- `from=(97.0, 25.0)` / `(-3.1, 25.0)`：**发球点**（inbound / outlet）
- 越位特征：`|recv_pos − to_pos| == dist`（接收人整体越位，非偏离走廊）

### 11.2 假设一（**部分成立**）：outlet 一传没有领传

`start_rebound_outlet` 把 `to_pos` 冻结为接球人**释放时刻**的位置，而接球人在
飞行期间继续跑动 —— 他永远不能恰好停在冻结点。改为调用
`lead_receiver_position()` 领传。

**结果：成立但非主因**。单独改这一处后仍有 9 条。

### 11.3 假设二（**被否证**）：`execute_pass` 也应叠加领传

推论：`target_lead_pos = to_pos` 这个名字有 lead 但没有实际外推，所以所有传球
都缺提前量。

**实验与结果：否证。** 在 `execute_pass` 里也调用 `lead_receiver_position()` 后，
`PASS_CORRIDOR_REACHABLE` 由 **9 → 28**（显著恶化）。

**原因**：决策层给出的 `CandidateAction::Pass.to_pos` **已经包含提前量**
（决策时按接球人运动外推），再叠加一次会把落点推到接球人在飞行期内
**到不了**的地方 —— 减速模型（round-6）使越位更严重。

已回退，并把该实验结论写进代码注释，防止后续重犯。

### 11.4 被排除的第三个解释：batch 与 single 路径不一致

中途观察到「batch 报 5 条、单场重放报 0 条」的疑似路径分歧。**实测证伪**：
分歧来自**我自己的检查脚本**——`--stream-mode frames` 的记录**没有 `frame`
包装**（记录本身就是 frame），我的脚本读 `t['frame']` 恒为空。修正脚本后单场
重放同样报 5 条，与 batch 一致。

这条记录的教训与 `impact_assessment.md §3` 同类：**观测工具的错误会伪装成
被测系统的缺陷**。纪律：报告分歧前先自检度量脚本。

### 11.5 净效果（8 seed full）

| 指标 | round-7 | round-8 | |
| --- | ---: | ---: | --- |
| `PASS_CORRIDOR_REACHABLE` | 8 | 9 | 持平 |
| `RHYTHM_DURATION` | 70 | **62** | 改善 |
| `TURNOVER_ACTOR_CONSISTENCY` | 3 | 4 | 略升 |
| Hard 合计 | 14 | 15 | 持平 |
| L1 violations | 0 | 0 | ✅ |

**诚实结论**：本轮的主要产出是**否证两个假设并文档化**，不是指标改善。
`PASS_CORRIDOR_REACHABLE` 的剩余 9 条需要不同性质的修复，见 §11.6。

### 11.6 剩余 9 条的正确修复方向（未实施）

越位量 5–12 ft 且方向沿传球线，说明**接球人的制动距离 > 传球飞行距离**：
他跑到落点时已经没有余量减速。三个可能的方向，需要实验区分：

1. **缩短飞行时长**：`min_pass_duration_seconds = 0.45s` 对 50 ft 传球意味着
   约 111 ft/s，但 `ball_max_speed_ftps = 85` —— 该传球在物理上不可达，
   引擎却允许。**这可能是真正的模型缺陷**：`pass_duration` 与
   `ball_max_speed` 之间没有约束关系。
2. **按接球人可达性约束落点**：若接收人在飞行期内无法到达，落点应改为
   他的可达点，而不是理想点。
3. **放宽评判口径**：走廊判定允许「接球人已在减速过程中」的容差。
   （**不推荐**：这是放宽门，属 `impact_assessment.md` 批评的模式。）

方向 1 最值得先验证：它同时解释「为什么长传总是出问题」。

---

## 12. Round-8 结论：制动距离自洽（Hard 15 → 12）

### 12.1 真正的根因（插桩定位）

`receive_stop_margin_ft` 是一个**固定**裕量（2.5 ft），但接球人所需的制动距离是
`v²/(2a)`，随接近速度增大：

```text
19.8 ft/s -> 需 5.60 ft
12.0 ft/s -> 需 2.06 ft
 5.5 ft/s -> 需 0.43 ft
```

当裕量（2.5 ft）**小于**所需制动距离（如 5.60 ft）时，「已到位则停下」的分支
**永远不可达** —— 接球人一边被 `sqrt(2as)` 减速、一边因 `d > margin` 继续被推着
走。逐 tick 实测（seed 1, rel seq=3484，45.3 ft 传球飞行 35 tick）：

```text
t=37612 PASS          A_5=(59.90,13.20)
...
t=37630 已到冻结点     A_5=(53.32,12.50)  距 to_pos(53.58,12.15) 仅 0.43 ft
...
t=37649 PASS_RECEIVED A_5=(47.75,11.00)  ← 又跑了 19 tick × 0.35 ft = 6.7 ft
```

**修复**：用物理所需制动距离 `v²/(2a)` 取代固定裕量（取两者较大值），使
`d <= stop_margin` 分支真正可达；进入后速度为 0（站住等球）。

### 12.2 净效果

| 指标 | round-7 | round-8 |
| --- | ---: | ---: |
| Hard 合计 | 15 | **12** |
| `PASS_CORRIDOR_REACHABLE` | 10 → 9 | 9 |
| `POSSESSION_DURATION_BOUNDS` | 1 | **0** |
| `RHYTHM_DURATION` | 70 | 66 |
| L1 violations | 0 | 0 |

### 12.3 本轮否证的两个假设（已文档化）

| 假设 | 验证方式 | 结果 |
| --- | --- | --- |
| `execute_pass` 应叠加领传 | 实现后实测 | **否证**：判定 9 → **28**（恶化）；决策层已含提前量，叠加使落点不可达。已回退并在代码注释登记 |
| 飞行时长要求超过 `ball_max_speed` | 纯算术 | **否证**：50 ft 传球等效速度 35.7 ft/s < 85 上限；`pass_duration` 与球速无冲突 |

### 12.4 归档：`PASS_CORRIDOR_REACHABLE` 剩余 9 条

剩余越位 5–7 ft，分布在 7 个种子，无单一发球点集中特征。它们需要
**接球人与传球时序的联合约束**（决策层在选落点时就应该考虑接球人的制动能力），
属决策/物理交叉，登记为下一轮对象，**不在本轮继续打补丁**。

**纪律记录**：本轮我连续做了三次「先改代码再验证」的尝试（叠加领传、固定裕量
归零、制动距离自洽），前两次被实测否证。正确顺序应是先做敏感性/算术分析再改
代码 —— 这正是 本周期历史状态记录 §39.7 已经写下的教训，我重复了一次。

---

## 13. Round-9：归因链的架构修复（Hard 12 → 9，且黄金哈希不变）

### 13.1 方法：从权威状态派生，而非补 fallback

`TURNOVER_ACTOR_CONSISTENCY` 3 条 Hard（`turnover_player_id` 为空）。
我没有加回退链，而是按 P1「状态是唯一事实源」逐层追问：**责任球员能否从权威
球态读出？** 用独立探针（`MatchEngine::ball_state()` 公开访问器，不需改引擎）
逐 tick 观察，得到三处**架构性**缺陷：

| # | 缺陷 | 证据 | 修复 |
| --- | --- | --- | --- |
| 1 | `Dead` 只带 `last_touch_team`，**不带球员** | 探针：`Dead { last_touch_team: Home, last_touch_player: None }` | `Dead` 增加 `last_touch_player` 载荷；新增唯一构造入口 `dead_state()`，从权威球态派生 |
| 2 | `settle_ball_for_period_break` 在构造 `Dead` **之后**立即清空 `last_passer_id` | 载荷刚写入就被抹掉 | 删除该清空：载荷已是唯一事实源，无需旁路字段存续 |
| 3 | `ControlTransfer` 载荷里**已有** `carrier_id`，派生却忽略它 | `ControlTransfer { carrier_id, .. } => self.last_passer_id.clone()` | 直接读 `carrier_id` |
| 4 | outlet 一传（`start_rebound_outlet`）**未设置** `last_passer_id` | 探针：`REBOUND → PASS → Dead(None)` | 与 `execute_pass` 一致地记录传球人 |

第 1 条是关键：`Dead` 缺载荷使「谁最后触球」这一事实**在状态里不存在**，
只能靠旁路字段的存活期——这正是 `gap.md §4.3` 禁止的「多源真相」。

### 13.2 结果

| 指标 | round-8 | round-9 |
| --- | ---: | ---: |
| `TURNOVER_ACTOR_CONSISTENCY` | 3 | **0** |
| Hard 合计 | 12 | **9** |
| L1 violations | 0 | 0 |
| **黄金哈希** | `0x9af1ff1c8d3a5710` | **不变** |

**哈希不变本身就是证据**：这四处都是**派生读取**的修正（从已有载荷读，而不是
改行为），所以逐 tick 轨迹不应改变。这比「重冻哈希 + 声称是设计内修复」强得多 ——
它可被第三方独立验证。

### 13.3 剩余 Hard 9 条（全部同一类）

`PASS_CORRIDOR_REACHABLE` 9 条，越位 5–7 ft。经 round-7/8 的排除，已确认：

- 不是评判器配对错误（已修 id→sequence 配对）
- 不是领传缺失（outlet 已补；`execute_pass` 叠加会恶化 9→28）
- 不是制动模型缺失（已按 `v²/(2a)` 自洽）
- 不是 `pass_duration` 与 `ball_max_speed` 冲突（算术否证）

剩下的唯一解释是 **决策层选落点时未考虑接球人的制动能力**：落点在其
「奔跑方向前方」足够远，但到达时没有余量减速。这需要决策层与物理层的
**联合约束**（落点必须在接球人的可达减速包线内），属跨子系统设计，不是补丁。
