#!/usr/bin/env python3
"""文档治理守卫：检查引用解析、契约面纪律和工作面结构。

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
    (re.compile(r"\bseed\s*=\s*\d+"), "实测 seed"),
    (re.compile(r"^\s*(?:cargo|python3|pytest|bash|sh|\./)\s", re.M), "可重跑命令"),
]


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
        if path.name == "README.md":
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
                        f"{path.name}:{line_no}: 契约面禁止{description} — "
                        f"{line.strip()[:100]}"
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


def main() -> int:
    docs_dir = DOCS
    if len(sys.argv) == 3 and sys.argv[1] == "--docs":
        docs_dir = Path(sys.argv[2]).resolve()
    contract_files = sorted(docs_dir.glob("*.md"))
    dev_files = sorted(docs_dir.glob("dev/**/*.md"))
    files = sorted({*contract_files, *dev_files})

    print(f"扫描 {len(contract_files)} 份契约文档 + {len(dev_files)} 份工作文档")
    ref_errors, total = check_refs(files)
    errors = ref_errors + check_contract_discipline(contract_files) + check_dev_structure(dev_files)
    if errors:
        print(f"\n❌ 文档守卫发现 {len(errors)} 处违规（共扫描 {total} 处引用）：")
        for error in errors:
            print(f"  {error}")
        return 1
    print(f"✅ 文档守卫全部通过（扫描 {len(files)} 个文档，{total} 处引用）")
    return 0


if __name__ == "__main__":
    sys.exit(main())
