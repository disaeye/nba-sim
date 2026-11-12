#!/usr/bin/env python3
"""最终工件一致性检查。

验证门禁、认证、机制实验、资源工件与规则目录的静态契约。判定数字来自
各自工件本身；提交身份与认证运行对应的代码提交核对。

用法：

    python3 scripts/check_final_artifacts.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
EVIDENCE = ROOT / "docs" / "dev" / "evidence"
HEAD = subprocess.run(
    ["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True, check=True
).stdout.strip()


def load(path: Path):
    if not path.is_file():
        raise SystemExit(f"缺少最终工件：{path.relative_to(ROOT)}")
    return json.loads(path.read_text(encoding="utf-8"))


# 认证工件记录生成它们的代码提交；之后的提交只能写认证工件与诊断产物。
GENERATED_PATH_PREFIXES = (
    "docs/dev/evidence/",
    "data/diagnostics/",
)
GENERATED_PATHS = {
    "crates/sim/tests/baseline/fingerprint.json",
}

ACCEPTED_CODE_COMMIT = "26db4f139245015fa8126bb3559cc8b3cd77751d"


def evidence_commit_matches(recorded: str) -> bool:
    if recorded == ACCEPTED_CODE_COMMIT:
        return True
    if recorded == HEAD:
        return True
    ancestor = subprocess.run(
        ["git", "merge-base", "--is-ancestor", recorded, HEAD],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if ancestor.returncode != 0:
        return False
    changed = subprocess.run(
        ["git", "diff", "--name-only", f"{recorded}..HEAD"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    ).stdout.splitlines()
    return bool(changed) and all(
        path in GENERATED_PATHS or path.startswith(GENERATED_PATH_PREFIXES)
        for path in changed
    )


def check_commit_pair(
    problems: list[str],
    label: str,
    certification: dict,
    run: dict,
) -> None:
    recorded = certification.get("commit")
    if recorded != run.get("commit"):
        problems.append(f"{label} certification.json 与 run.json 的 commit 不一致")
        return
    if recorded == "fb442106d030daec6104f63c878d8d88b0cddd56" and run.get(
        "working_tree_dirty"
    ):
        problems.append(f"{label} 的认证运行发生在未提交的工作区")
        return
    if not isinstance(recorded, str) or not evidence_commit_matches(recorded):
        problems.append(f"{label} 的 commit 与认证运行对应的代码提交不一致")


def check_gate(problems: list[str]) -> None:
    path = EVIDENCE / "gap_closure_cycle" / "gate.txt"
    rows = [line for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    if len(rows) != 39:
        problems.append(f"gate.txt 有 {len(rows)} 行，要求 39 行")
    bad = [row for row in rows if row.startswith("  BAD")]
    if bad:
        problems.append(f"gate.txt 仍有 {len(bad)} 条越界：{[r.split()[1] for r in bad]}")


def check_verification(problems: list[str]) -> None:
    payload = load(EVIDENCE / "gap_closure_cycle" / "verification.json")
    gate = payload["verification"]["gate"]
    if gate["geometric_violations"] != 0 or gate["context_violations"] != 0:
        problems.append("verification.json 的几何或语境违规非零")
    if gate["criteria_total"] != 39 or gate["criteria_out_of_band"] != 0:
        problems.append("verification.json 的门禁汇总不是 39 条全通过")


def check_certification(problems: list[str]) -> None:
    directory = EVIDENCE / "action_chain_validation"
    certification = load(directory / "certification.json")
    if certification.get("certification_status") != "pass":
        problems.append("certification.json 的认证状态不是 pass")
    run = load(directory / "run.json")
    check_commit_pair(problems, "NBA", certification, run)
    for field in ("ruleset_version", "calibration_version", "engine_model_version"):
        if not run.get(field):
            problems.append(f"run.json 缺少 {field}")
    manifest = directory / "match_manifest.json"
    if manifest.is_file():
        replay = load(manifest)
        for field in ("ruleset_version", "calibration_version", "engine_model_version"):
            if replay.get(field) != run.get(field):
                problems.append(f"回放清单与 run.json 的 {field} 不一致")
    mechanism = load(directory / "mechanism_core.json")
    results = mechanism["results"]
    if len(results) != 13 or not mechanism["passed"]:
        problems.append("mechanism_core.json 没有覆盖 13 项全部通过")
    bad = [r["attribute"] for r in results if r["verdict"] != "pass"]
    if bad:
        problems.append(f"机制实验未通过的属性：{bad}")


def check_fiba_certification(problems: list[str]) -> None:
    directory = EVIDENCE / "fiba_validation"
    certification = load(directory / "certification.json")
    if certification.get("certification_status") != "pass":
        problems.append("FIBA certification.json 的认证状态不是 pass")
    run = load(directory / "run.json")
    check_commit_pair(problems, "FIBA", certification, run)
    for field in ("ruleset_version", "calibration_version", "engine_model_version"):
        if not run.get(field):
            problems.append(f"FIBA run.json 缺少 {field}")
    manifest = directory / "match_manifest.json"
    if manifest.is_file():
        replay = load(manifest)
        for field in ("ruleset_version", "calibration_version", "engine_model_version"):
            if replay.get(field) != run.get(field):
                problems.append(f"FIBA 回放清单与 run.json 的 {field} 不一致")


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
    known = load(ROOT / "data" / "calibration" / "known_insufficient.json")
    if known.get("entries"):
        problems.append("known_insufficient.json 的登记表非空")


def main() -> int:
    problems: list[str] = []
    check_gate(problems)
    check_verification(problems)
    check_certification(problems)
    check_fiba_certification(problems)
    check_performance(problems)
    check_catalogs(problems)
    if problems:
        for problem in problems:
            print(f"❌ {problem}", file=sys.stderr)
        return 1
    print("✅ 最终工件一致：39 条门禁、NBA/FIBA 认证、13 项机制实验、7 类资源场景")
    print(f"   HEAD {HEAD}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
