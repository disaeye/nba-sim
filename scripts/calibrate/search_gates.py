#!/usr/bin/env python3
"""固定种子的门禁参数随机搜索。

按地址抽样（`DrawSource` 用 possession/tick 做散列）让参数一变就重排后续
全部抽样地址，因此判据对参数不是连续可微的：坐标下降与局部线性模型都会
追噪声。这里改成可复现的随机搜索：以当前最优参数为起点，扰动一小簇参数，
用门禁自己的 16 场判定打分，保留失败条数最少的结果。

每个 `sim2` 子进程受 2 GiB 地址空间上限约束；随机数种子固定，搜索可复现。

用法：

    python3 -u scripts/calibrate/search_gates.py --evals 240 \\
        > .work/evidence/search-gates.log
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import resource
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 每个参数的物理合理区间。随机扰动钳制在这里，避免搜索把接触底频推到
# 明显不合理的量级。区间只表达量纲边界，不表达拟合目标。
BOUNDS = {
    "shooting_foul_rate_two_point": (0.08, 0.32),
    "shooting_foul_rate_three_point": (0.01, 0.10),
    "and_one_rate": (0.08, 0.30),
    "cut_lane_scale": (0.30, 0.75),
    "cut_tendency": (0.0, 0.09),
    "cut_positional_bonus": (0.0, 0.05),
    "shoot_putback_bonus": (0.4, 1.1),
    "transition_window_seconds": (7.0, 12.0),
    "shot_aim_sigma_scale": (0.85, 1.45),
    "shot_distance_decay_three_scale_ft": (8.0, 13.0),
    "finish_min_distance_ft": (2.0, 4.2),
    "finish_priority": (0.25, 0.60),
    "drive_base": (1.2, 2.1),
    "seconds_per_possession": (14.0, 20.0),
    "play_call_rate": (0.60, 0.86),
    "gather_travel_rate": (0.01, 0.055),
    "pass_catch_steal": (0.30, 0.70),
    "reach_in_foul_rate": (0.008, 0.035),
    "off_ball_hold_rate": (0.002, 0.010),
    "hand_check_rate": (0.001, 0.008),
    "steal_gamble_base": (0.5, 1.0),
    "drive_finish_distance_ft": (4.5, 7.5),
}

KEYS = sorted(BOUNDS)


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


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
    raise KeyError(f"policy table has no value `{key}`")


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(f"policy table has no value `{key}`")


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
            report = json.loads(line[len("GATE ") :])
            failed = [layer for layer in report["layers"] if not layer["passed"]]
            return len(failed), report, output.stdout
    raise RuntimeError("sim2 produced no GATE line\n" + output.stdout[-2000:])


def score(report):
    """越界深度：按每条未通过判据超出带宽的部分累计，用于同失败数时比较。"""
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
    return total


def perturb(policy, rng, width):
    """从给定参数出发，随机扰动一小簇参数。"""
    trial = copy.deepcopy(policy)
    count = rng.choice((1, 1, 2, 2, 3))
    for key in rng.sample(KEYS, count):
        low, high = BOUNDS[key]
        current = get_value(trial, key)
        span = (high - low) * width
        candidate = current + rng.uniform(-span, span)
        set_value(trial, key, min(max(candidate, low), high))
    return trial


def main():
    parser = argparse.ArgumentParser(description="固定种子的门禁参数随机搜索")
    parser.add_argument("--evals", type=int, default=240, help="评估次数")
    parser.add_argument("--seed", type=int, default=20261110, help="随机数种子")
    parser.add_argument("--width", type=float, default=0.18, help="扰动幅度（占区间比例）")
    args = parser.parse_args()

    rng = random.Random(args.seed)
    base = read_policy()
    failed, report, _ = evaluate(base)
    best_failed, best_depth = failed, score(report)
    best_policy = copy.deepcopy(base)
    print(f"起始：失败 {failed} 条，越界深度 {best_depth:.5f}", flush=True)

    for iteration in range(1, args.evals + 1):
        trial = perturb(best_policy, rng, args.width)
        failed, report, _ = evaluate(trial)
        depth = score(report)
        better = (failed, depth) < (best_failed, best_depth)
        if better:
            best_failed, best_depth = failed, depth
            best_policy = trial
            bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
            print(
                f"[{iteration:4d}] 失败 {failed} 条，越界深度 {depth:.5f}  {bad}",
                flush=True,
            )
        if failed == 0:
            print(f"[{iteration:4d}] 39 条判据全部通过", flush=True)
            break

    write_policy(best_policy)
    failed, report, stdout = evaluate(best_policy)
    with open(os.path.join(ROOT, ".work", "evidence", "gate-final.log"), "w", encoding="utf-8") as handle:
        handle.write(stdout)
    print(
        f"最终：失败 {failed} 条，几何 {report['geometric_violations']}，"
        f"语境 {report['context_violations']}，通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
