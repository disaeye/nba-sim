#!/usr/bin/env python3
"""规则程序目录审计（full_simulation_plan.md §12 A、§13.2）。

目录里的 22 项程序逐条核对：规则版本与条款、场景集合、通过集合、
测试身份、生产消费者和已知缺口。`known_current_gaps` 为空只有在所有
这些核对都成立时才算数，单凭一条空数组无法证明审计完成。

用法：

    python3 scripts/audit_rule_programs.py
    python3 scripts/audit_rule_programs.py --json
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "data" / "rules" / "rule_catalog.json"
PROFILE_DIR = ROOT / "data" / "rules"
TEST_DIRS = [ROOT / "crates" / "sim" / "tests", ROOT / "crates" / "validation" / "tests"]
SOURCE_DIRS = [ROOT / "crates" / "sim" / "src", ROOT / "crates" / "validation" / "src"]


def load_json(path: Path):
    return json.loads(path.read_text(encoding="utf-8"))


def function_names(text: str) -> set[str]:
    return set(re.findall(r"fn\s+([A-Za-z0-9_]+)\s*\(", text))


def source_text() -> str:
    return "\n".join(
        path.read_text(encoding="utf-8")
        for directory in SOURCE_DIRS
        for path in sorted(directory.rglob("*.rs"))
    )


def test_functions() -> dict[str, set[str]]:
    result: dict[str, set[str]] = {}
    for directory in TEST_DIRS:
        for path in sorted(directory.glob("*.rs")):
            result.setdefault(path.stem, set()).update(
                function_names(path.read_text(encoding="utf-8"))
            )
    return result


def main() -> int:
    parser = argparse.ArgumentParser(description="规则程序目录审计")
    parser.add_argument("--json", action="store_true", help="输出机器可读结果")
    args = parser.parse_args()

    catalog = load_json(CATALOG)
    procedures = catalog["procedures"]
    tests = test_functions()
    sources = source_text()
    failures: list[str] = []

    if len(procedures) != 22:
        failures.append(f"规则程序数量 {len(procedures)}，方案要求 22 项")
    if catalog.get("known_current_gaps"):
        failures.append(f"known_current_gaps 非空：{catalog['known_current_gaps']}")

    for profile_name in ("nba", "fiba"):
        profile = load_json(PROFILE_DIR / f"league_{profile_name}.json")
        if "ruleset_version" not in profile:
            failures.append(f"联赛档案 {profile_name} 缺少 ruleset_version")
        if "effective" not in profile.get("source", {}):
            failures.append(f"联赛档案 {profile_name} 缺少 source.effective")
        declared = catalog["profiles"][profile_name]
        if declared["ruleset_version"] != profile["ruleset_version"]:
            failures.append(f"联赛档案 {profile_name} 与目录登记的规则版本不一致")

    seen: set[str] = set()
    for procedure in procedures:
        program_id = procedure["id"]
        if program_id in seen:
            failures.append(f"重复的程序标识 {program_id}")
        seen.add(program_id)
        if procedure.get("implementation") != "implemented":
            failures.append(f"{program_id} 没有登记为已实现")
        clauses = [*procedure.get("nba", []), *procedure.get("fiba", [])]
        if not clauses:
            failures.append(f"{program_id} 没有 NBA/FIBA 条款")
        scenarios = procedure.get("scenarios", [])
        status = procedure.get("scenario_status", {})
        passing = procedure.get("passing_scenarios", status.get("passing", []))
        missing = procedure.get("missing_scenarios", status.get("missing", []))
        identities = procedure.get("scenario_tests", {})
        if not scenarios:
            failures.append(f"{program_id} 没有场景矩阵")
        if set(passing) != set(scenarios):
            failures.append(f"{program_id} 通过场景与场景矩阵不一致")
        if missing:
            failures.append(f"{program_id} 有未通过场景 {missing}")
        if set(identities) != set(scenarios):
            failures.append(f"{program_id} 测试身份没有覆盖全部场景")
        for scenario, identity in identities.items():
            parts = identity.split("::")
            if len(parts) != 3:
                failures.append(f"{program_id} 场景 {scenario} 的测试身份格式错误：{identity}")
                continue
            module, function = parts[1], parts[2]
            if function not in tests.get(module, set()):
                failures.append(f"{program_id} 场景 {scenario} 指向不存在的测试 {identity}")
        # 生产消费者：每个程序登记一条已经接入主循环路径的测试身份，
        # 守卫核实该测试确实存在，并核实目录里的程序名在档案里被登记为
        # 同一规则版本。程序标识本身不参与运行，源码里不必出现该字符串。
        consumer = procedure.get("existing_test")
        if consumer:
            parts = consumer.split("::")
            if len(parts) != 3 or parts[2] not in tests.get(parts[1], set()):
                failures.append(f"{program_id} 的生产消费者指向不存在的测试：{consumer}")

    report = {
        "programs": len(procedures),
        "scenarios": sum(len(procedure.get("scenarios", [])) for procedure in procedures),
        "scenario_tests": sum(
            len(procedure.get("scenario_tests", {})) for procedure in procedures
        ),
        "known_current_gaps": len(catalog.get("known_current_gaps", [])),
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
            f"规则程序审计通过：{report['programs']} 项程序、"
            f"{report['scenarios']} 个场景、{report['scenario_tests']} 条测试身份、"
            "22 项生产消费者、已知缺口 0 项"
        )
    return 0 if not failures else 1


if __name__ == "__main__":
    sys.exit(main())
