#!/usr/bin/env python3
"""文档治理守卫：检查引用解析、规格面纪律和工作面结构。

引用格式为 ``<文档路径>.md §N`` 或在目标唯一时使用 ``<basename>.md §N``。
路径优先按仓库根解析；``dev/...`` 按 ``docs/dev/...`` 解析。短 basename
如果对应多个文档会被拒绝，避免历史归档与当前文档之间发生静默误指。
"""
from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
DOCS = ROOT / "docs"

# 支持 docs/dev/status.md、dev/status.md、status.md 三种书写；章节号保持数字层级。
REF = re.compile(
    r"(?<![\w/])(?P<target>[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*\.md)"
    r"\s*§(?P<section>\d+(?:\.\d+)*)"
)

CONTRACT_FORBIDDEN = [
    (re.compile(r"20\d{6}"), "日期戳（如 20260911）"),
    (re.compile(r"20[2-9]\d-\d{2}-\d{2}"), "ISO 日期"),
    (re.compile(r"~\s*\d+(?:\.\d+)?%"), "实测/完成度百分比"),
    # 规格面禁止把某轮次的实测计数写成设计前提（README §1）。“实测”后紧跟
    # 数字是完成度快照的高信号标志；决策面 decisions.md 以实测作为取舍依据，
    # 属 README §0 的独立面，连同 README.md 一并豁免。
    (re.compile(r"实测\s*\d+"), "实测计数（应移入工作面/决策面）"),
    (re.compile(r"\bseed\s*=\s*\d+"), "实测 seed"),
    (re.compile(r"^\s*(?:cargo|python3|pytest|bash|sh|\./)\s", re.M), "可重跑命令"),
]

# 仓库内代码/数据路径引用。守卫其**实在性**：文档（尤其规格面与当前工作面）
# 不得把已删除或从未存在的代码路径当作现状证据。历史归档（cycles/、evidence/）
# 允许保留当时结论（README §6），故排除。
CODE_REF = re.compile(
    r"(?<![\w./-])"
    r"(?P<path>(?:crates|scripts|data|bin|\.github)/[A-Za-z0-9_.\-]+"
    r"(?:/[A-Za-z0-9_.\-]+)*\.(?:rs|py|json|toml|sh|yml|yaml|js|html|css))"
)
HISTORICAL_PREFIXES = ("docs/dev/cycles/", "docs/dev/evidence/")

# 已删除的 shadow world 符号：曾在 engine 内实现全场感知（MatchWorld 逐 tick
# 双向同步驱动 PerceptionSystem），后整体移除，全场防守拓扑改由
# decision/potential_field.rs 的 DefensePotentialFieldSolver 涌现（ADR-015
# 修订注记）。status.md 自称「只写当前工作区可由代码/测试/守卫复核的结论」，
# 故不得再把这些符号当作现存机制陈述。
REMOVED_SYMBOLS = re.compile(
    r"sync_to_world|sync_world_players|PerceptionSystem|PerceptionSnapshot|"
    r"MatchWorld|match_world\.rs|Systems\.world|crate::world"
)
# 行内含这些标记 = 正在陈述「已移除/已超越」，属合法说明（含 supersede 注记），豁免。
REMOVAL_MARKERS = re.compile(r"移除|删除|已超越|超越|supersede|不再|已随|曾用|曾持有")


def all_markdown(docs_dir: Path) -> list[Path]:
    return sorted({*docs_dir.glob("*.md"), *docs_dir.glob("dev/**/*.md")})


def headings(path: Path) -> set[str]:
    text = path.read_text(encoding="utf-8")
    return {
        match.group(1)
        for match in re.finditer(r"^#{1,6}\s+(\d+(?:\.\d+)*)\b", text, re.M)
    }


def resolve_target(raw: str, files: list[Path]) -> tuple[Path | None, str | None]:
    by_rel = {p.relative_to(ROOT).as_posix(): p for p in files}
    by_docs_rel = {p.relative_to(DOCS).as_posix(): p for p in files}
    by_name: dict[str, list[Path]] = {}
    for path in files:
        by_name.setdefault(path.name, []).append(path)

    if raw.startswith("docs/"):
        path = by_rel.get(raw)
        return (path, None if path else f"未知目标文档 {raw}")
    if raw.startswith("dev/"):
        path = by_docs_rel.get(raw)
        return (path, None if path else f"未知目标文档 docs/{raw}")
    if raw.startswith(("current/", "evidence/", "cycles/")):
        dev_raw = f"dev/{raw}"
        path = by_rel.get(f"docs/{dev_raw}")
        return (path, None if path else f"未知目标文档 docs/{dev_raw}")

    matches = by_name.get(raw, [])
    if len(matches) == 1:
        return matches[0], None
    if not matches:
        return None, f"未知目标文档 {raw}"
    choices = ", ".join(p.relative_to(ROOT).as_posix() for p in matches)
    return None, f"短路径 {raw} 不唯一，请使用完整路径（候选：{choices}）"


def check_refs(files: list[Path]) -> tuple[list[str], int]:
    broken: list[str] = []
    total = 0
    heading_cache = {path: headings(path) for path in files}
    for path in files:
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for match in REF.finditer(line):
                total += 1
                raw = match.group("target")
                section = match.group("section")
                target, error = resolve_target(raw, files)
                if error or target is None:
                    broken.append(f"{path.relative_to(DOCS)}:{line_no}: {error or '无法解析目标文档'}")
                    continue
                sections = heading_cache[target]
                if not any(
                    number == section
                    or number.startswith(section + ".")
                    or section.startswith(number + ".")
                    for number in sections
                ):
                    broken.append(
                        f"{path.relative_to(DOCS)}:{line_no}: "
                        f"{target.relative_to(ROOT)} §{section} 无对应标题"
                    )
    return broken, total


def check_contract_discipline(contract_files: list[Path]) -> list[str]:
    violations: list[str] = []
    for path in contract_files:
        # README.md 是治理规则自身；decisions.md 是 README §0 定义的「决策面」，
        # 以实测数据作为取舍依据是其职责，均不适用规格面纪律。
        if path.name in {"README.md", "decisions.md"}:
            continue
        in_code = False
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("```"):
                in_code = not in_code
                continue
            if in_code:
                continue
            for pattern, description in CONTRACT_FORBIDDEN:
                if pattern.search(line):
                    violations.append(
                        f"{path.name}:{line_no}: 规格面禁止{description} — "
                        f"{line.strip()[:100]}"
                    )
    return violations


def check_code_references(contract_files: list[Path]) -> list[str]:
    """校验**规格面**文档引用的仓库代码/数据路径确实存在。

    历史事故：shadow world 重构删除了 `crates/engine/src/world.rs`、
    `PerceptionSystem`，防守候选评估从 `crates/decision/src/defense.rs` 迁入
    `potential_field.rs`，但规格面仍把已删路径当作单一事实源。旧守卫只校验
    doc↔doc 引用，无法发现。

    范围限于规格面：规格必须时空无关、始终指向现行代码。工作面（`docs/dev/`）
    与决策面 ADR（`decisions.md`）含大量时间点叙述（已完成重构的“before”路径、
    周期闭合记录），引用已改名/删除的路径是其正当历史职责（README §6），
    若强施本守卫会产生误报并迫使篡改历史，故不纳入。
    """
    violations: list[str] = []
    for path in contract_files:
        # README.md 是治理规则自身；decisions.md 是时间点决策记录，均不适用。
        if path.name in {"README.md", "decisions.md"}:
            continue
        seen: set[str] = set()
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            for match in CODE_REF.finditer(line):
                code_path = match.group("path")
                if code_path in seen:
                    continue
                seen.add(code_path)
                if not (ROOT / code_path).exists():
                    violations.append(
                        f"{path.name}:{line_no}: "
                        f"规格面引用了不存在的代码路径 `{code_path}`"
                    )
    return violations


def check_current_face_symbols(dev_files: list[Path]) -> list[str]:
    """status.md（当前快照面）不得把已删除的 shadow world 符号当作现存机制。

    历史事故：shadow world（MatchWorld/PerceptionSystem/sync_to_world）已整体
    移除，但 status.md 的门矩阵与闭环记录仍描述其「逐 tick 双向同步」，并引用
    已不存在的 tests/match_world.rs。status.md 开头自称「只写当前工作区可由
    代码/测试/守卫复核的结论」，此类陈述违反其自身契约，而路径实在性守卫
    （check_code_references）只查 crates/... 路径、且豁免工作面，无法发现。

    仅约束 status.md：plan.md/decisions.md 是设计历史与时间点裁定，允许叙述
    已删符号。带移除/超越标记的行（即在说明其已删，含 supersede 注记）豁免。
    """
    violations: list[str] = []
    for path in dev_files:
        if path.name != "status.md":
            continue
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if REMOVAL_MARKERS.search(line):
                continue
            match = REMOVED_SYMBOLS.search(line)
            if match:
                violations.append(
                    f"{path.name}:{line_no}: 当前快照面把已删除符号 "
                    f"`{match.group(0)}` 当作现存机制陈述（shadow world 已移除，"
                    f"见 ADR-015 修订注记）；如需保留历史请在该行加移除/超越标记"
                )
    return violations


def check_dev_structure(dev_files: list[Path]) -> list[str]:
    violations: list[str] = []
    for path in dev_files:
        if path.name == "README.md":
            continue
        content = path.read_text(encoding="utf-8").strip()
        if len(content) < 50:
            violations.append(
                f"{path.relative_to(DOCS)}: 文档内容过短（{len(content)} 字符），可能是误移"
            )
    return violations


# 编号命名空间：同一编号前缀不得被两个不同层级的文档共享。
#
# 历史事故：`current/plan.md` 曾用 `C1–C6` 命名周期任务，而 `charter.md` §4
# 的宪章红线就是 `C1–C4`；全仓有 49 处 “charter C1” / “宪章 C1” 引用，
# 两者撞名后读者无法判断 `C1` 指哪一条。旧守卫只校验文件路径与章节号，
# 不校验编号命名空间，因此无法发现。
#
# 规则：每个文档“拥有”它用 ###/#### 标题声明的编号前缀。同一前缀若被
# 两个文档同时拥有，则报错。`docs/dev/README.md` §4 是编号归属的权威，
# 新增编号前必须先在该表登记。
# 什么算“声明一个编号”。
#
# 标题里的编号可能有两种角色：
#   - **声明**：`### C1 · 无隐藏硬编码`、`## 3. D7 · 收敛 …`、`### G0 · 证据基线`
#     —— 编号是该节的**主身份**，后面紧跟分隔符（· / ：/ :）。
#   - **引用**：`## 19. 2026-09-11 D0 执行：…`、`### 33.1 D3.1 命中模型…`
#     —— 编号只是标题叙述的一部分（带日期、或前面已有内容），不属于本节身份。
#
# 旧守卫完全不做这个区分，因此无法发现 C1–C6 与 charter 红线 C1–C4 的
# 撞名（全仓 49 处 “charter C1” 引用）。只把“声明”纳入归属统计。
NUMBERED_DECLARATION = re.compile(
    r"^#{2,6}\s+"
    r"(?:[0-9]+(?:\.[0-9]+)*\.?\s+)?"   # 可选章节号（plan.md 风格 `## 3. D7`）
    r"([A-Z]{1,3})[0-9]+(?:\.[0-9]+)*"    # 编号标签
    r"\s*(?:·|：|:)"                        # 必须是声明：后跟分隔符
)

# 这些前缀已被 `docs/dev/README.md` §4 登记为跨文档共享的**同一套阶梯**：
# - `M`：roadmap.md 与 protocol.md 都定义里程碑验收
# - `G`：gap.md 的差距程序编号
# - `L`：quality.md 定义 L1–L3 检测网，gap.md §8 与 protocol.md 引用同一阶梯
SHARED_PREFIXES = {"M", "G", "L", "P", "R", "T", "TA", "F", "OQ", "PA", "E"}

# 周期归档是同一编号空间的历史区段（如 `D0–D6` 在 cycles、`D7–D13` 在当前
# 计划），不参与“当前文档拥有该前缀”的判定；否则归档与当前计划会被误判为冲突。
ARCHIVE_PREFIX = "docs/dev/cycles/"


def check_numbering_namespaces(files: list[Path]) -> list[str]:
    """校验编号前缀未被两个**当前**文档分别拥有（防止 C1 类碰撞）。

    历史事故：`current/plan.md` 曾用 `C1–C6` 命名周期任务，而 charter.md §4
    的宪章红线就是 `C1–C4`；两者都在当前工作面，读者无法判断 `C1` 指哪一条。
    """
    owners: dict[str, set[str]] = {}
    for path in files:
        rel = path.relative_to(ROOT).as_posix()
        if rel.startswith(ARCHIVE_PREFIX):
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            match = NUMBERED_DECLARATION.match(line)
            if not match:
                continue
            prefix = match.group(1)
            owners.setdefault(prefix, set()).add(rel)

    violations: list[str] = []
    for prefix, paths in sorted(owners.items()):
        if prefix in SHARED_PREFIXES or len(paths) < 2:
            continue
        listed = ", ".join(sorted(paths))
        violations.append(
            f"编号前缀 `{prefix}` 被多个文档同时用作条目标题：{listed} — "
            f"同一编号只能有一个归属（见 docs/dev/README.md §4）；"
            f"若确需共享，请在该表登记后加入 SHARED_PREFIXES"
        )
    return violations


def main() -> int:
    docs_dir = DOCS
    if len(sys.argv) == 3 and sys.argv[1] == "--docs":
        docs_dir = Path(sys.argv[2]).resolve()
    contract_files = sorted(docs_dir.glob("*.md"))
    dev_files = sorted(docs_dir.glob("dev/**/*.md"))
    files = sorted({*contract_files, *dev_files})

    print(f"扫描 {len(contract_files)} 份规格文档 + {len(dev_files)} 份工作文档")
    ref_errors, total = check_refs(files)
    errors = (
        ref_errors
        + check_contract_discipline(contract_files)
        + check_code_references(contract_files)
        + check_current_face_symbols(dev_files)
        + check_dev_structure(dev_files)
        + check_numbering_namespaces(files)
    )
    if errors:
        print(f"\n❌ 文档守卫发现 {len(errors)} 处违规（共扫描 {total} 处引用）：")
        for error in errors:
            print(f"  {error}")
        return 1
    print(f"✅ 文档守卫全部通过（扫描 {len(files)} 个文档，{total} 处引用）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
