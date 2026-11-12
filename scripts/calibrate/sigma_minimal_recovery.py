#!/usr/bin/env python3
"""近通过向量的最小扰动恢复：只调 shot_aim_sigma_scale 与 trap 先验。

`policy-before-band-edges.json` 在旧行为下门禁深度 0.005；它把出手模型
倍率漂到 1.1358，外部基准偏差 0.063（上限 0.05）。外部基准只依赖
sigma × phase 的乘积，因此只降低 sigma 一个键就能把基准拉回带内，其余
阶段倍率与已调好的分布杠杆保持不动。

每个候选同时测：外部基准偏差、39 条门禁越界深度、被包夹护球生产次数。
最优候选写进独立文件。

用法：

    python3 -u scripts/calibrate/sigma_minimal_recovery.py
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
WORK = ROOT / ".work" / "evidence" / "sigma-recovery"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
BASE = ROOT / ".work" / "evidence" / "policy-before-band-edges.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-sigma-recovery-best.json"
REPORT = ROOT / ".work" / "evidence" / "sigma-recovery-report.json"
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

SIGMAS = [1.06, 1.08, 1.09, 1.10, 1.11, 1.12, 1.13, 1.1358]
TRAP_BONUSES = [0.2, 0.3]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        observed = layer["observed"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)


def run(command):
    return subprocess.run(
        command, cwd=ROOT, capture_output=True, text=True,
        preexec_fn=limit_address_space,
    ).stdout


def write_policy(policy):
    (WORK / "calibration" / "policy.json").write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def main():
    prepare()
    base = json.loads(BASE.read_text(encoding="utf-8"))
    current = json.loads(POLICY.read_text(encoding="utf-8"))
    for section in ("weights", "geometry"):
        for key, value in current[section].items():
            base[section].setdefault(key, value)
    base["provenance"] = current["provenance"]

    rows = []
    best = None
    best_policy = None
    for sigma in SIGMAS:
        for bonus in TRAP_BONUSES:
            policy = copy.deepcopy(base)
            policy["weights"]["shot_aim_sigma_scale"] = sigma
            policy["weights"]["trap_protection_bonus"] = bonus
            write_policy(policy)
            deviation = float(run([str(PROBE), "bench", str(WORK / "calibration")]).strip().splitlines()[-1])
            if deviation > 0.05:
                print(f"sigma {sigma:.4f} bonus {bonus:.1f} 基准偏差 {deviation:.4f} FAIL", flush=True)
                continue
            gate_output = run([str(BINARY), "--gate-only", "--calibration", str(WORK / "calibration")])
            report = None
            for line in gate_output.splitlines():
                if line.startswith("GATE "):
                    report = json.loads(line[5:])
                    break
            if report is None:
                continue
            score = score_of(report)
            events, seeds = (int(v) for v in run([str(PROBE), str(WORK / "calibration")]).strip().splitlines()[-1].split())
            row = {
                "sigma": sigma,
                "bonus": bonus,
                "deviation": deviation,
                "depth": score[0],
                "failed": score[1],
                "trap_events": events,
                "trap_seeds": seeds,
                "failures": [layer["criterion"] for layer in report["layers"] if not layer["passed"]],
            }
            rows.append(row)
            print(
                f"sigma {sigma:.4f} bonus {bonus:.1f} 基准 {deviation:.4f} "
                f"{describe(score)} 包夹护球 {events} 次",
                flush=True,
            )
            if events > 0 and (best is None or score < best):
                best, best_policy = score, policy

    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    if best_policy is None:
        print("没有同时满足三项约束的候选", flush=True)
        return
    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"最优：{describe(best)}，候选向量 {BEST_VECTOR}", flush=True)


if __name__ == "__main__":
    main()
