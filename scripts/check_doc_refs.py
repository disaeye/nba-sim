#!/usr/bin/env python3
"""文档交叉引用守卫：校验 docs/*.md 中全部 `X.md §N[.M]` 引用可解析。

红线没有守卫只是愿望——文档体系同理（对抗性审查 2026-09-01 修复项 7）。
用法：python3 scripts/check_doc_refs.py [--docs docs]
退出码：0 = 全部可解析；1 = 存在失效引用。
"""
from __future__ import annotations
import re, sys
from pathlib import Path

DOCS = Path(__file__).resolve().parents[1] / "docs"
REF = re.compile(r"\b(design|architecture|quality|attributes|tactics|status|charter|README)\.md\s*§(\d+(?:\.\d+)*)")

def headings(path: Path) -> set[str]:
    nums = set()
    for m in re.finditer(r"^#{1,6}\s+(\d+(?:\.\d+)*)", path.read_text(encoding="utf-8"), re.M):
        nums.add(m.group(1))
    return nums

def main() -> int:
    docs_dir = DOCS
    if len(sys.argv) > 2 and sys.argv[1] == "--docs":
        docs_dir = Path(sys.argv[2])
    files = sorted(docs_dir.glob("*.md"))
    cache = {p.name: headings(p) for p in files}
    broken: list[str] = []
    total_refs = 0
    for p in files:
        for i, line in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
            for m in REF.finditer(line):
                total_refs += 1
                target, num = m.group(1) + ".md", m.group(2)
                nums = cache.get(target)
                if nums is None:
                    broken.append(f"{p.name}:{i}: 未知目标文档 {target}")
                    continue
                # Match either exact section (e.g. 1.2 matches 1.2) or prefix where parent exists (e.g. 2.1 matches 2)
                if not any(n == num or n.startswith(num + ".") or num.startswith(n + ".") for n in nums):
                    broken.append(f"{p.name}:{i}: {target} §{num} 无对应标题（可用：{sorted(nums)}）")
    if broken:
        print(f"❌ 文档交叉引用失效 {len(broken)} 处（共扫描 {total_refs} 处引用）：")
        for b in broken:
            print("  " + b)
        return 1
    print(f"✅ 文档交叉引用全部可解析（扫描 {len(files)} 个文档，共 {total_refs} 处引用）")
    return 0

if __name__ == "__main__":
    sys.exit(main())
