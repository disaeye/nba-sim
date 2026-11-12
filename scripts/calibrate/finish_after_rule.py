#!/usr/bin/env python3
"""以「八秒过半提前决策」后的行为为起点，收敛剩余越界判据。

评分来自权威门禁自己的 39 条判定。扰动范围只覆盖与当前越界判据
有直接因果链的参数，且钳制在物理合理区间内。

用法：

    python3 -u scripts/calibrate/finish_after_rule.py --evals 160
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import resource
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")
FINAL_LOG = os.path.join(ROOT, ".work", "evidence", "gate-final.log")
LIMIT = 2 * 1024 * 1024 * 1024

BOUNDS = {
    "cut_lane_scale": (0.30, 0.65, 0.025),
    "cut_tendency": (0.0, 0.08, 0.008),
    "cut_positional_bonus": (0.0, 0.02, 0.002),
    "shoot_putback_bonus": (0.55, 1.05, 0.045),
    "finish_min_distance_ft": (2.3, 3.8, 0.12),
    "finish_priority": (0.25, 0.60, 0.03),
    "drive_base": (1.35, 2.0, 0.06),
    "post_base": (0.40, 0.75, 0.03),
    "gather_travel_rate": (0.005, 0.055, 0.004),
    "shot_aim_sigma_scale": (0.95, 1.40, 0.035),
    "seconds_per_possession": (16.0, 20.0, 0.35),
    "pass_catch_steal": (0.35, 0.65, 0.025),
    "off_ball_hold_rate": (0.002, 0.009, 0.0008),
    "play_call_rate": (0.70, 0.86, 0.015),
}
KEYS = sorted(BOUNDS)


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (LIMIT, LIMIT))


def read_policy():
    with open(POLICY, encoding="utf-8") as handle:
        return json.load(handle)


def write_policy(policy):
    with open(POLICY, "w", encoding="utf-8") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


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


def evaluate(policy):
    write_policy(policy)
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
            report = json.loads(line[5:])
            failed = sum(1 for layer in report["layers"] if not layer["passed"])
            return failed, depth_of(report), report, output.stdout
    raise RuntimeError("sim2 produced no GATE line")


def main():
    parser = argparse.ArgumentParser(description="规则修复后的门禁收敛")
    parser.add_argument("--evals", type=int, default=160)
    parser.add_argument("--seed", type=int, default=31337)
    args = parser.parse_args()

    rng = random.Random(args.seed)
    base = read_policy()
    failed, depth, _, _ = evaluate(base)
    best = (failed, depth)
    best_policy = copy.deepcopy(base)
    best_stdout = None
    print(f"起始：失败 {failed} 条，深度 {depth:.5f}", flush=True)

    for iteration in range(1, args.evals + 1):
        trial = copy.deepcopy(best_policy)
        for key in rng.sample(KEYS, rng.choice((1, 1, 2, 2, 3))):
            low, high, step = BOUNDS[key]
            current = get_value(trial, key)
            candidate = current + rng.choice((-step, step))
            set_value(trial, key, min(max(candidate, low), high))
        failed, depth, report, stdout = evaluate(trial)
        if (failed, depth) < best:
            best = (failed, depth)
            best_policy = trial
            best_stdout = stdout
            bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
            print(f"[{iteration:3d}] 失败 {failed} 深度 {depth:.5f} {bad}", flush=True)
        if failed == 0:
            break

    write_policy(best_policy)
    failed, depth, report, stdout = evaluate(best_policy)
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
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
