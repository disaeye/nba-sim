#!/usr/bin/env python3
"""运行时表与结果机制守卫。

检查三件事：
1. 生产模块不读取验证表字段；
2. 生产入口不经 `Params` 全量加载；
3. 动作解析不再直接抽取突破、掩护、篮板、罚球的结果。
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
SIM = ROOT / "crates" / "sim" / "src"
PRODUCTION_DIRS = [
    SIM,
    ROOT / "crates" / "cli" / "src",
    ROOT / "crates" / "bball-wasm" / "src",
    ROOT / "crates" / "debug-server" / "src",
]
VALIDATION_FILES = {
    SIM / "gate.rs",
    SIM / "params.rs",
    SIM / "bin" / "sim2.rs",
    ROOT / "crates" / "cli" / "src" / "commands" / "validate_core.rs",
}
VALIDATION_TABLES = ("shot_make", "shot_profile", "possession", "rebound", "freethrow")
RESULT_DRAWS = (
    "purpose::DRIVE_TURNOVER",
    "purpose::STEAL_CREDIT",
    "purpose::FINISH_DISTANCE",
    "purpose::REBOUND_TEAM",
    "purpose::REBOUND_PICK",
    "purpose::REBOUND_LANDING",
    "purpose::SCREEN_ROUTE",
    "purpose::FREE_THROW_MAKE",
)
PRODUCTION_ENTRY_EXEMPT = {
    ROOT / "crates" / "cli" / "src" / "commands" / "validate_core.rs",
    ROOT / "crates" / "cli" / "src" / "commands" / "sim_core.rs",
    ROOT / "crates" / "sim" / "src" / "bin" / "sim2.rs",
}


def files() -> list[pathlib.Path]:
    found: list[pathlib.Path] = []
    for directory in PRODUCTION_DIRS:
        found.extend(path for path in directory.rglob("*.rs") if path.is_file())
    return found


def main() -> int:
    violations: list[str] = []
    for path in files():
        text = path.read_text(encoding="utf-8")
        relative = path.relative_to(ROOT)
        if path not in VALIDATION_FILES:
            for table in VALIDATION_TABLES:
                if re.search(rf"\bparams\.{table}\b", text):
                    violations.append(f"{relative}: 读取验证表 params.{table}")
            for draw in RESULT_DRAWS:
                if draw in text:
                    violations.append(f"{relative}: 直接抽取结果 {draw}")
        if re.search(r"(?<!Runtime)Params::(load|embedded)", text):
            if path not in PRODUCTION_ENTRY_EXEMPT and "cfg(test)" not in text:
                violations.append(f"{relative}: 生产路径使用 Params 全量加载")
    manifest = (SIM / "params.rs").read_text(encoding="utf-8")
    runtime_block = manifest.split("pub const RUNTIME_TABLES", 1)
    if len(runtime_block) != 2:
        violations.append("params.rs: 找不到 RUNTIME_TABLES")
    else:
        block = runtime_block[1].split("pub const VALIDATION_TABLES", 1)[0]
        for table in VALIDATION_TABLES:
            if re.search(rf'name:\s*"{table}"', block):
                violations.append(f"params.rs: 结果表 {table} 出现在运行时可加载清单")
    if violations:
        for item in violations:
            print(item)
        return 1
    print("runtime table guard: 0 violations")
    return 0


if __name__ == "__main__":
    sys.exit(main())
