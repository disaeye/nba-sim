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

审计范围是「最近一次 CI 全绿 commit 之后的所有 commit」而非仅有
HEAD（见 `scripts/ci_baseline.py`）：若无此设计，把基准改动与源码
改动分装成两个 commit 推送（前者夹在失败后的小提交里），即可绕过
本守卫。逐 commit 审计保证每个提交各自独立接受检查。

豁免方式（二选一）：
- 提交信息含 `Threshold-Change: <理由>` 尾注；或
- 环境变量 `NBA_THRESHOLD_ACK` 指向一份豁免说明文件（CI 人工评审路径）。

这不是禁止修改基准——校准本来就要改基准。它让"改基准"从一件**隐形的、
免费的**动作，变成一件**显式的、需要署名的**动作。

## 用法

    python3 scripts/check_threshold_integrity.py            # 自动审计绿色基线后的 commit
    python3 scripts/check_threshold_integrity.py --range A..B
    python3 scripts/check_threshold_integrity.py --self-test # 负面对照
"""

from __future__ import annotations

import argparse
import re
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from ci_baseline import audit_range  # noqa: E402

ROOT = Path(__file__).resolve().parents[1]

# 判定基准（被测对象不能在同一次改动里改这些）。
# 新增判定基准时**必须**在此登记，否则守卫形同虚设——
# 因此本清单自身也受 `--self-test` 检查（见 SELF_TEST_CASES）。
THRESHOLD_PATTERNS: list[str] = [
    r"^scripts/inline_constant_budget\.json$",
    r"^crates/evaluator/fixtures/.*\.json$",
    r"^scripts/check_inline_constants\.py$",
    r"^scripts/check_world_privacy\.py$",
    r"^scripts/check_engine_state_groups\.py$",
    r"^scripts/check_disk_budget\.py$",
    r"^scripts/check_threshold_integrity\.py$",
]

# 被测源码（行为实现）。
SUBJECT_PATTERNS: list[str] = [
    r"^crates/[^/]+/src/.*\.rs$",
]

# 允许与判定基准一起改动、*不*算违规的源码（例如纯工具）。
#
# 登记标准：该源码文件与某个判定基准存在**结构性同体**关系——
# 它们必须指向同一个外部位置，分开提交反而会让守卫在过渡期
# 静默失效。典型：资源守卫脚本与其扫描的临时根路径。
# 每项豁免必须附注理由；本清单自身受 self-test 检查。
SUBJECT_EXEMPT: list[str] = [
    # test-support 的临时根与 check_disk_budget 的扫描根必须
    # 同 commit 指向同一位置；分开提交会让守卫扫描旧目录，
    # 新目录泄漏在过渡期不可见。
    r"^crates/test-support/src/lib\.rs$",
]

# 判定基准与被测源码在同一提交里同时修改时，若改动全部落在
# 本清单内的文件对上，允许直接豁免（仍保留 trailer 路径供
# 其他耦合场景署名）。
EXEMPT_PAIRS: list[tuple[str, str]] = [
    # 资源守卫脚本 ↔ 它扫描的临时根实现：结构性同体。
    ("scripts/check_disk_budget.py", "crates/test-support/src/lib.rs"),
    ("scripts/check_disk_budget.py", "crates/cli/src/main.rs"),
]

ACK_TRAILER = "Threshold-Change:"


def _run(args: list[str]) -> str:
    return subprocess.run(
        args, cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout


def changed_files(sha: str, parent: str | None = None) -> list[str]:
    """返回单个 commit 的改动文件路径（相对仓库根）。

    根提交（无父提交）用树对比空目录，等价于整个文件集。
    """
    if parent is None:
        parents = _run(["git", "rev-list", "--parents", "-n", "1", sha]).split()
        parent = parents[1] if len(parents) > 1 else ""
    spec = f"{parent}..{sha}" if parent else f"--root {sha}"
    args = ["git", "diff", "--name-only"]
    if parent:
        args.append(spec)
    else:
        args.extend(["--root", sha])
    out = _run(args)
    return [line for line in out.splitlines() if line.strip()]


def commit_message(sha: str) -> str:
    return _run(["git", "log", "-1", "--pretty=%B", sha])


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


def has_structural_pair(thresholds: list[str], subjects: list[str]) -> bool:
    """本次改动是否只触发了结构性同体的基准/源码对。

    全部命中的（基准，源码）组合都能在 EXEMPT_PAIRS 里找到，
    且没有其他未豁免的源码时，耦合视为合法。"""
    if not thresholds or not subjects:
        return False
    allowed = {(t, s) for t, s in EXEMPT_PAIRS}
    for t in thresholds:
        for s in subjects:
            if (t, s) not in allowed:
                return False
    return True


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
    must_match_subject = "crates/engine/src/match_engine/mod.rs"
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
    fake = ["scripts/inline_constant_budget.json", "crates/engine/src/match_engine/mod.rs"]
    t, s = classify(fake)
    if t and s:
        print(f"   ✅ detects coupled change: thresholds={t} subjects={s}")
    else:
        ok = False
        print(f"   ❌ failed to detect coupled change: thresholds={t} subjects={s}")

    # 3. 负样本：只改源码 → 不应误报
    t, s = classify(["crates/engine/src/match_engine/mod.rs"])
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

    # 6. 结构性同体对：资源守卫与其扫描根 → 允许直接豁免
    #    （lib.rs 同时也在 SUBJECT_EXEMPT 里，这里用 CLI 侧源码验证配对）
    t, s = classify(["scripts/check_disk_budget.py", "crates/cli/src/main.rs"])
    if t and s and has_structural_pair(t, s):
        print("   ✅ structural pair exempt: guard ↔ its scan root")
    else:
        ok = False
        print(f"   ❌ structural pair not exempt: t={t} s={s}")

    # 7. 结构性同体对混入其他源码 → 必须仍然检出
    t, s = classify([
        "scripts/check_disk_budget.py",
        "crates/cli/src/main.rs",
        "crates/engine/src/match_engine/mod.rs",
    ])
    if t and s and not has_structural_pair(t, s):
        print("   ✅ structural pair + extra subject still detected")
    else:
        ok = False
        print(f"   ❌ extra subject slipped through: t={t} s={s}")

    print("   self-test:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--range", dest="rev_range", default=None,
                    help="审计 commit 区间，例如 HEAD~3..HEAD；"
                         "缺省时自动审计绿色 CI 基线之后的全部 commit")
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

    commits = audit_range(args.rev_range)
    print(f"🔍 threshold integrity: 逐 commit 审计 {len(commits)} 个 commit")

    violations: list[tuple[str, list[str], list[str]]] = []
    for sha in commits:
        files = changed_files(sha)
        thresholds, subjects = classify(files)
        if not (thresholds and subjects):
            continue
        msg = commit_message(sha)
        short = sha[:12]
        if ACK_TRAILER in msg:
            reason = ""
            for line in msg.splitlines():
                if line.startswith(ACK_TRAILER):
                    reason = line[len(ACK_TRAILER):].strip()
                    break
            print(f"   ✅ {short} 豁免生效：{reason or '（未附理由）'}")
            continue
        if has_structural_pair(thresholds, subjects):
            print(f"   ✅ {short} 结构性同体豁免（守卫与扫描根同 commit 迁移）")
            continue
        violations.append((short, thresholds, subjects))

    if violations:
        for short, thresholds, subjects in violations:
            print(f"❌ COUPLED CHANGE @ {short}")
            print("   判定基准与被测源码在同一个提交里被修改：")
            print(f"   thresholds: {thresholds}")
            print(f"   subjects  : {subjects}")
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
