#!/usr/bin/env python3
"""D22 状态组收敛守卫（plan.md §3.5）。

`MatchEngine` 的字段必须**只是** `crates/engine/src/match_engine/state.rs`
里声明的具名状态组，不得再有裸字段（`rules`、`possession`、`ball_pos_3d` …）。
裸字段一旦回流，阶段函数就能直接读写彼此的状态，前一版拆分留下的耦合会重新出现。

判据：

1. `MatchEngine` 的每个字段都是 `state.rs` 中已声明的状态组的**类型名**；
2. `state.rs` 中的状态组字段全部为 `pub(crate)`（组内字段对同 crate 可见，
   这是状态组的访问面，组本身不得被外部 crate 使用）；
3. `mod.rs` 行数 ≤ 400（编排层体量）。

负面对照（`--self-test`）：分别注入一条裸字段、一条 `pub` 组字段、一行超限的
`mod.rs`，三次守卫都必须变红；未注入时的基线必须为绿。

退出码：0 = 通过，1 = 发现违规。
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ENGINE_DIR = ROOT / "crates" / "engine" / "src" / "match_engine"
MOD_SRC = ENGINE_DIR / "mod.rs"
STATE_SRC = ENGINE_DIR / "state.rs"

# `mod.rs` 行数上限（plan.md §3.5）。
MOD_MAX_LINES = 400

# 十个状态组的组名（`docs/architecture.md` §4.3 为单一事实源）。
# 这里登记的是**字段名**，其类型必须指向 `state.rs` 里的同名结构体。
STATE_GROUP_FIELDS = (
    "audit",
    "ball",
    "clock",
    "config",
    "flow",
    "journal",
    "ledger",
    "observations",
    "possession_ctx",
    "systems",
)


def struct_body(source: str) -> str:
    """返回 `MatchEngine` 结构体声明的内容（花括号之间）。"""
    match = re.search(r"^pub struct MatchEngine \{", source, re.M)
    if not match:
        raise SystemExit("❌ 守卫失效：在 mod.rs 中找不到 `pub struct MatchEngine`")
    rest = source[match.end():]
    depth = 1
    for index, char in enumerate(rest):
        if char == "{":
            depth += 1
        elif char == "}":
            depth -= 1
            if depth == 0:
                return rest[:index]
    raise SystemExit("❌ 守卫失效：`MatchEngine` 结构体花括号不闭合")


def declared_groups(state_source: str) -> set[str]:
    """返回 `state.rs` 中声明的状态组类型名。"""
    return set(re.findall(r"^pub\(crate\) struct (\w+) \{", state_source, re.M))


def find_bare_fields(source: str, state_source: str) -> list[str]:
    """返回 `MatchEngine` 上的裸字段（类型不是已声明的状态组）。"""
    groups = declared_groups(state_source)
    body = struct_body(source)
    violations = []
    for name, type_text in re.findall(
        r"^\s{4}([a-z_][a-z_0-9]*):\s*([^,\n]+),", body, re.M
    ):
        # 组的类型写作 `state::<Group>`。
        match = re.fullmatch(r"state::(\w+)", type_text.strip())
        if not match or match.group(1) not in groups:
            violations.append(name)
    return sorted(violations)


def find_public_group_fields(state_source: str) -> list[str]:
    """返回状态组内不是 `pub(crate)` 的字段（`pub` 会泄漏到外部 crate）。"""
    violations = []
    for block in re.finditer(
        r"^pub\(crate\) struct (\w+) \{(.*?)^\}", state_source, re.M | re.S
    ):
        group, body = block.group(1), block.group(2)
        for line in body.split("\n"):
            stripped = line.strip()
            if stripped.startswith("pub ") and not stripped.startswith("pub(crate)"):
                field = re.match(r"pub (\w+):", stripped)
                if field:
                    violations.append(f"{group}.{field.group(1)}")
    return sorted(violations)


def mod_line_count(source: str) -> int:
    return source.rstrip("\n").count("\n") + 1


def report(mod_source: str, state_source: str) -> int:
    """检查一份（可能被注入的）源码，打印违规并返回退出码。"""
    failed = False

    bare = find_bare_fields(mod_source, state_source)
    if bare:
        failed = True
        print("❌ MatchEngine 出现裸字段 —— 状态组收敛被绕过：")
        for name in bare:
            print(f"   - MatchEngine.{name}")
        print("   字段必须归属某个具名状态组（state.rs），见 docs/architecture.md §4.3。")

    public = find_public_group_fields(state_source)
    if public:
        failed = True
        print("❌ 状态组字段泄漏到外部 crate（必须是 pub(crate)）：")
        for name in public:
            print(f"   - {name}")

    lines = mod_line_count(mod_source)
    if lines > MOD_MAX_LINES:
        failed = True
        print(f"❌ mod.rs 行数 {lines} > 上限 {MOD_MAX_LINES}：编排层不得回流业务逻辑。")

    return 1 if failed else 0


def self_test() -> int:
    """三个负面对照：裸字段、pub 组字段、超限 mod.rs 都必须使守卫变红。"""
    original_mod = MOD_SRC.read_text(encoding="utf-8")
    original_state = STATE_SRC.read_text(encoding="utf-8")

    # 基线必须为绿。
    if report(original_mod, original_state) != 0:
        print("❌ self-test: 基线源码已是红的，无法作为对照")
        return 1

    cases = [
        (
            "裸字段",
            original_mod.replace(
                "    clock: state::MatchClock,",
                "    clock: state::MatchClock,\n    shot_clock: f32,",
                1,
            ),
            original_state,
        ),
        (
            "pub 组字段",
            original_mod,
            original_state.replace("    pub(crate) rules:", "    pub rules:", 1),
        ),
        (
            "mod.rs 超限",
            original_mod + "// padding\n" * (MOD_MAX_LINES + 1),
            original_state,
        ),
    ]

    for label, mutated_mod, mutated_state in cases:
        if mutated_mod == original_mod and mutated_state == original_state:
            print(f"❌ self-test: 注入「{label}」失败，源码未发生变化")
            return 1
        if report(mutated_mod, mutated_state) != 1:
            print(f"❌ self-test: 守卫未检出注入的「{label}」")
            return 1

    print("✅ State-group guard self-test passed (detects bare fields, public fields, oversized mod.rs).")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    code = report(
        MOD_SRC.read_text(encoding="utf-8"), STATE_SRC.read_text(encoding="utf-8")
    )
    if code == 0:
        print(
            f"✅ State-group guard passed (MatchEngine holds only the "
            f"{len(STATE_GROUP_FIELDS)} named state groups; mod.rs "
            f"{mod_line_count(MOD_SRC.read_text(encoding='utf-8'))} <= {MOD_MAX_LINES} lines)."
        )
    return code


if __name__ == "__main__":
    sys.exit(main())
