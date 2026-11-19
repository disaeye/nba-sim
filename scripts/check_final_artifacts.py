#!/usr/bin/env python3
"""检查规则目录与资源工件。"""

from __future__ import annotations

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "docs" / "dev" / "evidence"


def load(path: Path):
    if not path.is_file():
        raise SystemExit(f"缺少最终工件：{path.relative_to(ROOT)}")
    return json.loads(path.read_text(encoding="utf-8"))


def check_performance(problems: list[str]) -> None:
    payload = load(EVIDENCE / "performance.json")
    if not payload["passed"] or payload["failures"]:
        problems.append("performance.json 标记失败")
    if payload["address_space_limit_bytes"] != 2 * 1024 * 1024 * 1024:
        problems.append("performance.json 的地址空间上限不是 2 GiB")
    scenarios = payload["scenarios"]
    if len(scenarios) != 7:
        problems.append(f"performance.json 有 {len(scenarios)} 个场景，要求 7 个")
    for scenario in scenarios:
        sample = scenario["sample"]
        process = scenario["process"]
        for field in ("ticks", "events", "actions", "overtimes", "allocations_per_tick"):
            if field not in sample:
                problems.append(f"场景 {scenario['name']} 缺少 {field}")
        for field in ("peak_rss_bytes", "elapsed_seconds", "exit_code"):
            if field not in process:
                problems.append(f"场景 {scenario['name']} 缺少 {field}")
        if process.get("peak_rss_bytes", 0) > payload["address_space_limit_bytes"]:
            problems.append(f"场景 {scenario['name']} 峰值内存超过上限")


def check_catalogs(problems: list[str]) -> None:
    catalog = load(ROOT / "data" / "rules" / "rule_catalog.json")
    if catalog.get("known_current_gaps"):
        problems.append("rule_catalog.json 的 known_current_gaps 非空")


def main() -> int:
    problems: list[str] = []
    check_performance(problems)
    check_catalogs(problems)
    if problems:
        for problem in problems:
            print(f"❌ {problem}", file=sys.stderr)
        return 1
    print("规则目录与资源工件检查通过")
    return 0


if __name__ == "__main__":
    sys.exit(main())
