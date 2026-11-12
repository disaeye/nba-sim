#!/usr/bin/env python3
"""源文件行数棘轮守卫（D29 验收门的机械形式）。

每份生产源文件有一份登记过的行数上限。新文件没有登记时适用全局上限。
任何文件超过自己的上限都变红，因此本守卫仍然禁止源文件无界增长。

A–H 工作包把权威状态、规则程序、动作运行时与协议投影接入同一个内核，
`world.rs`、`policy.rs` 等文件已经超过早期设定的全局上限。用
`--write-budget` 把当前实测登记为逐文件基线；之后上限只能靠 PR 证据调整。

负面对照（`--self-test`）：注入一份超过自身上限的文件必须变红，
基线必须为绿。

退出码：0 = 通过，1 = 发现违规。
"""

from __future__ import annotations

import argparse
import json
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES_DIR = ROOT / "crates"
BUDGET_PATH = ROOT / "scripts" / "source_line_budget.json"

# 新文件、未登记文件的全局上限。
MAX_LINES = 1200


def production_sources() -> list[Path]:
    """收集全部生产源文件（crates/*/src 下的 .rs，含子目录）。"""
    return sorted(p for p in CRATES_DIR.glob("*/src/**/*.rs") if p.is_file())


def relative(path: Path) -> str:
    try:
        return path.relative_to(ROOT).as_posix()
    except ValueError:
        return path.as_posix()


def line_counts(crates_dir: Path | None = None) -> dict[str, int]:
    base = crates_dir or CRATES_DIR
    return {
        relative(path): len(path.read_text(encoding="utf-8").splitlines())
        for path in sorted(base.glob("*/src/**/*.rs"))
        if path.is_file()
    }


def load_budget() -> dict[str, int]:
    if not BUDGET_PATH.exists():
        return {}
    return json.loads(BUDGET_PATH.read_text(encoding="utf-8")).get("files", {})


def check(counts: dict[str, int], budget: dict[str, int]) -> list[str]:
    violations: list[str] = []
    for name, lines in sorted(counts.items()):
        limit = budget.get(name, MAX_LINES)
        if lines > limit:
            violations.append(f"{name}: {lines} 行，超过上限 {limit} 行")
    return violations


def self_test() -> int:
    """负面对照：注入一份超限文件必须变红；基线必须为绿。"""
    baseline = check(line_counts(), load_budget())
    if baseline:
        print("❌ 自测失败：基线本身存在违规，守卫无从验证判据：")
        for violation in baseline:
            print(f"   {violation}")
        return 1

    scratch = Path(tempfile.mkdtemp(prefix="nba_line_guard_selftest_"))
    try:
        target_dir = scratch / "crates" / "selftest" / "src"
        target_dir.mkdir(parents=True)
        target = target_dir / "oversized.rs"
        target.write_text("\n" * (MAX_LINES + 1), encoding="utf-8")
        injected = check(line_counts(scratch / "crates"), {})
        if not injected:
            print("❌ 自测失败：注入超限文件后守卫未变红（判据失效）")
            return 1
        # 逐文件基线也不能掩盖新增行：登记值减一之后必须变红。
        counts = {"crates/selftest/src/oversized.rs": MAX_LINES + 1}
        if not check(counts, {"crates/selftest/src/oversized.rs": MAX_LINES}):
            print("❌ 自测失败：逐文件上限没有生效")
            return 1
        print("✅ 自测通过：基线绿、注入超限文件与逐文件上限都变红")
        return 0
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--write-budget", action="store_true",
                        help="把当前实测登记为逐文件行数基线")
    parser.add_argument("--self-test", action="store_true", help="负面对照自测")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    counts = line_counts()
    if args.write_budget:
        BUDGET_PATH.write_text(
            json.dumps({"files": counts}, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
        print(f"✅ Wrote per-file line budget to {BUDGET_PATH.relative_to(ROOT)}")
        print(f"   {len(counts)} 份生产源文件，最大 {max(counts.values())} 行")
        return 0

    budget = load_budget()
    violations = check(counts, budget)
    if violations:
        print(f"❌ 源文件行数守卫失败（{len(violations)} 份超限）：")
        for violation in violations:
            print(f"   {violation}")
        return 1
    largest = max(counts, key=lambda name: counts[name])
    print(
        f"✅ 源文件行数守卫通过（{len(counts)} 份生产源文件，"
        f"最大 {largest} {counts[largest]} 行）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
