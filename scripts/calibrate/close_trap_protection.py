#!/usr/bin/env python3
"""按最小战术先验收敛被包夹护球的生产链与 39 条门禁。

每个候选值同时测两件事：120 场短比赛里 `trap_protect` 是否由生产路径产
出，以及权威门禁的越界深度。候选按先验值从小到大检查，找到第一个同时
满足生产证据与门禁全通过的值即写回主参数表。

用法：

    python3 -u scripts/calibrate/close_trap_protection.py
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
WORK = ROOT / ".work" / "evidence" / "trap-calibration"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
REPORT = ROOT / ".work" / "evidence" / "trap-calibration-report.json"
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 先验值按从小到大检查：能产出被包夹护球且不破坏门禁的最小值优先。
BONUSES = [round(0.4 + index * 0.1, 1) for index in range(25)]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth_of(report):
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


def score_of(report):
    return (
        depth_of(report),
        sum(1 for layer in report["layers"] if not layer["passed"]),
    )


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)


def run(command):
    process = subprocess.run(
        command,
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        preexec_fn=limit_address_space,
    )
    return process.stdout


def evaluate(bonus):
    calibration = WORK / "calibration"
    policy = json.loads(POLICY.read_text(encoding="utf-8"))
    policy["weights"]["trap_protection_bonus"] = bonus
    (calibration / "policy.json").write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    gate_output = run([str(BINARY), "--gate-only", "--calibration", str(calibration)])
    gate = None
    for line in gate_output.splitlines():
        if line.startswith("GATE "):
            gate = json.loads(line[5:])
            break
    production = run([str(PROBE), str(calibration)]).strip().splitlines()
    events, seeds = (int(value) for value in production[-1].split())
    return {
        "bonus": bonus,
        "depth": None if gate is None else depth_of(gate),
        "failed": None if gate is None else sum(
            1 for layer in gate["layers"] if not layer["passed"]
        ),
        "score": None if gate is None else score_of(gate),
        "trap_events": events,
        "trap_seeds": seeds,
        "gate": gate,
        "gate_output": gate_output,
    }


def main():
    prepare()
    results = []
    best = None
    for bonus in BONUSES:
        row = evaluate(bonus)
        results.append(
            {
                key: value
                for key, value in row.items()
                if key not in ("gate", "gate_output")
            }
        )
        print(
            f"先验 {bonus:.1f} 深度 {row['depth']} 失败 {row['failed']} "
            f"包夹护球 {row['trap_events']} 次 / {row['trap_seeds']} 场",
            flush=True,
        )
        REPORT.write_text(
            json.dumps(results, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        if row["trap_events"] > 0 and row["depth"] == 0.0:
            best = row
            break
        if row["trap_events"] > 0 and best is None:
            best = row

    if best is None:
        raise SystemExit("没有找到能产出被包夹护球的战术先验")
    policy = json.loads(POLICY.read_text(encoding="utf-8"))
    policy["weights"]["trap_protection_bonus"] = best["bonus"]
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(best["gate_output"], encoding="utf-8")
    print(
        f"保留先验 {best['bonus']:.1f}：深度 {best['depth']} 失败 {best['failed']} "
        f"包夹护球 {best['trap_events']} 次 / {best['trap_seeds']} 场",
        flush=True,
    )


if __name__ == "__main__":
    main()
