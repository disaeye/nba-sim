# NBA-Sim 文档地图

> 文档体系的单一导航面。修订规则：**改设计 → 改对应契约文档；改状态 → 只改 status.md。**

## 结构

```
docs/
├── README.md           ← 本文档：地图与修订规则
├── charter.md          ← 北极星与红线（要做什么、什么算好；不可修订条款）
├── architecture.md     ← 系统架构（分层、数据流、球权状态机、阶段管线、决策、语义/裁决、多联赛）
├── quality.md          ← 质量与评判体系（L1/L2/L3 检测网、参考分布、sanity net、确定性、工具链、性能预算）
├── design.md           ← 执行路线（里程碑 M1–M10、校准协议、验收标准指针）
├── attributes.md       ← 球员属性契约（分类学、值语义、锚点、映射边界、迁移路线）
├── tactics.md          ← 阵容与战术契约（四层架构、战术档案、适配层、迁移路线）
└── status.md           ← 唯一漂移面（现状审计、差距矩阵、完成度、变更记录汇总）
```

## 各文档职责

| 文档 | 回答的问题 | 生命周期 | 修订触发 |
|------|-----------|---------|---------|
| `charter.md` | 要做出什么、什么算好、什么不可触碰 | 项目全程 | 项目所有者显式修订（§9） |
| `architecture.md` | 系统如何组织、模块如何交互、状态如何流转 | 架构稳定期 | 架构决策变更（需证据） |
| `quality.md` | 如何检测偏差、如何评判真实度、如何闭环校准 | 检测体系稳定期 | 评判准则/工具链变更 |
| `design.md` | 按什么顺序做、每步验收什么 | 执行期（随里程碑推进） | 里程碑完成/调整 |
| `attributes.md` | 球员能力是什么、值语义是什么、如何标定 | 契约稳定期 | schema 修订（升版本） |
| `tactics.md` | 阵容/战术/适配/教练如何组织 | 契约稳定期 | schema 修订（升版本） |
| `status.md` | 现在做到哪了、差距是什么、下一步是什么 | **持续漂移** | 每次审计/里程碑推进 |

## 修订规则（固化纪律）

1. **设计文档**（charter/architecture/quality/attributes/tactics）：**零"现状/截至日期/完成度/差距矩阵"**——只写"应该是什么"，不写"现在是什么"。现状一律去 status.md；
2. **status.md**：唯一的漂移面。所有"现状审计""差距矩阵""完成度百分比""当前缺陷清单"集中此处；设计文档引用它（`见 status.md §X`）但不内嵌；
3. **design.md**：执行路线（里程碑 + 协议），允许引用 status.md 的完成度，但不复制其内容；
4. **变更记录**：各文档的变更记录汇总到 status.md §6，设计文档自身不再保留变更记录表（避免状态污染设计）。
5. **交叉引用机械守卫**：`python3 scripts/check_doc_refs.py` 校验全部跨文档 `X.md §N` 引用可解析（CI 步骤 `docs`）；语义指向抽查随评审进行——可解析不等于指对了章节。

## 阅读顺序

- **新成员**：README → charter → architecture → quality → attributes/tactics → design → status
- **引擎贡献者**：architecture → quality → design → status
- **数据/档案作者**：attributes → tactics → architecture §7（领域层）
- **QA/工具作者**：quality → design §3.4（工具链验收）→ status §5（工具链现状）
