#!/usr/bin/env python3
"""针对剩余越界判据的定向组合验证。

`scan_turnovers.py` 给出单变量响应。这里把能同时降低失误率、保住
抢断数与篮下占比的候选组合逐个交给权威门禁，不用局部线性外推。

用法：

    python3 -u scripts/calibrate/combine_gates.py
"""

from __future__ import annotations

import copy
import json
import os
import resource
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")
FINAL_LOG = os.path.join(ROOT, ".work", "evidence", "gate-final.log")
LIMIT = 2 * 1024 * 1024 * 1024

# 每组是 {参数: 取值}。基线来自 `scan_turnovers.py` 的单变量响应方向。
COMBINATIONS = [
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470, "hand_check_rate": 0.01100},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470, "cut_positional_bonus": 0.00623},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470, "drive_turnover_rate": 0.00020},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470, "shoot_putback_bonus": 1.02604},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.53470, "cut_lane_scale": 0.48513},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.61470},
    {"off_ball_hold_rate": 0.00100, "pass_catch_steal": 0.53470},
    {"off_ball_hold_rate": 0.00100, "pass_catch_steal": 0.61470},
    {"off_ball_hold_rate": 0.00100, "pass_catch_steal": 0.61470, "hand_check_rate": 0.01100},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.37470},
    {"off_ball_hold_rate": 0.00243, "cut_lane_scale": 0.48513},
    {"off_ball_hold_rate": 0.00243, "drive_turnover_rate": 0.00020, "cut_lane_scale": 0.48513},
    {"off_ball_hold_rate": 0.00100, "drive_turnover_rate": 0.00020, "cut_lane_scale": 0.48513},
    {"pass_catch_steal": 0.53470, "cut_lane_scale": 0.48513},
    {"pass_catch_steal": 0.61470, "cut_lane_scale": 0.48513},
    {"pass_catch_steal": 0.61470, "cut_lane_scale": 0.56513},
    {"off_ball_hold_rate": 0.00243, "pass_catch_steal": 0.61470, "cut_positional_bonus": 0.00623},
    {"off_ball_hold_rate": 0.00100, "pass_catch_steal": 0.61470, "cut_positional_bonus": 0.00623},
    {"off_ball_hold_rate": 0.00100, "pass_catch_steal": 0.61470, "shoot_putback_bonus": 1.02604},
]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (LIMIT, LIMIT))


def read_policy():
    with open(POLICY, encoding="utf-8") as handle:
        return json.load(handle)


def write_policy(policy):
    with open(POLICY, "w", encoding="utf-8") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def evaluate():
    output = subprocess.run(
        [BINARY, "--gate-only"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
        preexec_fn=limit_address_space,
    )
    for line in output.stdout.splitlines():
        if line.startswith("GATE "):
            return json.loads(line[5:]), output.stdout
    raise RuntimeError("sim2 produced no GATE line")


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            band = detail.split("band ")[1].rstrip(")")
            low, high = (float(value) for value in band.split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            bounds = detail.removeprefix("区间 ").split("..")
            low, high = (float(value) for value in bounds)
            observed = layer["observed"]
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def main():
    original = read_policy()
    base_report, _ = evaluate()
    best = (sum(1 for layer in base_report["layers"] if not layer["passed"]), depth_of(base_report))
    best_policy = copy.deepcopy(original)
    best_stdout = None
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.5f}", flush=True)

    try:
        for index, combination in enumerate(COMBINATIONS, 1):
            trial = copy.deepcopy(original)
            for key, value in combination.items():
                set_value(trial, key, value)
            write_policy(trial)
            report, stdout = evaluate()
            failed = sum(1 for layer in report["layers"] if not layer["passed"])
            depth = depth_of(report)
            bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
            print(f"[{index:2d}] 失败 {failed} 条，深度 {depth:.5f} {bad} {combination}", flush=True)
            if (failed, depth) < best:
                best = (failed, depth)
                best_policy = trial
                best_stdout = stdout
            if failed == 0:
                break
    finally:
        write_policy(best_policy)

    report, stdout = evaluate()
    if best_stdout is None:
        best_stdout = stdout
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    print(
        f"最终：失败 {failed} 条，几何 {report['geometric_violations']}，"
        f"语境 {report['context_violations']}，通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
