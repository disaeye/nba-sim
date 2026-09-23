#!/usr/bin/env python3
"""源文件行数上限守卫（D29 验收门的机械形式）。

D29（`docs/dev/current/plan.md` §10）把「全工作区无任何源文件超过 1200 行」
定为验收门，但该门此前只存在于文档里，没有任何机械守卫——`ball_flight/mod.rs`
在 ADR-017 接触模型落地后回涨到 1289 行，无人察觉。本守卫把门变成机械判定：

1. `crates/*/src/**/*.rs`（生产源文件）每份 ≤ 1200 行；
2. 测试文件（`crates/*/tests/**/*.rs`）不在管辖范围：测试文件的可读性
   边界与生产文件不同，D29 的门本来就只对 `src` 生效。

负面对照（`--self-test`）：临时注入一份超限文件必须变红，删除后恢复绿；
基线必须为绿。

退出码：0 = 通过，1 = 发现违规。
"""

from __future__ import annotations

import argparse
import shutil
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CRATES_DIR = ROOT / "crates"

# 行数上限（D29 验收门，plan.md §10.3/§10.8）。
MAX_LINES = 1200


def production_sources() -> list[Path]:
    """收集全部生产源文件（crates/*/src 下的 .rs，含子目录）。"""
    return sorted(p for p in CRATES_DIR.glob("*/src/**/*.rs") if p.is_file())


def check(limit: int = MAX_LINES, crates_dir: Path | None = None) -> list[str]:
    violations: list[str] = []
    base = crates_dir or CRATES_DIR
    for path in sorted(base.glob("*/src/**/*.rs")):
        if not path.is_file():
            continue
        lines = path.read_text(encoding="utf-8").splitlines()
        if len(lines) > limit:
            try:
                display = path.relative_to(ROOT)
            except ValueError:
                display = path
            violations.append(
                f"{display}: {len(lines)} 行，超过 {limit} 行上限"
                "（D29：按职责划分，见 docs/dev/current/plan.md §10）"
            )
    return violations


def self_test() -> int:
    """负面对照：注入一份超限文件必须变红；基线必须为绿。"""
    baseline = check()
    if baseline:
        print("❌ 自测失败：基线本身存在违规，守卫无从验证判据：")
        for v in baseline:
            print(f"   {v}")
        return 1

    scratch = Path(tempfile.mkdtemp(prefix="nba_line_guard_selftest_"))
    try:
        target_dir = scratch / "crates" / "selftest" / "src"
        target_dir.mkdir(parents=True)
        target = target_dir / "oversized.rs"
        target.write_text("\n" * (MAX_LINES + 1), encoding="utf-8")
        injected = check(crates_dir=scratch / "crates")
        if not injected:
            print("❌ 自测失败：注入超限文件后守卫未变红（判据失效）")
            return 1
        print("✅ 自测通过：基线绿、注入超限文件后变红")
        return 0
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true", help="负面对照自测")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    violations = check()
    if violations:
        print(f"❌ 源文件行数守卫失败（{len(violations)} 份超限）：")
        for v in violations:
            print(f"   {v}")
        return 1
    files = production_sources()
    largest = max(production_sources(), key=lambda p: len(p.read_text(encoding="utf-8").splitlines()))
    print(
        f"✅ 源文件行数守卫通过（{len(files)} 份生产源文件，"
        f"最大 {largest.relative_to(ROOT)} {len(largest.read_text(encoding='utf-8').splitlines())} 行 ≤ {MAX_LINES}）"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
