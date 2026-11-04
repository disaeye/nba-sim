# NBA 与 FIBA 官方规则来源核验记录

本文登记 `data/rules/league_*.json`、`rule_catalog.json` 与 `action_catalog.json` 的来源范围。官方页面和规则 PDF 是规则条款的主要依据；项目历史文档、当前代码和二手说明不作为规则依据。

## NBA

- 采用版本：2025-26。
- 官方 PDF：[Official 2025-26 NBA Playing Rules](https://cdn.nba.com/manage/2026/01/Official-2025-26-NBA-Playing-Rules.pdf)
- 官方在线条文：[Rule 2](https://official.nba.com/rule-no-2-duties-of-the-officials/)、[Rule 3](https://official.nba.com/rule-no-3-players-substitutes-and-coaches/)、[Rule 4](https://official.nba.com/rule-no-4-definitions/)、[Rule 5](https://official.nba.com/rule-no-5-scoring-and-timing/)、[Rule 6](https://official.nba.com/rule-no-6-putting-ball-in-play-live-dead-ball/)、[Rule 7](https://official.nba.com/rule-no-7-24-second-clock/)、[Rule 8](https://official.nba.com/rule-no-8-out-of-bounds-and-throw-in/)、[Rule 9](https://official.nba.com/rule-no-9-free-throws-and-penalties/)、[Rule 10](https://official.nba.com/rule-no-10-violations-and-penalties/)、[Rule 11](https://official.nba.com/rule-no-11-basket-interference-goaltending/)、[Rule 12](https://official.nba.com/rule-no-12-fouls-and-penalties/)、[Rule 13](https://official.nba.com/rule-no-13-instant-replay/)、[Rule 14](https://official.nba.com/rule-no-14-coachs-challenge/)。
- 核验边界：官方 PDF 页面已由官方索引定位，直接全文抓取请求中止；在线条文页面已完整读取。PDF 未逐页人工核对的条款均保留为待核验，不能从摘要、搜索片段或旧版规则作完成判定。
- 已从官方在线条文核对：比赛时长、节末投篮与罚球处理、开场及节次开始、球队控制的定义、24 秒计时启动与重置、发球五秒、走步与运球、三秒、八秒、回场、干扰球、罚球站位、常规与加时球队犯规罚则、末段球队犯规例外、暂停配额与强制暂停、换人、录像复核及教练挑战。
- NBA 2025-26 规则中，常规节四次普通球队犯规后进入罚则；加时前三次普通球队犯规适用未进入罚则的发球处理，之后进入罚则。末段另有球队犯规容额。`data/rules/league_nba.json` 现有 bonus 标量不能表达加时独立阈值、容额和具体犯规类型；这些字段当前均标为待建模，不能据现有数值宣称符合规则。
- 比赛时钟在第一、二、三节最后一分钟的命中后停表，在第四节与加时最后两分钟的命中后停表；其他时间的命中后继续计时。暂停有每场、第四节、第四节特定后段及加时多组限制，另有每节两次强制暂停。现有标量字段不足以单独表达强制暂停触发及其时长来源。

## FIBA

- 采用版本：Official Basketball Rules 2026 v1.1，自 2026-10-01 生效。
- 官方规则：[Official Basketball Rules 2026 v1.1](https://assets.fiba.basketball/image/upload/documents-corporate-fiba-official-rules-2026-v1-1.pdf)
- 官方解释：[Official Basketball Rules 2026 Interpretations v1.0](https://assets.fiba.basketball/image/upload/documents-corporate-fiba-official-rules-2026-interpretations-v1-0.pdf)
- 官方改动说明：[FIBA Official Basketball Rules Changes v1.1](https://assets.fiba.basketball/image/upload/documents-corporate-fiba-official-rules-2026-rule-changes-v1-1-en.pdf)
- 官方变更摘要：[FIBA Official Basketball Rules 2026 to take effect October 1](https://about.fiba.basketball/en/news/fiba-official-basketball-rules-2026-to-take-effect-october-1)
- 规则 PDF 全文已抓取，记录为 108 页。2026 版重要变化包括技术犯规分为两类、disruptive foul 与 flagrant foul 取代旧 unsportsmanlike foul 类型、末段发球相关判罚与 IRS 审查条件更新。
- 已核对的核心程序：四节各十分钟；加时五分钟并重复至比分分出胜负；节间两分钟、中场十五分钟；开场跳球，其余节次与加时按交替拥有箭头从中线延长线发球；比赛时钟按合法跳球触球、发球合法触球、末罚不中合法触球启动，按裁判哨声、节末及指定命中情况停止；进攻时钟二十四秒，篮圈触球后同队重获控制十四秒、对手重获控制二十四秒；发球五秒；后场八秒；进攻限制区三秒；贴身防守五秒；每节球队第四次犯规后进入球队犯规罚则；加时球队犯规累计进第四节球队犯规数；个人犯规五次离场；暂停每次一分钟，上半场两次、下半场三次（第四节最后两分钟最多两次）、每个加时一次。

## 来源与程序覆盖索引

完整程序和场景 ID 见 `data/rules/rule_catalog.json`。动作家族、代码候选路径、领域类型及生产证明场景见 `data/rules/action_catalog.json`。当前状态枚举按 `planned`、`partial`、`implemented`、`verified` 区分；目录结构测试不替代行为测试。

## 已确认的档案差异

1. 加时球队犯规的阈值与累计已经分开表示：`FoulPolicy::bonus_fouls_overtime` 给 NBA 加时的独立门槛（4），`FoulPolicy::overtime_fouls_carry_from_fourth` 给 FIBA Article 42.1.3 的「加时犯规计入第四节累计」（NBA 为否，FIBA 为是）。
2. NBA 第四节球队犯规限额与末段限额、球队犯规例外都已在 `TimeoutPolicy` 与 `FoulPolicy` 里分别表达并有消费者；强制暂停的触发条件与时长仍未建模——那是规则的第四节两个强制暂停点，引擎当前不自动叫。
3. 恶意犯规已经分两级并带被逐：二级恶意犯规把犯规者逐出球场并留下被逐事实（NBA Rule 12B §IV.2）。FIBA 的 disruptive foul（2026 规则的干扰犯规）还没有单独的类型，它与 NBA 的技术犯规在罚则上不同。
4. 贴身防守五秒程序由 `LeagueProfile::closely_guarded_seconds` 与 `closely_guarded_distance_ft` 表达（NBA 六英尺、FIBA 一米），引擎在收球后被贴身盯防时按这两个字段判定违例。
5. 发球地点已经细分：出界点、罚球线延长线上的边线点（`InboundLocation::Spot`）与本方底线（`OwnBaseline`）三类，犯规、暂停、命中与三类违例各自给出正确的地点。中断恢复上下文（`ResumeContext`）已经建模并由暂停与发球程序共用。
6. 时间类规则的状态：`backcourt_seconds`（后场八秒）、回场判定与八秒的「踢球」例外（NBA Rule 10 §VIII.1）都有消费者。`inbound_seconds` 已经消费：发球五秒从发球程序开始那一刻算起，超时是发球违例，球权交给对方，对方在原来的发球点发球（`World::inbound_started_at` 与 `advance_phase` 的发球分支）。`three_second_lane_seconds` 与守方三秒都已启用：清空禁区的落点修正落在「球员自己的动作落点」之外——正在跑位与切入的球员不被拉回站位，低位与顺下两个本身就要求人在禁区里的角色另有豁免（NBA Rule 10 §VI.2）。实测每场 0.8 次进攻三秒违例与 1.6 次守方三秒技术犯规，回归见 `crates/sim/tests/lane_clock.rs`。
7. NBA 在线条文完整已读取；官方 PDF 仍须逐项比对文字与例外。未核验部分继续保持待核验状态。
