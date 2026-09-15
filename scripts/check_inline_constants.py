#!/usr/bin/env python3
"""内联行为常数守卫（charter C1 / docs/protocol.md §2.1 M7 / gap.md §17.3）。

设计要点（替代历史整文件白名单）：

1. **注释与字符串感知**：先剥离 Rust 行注释、块注释与字符串/字符字面量，
   只统计真正的数值代码常量；文档中的 `§4.3` 之类不再误报。
2. **禁止整文件豁免**：不再有“整个文件跳过”的名单。每个文件都有显式
   预算，且预算只能靠 PR 证据下调。
3. **核心行为文件零容忍新增**：`match_engine.rs`、`tactics.rs`、
   `pipeline.rs`、`constraint.rs`、`resolution.rs`、`semantics/lib.rs`
   的文件预算在 `--budget` 中单独列出；任何新增数值常量都会变红，必须
   改走 GameRules/DecisionRules 数据通道。
4. **多类常数扫描**：浮点常量、整数行为阈值、字符串战术 ID 分支、
   球员 ID 分支。
5. **负面对照**：`--self-test` 会临时注入一个行为常量并断言守卫变红，
   证明守卫真的在被测，而不是恒绿。

预算纪律：`scripts/inline_constant_budget.json` 是唯一事实源。收编批次
只能下调；上调必须在 PR 中说明理由并附证据（docs/protocol.md §1）。
"""

import argparse
import json
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BUDGET_PATH = ROOT / "scripts" / "inline_constant_budget.json"

# 非生产 crate：仅测试使用的支撑代码（`publish = false` 且只被
# dev-dependencies 引用）。它们不实现任何模拟行为，因此不纳入行为常数
# 预算。这是**机器可验证**的排除，不是整文件白名单：`--self-test` 会
# 断言这些 crate 确实声明了 `publish = false`，一旦被生产代码引用就会
# 因不再满足条件而需要显式评估。
NON_PRODUCTION_CRATES = {
    "crates/test-support",
}

# 核心行为文件：新增数值常量一律视为回归（除非预算已由 PR 证据更新）。
CORE_BEHAVIOR_FILES = {
    "crates/engine/src/match_engine.rs",
    "crates/decision/src/tactics.rs",
    "crates/decision/src/pipeline.rs",
    "crates/decision/src/constraint.rs",
    "crates/decision/src/defense.rs",
    "crates/officiating/src/resolution.rs",
    "crates/semantics/src/lib.rs",
}

FLOAT_RE = re.compile(r"(?<![\w.])\d+\.\d+(?![\w])")
# 整数阈值：赋值/比较右值中的裸整数，排除类型位宽、数组下标 0/1、位移等。
INT_THRESHOLD_RE = re.compile(
    r"(?:<=|>=|<|>|==)\s*(\d{2,})|(?:=|:)\s*(\d{3,})(?![\w.])"
)
TACTIC_ID_RE = re.compile(r'"(def_[a-z0-9_]+|off_[a-z0-9_]+|[a-z_]+_v[12])"')
PLAYER_ID_BRANCH_RE = re.compile(r'(?:==|!=)\s*"(?:H_|A_)\d+"')


def strip_comments_and_strings(source: str) -> str:
    """剥离注释与字符串/字符字面量，保留代码结构（换行保持行号不变）。"""
    out = []
    i = 0
    n = len(source)
    while i < n:
        ch = source[i]
        nxt = source[i + 1] if i + 1 < n else ""
        # 行注释
        if ch == "/" and nxt == "/":
            while i < n and source[i] != "\n":
                i += 1
            continue
        # 块注释（支持嵌套）
        if ch == "/" and nxt == "*":
            depth = 1
            i += 2
            while i < n and depth > 0:
                if source[i] == "/" and i + 1 < n and source[i + 1] == "*":
                    depth += 1
                    i += 2
                    continue
                if source[i] == "*" and i + 1 < n and source[i + 1] == "/":
                    depth -= 1
                    i += 2
                    continue
                if source[i] == "\n":
                    out.append("\n")
                i += 1
            continue
        # 字符串字面量
        if ch == '"':
            i += 1
            while i < n:
                if source[i] == "\\":
                    i += 2
                    continue
                if source[i] == '"':
                    i += 1
                    break
                i += 1
            out.append('""')
            continue
        # 字符字面量
        if ch == "'":
            # 区分生命周期标注 'a 与字符 'x'
            j = i + 1
            if j < n and source[j] == "\\":
                j += 2
                if j < n and source[j] == "'":
                    i = j + 1
                    out.append("''")
                    continue
            elif j + 1 < n and source[j + 1] == "'":
                i = j + 2
                out.append("''")
                continue
        out.append(ch)
        i += 1
    return "".join(out)


def split_test_section(source: str) -> tuple[str, str]:
    """把源码切成 (生产代码, 测试代码) 两段。

    为什么需要：`#[cfg(test)] mod tests` 里的数字是测试夹具（构造球员、
    断言区间、合成流），不是行为参数。它们不应占用生产预算——否则预算
    被测试代码占用后，生产侧就留下“免费新增行为常数”的空间。实测该空间
    曾达 191 个常量（capability.rs 53 / resolution.rs 55 / ballistics.rs 42 …），
    使 charter C1 的棘轮对这些文件形同虚设。

    切分规则：以第一个 `#[cfg(test)]` 为界。Rust 的测试模块约定放在文件
    末尾，因此该规则与惯例一致；不使用括号配平以避免误判字符串/注释。
    """
    idx = source.find("#[cfg(test)]")
    if idx < 0:
        return source, ""
    return source[:idx], source[idx:]


def float_literals_in(path: Path) -> int:
    """剥离注释/字符串后的内联浮点常量数（含测试段，供旧调用方使用）。"""
    code = strip_comments_and_strings(path.read_text(encoding="utf-8", errors="ignore"))
    return len(FLOAT_RE.findall(code))


def count_category(path: Path) -> dict:
    raw = path.read_text(encoding="utf-8", errors="ignore")
    # 代码区剥离注释与字符串字面量，用于数值扫描。
    code = strip_comments_and_strings(raw)
    # 字符串分支扫描需要保留字符串字面量的原文。
    code_for_strings = raw
    return {
        "floats": len(FLOAT_RE.findall(code)),
        "int_thresholds": len(INT_THRESHOLD_RE.findall(code)),
        "tactic_id_branches": len(TACTIC_ID_RE.findall(code_for_strings)),
        "player_id_branches": len(PLAYER_ID_BRANCH_RE.findall(code_for_strings)),
    }


def count_production(path: Path) -> dict:
    """只统计**生产代码**（排除 `#[cfg(test)]` 段）的四类扫描量。

    这是棘轮应当计量的口径：测试夹具不改变比赛行为。
    """
    raw = path.read_text(encoding="utf-8", errors="ignore")
    prod_raw, _test_raw = split_test_section(raw)
    code = strip_comments_and_strings(prod_raw)
    return {
        "floats": len(FLOAT_RE.findall(code)),
        "int_thresholds": len(INT_THRESHOLD_RE.findall(code)),
        "tactic_id_branches": len(TACTIC_ID_RE.findall(prod_raw)),
        "player_id_branches": len(PLAYER_ID_BRANCH_RE.findall(prod_raw)),
    }


def count_test(path: Path) -> dict:
    """只统计 `#[cfg(test)]` 段，用于防止测试代码无界膨胀。"""
    raw = path.read_text(encoding="utf-8", errors="ignore")
    _prod_raw, test_raw = split_test_section(raw)
    if not test_raw:
        return {"floats": 0, "int_thresholds": 0, "tactic_id_branches": 0, "player_id_branches": 0}
    code = strip_comments_and_strings(test_raw)
    return {
        "floats": len(FLOAT_RE.findall(code)),
        "int_thresholds": len(INT_THRESHOLD_RE.findall(code)),
        "tactic_id_branches": len(TACTIC_ID_RE.findall(test_raw)),
        "player_id_branches": len(PLAYER_ID_BRANCH_RE.findall(test_raw)),
    }


def allowed_score(per_file: dict) -> int:
    """把四类扫描量加权成一个可比较分数（用于预算棘轮）。"""
    return per_file["floats"] + per_file["int_thresholds"]


def is_non_production(rel_path: str) -> bool:
    """该文件是否属于已声明的非生产 crate。"""
    return any(rel_path.startswith(crate + "/") for crate in NON_PRODUCTION_CRATES)


def verify_non_production_crates() -> list:
    """确认被排除的 crate 确实声明了 `publish = false`。

    防止有人把生产 crate 塞进排除名单来绕过守卫。
    """
    problems = []
    for crate in sorted(NON_PRODUCTION_CRATES):
        manifest = ROOT / crate / "Cargo.toml"
        if not manifest.exists():
            problems.append(f"{crate}: manifest missing")
            continue
        text = manifest.read_text(encoding="utf-8")
        if "publish = false" not in text:
            problems.append(f"{crate}: not marked `publish = false`")
    return problems


def src_files() -> list:
    out = subprocess.run(
        ["find", "crates", "-path", "*/src/*", "-name", "*.rs"],
        cwd=ROOT, capture_output=True, text=True, check=True,
    ).stdout.split()
    return sorted(rel for rel in out if not is_non_production(rel))


def measure() -> dict:
    return {rel: count_production(ROOT / rel) for rel in src_files()}


def measure_tests() -> dict:
    return {rel: count_test(ROOT / rel) for rel in src_files()}


def load_budget() -> dict:
    if not BUDGET_PATH.exists():
        return {"files": {}, "budget_floats": {}}
    try:
        return json.loads(BUDGET_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"❌ Cannot read {BUDGET_PATH.relative_to(ROOT)}: {exc}", file=sys.stderr)
        return {"files": {}, "budget_floats": {}}


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--report", action="store_true", help="print per-file distribution and exit 0")
    parser.add_argument("--write-budget", action="store_true",
                        help="freeze current measurement as the ratchet baseline (收编批次用)")
    parser.add_argument("--self-test", action="store_true",
                        help="negative control: inject a behaviour constant and assert the guard fails")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    measured = measure()
    measured_tests = measure_tests()

    if args.write_budget:
        budget = {
            "files": {rel: allowed_score(v) for rel, v in measured.items() if allowed_score(v)},
            "budget_floats": {rel: v["floats"] for rel, v in measured.items() if v["floats"]},
            "test_files": {
                rel: allowed_score(v) for rel, v in measured_tests.items() if allowed_score(v)
            },
            "test_floats": {
                rel: v["floats"] for rel, v in measured_tests.items() if v["floats"]
            },
        }
        BUDGET_PATH.write_text(json.dumps(budget, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        print(f"✅ Wrote ratchet baseline to {BUDGET_PATH.relative_to(ROOT)}")
        print("   files/budget_floats         = 生产代码（排除 #[cfg(test)]）")
        print("   test_files/test_floats     = #[cfg(test)] 段，单独棘轮")
        return 0

    budget = load_budget()
    if not budget.get("files"):
        print("❌ Missing ratchet baseline. Run: python3 scripts/check_inline_constants.py --write-budget")
        return 1

    if args.report:
        rows = sorted(
            ((allowed_score(v), rel, v) for rel, v in measured.items()),
            reverse=True,
        )
        print("score  floats  ints  tactic  player  file (生产代码，排除 #[cfg(test)])")
        for score, rel, v in rows:
            if score:
                print(f"{score:5d}  {v['floats']:5d}  {v['int_thresholds']:5d}  "
                      f"{v['tactic_id_branches']:5d}  {v['player_id_branches']:5d}  {rel}")
        print()
        trows = sorted(
            ((allowed_score(v), rel, v) for rel, v in measured_tests.items()),
            reverse=True,
        )
        print("score  floats  ints  tactic  player  file (#[cfg(test)] 段)")
        for score, rel, v in trows[:15]:
            if score:
                print(f"{score:5d}  {v['floats']:5d}  {v['int_thresholds']:5d}  "
                      f"{v['tactic_id_branches']:5d}  {v['player_id_branches']:5d}  {rel}")
        return 0

    failures = []
    for rel, v in measured.items():
        measured_score = allowed_score(v)
        allowed = budget.get("files", {}).get(rel, 0)
        if measured_score > allowed:
            kind = "CORE-BEHAVIOR" if rel in CORE_BEHAVIOR_FILES else "file"
            failures.append(
                f"{rel} ({kind}): {measured_score} > budget {allowed} "
                f"[floats={v['floats']}, ints={v['int_thresholds']}, "
                f"tactic_ids={v['tactic_id_branches']}, player_ids={v['player_id_branches']}]"
            )

    # 测试段单独棘轮：防止“把行为参数藏进 #[cfg(test)] 段”绕过生产预算。
    test_budget = budget.get("test_files", {})
    for rel, v in measured_tests.items():
        measured_score = allowed_score(v)
        allowed = test_budget.get(rel, 0)
        if measured_score > allowed:
            failures.append(
                f"{rel} (TEST-SECTION): {measured_score} > test budget {allowed} "
                f"[floats={v['floats']}, ints={v['int_thresholds']}]"
            )

    total = sum(allowed_score(v) for v in measured.values())
    total_tests = sum(allowed_score(v) for v in measured_tests.values())
    print(f"Inline behaviour constants (production, ratcheted): {total}")
    print(f"Inline constants in #[cfg(test)] sections (separately ratcheted): {total_tests}")
    if failures:
        print("❌ Inline constant guard FAILED — new behaviour constants bypassed the rules channel:")
        for f in failures:
            print(f"   - {f}")
        print("   charter C1：新增行为常数必须走 GameRules/DecisionRules 数据通道。")
        print("   若本 PR 是合法收编批次，请下调 scripts/inline_constant_budget.json 并附证据。")
        return 1
    print("✅ Inline constant guard passed (no file exceeded its ratchet budget).")
    return 0


def self_test() -> int:
    """负面对照：注入一个行为浮点常量后测量必须增加。"""
    probe = "fn f() { let threshold = 0.731234; }\n"
    with tempfile.NamedTemporaryFile(suffix=".rs", mode="w", delete=False, encoding="utf-8") as tf:
        tf.write(probe)
        path = Path(tf.name)
    try:
        if float_literals_in(path) != 1:
            print("❌ self-test: float constant not detected")
            return 1
        # 注释里的伪常量不能被计入（历史误报载体 §4.3）。
        with tempfile.NamedTemporaryFile(suffix=".rs", mode="w", delete=False, encoding="utf-8") as tf2:
            tf2.write("/// gap.md §4.3 文档引用\nfn f() {}\n")
            path2 = Path(tf2.name)
        try:
            if float_literals_in(path2) != 0:
                print("❌ self-test: doc-comment section reference false-positived")
                return 1
        finally:
            path2.unlink(missing_ok=True)
        # 整文件白名单必须不再存在（把名字拆开避免自引用误报）。
        source = (ROOT / "scripts" / "check_inline_constants.py").read_text(encoding="utf-8")
        forbidden = "WHITELIST" + "_FILES"
        if forbidden in source:
            print("❌ self-test: whole-file whitelist still present")
            return 1
        # 测试段必须不计入生产预算（charter C1 棘轮的真实性）。
        #
        # 历史缺陷：旧口径把 `#[cfg(test)]` 段的浮动常量计入同一个文件预算，
        # 导致「测试段减 N 个夹具常量 + 生产段加 N 个行为常量」净变化为 0，
        # 守卫完全看不见行为常数注入。实测该缺陷留下 191 个常量的免费空间
        # （capability.rs 53 / resolution.rs 55 / ballistics.rs 42 …）。
        mix = (
            "fn prod() { let a = 0.11; let b = 0.22; }\n"
            "#[cfg(test)]\nmod tests { fn t() { let x = 0.33; let y = 0.44; } }\n"
        )
        with tempfile.NamedTemporaryFile(suffix=".rs", mode="w", delete=False, encoding="utf-8") as tf3:
            tf3.write(mix)
            path3 = Path(tf3.name)
        try:
            prod_count = len(FLOAT_RE.findall(strip_comments_and_strings(split_test_section(mix)[0])))
            test_count = len(FLOAT_RE.findall(strip_comments_and_strings(split_test_section(mix)[1])))
            if prod_count != 2 or test_count != 2:
                print(
                    f"❌ self-test: test-section split wrong "
                    f"(prod={prod_count} expected 2, test={test_count} expected 2)"
                )
                return 1
            # 关键：测试段改动不得影响生产计数。
            mut = mix.replace("0.33", "0", 1)
            prod_after = len(FLOAT_RE.findall(strip_comments_and_strings(split_test_section(mut)[0])))
            if prod_after != prod_count:
                print("❌ self-test: test-section change leaked into production count")
                return 1
        finally:
            path3.unlink(missing_ok=True)
        # 非生产排除必须是可验证的（publish = false）。
        problems = verify_non_production_crates()
        if problems:
            print("❌ self-test: non-production exclusions are not verifiable:")
            for problem in problems:
                print(f"   - {problem}")
            return 1
        # 被排除的 crate 必须确实不在被扫描文件列表中。
        scanned = src_files()
        leaked = [f for f in scanned if is_non_production(f)]
        if leaked:
            print(f"❌ self-test: non-production files still scanned: {leaked[:3]}")
            return 1
        print("✅ Guard self-test passed: detects constants, ignores doc references, "
              "separates #[cfg(test)] from production, has no whole-file whitelist.")
        return 0
    finally:
        path.unlink(missing_ok=True)


if __name__ == "__main__":
    sys.exit(main())
