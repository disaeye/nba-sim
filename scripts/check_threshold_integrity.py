#!/usr/bin/env python3
"""阈值完整性守卫（Ruler Independence Guard）。

## 为什么需要这个守卫

一个判定基准如果可以被被测对象在同一次改动里改掉，它就不是基准。

本项目已实测的两个事故：

1. `scripts/inline_constant_budget.json` 是与源码**同一个 commit** 创建的
   （`f0d0944`，纯新增），从第一天起就等于当时的实测值。它只能检测"新增
   常数"，永远不能推动"收编常数"。实测把某个文件的预算上调 10 后守卫依旧
   绿——棘轮可以被同一个 PR 无声放宽。
2. `crates/evaluator/fixtures/*.json` 的参考分布带决定了评判结果。若与
   引擎改动同 PR 修改，任何"真实度改善"都可以由放宽带来，而不是由修复带来。

## 本条守卫的规则

**同一个提交不得同时修改「判定基准」与「被测源码」，除非显式豁免。**

豁免方式（二选一）：
- 提交信息含 `Threshold-Change: <理由>` 尾注；或
- 环境变量 `NBA_THRESHOLD_ACK` 指向一份豁免说明文件（CI 人工评审路径）。

这不是禁止修改基准——校准本来就要改基准。它让"改基准"从一件**隐形的、
免费的**动作，变成一件**显式的、需要署名的**动作。

## 用法

    python3 scripts/check_threshold_integrity.py            # 检查 HEAD
    python3 scripts/check_threshold_integrity.py --range A..B
    python3 scripts/check_threshold_integrity.py --self-test # 负面对照
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

# 判定基准（被测对象不能在同一次改动里改这些）。
# 新增判定基准时**必须**在此登记，否则守卫形同虚设——
# 因此本清单自身也受 `--self-test` 检查（见 SELF_TEST_CASES）。
THRESHOLD_PATTERNS: list[str] = [
    r"^scripts/inline_constant_budget\.json$",
    r"^crates/evaluator/fixtures/.*\.json$",
    r"^scripts/check_inline_constants\.py$",
    r"^scripts/check_world_privacy\.py$",
    r"^scripts/check_disk_budget\.py$",
    r"^scripts/check_threshold_integrity\.py$",
]

# 被测源码（行为实现）。
SUBJECT_PATTERNS: list[str] = [
    r"^crates/[^/]+/src/.*\.rs$",
]

# 允许与判定基准一起改动、*不*算违规的源码（例如纯工具）。
SUBJECT_EXEMPT: list[str] = []

ACK_TRAILER = "Threshold-Change:"


def _run(args: list[str]) -> str:
    return subprocess.run(
        args, cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout


def changed_files(rev_range: str | None) -> list[str]:
    """返回本次改动涉及的文件路径（相对仓库根）。"""
    if rev_range:
        out = _run(["git", "diff", "--name-only", rev_range])
    else:
        # 优先比较 HEAD~1..HEAD；单提交仓库或首个提交时回退到 HEAD 树。
        try:
            out = _run(["git", "diff", "--name-only", "HEAD~1..HEAD"])
        except subprocess.CalledProcessError:
            out = _run(["git", "show", "--pretty=", "--name-only", "HEAD"])
    return [l for l in out.splitlines() if l.strip()]


def commit_message(rev_range: str | None) -> str:
    if rev_range:
        rev = rev_range.split("..")[-1] or "HEAD"
    else:
        rev = "HEAD"
    try:
        return _run(["git", "log", "-1", "--pretty=%B", rev])
    except subprocess.CalledProcessError:
        return ""


def matches_any(path: str, patterns: list[str]) -> bool:
    return any(re.match(p, path) for p in patterns)


def classify(files: list[str]) -> tuple[list[str], list[str]]:
    thresholds = [f for f in files if matches_any(f, THRESHOLD_PATTERNS)]
    subjects = [
        f
        for f in files
        if matches_any(f, SUBJECT_PATTERNS) and not matches_any(f, SUBJECT_EXEMPT)
    ]
    return thresholds, subjects


def check_self_registration() -> list[str]:
    """守卫的自我防护：断言清单非空且能识别已知基准/被测样本。

    若有人把 THRESHOLD_PATTERNS 清空，守卫会永远通过——
    这种"守卫自身的静默失效"与它要防的缺陷同构，因此必须自检。
    """
    problems: list[str] = []
    if not THRESHOLD_PATTERNS:
        problems.append("THRESHOLD_PATTERNS is empty — the guard cannot detect anything")
    if not SUBJECT_PATTERNS:
        problems.append("SUBJECT_PATTERNS is empty — the guard cannot detect anything")

    must_match_threshold = "scripts/inline_constant_budget.json"
    must_match_subject = "crates/engine/src/match_engine.rs"
    if not matches_any(must_match_threshold, THRESHOLD_PATTERNS):
        problems.append(f"guard fails to classify {must_match_threshold} as a threshold")
    if not matches_any(must_match_subject, SUBJECT_PATTERNS):
        problems.append(f"guard fails to classify {must_match_subject} as a subject")

    # 负样本：普通文档不应被算作基准，否则守卫会天天误报并被忽略。
    if matches_any("docs/dev/status.md", THRESHOLD_PATTERNS):
        problems.append("docs/ must not be classified as a threshold")
    return problems


def self_test() -> int:
    """负面对照：伪造一次「同提交修改基准 + 源码」必须被判红。"""
    print("🧪 threshold-integrity guard self-test")
    ok = True

    # 1. 自我防护
    problems = check_self_registration()
    if problems:
        ok = False
        for p in problems:
            print(f"   ❌ self-registration: {p}")
    else:
        print("   ✅ self-registration: 清单可识别已知基准与被测源码")

    # 2. 正样本：同时改预算与源码 → 必须检出
    fake = ["scripts/inline_constant_budget.json", "crates/engine/src/match_engine.rs"]
    t, s = classify(fake)
    if t and s:
        print(f"   ✅ detects coupled change: thresholds={t} subjects={s}")
    else:
        ok = False
        print(f"   ❌ failed to detect coupled change: thresholds={t} subjects={s}")

    # 3. 负样本：只改源码 → 不应误报
    t, s = classify(["crates/engine/src/match_engine.rs"])
    if t == [] and s:
        print("   ✅ no false positive: source-only change is clean")
    else:
        ok = False
        print(f"   ❌ false positive on source-only change: thresholds={t}")

    # 4. 负样本：只改文档 → 不应误报
    t, s = classify(["docs/dev/status.md", "docs/dev/gap.md"])
    if t == [] and s == []:
        print("   ✅ no false positive: docs-only change is clean")
    else:
        ok = False
        print(f"   ❌ docs misclassified: thresholds={t} subjects={s}")

    # 5. 基准单独改动 → 允许（这就是校准 PR）
    t, s = classify(["crates/evaluator/fixtures/nba.v2.json"])
    if t and s == []:
        print("   ✅ threshold-only change is allowed (calibration PR path)")
    else:
        ok = False
        print(f"   ❌ threshold-only change was rejected: {t} {s}")

    print("   self-test:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--range", dest="rev_range", default=None,
                    help="git diff 范围，例如 HEAD~1..HEAD")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    problems = check_self_registration()
    if problems:
        print("❌ threshold-integrity guard is misconfigured:")
        for p in problems:
            print(f"   - {p}")
        return 1

    files = changed_files(args.rev_range)
    thresholds, subjects = classify(files)
    msg = commit_message(args.rev_range)

    print(f"🔍 threshold integrity: {len(files)} changed files")
    if thresholds:
        print(f"   thresholds touched: {len(thresholds)}")
        for f in thresholds:
            print(f"      - {f}")
    if subjects:
        print(f"   subjects touched  : {len(subjects)}")

    if thresholds and subjects:
        if ACK_TRAILER in msg:
            print(f"   ✅ acknowledged via `{ACK_TRAILER}` trailer — allowed")
            return 0
        print("❌ COUPLED CHANGE: this commit modifies BOTH a judgment threshold")
        print("   and the source code that the threshold judges.")
        print()
        print("   Why this is blocked: a threshold that the measured object can")
        print("   change in the same commit is not a threshold. See the module")
        print("   docstring for the two recorded incidents.")
        print()
        print("   To proceed, EITHER:")
        print("     (a) split the commit (recommended: land the source fix first,")
        print("         observe the failure, then update the threshold); or")
        print(f"     (b) add a `{ACK_TRAILER} <reason>` trailer to the commit")
        print("         message, so relaxing the ruler is an explicit, signed act.")
        return 1

    print("✅ threshold integrity guard passed (no coupled ruler change).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
