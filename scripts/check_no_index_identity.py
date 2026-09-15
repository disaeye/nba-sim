#!/usr/bin/env python3
"""身份去索引化守卫（P-2 / round-11 Step4c）。

## 为什么需要这个守卫

本项目曾存在系统性的「顺序即身份」耦合（round-11 实测四处）：

1. `builtin_attributes(index)` / `builtin_tendencies(index)` —— 能力/倾向按数组下标分派；
2. `id = format!("{}_{}", prefix, index + 1)` —— id 本身编码下标；
3. `builtin_roles(index)` + `PlayerData.roles` —— 角色按下标分派；
4. `default_lineup` 取**数组前 5 个**为首发，`validate_team` 用
   `ids.len() <= 5` 判定首发（该 `ids` 实为查重 HashSet，语义上是插入计数）。

后果：轮转名册数组就能改变首发阵容与处理球人 —— 违反
`attributes.md §2.7/§2.9/T1`（roles 必须移除）与 `tactics.md TA3`
（角色是槽位不是身份）。

契约条款存在了很久，但没有**机械守卫**，所以耦合一直存活。本守卫把它
变成可判定的。

## 检查项

- **A. 名册档案不得携带 `roles` 字段**（契约要求移除）。
- **B. 不得按数组下标分派球员属性/倾向/角色**：禁止
  `fn builtin_*（index: usize)` 形式的球员数据生成器；
  禁止 `players[index]` / `.nth(index)` 直接取球员身份。
- **C. 首发不得由数组位置决定**：禁止 `starters[0]` 用作身份，
  禁止 `ids.len() <= 5` 这类"位置即首发"判定。
- **D. 不得硬编码球员 id**（`"H_1"` / `"A_1"` 等）作为回退。

## 用法

    python3 scripts/check_no_index_identity.py
    python3 scripts/check_no_index_identity.py --self-test
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]

SRC_GLOBS = ["crates/*/src/**/*.rs", "crates/*/tests/**/*.rs", "crates/*/src/*.rs"]
ROSTER_GLOBS = ["data/roster/*.json"]

# A. 名册档案禁止的字段
FORBIDDEN_ROSTER_FIELDS = ["roles"]

# B. 球员数据生成器按 index 分派
PLAYER_INDEX_FN = re.compile(
    r"fn\s+(builtin_attributes|builtin_tendencies|builtin_roles|builtin_players)\s*\(\s*index\s*:\s*usize"
)
# B2. 以数组下标直接取球员身份（保守模式：只查最明确的形态）
PLAYER_INDEX_LOOKUP = re.compile(r"players\s*\[\s*index\s*\]|players\s*\.\s*nth\s*\(\s*index\s*\)")

# C. 首发/初始持球人由数组位置决定
STARTERS_POSITIONAL = re.compile(r"starters\s*\[\s*0\s*\]")
STARTERS_LEN_HEURISTIC = re.compile(r"ids\s*\.\s*len\s*\(\s*\)\s*<=\s*5")

# D. 硬编码球员 id
HARDCODED_PLAYER_ID = re.compile(r'"(H|A)_[0-9]{1,2}"')


def rust_files() -> list[Path]:
    out: list[Path] = []
    for g in SRC_GLOBS:
        out.extend(ROOT.glob(g))
    return sorted(set(p for p in out if p.is_file()))


def check_rosters() -> list[str]:
    problems: list[str] = []
    files = sorted(ROOT.glob("data/roster/*.json"))
    if not files:
        problems.append("data/roster/*.json is missing — roster must be a data asset")
        return problems
    for f in files:
        try:
            data = json.loads(f.read_text(encoding="utf-8"))
        except json.JSONDecodeError as e:
            problems.append(f"{f.relative_to(ROOT)}: invalid JSON ({e})")
            continue
        for i, p in enumerate(data.get("players", [])):
            for field in FORBIDDEN_ROSTER_FIELDS:
                if field in p:
                    problems.append(
                        f"{f.relative_to(ROOT)}: player[{i}] carries forbidden field "
                        f"`{field}` (attributes.md §2.9 requires roles to be removed)"
                    )
    return problems


def check_sources() -> list[str]:
    problems: list[str] = []
    for f in rust_files():
        rel = f.relative_to(ROOT)
        # 跳过注释行（历史注释会提到被删除的旧实现）；
        # 并跳过 `#[cfg(test)]` 之后的测试模块 —— 合成夹具把 id 当不透明
        # 标签是合法的，且本轮已按硬编码 id 判定只覆盖**生产**逻辑。
        lines = []
        in_test_mod = False
        for n, line in enumerate(f.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#[cfg(test)]"):
                in_test_mod = True
            if in_test_mod:
                continue
            s = line.strip()
            if s.startswith("//") or s.startswith("///") or s.startswith("//!"):
                continue
            lines.append((n, line))

        for n, line in lines:
            if PLAYER_INDEX_FN.search(line):
                problems.append(
                    f"{rel}:{n}: player data generator dispatches on array index "
                    f"({line.strip()[:70]}…) — identity must not come from position (P-2)"
                )
            if PLAYER_INDEX_LOOKUP.search(line):
                problems.append(
                    f"{rel}:{n}: player identity taken by array index ({line.strip()[:70]}…)"
                )
            if STARTERS_POSITIONAL.search(line):
                problems.append(
                    f"{rel}:{n}: `starters[0]` used as identity — starters must come from "
                    f"the roster `starter` flag, not array position ({line.strip()[:60]}…)"
                )
            if STARTERS_LEN_HEURISTIC.search(line):
                problems.append(
                    f"{rel}:{n}: `ids.len() <= 5` used to decide starters — `ids` is a "
                    f"dedup set, its len is an insertion count, not an index"
                )
            # D. 硬编码球员 id **只对生产代码**判定。
            #
            # 理由：测试夹具把 id 当作不透明标签使用是合法的（它构造合成事件流，
            # 不依赖名册身份语义）。真正的不变量是：**生产逻辑不得依赖具体 id**
            # —— 那才是"顺序/名字即身份"的耦合。
            if "/tests/" not in str(rel):
                m = HARDCODED_PLAYER_ID.search(line)
                if m:
                    problems.append(
                        f"{rel}:{n}: hardcoded player id {m.group(0)} in production code — "
                        f"identity must be derived from the roster asset + capability, "
                        f"not from a literal id"
                    )
    return problems


def self_test() -> int:
    print("🧪 no-index-identity guard self-test")
    ok = True

    cases = [
        ("fn builtin_attributes(index: usize) -> X {", PLAYER_INDEX_FN, True),
        ("fn builtin_tendencies(index: usize) {", PLAYER_INDEX_FN, True),
        ("let p = players[index];", PLAYER_INDEX_LOOKUP, True),
        ("starters[0].clone()", STARTERS_POSITIONAL, True),
        ("let is_starter = ids.len() <= 5;", STARTERS_LEN_HEURISTIC, True),
        ('let id = "H_1".to_string();', HARDCODED_PLAYER_ID, True),
        # negatives
        ("let score = p.attributes.passing;", HARDCODED_PLAYER_ID, False),
    ]
    for text, pat, should_match in cases:
        hit = bool(pat.search(text))
        label = "detects" if should_match else "ignores"
        if hit == should_match:
            print(f"   ✅ {label}: {text[:48]}")
        else:
            ok = False
            print(f"   ❌ failed to {label}: {text[:48]}")

    # 守卫自身可用性
    if not rust_files():
        ok = False
        print("   ❌ guard cannot find any Rust source files")
    else:
        print(f"   ✅ guard sees {len(rust_files())} Rust files")

    print("   self-test:", "PASS" if ok else "FAIL")
    return 0 if ok else 1


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    problems = check_rosters() + check_sources()
    if problems:
        print("❌ identity-by-position violations found:")
        for p in problems:
            print(f"   - {p}")
        print()
        print("   charter/契约：attributes.md §2.7/§2.9/T1（roles 移除）、")
        print("   tactics.md TA3（角色是槽位不是身份）。")
        print("   身份必须来自球员档案 + 能力适配，不得来自数组位置。")
        return 1

    print("✅ no-index-identity guard passed (identity comes from data, not position).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
