#!/usr/bin/env python3
"""收尾扫描：把剩余越界判据收敛到 0 条。

从当前最优参数出发，每次只扰动一小簇参数，用权威门禁自己的 16 场
判定打分。越界深度按每条判据自己的带宽计算，过程层的区间也计入。

用法：

    python3 -u scripts/calibrate/finish_gates.py --evals 60
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

# (下界, 上界, 步长)。方向由随机搜索自己选。
LEVERS = {
    "pass_catch_steal": (0.30, 0.70, 0.04),
    "cut_lane_scale": (0.35, 0.65, 0.04),
    "off_ball_hold_rate": (0.001, 0.010, 0.0015),
    "hand_check_rate": (0.001, 0.008, 0.0015),
    "reach_in_foul_rate": (0.008, 0.030, 0.004),
    "drive_turnover_rate": (0.0001, 0.0008, 0.0002),
    "finish_min_distance_ft": (2.4, 3.6, 0.2),
    "seconds_per_possession": (15.0, 20.0, 0.6),
    "gather_travel_rate": (0.010, 0.055, 0.006),
    "cut_positional_bonus": (0.0, 0.02, 0.003),
    "shoot_putback_bonus": (0.6, 1.05, 0.06),
    "shot_aim_sigma_scale": (0.95, 1.35, 0.05),
}


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
            total += abs(layer["observed"] - layer["target"]) / max(
                abs(layer["target"]), 1e-6
            )
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
    parser = argparse.ArgumentParser(description="收尾门禁参数扫描")
    parser.add_argument("--evals", type=int, default=60)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    rng = random.Random(args.seed)
    base = read_policy()
    failed, depth, _, _ = evaluate(base)
    best = (failed, depth)
    best_policy = copy.deepcopy(base)
    print(f"起始：失败 {failed} 条，越界深度 {depth:.5f}", flush=True)

    for iteration in range(1, args.evals + 1):
        trial = copy.deepcopy(best_policy)
        for key in rng.sample(sorted(LEVERS), rng.choice((1, 1, 2, 2))):
            low, high, step = LEVERS[key]
            current = get_value(trial, key)
            candidate = current + rng.choice((-step, step))
            set_value(trial, key, min(max(candidate, low), high))
        failed, depth, report, _ = evaluate(trial)
        if (failed, depth) < best:
            best = (failed, depth)
            best_policy = trial
            bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
            print(f"[{iteration:3d}] 失败 {failed} 条，深度 {depth:.5f} {bad}", flush=True)
        if failed == 0:
            break

    write_policy(best_policy)
    failed, depth, report, stdout = evaluate(best_policy)
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(stdout)
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
