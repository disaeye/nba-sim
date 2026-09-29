#!/usr/bin/env python3
"""球态单一写入口守卫（R8 / architecture.md §3.2）。

## 为什么需要这个守卫

`BallState` 是球权归属的唯一事实源（P1）。规格要求比赛推进中的所有
球态变更都经唯一写通道 `MatchEngine::transition_ball_state`
（领域层纯函数转换表校验），物理层与兄弟模块不得直接改写 `ball_state`
字段。历史上该字段是 `pub(crate)`，任何同 crate 模块都能就地赋值，
写入纪律只靠约定维持——本守卫把它变成可判定的。

## 检查项

- **A. 直接赋值**：`…ball_state = …`（含 `self.ball.ball_state = x`）
  只允许出现在 `state.rs`（`BallRuntime::set_ball_state` 实现体）；
- **B. 结构体字面量初始化**：`ball_state:` 显式字段初始化只允许出现在
  `state.rs`（`BallRuntime::new` 用简写形式，不会命中本模式）；
- **C. 写方法调用**：`set_ball_state(` 只允许出现在
  `state.rs`（定义）、`ball_flight/write_entry.rs`（唯一生产写通道
  `transition_ball_state`）与 `test_hooks.rs`（显式命名的测试后门）。

测试文件（`crates/*/tests/`）不在扫描范围：集成测试经
`set_ball_state_for_test` 构造场景，没有直接字段访问权。

## 用法

    python3 scripts/check_ball_state_writes.py
    python3 scripts/check_ball_state_writes.py --self-test
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
ENGINE_DIR = ROOT / "crates" / "engine" / "src" / "match_engine"

# A. 字段直接赋值（排除 `==` 比较）。
ASSIGN = re.compile(r"\.ball_state\s*=(?!=)")
# B. 结构体字面量的显式字段初始化。
STRUCT_INIT = re.compile(r"^\s*ball_state\s*:")
# C. 写方法调用（定义与授权调用点）。
SETTER_CALL = re.compile(r"\bset_ball_state\s*\(")

# 允许出现各模式的文件（相对 ENGINE_DIR）。
ASSIGN_ALLOWED = {"state.rs"}
STRUCT_INIT_ALLOWED = {"state.rs"}
SETTER_ALLOWED = {"state.rs", "ball_flight/write_entry.rs", "test_hooks.rs"}


def engine_sources() -> list[tuple[Path, str]]:
    out: list[tuple[Path, str]] = []
    for f in sorted(ENGINE_DIR.rglob("*.rs")):
        rel = str(f.relative_to(ENGINE_DIR))
        out.append((f, rel))
    return out


def strip_comments_and_tests(text: str) -> list[tuple[int, str]]:
    """去掉注释行与 `#[cfg(test)]` 之后的测试模块，返回 (行号, 原行)。"""
    lines: list[tuple[int, str]] = []
    in_test_mod = False
    for n, line in enumerate(text.splitlines(), 1):
        if line.strip().startswith("#[cfg(test)]"):
            in_test_mod = True
        if in_test_mod:
            continue
        s = line.strip()
        if s.startswith("//"):
            continue
        lines.append((n, line))
    return lines


def check() -> list[str]:
    problems: list[str] = []
    for f, rel in engine_sources():
        lines = strip_comments_and_tests(f.read_text(encoding="utf-8"))
        for n, line in lines:
            if ASSIGN.search(line) and rel not in ASSIGN_ALLOWED:
                problems.append(
                    f"{rel}:{n}: direct ball_state assignment outside the single "
                    f"write channel: {line.strip()[:70]}"
                )
            if STRUCT_INIT.match(line) and rel not in STRUCT_INIT_ALLOWED:
                problems.append(
                    f"{rel}:{n}: ball_state struct-literal init outside state.rs: "
                    f"{line.strip()[:70]}"
                )
            if SETTER_CALL.search(line) and rel not in SETTER_ALLOWED:
                problems.append(
                    f"{rel}:{n}: set_ball_state call outside the authorized "
                    f"write channel/test hook: {line.strip()[:70]}"
                )
    return problems


def self_test() -> int:
    print("🧪 ball-state single-writer guard self-test")
    ok = True
    cases = [
        ("self.ball.ball_state = next;", ASSIGN, True),
        ("engine.ball.ball_state=state;", ASSIGN, True),
        ("if self.ball.ball_state == next {}", ASSIGN, False),
        ("    ball_state: BallTrajectoryKind::Held { .. },", STRUCT_INIT, True),
        ("    let s = ball_state_of();", STRUCT_INIT, False),
        ("self.ball.set_ball_state(next);", SETTER_CALL, True),
        ("let s = set_ball_state_of();", SETTER_CALL, False),
    ]
    for text, pat, should_match in cases:
        hit = bool(pat.search(text))
        label = "detects" if should_match else "ignores"
        if hit == should_match:
            print(f"   ✅ {label}: {text[:48]}")
        else:
            ok = False
            print(f"   ❌ failed to {label}: {text[:48]}")

    sources = engine_sources()
    if not sources:
        ok = False
        print("   ❌ guard cannot find engine match_engine sources")
    else:
        print(f"   ✅ guard sees {len(sources)} engine source files")

    print("   self-test:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    problems = check()
    if problems:
        print("❌ ball-state single-writer violations found:")
        for p in problems:
            print(f"   - {p}")
        print()
        print("   规格：architecture.md §3.2 —— 球态只能经")
        print("   MatchEngine::transition_ball_state 写入（测试后门须显式命名 *_for_test）。")
        return 1

    print("✅ ball-state single-writer guard passed (all writes via transition_ball_state).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
