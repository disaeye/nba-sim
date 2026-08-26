# NBA-Sim Engine (Pure Rust)

工业级高性能 2D 篮球仿真与物理动力学引擎。

## 目录结构

```text
nba-sim/
├── engine/                 # 【后端】纯 Rust 仿真与物理引擎
│   ├── Cargo.toml
│   ├── src/
│   │   ├── main.rs         # CLI 入口
│   │   ├── lib.rs          # 库导出
│   │   ├── movement.rs     # Rapier2D 物理运动与碰撞控制器
│   │   ├── simulation.rs   # 比赛推进与状态机
│   │   ├── tactics.rs      # 战术意图生成器
│   │   ├── protocol.rs     # StreamTick 数据协议
│   │   └── court.rs        # 球场几何规格
│   └── tests/
│       └── physics_invariants.rs  # 原生物理硬约束测试
│
├── web/                    # 【前端】纯静态展示端 (Canvas/WebGL)
│   ├── index.html
│   ├── app.js
│   ├── styles.css
│   └── game.ticks.ndjson
│
├── bin/                    # 【工具链】统一执行脚本
│   └── sim.sh              # 一键仿真并输出到 web/ 供浏览器直接预览
│
├── config/                 # 球场规格与配置
└── docs/                   # 架构与文档
```

## 快速使用

### 1. 运行单场仿真
```bash
./bin/sim.sh 42
```
*在 300 毫秒内生成 7,200 帧（720 秒完整回放）零物理异常的比赛数据流并写入 `web/game.ticks.ndjson`。*

### 2. 运行物理硬约束回归测试
```bash
cd engine
cargo test --release
```
