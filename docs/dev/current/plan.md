# 当前周期计划 · 从“可证明”推进到“体系化落地”

> 文档类型：当前未完成工作的执行计划。
> 规则：本文件不复述历史 Round；任务关闭后只把结论写入 `docs/dev/status.md`，把过程留在周期归档。
> 依赖入口：稳定设计见 `docs/architecture.md`、`docs/quality.md`、`docs/tactics.md`；取舍见 `docs/decisions.md`；当前判断见 `docs/dev/status.md`。
> 编号：本周期任务块使用 `D14–D21`（历史周期 `D0–D6` 见 `cycles/20260911_first-principles/`，`D7–D13` 见 `cycles/20260916_convergence/`；编号规则见 `docs/dev/README.md` §4）。

## 0. 周期目标

上一周期（`20260916_convergence`）已完成公共边界私有化、球态转移穷举测试、篮板端到端扰动、NBA/FIBA 情景矩阵和统计门全部关闭（七项构成指标全绿）。

本周期聚焦上周期结转的结构性债务与未接线子系统，完成体系化落地：

1. **统一快照**：落地最小只读 `snapshot` 投影，消除外部直接依赖内部 getter 的分散耦合；
2. **球态纯化**：解耦 `carrier_idx`，消除双重 holder 依赖；
3. **阶段拆分**：将 2082 行的 `step_inner` 拆分为窄签名阶段函数；
4. **防守责任链**：补齐 `schemes.json` 数据资产，打通 switch/drop/hedge/recover 责任链；
5. **零消费处置**：接线或清理 `GameRules` 21 个零消费字段与 6 个零消费能力维度；
6. **全维度覆盖**：为剩余能力维度建立端到端因果扰动与负面对照测试；
7. **多联赛余量**：覆盖 FIBA 特有的交替拥有与罚球进出情景。

## 1. 执行纪律

- 一个任务只有在代码、针对性测试和守卫都通过后才能关闭；
- 运行结果必须标明 seed、scope、league profile 和输出模式；
- 行为参数改动先用规则覆盖做 A/B，再决定是否改默认值；
- 结构重构与行为校准分开提交和验收；
- 统计基准保持守卫：任何改动必须保证 `./scripts/run-tests.sh` 45 套件全绿，且 16-seed 构成指标不破带；
- 当前状态只更新 `docs/dev/status.md`，原始输出进入 `docs/dev/evidence/`。

## 2. 任务依赖

```text
D14 最小只读 snapshot 投影 ───────────► D21 周期出口与归档
  │                                           ▲
  ├─► D15 carrier_idx 解耦与球态归一           │
  │     └─► D16 step_inner 窄签名拆分         │
  │                                           │
  ├─► D17 防守方案责任链落地                   │
  │     └─► D18 GameRules/能力零消费处置      │
  │                                           │
  ├─► D19 剩余能力维度因果扰动覆盖             │
  └─► D20 FIBA 交替拥有与罚球情景覆盖 ─────────┘
```

## 3. D14 · 落地 `MatchEngine` 最小只读 `snapshot` 投影

### 3.1 背景与现状

上周期（D7.1）已将 16 个真相字段全部私有化，但外部调用者（CLI、评判器、测试、回放）目前通过散落的只读 getter 获取状态。按 `docs/architecture.md`，应当提供轻量级不可变只读投影。

### 3.2 工作项

- 在 `crates/engine/src/snapshot.rs`（或等价位置）定义只读 `EngineSnapshot`；
- 包含回合状态、时钟、比分、球态、球员位置与动作摘要；
- 迁移 CLI 与 evaluator 消费点，减少对单个 getter 的直接调用；
- 验证黄金哈希与事件流不受影响。

### 3.3 出口门

- CLI、evaluator、tests 统一从 `engine.snapshot()` 获取只读视图；
- 无性能退化（基准 tick 耗时保持 < 50µs）；
- 全测试套件通过。

## 4. D15 · `carrier_idx` 解耦与球态归一

### 4.1 背景与现状

上周期调查（`evidence/problem.md` §25）确认直接删除 `carrier_idx` 会使 8-seed `total_p50` 产生非预期漂移（213.5→212.0）。原因在于无持球人球态（`LooseBall`、`RimRebound`、`Dead`）下，旧逻辑回退到名单同序球员，新逻辑为 `None`。

### 4.2 工作项

- 依据 `docs/gap.md` §5.2 状态边定义，形式化澄清无持球人球态下的球权与动作归属；
- 将 `carrier_idx` 的隐式名单索引彻底替换为明确的球态枚举属性；
- 对齐行为后切断旁路字段写入；
- 验证 16-seed 统计指标与 L1 账本保持零违规。

### 4.3 出口门

- 不存在由两个可写字段共同决定 holder/possession 的路径；
- `carrier_idx` 字段从引擎结构体中移除；
- 全套件与构成指标测试通过。

## 5. D16 · `step_inner` 窄签名阶段拆分

### 5.1 背景与现状

`crates/engine/src/match_engine.rs` 中的 `step_inner` 当前长达 2082 行，单体函数聚合了决策、运动推进、冲突与犯规判定、统计写入与事件发射，阶段写权限与执行顺序缺乏编译期约束。

### 5.2 工作项

- 按 `docs/architecture.md` 拆分四个窄签名阶段：
  1. `phase_decision(&self, ...) -> DecisionBundle`
  2. `phase_motion(&mut self, &DecisionBundle, ...)`
  3. `phase_resolution(&mut self, ...)`
  4. `phase_accounting_and_events(&mut self, ...)`
- 阶段之间通过参数显式传递不可变输入；
- 每一阶段提供独立的隔离单元测试。

### 5.3 出口门

- `step_inner` 成为纯调度器，代码行数降至 < 200 行；
- 各阶段函数有明确的窄输入与输出；
- 45 测试套件全绿，黄金哈希确定性保持或有意受控迁移。

## 6. D17 · 防守方案责任链落地

### 6.1 背景与现状

`evidence/problem.md` §28 实证：`DefensiveSystem` 的四个子配置（`SchemeAssignments` 等）零消费，`decision/src/defense.rs` 的评估函数零调用。`data/defense/schemes.json` 目前仅有 4 个几何倍率，缺乏 switch/drop/hedge/recover 的行为规则字段。

### 6.2 工作项

- 扩展 `data/defense/schemes.json`，补充责任链所需规则参数；
- 接通 `decision/src/defense.rs` 中的责任指派函数，使防守人按方案执行具体防守动作；
- 为 Drop、Switch、Hedge、Blitz 分别建立控制场景用例；
- 建立反事实场景测试：方案改变必须带来防守责任与对位几何的显著差异。

### 6.3 出口门

- 至少三种防守方案（Drop、Switch、Hedge）在控制测试中表现出符合战术定义的结构化责任差异；
- `DefensiveSystem` 字段不再全零消费；
- 统计指标保持在带。

## 7. D18 · `GameRules` 与能力维度的零消费处置

### 7.1 背景与现状

`evidence/problem.md` §27 逐一实证了 21 个 `GameRules` 零消费字段；§29 实证了 6 个零消费能力维度（`agility`、`shooting_close`、`cut_frequency`、`screen_frequency`、`offensive_rebound_frequency`、`transition_sprint`）。

### 7.2 工作项

- 逐个子系统分类处置：
  - 属于未开发子系统（如 transition、clutch）的字段：若不在本周期范围内，移入明确的 feature flag 或文档化暂不启用清单；
  - 属于本周期范围（防守、犯规、投篮）的字段：接通消费链并编写响应测试；
  - 确认废弃的冗余字段：安全删除并同步更新 schema 与 default；
- 同步更新 6 个零消费能力维度，建立接线或标记清单。

### 7.3 出口门

- 活跃 `GameRules` 与 `PlayerAttributes` 中的每一个保留字段都有代码消费或有明确的未启用状态注解；
- 无静默死字段；
- 全测试套件通过。

## 8. D19 · 剩余能力维度因果扰动覆盖

### 8.1 背景与现状

上周期（D10.2）已为篮板三维度（`rebounding_offensive`、`rebounding_defensive`、`vertical`）补齐端到端因果扰动。全套 29 维度中，仍有 `passing`、`shooting_mid`、`decision_iq`、`strength` 等核心维度缺少单调性与断路测试。

### 8.2 工作项

- 为 `shooting_mid`、`passing`、`decision_iq`、`strength`、`perimeter_defense`、`interior_defense` 建立扰动测试套件；
- 每项测试包含：
  - 单调性检验（能力提升则对应产出单调提升/失误单调下降）；
  - 故意断路负面对照（断路必红、恢复必绿）；
- 统一集成至 `crates/engine/tests/attribute_perturbation.rs`。

### 8.3 出口门

- 核心能力维度扰动测试覆盖率达到 ≥ 80%；
- 每项测试均有负面对照证据；
- 45 套件全绿。

## 9. D20 · FIBA 交替拥有与罚球情景覆盖

### 9.1 背景与现状

上周期（D11.1）已建立 6 个 FIBA 程序级情景用例。FIBA 规则特有的交替拥有箭头（争球程序）以及罚球违例进出情景尚未形成独立验证用例。

### 9.2 工作项

- 在 `crates/engine/tests/fiba_scenarios.rs` 中新增交替拥有情景测试；
- 覆盖争球触发、球权箭头翻转、节初发球使用箭头的全流程；
- 覆盖罚球进出与加罚违例程序差异。

### 9.3 出口门

- 交替拥有与罚球程序情景测试通过；
- NBA 与 FIBA 在争球场景下的分歧可证明（NBA 跳球 vs FIBA 箭头）；
- 保持 0 账本违规。

## 10. D21 · 周期出口与归档

### 10.1 工作项

- 在 `docs/dev/status.md` 记录本周期收敛成果、关闭项与转交项；
- 核对 `docs/dev/README.md` §7 归档条件；
- 归档本文件至 `docs/dev/cycles/YYYYMMDD_systematization/`。

## 11. 暂不纳入本周期

- UI 视觉与路线动画；
- 经营层成长与交易系统；
- NCAA 规则闭环；
- WASM 存废判定。
