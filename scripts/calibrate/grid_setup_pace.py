#!/usr/bin/env python3
"""定向 2D 网格：setup 消虚假八秒失误，pace 补偿节奏。

探针确认：死球回合首次决策调度晚于八秒时限，持球人停在后场直到
八秒违例。缩小 `possession_setup_seconds` 让首次决策提前，虚假失误
消失；但节奏变快会让 possessions_per_game 等判据越界，因此同时增大
`seconds_per_possession` 补偿。两组参数各 4 路并行评估，评分直接来自
权威门禁的 39 条判定。

用法：

    python3 -u scripts/calibrate/grid_setup_pace.py
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
WORK = ROOT / ".work" / "evidence" / "setup-pace-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

SETUPS = [5.0, 5.5, 6.0, 6.5, 7.0, 7.5, 8.0]
PACES = [18.0, 19.5, 21.0, 22.5, 24.0]


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
    processes = []
    for index, policy in enumerate(candidates):
        path = WORK / f"worker-{index}" / "calibration" / "policy.json"
        path.write_text(json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
        processes.append(
            subprocess.Popen(
                [str(BINARY), "--gate-only", "--calibration", str(WORK / f"worker-{index}" / "calibration")],
                cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
            )
        )
    results = []
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
            continue
        results.append((depth_of(report), report, output))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    combos = [(s, p) for s in SETUPS for p in PACES]
    best = None
    best_policy = None
    best_stdout = None
    print(f"网格 {len(combos)} 组，{WORKERS} 路并行", flush=True)
    for start in range(0, len(combos), WORKERS):
        batch = combos[start : start + WORKERS]
        candidates = []
        for setup, pace in batch:
            trial = copy.deepcopy(base)
            trial["geometry"]["possession_setup_seconds"] = setup
            trial["weights"]["seconds_per_possession"] = pace
            candidates.append(trial)
        results = evaluate(candidates)
        for (setup, pace), trial, (depth, report, stdout) in zip(batch, candidates, results):
            if report is None:
                print(f"setup {setup:4.1f} pace {pace:4.1f} 引擎失败", flush=True)
                continue
            failed = sum(1 for l in report["layers"] if not l["passed"])
            turnover = next(
                (l["relative_error"] for l in report["layers"]
                 if l["criterion"] == "turnover_per_possession"), 0.0
            )
            if best is None or (failed, depth) < best:
                best = (failed, depth)
                best_policy = trial
                best_stdout = stdout
                print(
                    f"setup {setup:4.1f} pace {pace:4.1f} 失败 {failed:2d} "
                    f"深度 {depth:.5f} turnover {turnover:+.4f}",
                    flush=True,
                )
            if failed == 0:
                break
        if best[0] == 0:
            break
    POLICY.write_text(json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    final = evaluate([best_policy] * WORKERS)[0]
    FINAL_LOG.write_text(best_stdout or final[2], encoding="utf-8")
    print(
        f"最终：失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"深度 {final[0]:.5f}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)
    print(
        f"setup {best_policy['geometry']['possession_setup_seconds']} "
        f"pace {best_policy['weights']['seconds_per_possession']}",
        flush=True,
    )


if __name__ == "__main__":
    main()
