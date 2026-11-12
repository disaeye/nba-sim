#!/usr/bin/env python3
"""在独立校准目录里测量 trap_protection_bonus 的生产次数与门禁深度。

不写主参数表，只输出表格，用于选择对 39 条判据扰动最小的先验值。

用法：

    python3 -u scripts/calibrate/measure_trap_bonus.py
"""

from __future__ import annotations

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
WORK = ROOT / ".work" / "evidence" / "trap-bonus-measure"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024
BONUSES = [0.2, 0.3, 0.4, 0.5, 0.6, 0.8, 1.0, 1.4, 2.0]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            observed = layer["observed"]
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def main():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)
    base = json.loads(POLICY.read_text(encoding="utf-8"))

    rows = []
    for bonus in BONUSES:
        policy = json.loads(json.dumps(base))
        policy["weights"]["trap_protection_bonus"] = bonus
        (calibration / "policy.json").write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        gate = subprocess.run(
            [str(BINARY), "--gate-only", "--calibration", str(calibration)],
            cwd=ROOT, capture_output=True, text=True,
            preexec_fn=limit_address_space,
        )
        report = None
        for line in gate.stdout.splitlines():
            if line.startswith("GATE "):
                report = json.loads(line[5:])
                break
        production = subprocess.run(
            [str(PROBE), str(calibration)],
            cwd=ROOT, capture_output=True, text=True,
            preexec_fn=limit_address_space,
        )
        events, seeds = (int(v) for v in production.stdout.strip().splitlines()[-1].split())
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        d = depth(report)
        rows.append((bonus, d, failed, events, seeds))
        print(
            f"先验 {bonus:4.1f} 深度 {d:.6f} 失败 {failed:2d} 包夹护球 {events:3d} 次 / {seeds:3d} 场",
            flush=True,
        )
    (WORK / "report.json").write_text(
        json.dumps(
            [
                {"bonus": b, "depth": d, "failed": f, "events": e, "seeds": s}
                for b, d, f, e, s in rows
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )


if __name__ == "__main__":
    main()
