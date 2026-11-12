#!/usr/bin/env python3
"""生产事实链守卫（full_simulation_plan.md §8.1、§12 C）。

每个 `IncidentKind` 都必须有 `detect_*` 判定函数、主循环或其生产消费者
的调用点，以及裁决层的处理分支。测试注入的判定入口不算生产证据：公开
入口里出现 `#[cfg(test)]`、测试专用命名或 `test_` 前缀直接失败。

用法：

    python3 scripts/check_production_incidents.py
    python3 scripts/check_production_incidents.py --json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
INCIDENT = ROOT / "crates" / "sim" / "src" / "incident.rs"
DETECT = ROOT / "crates" / "sim" / "src" / "detect.rs"
ADJUDICATE = ROOT / "crates" / "sim" / "src" / "adjudicate_incidents.rs"
SOURCE_DIR = ROOT / "crates" / "sim" / "src"


def rust_sources(exclude: set[Path] | None = None) -> dict[Path, str]:
    exclude = exclude or set()
    return {
        path: path.read_text(encoding="utf-8")
        for path in sorted(SOURCE_DIR.rglob("*.rs"))
        if path not in exclude
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="生产事实链守卫")
    parser.add_argument("--json", action="store_true", help="输出机器可读结果")
    args = parser.parse_args()

    incident = INCIDENT.read_text(encoding="utf-8")
    detect = DETECT.read_text(encoding="utf-8")
    adjudicate = ADJUDICATE.read_text(encoding="utf-8")
    sources = rust_sources(exclude={INCIDENT, DETECT, ADJUDICATE})

    enum_match = re.search(
        r"pub enum IncidentKind \{(.*?)\n\}", incident, re.DOTALL
    )
    if enum_match is None:
        raise SystemExit("无法定位 IncidentKind 枚举")
    variants = re.findall(r"^\s{4}([A-Z][A-Za-z0-9]*),", enum_match.group(1), re.MULTILINE)
    str_names = dict(re.findall(r"Self::([A-Za-z0-9]+) => \"([a-z_]+)\"", incident))
    detectors = set(re.findall(r"pub fn (detect_[a-z_]+)\(", detect))
    failures: list[str] = []

    if len(variants) != len(set(variants)):
        failures.append("IncidentKind 有重复变体")
    if set(str_names) != set(variants):
        failures.append("IncidentKind::as_str 没有覆盖全部变体")
    for variant in variants:
        if f"IncidentKind::{variant}" not in detect:
            failures.append(f"{variant} 在 detect.rs 里没有原始事实构造")
        if variant not in adjudicate:
            failures.append(f"{variant} 在裁决层没有消费者")

    # 生产调用点：detect_* 从生产模块被调用，或由 detect_sampled 汇总后
    # 在 World::step 里进入事实队列。
    calls: dict[str, list[str]] = {}
    for path, text in sources.items():
        for name in re.findall(r"(?:crate::)?detect::(detect_[a-z_]+)\(", text):
            calls.setdefault(name, []).append(str(path.relative_to(ROOT)))
    sampled = re.findall(r"incidents\.extend\((detect_[a-z_]+)\(world\)\)", detect)
    internal = re.findall(r"\b(detect_[a-z_]+)\(world(?:,\s*[^)]*)?\)", detect)
    for name in internal:
        calls.setdefault(name, []).append("detect.rs::内部调用")
    wrapper = {
        "detect_flagrant": "detect_flagrant_with",
        "detect_clear_path": "detect_clear_path_with",
    }
    for outer, inner in wrapper.items():
        if outer in calls:
            calls.setdefault(inner, []).extend(calls[outer])
    unused = sorted(detectors - set(calls))
    if unused:
        failures.append(f"判定函数没有生产调用点：{unused}")
    for name in re.findall(r"pub fn (detect_[a-z_]+)\(", detect):
        if "#[cfg(test)]" in detect[max(0, detect.find(f"pub fn {name}") - 200) : detect.find(f"pub fn {name}")]:
            failures.append(f"{name} 带测试专用条件编译")

    world_step = (SOURCE_DIR / "world.rs").read_text(encoding="utf-8")
    if "detect_sampled(self)" not in world_step:
        failures.append("World::step 没有调用 detect_sampled")
    if "adjudicate_incidents" not in world_step and "RawIncident" not in world_step:
        failures.append("World::step 没有把原始事实交给裁决层")

    for path, text in sources.items():
        for name in re.findall(r"pub fn ((?:inject|test_only|test_)[a-z_]*)", text):
            failures.append(f"{path.relative_to(ROOT)} 暴露测试注入入口 {name}")

    report = {
        "incident_kinds": len(variants),
        "detectors": len(detectors),
        "production_calls": sum(len(names) for names in calls.values()),
        "sampled_detectors": len(sampled),
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
            f"生产事实链通过：{report['incident_kinds']} 种事实、"
            f"{report['detectors']} 个判定函数、{report['production_calls']} 处生产调用、"
            f"{report['sampled_detectors']} 个主循环汇总判定"
        )
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
