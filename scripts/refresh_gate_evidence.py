#!/usr/bin/env python3
"""从当前权威门禁重新生成 `gate.txt` 与 `verification.json` 的门禁段。

判定入口是 `nba-sim sim-core --gate`（内部固定 1..=16 场，与
`sim2 --gate-only` 同一批种子、同一份判定代码）。几何违规、语境违规和
39 条判据的观测值都直接来自这次运行，不复制历史数字。

用法：

    python3 scripts/refresh_gate_evidence.py
"""

from __future__ import annotations

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLI = ROOT / "target" / "release" / "nba-sim"
GATE_TXT = ROOT / "docs" / "dev" / "evidence" / "gap_closure_cycle" / "gate.txt"
VERIFICATION = ROOT / "docs" / "dev" / "evidence" / "gap_closure_cycle" / "verification.json"


def run_gate():
    result = subprocess.run(
        [str(CLI), "sim-core", "--seed", "1", "--gate"],
        cwd=ROOT,
        capture_output=True,
        text=True,
    )
    if result.returncode != 0 and not any(
        line.startswith("几何违规") for line in result.stdout.splitlines()
    ):
        raise SystemExit(
            f"sim-core --gate failed (exit {result.returncode}):\n"
            f"{result.stdout[-2000:]}\n{result.stderr[-2000:]}"
        )
    return result.stdout


def main() -> int:
    output = run_gate()
    lines: list[str] = []
    geometric = context = distribution_failures = None
    observed: dict[str, float] = {}
    targets: dict[str, float] = {}
    out_of_band: dict[str, float] = {}
    for line in output.splitlines():
        if line.startswith("几何违规"):
            parts = line.split()
            geometric = int(parts[1])
            context = int(parts[3])
            distribution_failures = int(parts[5])
            continue
        stripped = line.strip()
        if not (stripped.startswith("OK ") or stripped.startswith("BAD ")):
            continue
        status, rest = stripped.split(None, 1)
        fields = rest.split()
        criterion = fields[0]
        value = float(fields[2])
        target = float(fields[4])
        observed[criterion] = value
        targets[criterion] = target
        if status == "BAD":
            out_of_band[criterion] = value
        lines.append(
            f"  {status:3s} {criterion:<40s} observed {value:.3f}  target {target:.3f}"
        )
    if geometric is None or not lines:
        raise SystemExit("sim-core --gate 没有给出四层判定输出")
    if len(lines) != 39:
        raise SystemExit(f"门禁判据数量 {len(lines)}，方案要求 39 条")

    GATE_TXT.write_text("\n".join(lines) + "\n", encoding="utf-8")
    payload = json.loads(VERIFICATION.read_text(encoding="utf-8"))
    payload["verification"]["gate"] = {
        "command": "nba-sim sim-core --seed 1 --gate",
        "geometric_violations": geometric,
        "context_violations": context,
        "criteria_total": len(lines),
        "criteria_out_of_band": len(out_of_band),
        "out_of_band": out_of_band,
        "artifact": "docs/dev/evidence/gap_closure_cycle/gate.txt",
    }
    VERIFICATION.write_text(
        json.dumps(payload, ensure_ascii=False, indent=2) + "\n", encoding="utf-8"
    )
    print(
        f"门禁证据刷新：{len(lines)} 条判据，几何违规 {geometric}，"
        f"语境违规 {context}，越界 {len(out_of_band)} 条"
    )
    return 0 if distribution_failures == 0 else 1


if __name__ == "__main__":
    sys.exit(main())
