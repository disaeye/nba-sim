# 真实比赛过程研究证据

> 研究输入与拟合数据分开登记；本页只报告可观测指标、样本量、不确定范围与证据不足。研究结果不进入自动构建、CI 或发布门禁。

## 输入与样本

| 项目 | 内容 |
| --- | --- |
| 参考数据 | `nbastats_2024.csv`，2024-25 NBA 常规赛逐事件归档，1,230 场 |
| 来源 | `shufinskiy/nba_data datasets/nbastats_2024.tar.xz`，底层为 stats.nba.com PlayByPlay |
| 归档 SHA-256 | `8f370da00ad0a6d4157921bd161d56ea32647a165d906516f2b36384a6a4989a` |
| 参考回合口径 | 同一节内比赛钟差；不跨节配对 |
| 模拟样本 | `cargo run -q -p nba-sim-cli -- sim-core --seed 1 --games 4 --json`，4 场 |
| 复用关系 | 研究脚本只读归档与模拟输出，不把参考数据送入参数拟合 |
| 不确定范围 | 按比赛聚类的 percentile bootstrap，B=2000，脚本种子 `20261102` |

## 指标与结果

时间单位为秒。区间为 95% 区间。`本队出手占比` 表示该来源事件之后先出现本队出手的样本比例。

| 指标 | 参考样本 | 参考均值 [区间] | 参考本队出手占比 | 模拟样本 | 模拟均值 [区间] | 模拟本队出手占比 | 差值 |
| --- | ---: | --- | ---: | ---: | --- | ---: | ---: |
| `PROC-DREB-FIRST-SHOT` 防守篮板后首次出手 | 60,917 | 9.707 [9.651, 9.765] | 0.788 | 290 | 10.880 [10.172, 11.593] | 0.759 | +1.172 |
| `PROC-OREB-FIRST-SHOT` 前场篮板后首次出手 | 24,297 | 4.103 [4.040, 4.167] | 0.829 | 69 | 4.583 [4.178, 5.344] | 0.945 | +0.480 |
| `PROC-STEAL-FIRST-SHOT` 抢断后首次出手 | 15,435 | 7.220 [7.141, 7.300] | 0.787 | 98 | 9.638 [8.093, 10.629] | 0.797 | +2.418 |

三项都只描述可观测时间与出手先后，不据此宣称过程已经与参考一致。模拟样本只有 4 场，区间宽度反映这个样本量。

## 证据不足

| 指标 | 状态 | 原因 |
| --- | --- | --- |
| `PROC-OREB-SHOT-ZONE` 前场篮板后出手区域构成 | `InsufficientEvidence` | 参考 `shotdetail` 为 2023-24，逐事件归档为 2024-25；`GAME_ID` 不相交，不能按事件联结。 |
| `PROC-PASS-COUNT` 逐回合传球数与结果 | `InsufficientEvidence` | 逐事件归档不含每次传球与接球人。 |
| `PROC-SCREEN-USE` 掩护发生、使用与后续 | `InsufficientEvidence` | 没有追踪或人工标注，掩护类型与使用无法可靠辨认。 |
| `PROC-HELP-CLOSE` 协防、补位、扑防顺序 | `InsufficientEvidence` | 归档只有结果事实，没有十人位置与朝向。 |
| `PROC-CORNER-WINDOW` 底角窗口持续时间 | `InsufficientEvidence` | 缺少连续位置与防守到达时间。 |

## 复现

```bash
cargo run -q -p nba-sim-cli -- sim-core --seed 1 --games 4 --json > .work/simcore-sample4.json
python3 scripts/research/process_study.py \
  --pbp .work/pbp/nbastats_2024.csv \
  --shotdetail .work/r7/shotdetail_2023.csv \
  --sim .work/simcore-sample4.json \
  --out docs/dev/evidence/process_research.json
```

脚本输出 `docs/dev/evidence/process_research.json`。该文件逐项记录状态、样本量、区间、局限与证据不足，不计算总通过率。