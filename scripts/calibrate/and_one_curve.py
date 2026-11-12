#!/usr/bin/env python3
"""从 2 条越界向量打印 and_one_rate 的逐判据响应曲线。

用法：

    python3 -u scripts/calibrate/and_one_curve.py
"""

from __future__ import annotations

import copy
import json
import os
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "and-one-curve"
BINARY = ROOT / "target" / "release" / "sim2"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "and-one-curve-report.json"

CRITERIA = (
    "creation_share_cut",
    "and_one_per_game",
    "fta_per_game",
    "shooting_fouls_per_game",
    "steals_per_game",
    "turnover_per_possession",
)
VALUES = [0.193665 + i * 0.02 for i in range(18)]


def limit_address_space():
    limit = 2 * 1024 * 1024 * 1024
    resource.setrlimit(resource.RLIMIT_AS, (limit, limit))


def main():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)
    base = json.loads(BASE.read_text(encoding="utf-8"))

    rows = []
    for value in VALUES:
        policy = copy.deepcopy(base)
        policy["weights"]["and_one_rate"] = value
        (calibration / "policy.json").write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        output = subprocess.run(
            [str(BINARY), "--gate-only", "--calibration", str(calibration)],
            cwd=ROOT, capture_output=True, text=True,
            preexec_fn=limit_address_space,
        ).stdout
        report = None
        for line in output.splitlines():
            if line.startswith("GATE "):
                report = json.loads(line[5:])
                break
        if report is None:
            continue
        by_name = {layer["criterion"]: layer for layer in report["layers"]}
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        errors = {name: by_name[name]["relative_error"] for name in CRITERIA}
        rows.append({"value": value, "failed": failed, "errors": errors})
        print(
            f"and_one_rate {value:.4f} 失败 {failed:2d} "
            + " ".join(f"{name[:12]}={errors[name]:+.3f}" for name in CRITERIA),
            flush=True,
        )
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


if __name__ == "__main__":
    main()
