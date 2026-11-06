# NBA-Sim · 观测视图

> 定位：浏览器端的比赛呈现层。输入只有协议帧，输出是可读的球场画面；本层不写回比赛状态，也不参与裁决。
> 上游：`docs/protocol.md`（帧与流字段）、`docs/architecture.md` §1.2（应用层边界）。

## 1. 两套渲染器

页面在同一块舞台区域里保留两套互斥的渲染器，共用 `crates/protocol` 的 `RenderFrame`：

| 视图 | 实现 | 用途 |
| --- | --- | --- |
| 2.5D 球场 | `crates/debug-server/static/court3d.js` | 正交斜视投影的小人模型、篮架、地板材质与固定俯角镜头；默认视图 |
| 二维战术板 | `crates/debug-server/static/app.js` 的 `drawCourt` | 俯视战术图层：势能场、干扰罩、卡位弧、投篮抛物线、篮网与篮板灯 |

切换方式：舞台右上角的 `2.5D 球场` 与 `战术板` 按钮、键盘 `V`、或 URL 参数 `?court=2d`。二维视图的绘制逻辑保持原状，2.5D 视图不绘制战术图层。

镜头使用 `OrthographicCamera`，俯角固定 34°，禁止旋转，同一高度的球员在画面里尺寸一致，球场不产生透视收缩。三个预设用 `镜头` 按钮或键盘 `C` 循环：全场（默认，宽度覆盖 104 英尺，四条边线与篮架全部入画）、跟拍（跟随球，宽度覆盖 62 英尺）、俯视（俯角 89.5°）。全场与俯视支持拖动平移与滚轮缩放，平移范围限制在球场内；跟拍由程序驱动镜头。

小人模型保留真实朝向：模型偏航等于 `atan2(facing_x, facing_y)`，背对镜头就展示背面，背身单打、背身防守与远离镜头的跑动都按实际姿态呈现。模型加载时按骨骼与顶点量出原始高度并缩放到 7 英尺，同时量出脚骨指向得到的资产前向偏差，渲染时加到偏航上；两只脚骨的中点高度与网格最低点都用于校验落地。材质用三段式卡通渐层（`MeshToonMaterial` 加四阶梯度贴图），体积光被拆成平坦色块；球衣（`Shirt`）与短裤（`Pants`）按队伍重新着色，皮肤、头发、袜子保留原色。转向按帧间隔平滑（14/s 的逼近速率），180° 转身经过侧面而不是瞬切。

## 2. 2.5D 视图的数据来源

| 画面元素 | 字段 | 说明 |
| --- | --- | --- |
| 球员位置 | `players[].x`、`players[].y` | 归一化坐标，乘球场尺寸后落在 94×50 英尺世界坐标 |
| 球员朝向 | `players[].facing_x`、`facing_y` | 模型偏航取 `atan2(facing_x, facing_y)`；缺失时退回位置差分方向 |
| 篮球 | `ball.x`、`ball.y`、`ball.z`、`ball.status`、`ball.holderId` | `z` 是英尺高度；`held` 时挂在持球人右手骨骼上 |
| 一次性动作 | `eventType`、`events[]` | `SHOT`、`PASS`、`REBOUND` 触发对应动画；比分上升时同队球员鼓掌 |
| 队伍与号码 | `players[].team`、`players[].jersey` | 用于球衣着色与头顶号码牌 |

当前协议里的 `players[].action` 表示站位槽位（`handler`、`wing`、`corner`、`top`、`post`、`dunker`）；`action_phase` 只在部分动作窗口出现，尚未与动作身份、技术变体和生命周期形成完整帧数据。现有 2.5D 动画范围仅覆盖基础移动、投篮/罚球、传球、篮板与得分庆祝，不能据此判定动作目录已完成动画覆盖。

动画补齐后的协议要求与验收范围见 `docs/dev/current/full_simulation_plan.md` H 和 `docs/protocol.md` 的动作动画数据验收节。完成后，本节需列出正式动作帧字段与动作到骨骼片段、二维表现的覆盖关系；每个动作目录项须对应自动化测试。

- 持续状态按帧间速度选择：低于 1.2 英尺每秒为站立，低于 7.5 为走，其余为跑；步频按速度缩放，减少脚底打滑。
- 一次性动作由事件触发，只对持球人或离球最近的球员播放；暂停与拖动进度条期间不触发。动画映射：站立 `Man_Idle`、走 `Man_Walk`、跑 `Man_Run`、出手与传球 `Man_Punch`、抢篮板 `Man_Jump`、庆祝 `Man_Clapping`。

帧率是 `TICK_SECONDS = 0.05`（20 Hz）。渲染层用 `state.idx + state.accumulator` 做位置、朝向与球高的线性插值，再按真实帧间隔推进骨骼动画。

## 3. 资产与依赖

- `crates/debug-server/static/assets/players.glb`：Quaternius 的 Animated Men Characters（休闲男装），31 根蒙皮骨骼、11 段动画（`Man_Idle`、`Man_Walk`、`Man_Run`、`Man_Jump`、`Man_Punch`、`Man_Clapping` 等），六个纯色材质，CC0 1.0。原始 FBX 用 three.js `FBXLoader` 读取、按材质压缩重组几何后用 `GLTFExporter` 导出为 glb；署名、来源与转换流程写在同目录的 `ATTRIBUTION.md`。
- `crates/debug-server/static/vendor/three/`：three.js 0.186.1（MIT）以及 `GLTFLoader`、`SkeletonUtils`、`OrbitControls` 三个官方附加模块，按上游目录结构存放，由 `index.html` 的 importmap 解析 `three` 与 `three/addons/`。
- 加载顺序：页面先完成推演，再动态 `import("./court3d.js")`；模型与渲染库不影响首屏的表格与分析面板。

## 4. 本地服务与验证

调试服务器（`crates/debug-server/src/static_files.rs`）直接读取 `crates/debug-server/static/` 下的文件，改前端不需要重新编译。Vercel 部署使用同一目录，因此本地与线上的资源路径一致。

```bash
cargo run -p nba-debug-server           # 默认监听 127.0.0.1:31100
node scripts/test_court3d.mjs           # 2.5D 视图：场景图、位置、朝向、镜头、像素采样、控制台异常
python3 scripts/test_court_visuals.py   # 二维战术板：自带 `?court=2d`
python3 scripts/test_ui.py              # 页面启动与 DOM 摘要
```

`scripts/test_court3d.mjs` 通过 DevTools 协议驱动真实浏览器（默认 `google-chrome`，可用 `CHROME_BIN` 指定），检查十名球员可见、蒙皮网格已实例化、球员位置与插值后的帧坐标误差小于 0.01 英尺、小人偏航与 `facing` 误差小于 0.001 弧度、资产前向对齐误差小于 0.02 弧度、背向镜头的球员全部显示背面、模型落地最低点与头高在合理区间、卡通渐层材质已应用、全场四角在镜头内、正交投影下同一高度的球员尺寸一致、球在持球人手上、像素采样非单色、切换视图与镜头循环可用、平移被限制在球场内、控制台没有异常。

## 5. 边界

- 引擎不下发球员腾空高度，起跳只由动画片段表达；球的高度直接使用 `ball.z`。
- 球员身高与球衣颜色是呈现参数（模型缩放到 7 英尺），不参与模拟，也不影响判定。
- 2.5D 视图不消费战术图层所需的中间量（干扰罩、卡位弧、势能场），这些只在二维战术板里绘制。
- 固定正交镜头下背对镜头的球员会遮挡自己的胸前面，这是该视角的真实结果，不是渲染缺失。
- 正交投影下球员尺寸不随位置变化，代价是球场在画面上不产生远小近大的深度提示，深度由阴影与遮挡表达。
