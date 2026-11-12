#!/usr/bin/env python3
"""小步长单变量扫描：只找能让剩余越界判据进带的邻域解。

当前基线只有 `turnover_per_possession` 越界（约 +0.23，带宽 +0.15），
需要把相对误差压下约 0.08。大步长会重排按地址抽样的整条链，因此这里
只对与失误率有直接因果链的参数做小幅邻域扫描，评分来自权威门禁。

用法：

    python3 -u scripts/calibrate/fine_gates.py
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

# 参数 → 相对当前值的倍率候选。1.0 是基线，只在 0.7–1.3 之间微调。
FACTORS = [0.75, 0.82, 0.88, 0.94, 1.06, 1.12, 1.18, 1.25]
LEVERS = [
    "off_ball_hold_rate",
    "reach_in_foul_rate",
    "hand_check_rate",
    "gather_travel_rate",
    "pass_catch_steal",
    "steal_gamble_base",
    "drive_turnover_rate",
    "backcourt_margin_ft",
    "possession_setup_seconds",
    "seconds_per_possession",
    "shoot_putback_bonus",
    "cut_positional_bonus",
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
    original = read_policy()
    base, base_stdout = evaluate()
    best = (sum(1 for l in base["layers"] if not l["passed"]), depth_of(base))
    best_policy = copy.deepcopy(original)
    best_stdout = base_stdout
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.5f}", flush=True)

    try:
        for key in LEVERS:
            value = get_value(original, key)
            for factor in FACTORS:
                if value == 0.0:
                    continue
                trial = copy.deepcopy(original)
                set_value(trial, key, value * factor)
                write_policy(trial)
                report, stdout = evaluate()
                failed = sum(1 for l in report["layers"] if not l["passed"])
                depth = depth_of(report)
                turnover = next(
                    l["relative_error"]
                    for l in report["layers"]
                    if l["criterion"] == "turnover_per_possession"
                )
                score = (failed, depth)
                print(
                    f"{key:28s} x{factor:.2f} → {value * factor:.5f} "
                    f"失败 {failed:2d} 深度 {depth:.5f} turnover {turnover:+.4f}",
                    flush=True,
                )
                if score < best:
                    best = score
                    best_policy = trial
                    best_stdout = stdout
                if failed == 0:
                    break
            if best[0] == 0:
                break
    finally:
        write_policy(best_policy)

    report, stdout = evaluate()
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
    print(
        f"最终：失败 {sum(1 for l in report['layers'] if not l['passed'])} 条，"
        f"通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
