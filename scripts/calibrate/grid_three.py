#!/usr/bin/env python3
"""三杠杆定向网格：解当前三条越界判据。

敏感性矩阵给出：
- transition_window_seconds 上调 → transition 份额 +0.202、fta −0.072；
- shooting_foul_rate_two_point 下调 → fta −0.245、transition +0.082；
- pass_catch_steal 上调 → steals +0.116、fta +0.082。

当前误差：transition −0.223、fta +0.268、steals −0.209。
这组组合应能把三条同时拉进 ±0.15 带内。评分来自权威门禁。

用法：

    python3 -u scripts/calibrate/grid_three.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "grid-three"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

GRID = {
    "transition_window_seconds": [9.959, 10.2, 10.5, 10.8],
    "shooting_foul_rate_two_point": [0.185, 0.175, 0.165, 0.155],
    "pass_catch_steal": [0.455, 0.50, 0.55, 0.60],
}


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


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    for index in range(WORKERS):
        calibration = WORK / f"worker-{index}" / "calibration"
        calibration.mkdir(parents=True)
        for path in CALIBRATION.glob("*.json"):
            shutil.copy2(path, calibration / path.name)
        (WORK / f"worker-{index}" / "tactics").symlink_to(
            TACTICS, target_is_directory=True
        )


def evaluate(candidates):
    results = []
    for start in range(0, len(candidates), WORKERS):
        batch = candidates[start : start + WORKERS]
        processes = []
        for index, policy in enumerate(batch):
            path = WORK / f"worker-{index}" / "calibration" / "policy.json"
            path.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            processes.append(
                subprocess.Popen(
                    [str(BINARY), "--gate-only", "--calibration",
                     str(WORK / f"worker-{index}" / "calibration")],
                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                )
            )
        for process in processes:
            output = process.stdout.read().decode("utf-8", errors="replace")
            process.wait()
            report = None
            for line in output.splitlines():
                if line.startswith("GATE "):
                    report = json.loads(line[5:])
                    break
            if report is None:
                results.append((1e6, None, output))
            else:
                results.append((depth_of(report), report, output))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    keys = list(GRID)
    combos = list(itertools.product(*(GRID[k] for k in keys)))
    candidates = []
    for values in combos:
        trial = copy.deepcopy(base)
        for key, value in zip(keys, values):
            if key in trial["weights"]:
                trial["weights"][key] = value
            else:
                trial["geometry"][key] = value
        candidates.append(trial)
    print(f"网格 {len(candidates)} 组", flush=True)

    results = evaluate(candidates)
    for values, trial, (depth, report, stdout) in zip(combos, candidates, results):
        if report is None:
            continue
        failed = [l for l in report["layers"] if not l["passed"]]
        score = (len(failed), depth)
        if score < best:
            best = score
            best_policy = trial
            best_stdout = stdout
            print(
                f"{dict(zip(keys, values))} 失败 {len(failed)} 深度 {depth:.5f} "
                f"{[l['criterion'] for l in failed]}",
                flush=True,
            )
        if best[0] == 0:
            break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best_policy])[0]
    FINAL_LOG.write_text(best_stdout if best[0] == 0 else final[2], encoding="utf-8")
    print(
        f"最终：失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"深度 {final[0]:.6f}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
