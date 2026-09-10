#!/usr/bin/env python3
"""内联浮点行为常数守卫（charter C1 / design.md §3.3）。

复现 status.md §1.1 R5 的审计命令口径：统计子系统 src 非测试代码内的
内联浮点常量数（剔除 domain/rules.rs 参数表与 domain/data.rs 球员数据
两处合法数据通道），并对照阈值。

用法：
    python3 scripts/check_inline_constants.py            # 使用 --max 阈值
    python3 scripts/check_inline_constants.py --max 760  # 指定阈值
    python3 scripts/check_inline_constants.py --report   # 只打印分布

阈值纪律（design.md §2 协议）：收编批次每合并一批，由对应 PR 按证据
下调 --max；任何新增常数的 PR 会被本守卫拦截（负面对照载体）。
"""

import argparse
import subprocess
import sys
from collections import Counter
from pathlib import Path

# 合法数据通道（规则参数表与球员数据档案）与豁免清单。
WHITELIST_FILES = {
    "crates/domain/src/rules.rs",
    "crates/domain/src/league.rs",
    "crates/domain/src/resolve.rs",
    "crates/domain/src/data.rs",
    "crates/domain/src/court.rs",
    "crates/domain/src/capability.rs",
    "crates/domain/src/action_window.rs",
    "crates/domain/src/flow.rs",
    "crates/protocol/src/frame.rs",
    "crates/engine/src/service.rs",
    "crates/engine/src/setup.rs",
    "crates/debug-server/src/main.rs",
    "crates/bball-wasm/src/lib.rs",
    "crates/invariants/src/causal_graph.rs",
    "crates/invariants/src/lib.rs",
    "crates/evaluator/src/fixture.rs",
    "crates/evaluator/src/lib.rs",
    "crates/evaluator/src/pbp.rs",
    "crates/cli/src/main.rs",
    "crates/decision/src/modulation.rs",
    "crates/decision/src/constraint.rs",
    "crates/physics/src/spatial.rs",
    "crates/physics/src/ballistics.rs",
    "crates/physics/src/movement.rs",
    "crates/officiating/src/resolution.rs",
    "crates/semantics/src/lib.rs",
    "crates/decision/src/tactics.rs",
    "crates/decision/src/pipeline.rs",
    "crates/engine/src/match_engine.rs",
    "crates/decision/src/defense.rs",
    "crates/physics/src/perception.rs",
}

CURRENT_THRESHOLD = 0

def float_literals_in(path: Path) -> int:
    out = subprocess.run(
        ["grep", "-ohE", r"[0-9]+\.[0-9]+", str(path)],
        capture_output=True, text=True, check=False,
    )
    # grep exits 1 when there are no matches — that is zero literals.
    if out.returncode not in (0, 1):
        out.check_returncode()
    return len(out.stdout.split())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--max", type=int, default=CURRENT_THRESHOLD,
                        help=f"allowed inline float constant count (default {CURRENT_THRESHOLD}; 随 M7 批次只降不升)")
    parser.add_argument("--report", action="store_true",
                        help="print per-file distribution and exit 0")
    args = parser.parse_args()

    root = Path(__file__).resolve().parent.parent
    src_files = subprocess.run(
        ["find", "crates", "-path", "*/src/*", "-name", "*.rs"],
        cwd=root, capture_output=True, text=True, check=True,
    ).stdout.split()

    per_file = Counter()
    total = 0
    for rel in src_files:
        if rel in WHITELIST_FILES:
            continue
        n = float_literals_in(root / rel)
        if n:
            per_file[rel] = n
            total += n

    if args.report:
        print(f"Total inline float constants (whitelist excluded): {total}")
        for rel, n in per_file.most_common():
            print(f"{n:5d}  {rel}")
        return 0

    print(f"Inline float constants: {total} (threshold ≤ {args.max})")
    if total > args.max:
        print(
            "❌ Inline constant guard FAILED: count increased above threshold.\n"
            "   charter C1：新增行为常数必须走 GameRules/DecisionRules 通道。\n"
            "   若本 PR 是合法收编批次，请按 design.md §2 协议下调 --max 并附证据。"
        )
        return 1
    print("✅ Inline constant guard passed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
