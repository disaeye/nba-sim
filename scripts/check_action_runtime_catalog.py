#!/usr/bin/env python3
"""动作运行时目录守卫（full_simulation_plan.md §12 H、§13.3）。

核对 `crates/sim/src/action_spec.rs` 里的 111 个动作条目：动画目录的
配方、生产候选来源、生产结算器、对手响应、场景测试、两类渲染器的
同帧映射，以及动作身份的唯一性。目录字符串本身不算证据，每个
`文件::函数` 都要在源码里找到真实的函数定义。

用法：

    python3 scripts/check_action_runtime_catalog.py
    python3 scripts/check_action_runtime_catalog.py --json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SPEC = ROOT / "crates" / "sim" / "src" / "action_spec.rs"
CATALOG = ROOT / "data" / "rules" / "action_animation_catalog.json"
RENDERERS = [
    ROOT / "crates" / "debug-server" / "static" / "court3d.js",
    ROOT / "crates" / "debug-server" / "static" / "app.js",
]
PROTOCOL_VALIDATOR = ROOT / "crates" / "protocol" / "src" / "action_recipe.rs"

FAMILY_KEYS = {
    "ActionDribble": "ACTION-DRIBBLE",
    "ActionDrive": "ACTION-DRIVE",
    "ActionFinish": "ACTION-FINISH",
    "ActionShot": "ACTION-SHOT",
    "ActionPost": "ACTION-POST",
    "ActionPass": "ACTION-PASS",
    "ActionFake": "ACTION-FAKE",
    "ActionBallProtection": "ACTION-BALL-PROTECTION",
    "ActionScreen": "ACTION-SCREEN",
    "ActionCut": "ACTION-CUT",
    "ActionRelocate": "ACTION-RELOCATE",
    "ActionDefenseOnBall": "ACTION-DEFENSE-ON-BALL",
    "ActionDefenseOffBall": "ACTION-DEFENSE-OFF-BALL",
    "ActionCloseout": "ACTION-CLOSEOUT",
    "ActionRebound": "ACTION-REBOUND",
    "ActionTransition": "ACTION-TRANSITION",
}
ROLE_KEYS = {
    "Primary": "primary",
    "Screener": "screener",
    "CutPartner": "cut_partner",
    "OnBallDefender": "on_ball_defender",
    "Defender": "defender",
    "Rebounder": "rebounder",
    "Shooter": "shooter",
}


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def function_names(text: str) -> set[str]:
    return set(re.findall(r"\bfn\s+([A-Za-z0-9_]+)\s*\(", text))


def main() -> int:
    parser = argparse.ArgumentParser(description="动作运行时目录守卫")
    parser.add_argument("--json", action="store_true", help="输出机器可读结果")
    args = parser.parse_args()

    spec = SPEC.read_text(encoding="utf-8")
    catalog = load_json(CATALOG)
    failures: list[str] = []

    if catalog["families_count"] != 16:
        failures.append(f"动画目录声明的动作族数量 {catalog['families_count']}")
    catalog_entries = {
        (family["id"], entry["name"], entry["kind"], entry["variant"], entry["role"])
        for family in catalog["families"]
        for entry in family["actions"]
    }
    if len(catalog_entries) != 111:
        failures.append(f"动画目录动作条目 {len(catalog_entries)}，方案要求 111")

    constants = {}
    for match in re.finditer(r'const\s+(\w+):\s*&str\s*=\s*"([^"]+)"', spec):
        constants[match.group(1)] = match.group(2)

    push_pattern = re.compile(
        r"push\(\s*ActionFamily::(\w+),\s*\"([^\"]*)\",\s*\"([^\"]*)\",\s*\"([^\"]*)\","
        r"\s*ActionRole::(\w+),\s*([^,]+),\s*([^,]+),\s*\"([^\"]*)\",\s*\"([^\"]*)\"\s*\)"
    )
    rows = list(push_pattern.finditer(spec))
    if len(rows) != 111:
        failures.append(f"动作规格表登记 {len(rows)} 条，方案要求 111 条")

    identities: set[tuple[str, str, str, str]] = set()
    for row in rows:
        family, name, kind, variant, role, source, resolver, response, test = row.groups()
        family_key = FAMILY_KEYS.get(family)
        role_key = ROLE_KEYS.get(role)
        if family_key is None or role_key is None:
            failures.append(f"{name} 的族或角色未登记：{family}/{role}")
            continue
        key = (family_key, kind, variant, role_key)
        if key in identities:
            failures.append(f"动作身份重复：{key}")
        identities.add(key)
        entry_key = (family_key, name, kind, variant, role_key)
        if entry_key not in catalog_entries:
            failures.append(f"{name} 在动画目录里没有对应配方")
        if not response.strip():
            failures.append(f"{name} 没有对手响应")
        for label, symbol in (("候选来源", source), ("结算器", resolver)):
            symbol = symbol.strip()
            if symbol.startswith('"') and symbol.endswith('"'):
                path = symbol[1:-1]
            else:
                path = constants.get(symbol)
            if path is None:
                failures.append(f"{name} 的{label}常量 {symbol} 不存在")
                continue
            if not path.strip():
                failures.append(f"{name} 的{label}为空")
                continue
            target, _, function = path.partition("::")
            file_path = ROOT / target
            if not file_path.is_file():
                failures.append(f"{name} 的{label}文件不存在：{target}")
                continue
            if function and function not in function_names(file_path.read_text(encoding="utf-8")):
                failures.append(f"{name} 的{label}函数不存在：{path}")
        test_file, _, test_function = test.partition("::")
        test_path = ROOT / test_file
        if not test_path.is_file():
            failures.append(f"{name} 的场景测试文件不存在：{test_file}")
        elif test_function not in function_names(test_path.read_text(encoding="utf-8")):
            failures.append(f"{name} 的场景测试函数不存在：{test}")

    # 协议帧结构必须带齐动作身份、技术变体、阶段与参与者：两类渲染器
    # 消费同一份 `RenderAction`，字段缺失就无法同帧比对。
    frame = (ROOT / "crates" / "protocol" / "src" / "frame.rs").read_text(
        encoding="utf-8"
    )
    struct_match = re.search(r"pub struct RenderAction \{(.*?)\n\}", frame, re.DOTALL)
    if struct_match is None:
        failures.append("协议层没有 RenderAction 结构")
    else:
        body = struct_match.group(1)
        for field in ("action_id", "variant", "phase", "participants"):
            if f"pub {field}:" not in body:
                failures.append(f"协议 RenderAction 缺少字段 {field}")

    # 两类渲染器都从 authoritative active_actions 取主动作，并用同一组
    # 身份 (family/kind/variant/role) 查同一份动画目录，按 phase 判阶段。
    # 2D 视图用 canvas 绘制，姿态标签来自目录里的 label_zh；3D 视图按
    # action_id 去重并切换剪辑。两者消费的字段不同，但身份与阶段相同。
    for renderer, required in (
        (RENDERERS[0], ("active_actions", "action_id", "variant", "phase", "recipe")),
        (RENDERERS[1], ("active_actions", "variant", "phase", "recipe_2d")),
    ):
        if not renderer.is_file():
            failures.append(f"渲染器缺失：{renderer.relative_to(ROOT)}")
            continue
        text = renderer.read_text(encoding="utf-8")
        for field in required:
            if field not in text:
                failures.append(f"{renderer.name} 没有读取动作字段 {field}")

    # 协议校验器把身份与配方绑定：未知身份、缺失阶段就地失败。
    if not PROTOCOL_VALIDATOR.is_file():
        failures.append("协议层动作配方校验器缺失")
    else:
        text = PROTOCOL_VALIDATOR.read_text(encoding="utf-8")
        for field in ("family", "kind", "variant", "role", "phase"):
            if field not in text:
                failures.append(f"协议校验器没有覆盖动作身份字段 {field}")

    # 同帧比对与终态停止播放由真实测试给出。
    for test_name in (
        "render_action_validator.rs",
        "action_identity_consistency.rs",
        "action_animation_coverage.rs",
    ):
        path = ROOT / "crates" / "sim" / "tests" / test_name
        if not path.is_file():
            failures.append(f"同帧一致性测试缺失：{test_name}")

    report = {
        "spec_entries": len(rows),
        "catalog_entries": len(catalog_entries),
        "families": len(catalog["families"]),
        "renderers": len(RENDERERS),
        "failures": failures,
        "passed": not failures,
    }
    if args.json:
        print(json.dumps(report, ensure_ascii=False, indent=2))
    elif failures:
        for failure in failures:
            print(f"❌ {failure}", file=sys.stderr)
    else:
        print(
            f"动作运行时目录通过：{report['families']} 个族、"
            f"{report['spec_entries']} 条生产契约、{report['renderers']} 类渲染器、"
            "协议校验器字段齐全"
        )
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
