#!/usr/bin/env python3
"""二维网格：节奏 × 抢断，收敛最后一条判据及其耦合项。

单变量扫描显示 `seconds_per_possession` 上调能把 turnover 拉进带，代价是
`steals_per_game` 出带；`pass_catch_steal` 与 `steal_gamble_base` 是抢断的
直接杠杆。这里对几组杠杆做二维网格，评分始终来自权威门禁的 39 条判定。

每个 worker 持有独立校准目录，4 路并行。全局最优写回参数表。

用法：

    python3 -u scripts/calibrate/pair_grid.py
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
WORK = ROOT / ".work" / "evidence" / "pair-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

PACE = [18.0, 18.4, 18.8, 19.2, 19.6, 20.0]
STEAL = [0.44, 0.48, 0.52, 0.56, 0.60, 0.64, 0.68]
GAMBLE = [0.70, 0.80, 0.90, 1.00, 1.10]

PAIRS = [
    ("seconds_per_possession", PACE, "pass_catch_steal", STEAL),
    ("seconds_per_possession", PACE, "steal_gamble_base", GAMBLE),
    ("seconds_per_possession", PACE, "off_ball_hold_rate", [0.004, 0.005, 0.006, 0.007, 0.008]),
]


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


def get_value(policy, key):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            return policy[section][key]
    raise KeyError(key)


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    for key_a, values_a, key_b, values_b in PAIRS:
        combos = [(a, b) for a in values_a for b in values_b]
        candidates = []
        for a, b in combos:
            trial = copy.deepcopy(base)
            set_value(trial, key_a, a)
            set_value(trial, key_b, b)
            candidates.append(trial)
        results = evaluate(candidates)
        for (a, b), trial, (depth, report, stdout) in zip(combos, candidates, results):
            if report is None:
                continue
            failed = sum(1 for l in report["layers"] if not l["passed"])
            score = (failed, depth)
            if score < best:
                best = score
                best_policy = trial
                best_stdout = stdout
                turnover = next(
                    (l["relative_error"] for l in report["layers"]
                     if l["criterion"] == "turnover_per_possession"), 0.0
                )
                steals = next(
                    (l["relative_error"] for l in report["layers"]
                     if l["criterion"] == "steals_per_game"), 0.0
                )
                print(
                    f"{key_a}={a} {key_b}={b} 失败 {failed} 深度 {depth:.5f} "
                    f"turnover {turnover:+.4f} steals {steals:+.4f}",
                    flush=True,
                )
        print(f"组合 {key_a} × {key_b} 完成，全局最优 {best}", flush=True)
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
