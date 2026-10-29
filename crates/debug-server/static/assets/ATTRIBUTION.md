# 三维视图资产署名

## 球员模型

- 文件：`players.glb`
- 名称：Animated Men Characters（男性休闲装角色，内置 11 段骨骼动画）
- 作者：Quaternius（https://quaternius.com）
- 许可：CC0 1.0 Universal（公共领域奉献）
- 许可原文：https://creativecommons.org/publicdomain/zero/1.0/
- 上游页面：https://quaternius.com
- 获取途径：https://github.com/beep2bleep/FreeAssetsByKenneyNLandQuaternius 的 `FreeModels by Quaternius[Patreon]/Characters and Animals/Animated Men Characters - Feb 2019/FBX/Male_Casual.fbx`

原始资产是 FBX。转换流程：three.js `FBXLoader` 读取，按材质把上百个几何分组压缩重组成 6 个蒙皮网格，`GLTFExporter` 导出为 glb。材质只有纯色（Shirt、Skin、Pants、Eyes、Socks、Hair），无外部贴图；球衣与短裤的颜色由页面按队伍重新着色。

动画片段：`Man_Idle`、`Man_Walk`、`Man_Run`、`Man_Jump`、`Man_Punch`、`Man_Clapping` 等 11 段。

## 渲染库

- 名称：three.js 0.186.1
- 许可：MIT
- 上游：https://github.com/mrdoob/three.js

`vendor/three/` 目录保留上游文件原样，仅作为静态依赖分发。
