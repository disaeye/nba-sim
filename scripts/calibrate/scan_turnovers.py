#!/usr/bin/env python3
"""对剩余越界判据做单变量扫描。

从当前最优参数出发，每次只改一个参数的一个步长，用权威门禁测两条
判据的响应。输出方向矩阵，供一次性设置参数，不写回参数表。

用法：

    python3 -u scripts/calibrate/scan_turnovers.py
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
LIMIT = 2 * 1024 * 1024 * 1024

FOCUS = ["turnover_per_possession", "zone_share_rim", "steals_per_game", "possessions_per_game"]

LEVERS = {
    "pass_catch_steal": 0.08,
    "gather_travel_rate": 0.012,
    "reach_in_foul_rate": 0.008,
    "off_ball_hold_rate": 0.003,
    "hand_check_rate": 0.003,
    "cut_positional_bonus": 0.005,
    "bad_pass_rate": 0.0003,
    "pass_turnover_rate": 0.0003,
    "drive_turnover_rate": 0.0003,
    "shoot_putback_bonus": 0.12,
    "finish_min_distance_ft": 0.4,
    "seconds_per_possession": 1.2,
    "cut_lane_scale": 0.08,
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


def errors():
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
            return {layer["criterion"]: layer["relative_error"] for layer in report["layers"]}
    raise RuntimeError("sim2 produced no GATE line")


def main():
    original = read_policy()
    try:
        base = errors()
        print("BASE", json.dumps({k: round(base[k], 4) for k in FOCUS}), flush=True)
        for key, step in LEVERS.items():
            value = get_value(original, key)
            for label, candidate in (("up", value + step), ("down", max(0.0, value - step))):
                trial = copy.deepcopy(original)
                set_value(trial, key, candidate)
                write_policy(trial)
                measured = errors()
                delta = {name: round(measured[name] - base[name], 4) for name in FOCUS}
                print(
                    f"  {key:32s} {label:4s} {value:.5f} → {candidate:.5f} "
                    f"{json.dumps(delta)}",
                    flush=True,
                )
    finally:
        write_policy(original)


if __name__ == "__main__":
    main()
