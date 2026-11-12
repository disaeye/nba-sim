#!/usr/bin/env python3
"""pass_catch_base 的细步长一维扫描。

zero_key_grid 在 0.34 -> 0.45 之间跳过了 11 个档位；0.45 一次清掉 FTA 与
抢断，却让另外两条越界。这里从主参数表的快照出发，按 0.01 步长扫
0.34..0.50，报告四项目标判据与总深度。

用法：

    python3 -u scripts/calibrate/fine_pass_catch.py
"""

from __future__ import annotations

import copy
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "fine-pass-catch"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
REPORT = ROOT / ".work" / "evidence" / "fine-pass-catch-report.json"
CRITERIA = [
    "creation_share_cut",
    "creation_share_pull_up",
    "fta_per_game",
    "steals_per_game",
    "and_one_per_game",
    "turnover_per_possession",
]


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
    observed = layer["observed"]
    if detail.startswith("relative error"):
        low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
        error = layer["relative_error"]
        return (low - error) if error < low else (error - high if error > high else 0.0)
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        if observed < low:
            return (low - observed) / max(abs(high - low), 1e-6)
        if observed > high:
            return (observed - high) / max(abs(high - low), 1e-6)
        return 0.0
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return max(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return abs(min(0.0, (observed - bound) / max(abs(bound), 1e-6)))
    return abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)


def gate(calibration):
    output = subprocess.run(
        [str(BINARY), "--gate-only", "--calibration", str(calibration)],
        cwd=ROOT, capture_output=True, text=True,
    ).stdout
    for line in output.splitlines():
        if line.startswith("GATE "):
            return json.loads(line[5:])
    return None


def main():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)
    base = json.loads((calibration / "policy.json").read_text(encoding="utf-8"))

    rows = []
    for step in range(34, 51):
        value = step / 100.0
        policy = copy.deepcopy(base)
        policy["weights"]["pass_catch_base"] = value
        (calibration / "policy.json").write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        report = gate(calibration)
        if report is None:
            continue
        by_name = {layer["criterion"]: layer for layer in report["layers"]}
        errors = {name: by_name[name]["relative_error"] for name in CRITERIA}
        depth = sum(excess(layer) for layer in report["layers"])
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        rows.append(
            {
                "value": value,
                "depth": depth,
                "failed": failed,
                "errors": errors,
                "failures": [
                    layer["criterion"] for layer in report["layers"] if not layer["passed"]
                ],
            }
        )
        print(
            f"pass_catch_base {value:.2f} 深度 {depth:.4f} 失败 {failed:2d} "
            + " ".join(f"{errors[name]:+.3f}" for name in CRITERIA),
            flush=True,
        )

    REPORT.write_text(
        json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    best = min(rows, key=lambda row: (row["failed"], row["depth"]))
    print(
        f"最优 pass_catch_base {best['value']:.2f}：失败 {best['failed']} 条，"
        f"深度 {best['depth']:.6f}",
        flush=True,
    )


if __name__ == "__main__":
    main()
