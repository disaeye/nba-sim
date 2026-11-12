#!/usr/bin/env python3
"""测量 policy.json 每个参数对 16 场权威门禁判据的响应方向。

一次只改一个参数，跑 `sim2 --gate`，记录 39 条判据的相对误差变化。
输出的是敏感性矩阵，供按因果链一次性设置参数；这里不写回参数。

用法：

    sensitivity.py > .work/evidence/sensitivity.log
"""

from __future__ import annotations

import copy
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")

# 只登记与当前越界判据有直接因果链的参数。键是 policy 参数名，值是
# 相对步长：`value * (1 ± step)`，距离与时间类用绝对值步长。
LEVERS = [
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "and_one_rate",
    "off_ball_hold_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "gather_travel_rate",
    "steal_gamble_base",
    "steal_skill_base",
    "pass_catch_steal",
    "cut_tendency",
    "cut_positional_bonus",
    "cut_lane_scale",
    "shoot_putback_bonus",
    "putback_window_seconds",
    "transition_window_seconds",
    "transition_take_window_seconds",
    "drive_base",
    "drive_finish_distance_ft",
    "drive_rim_finish_ft",
    "finish_priority",
    "finish_min_distance_ft",
    "seconds_per_possession",
    "shot_aim_sigma_scale",
]

FOCUS = [
    "zone_share_rim",
    "zone_share_near",
    "zone_share_three",
    "creation_share_cut",
    "creation_share_putback",
    "creation_share_transition",
    "creation_share_set",
    "creation_share_drive",
    "turnover_per_possession",
    "fga_per_possession",
    "fta_per_game",
    "points_per_game",
    "oreb_per_game",
    "possessions_per_game",
    "blocks_per_game",
    "steals_per_game",
    "shooting_fouls_per_game",
    "and_one_per_game",
]


def read_policy():
    with open(POLICY, encoding="utf-8") as handle:
        return json.load(handle)


def write_policy(policy):
    with open(POLICY, "w", encoding="utf-8") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def current_value(policy, key):
    for section in ("weights", "geometry"):
        if key in policy.get(section, {}):
            return policy[section][key]
    raise KeyError(f"policy table has no value `{key}`")


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy.get(section, {}):
            policy[section][key] = value
            return
    raise KeyError(f"policy table has no value `{key}`")


def errors():
    output = subprocess.run(
        [BINARY, "42", "--gate"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    for line in output.stdout.splitlines():
        if line.startswith("GATE "):
            report = json.loads(line[len("GATE ") :])
            return {
                layer["criterion"]: layer["relative_error"]
                for layer in report["layers"]
            }
    raise RuntimeError("sim2 produced no GATE line")


def step_for(key, value):
    if key.endswith("_seconds") or key.endswith("_ft"):
        return max(abs(value) * 0.25, 0.25)
    return max(abs(value) * 0.3, 0.01)


def main():
    original = read_policy()
    try:
        base = errors()
        print("BASE", json.dumps({key: round(base[key], 4) for key in FOCUS}), flush=True)

        for key in LEVERS:
            value = current_value(original, key)
            step = step_for(key, value)
            results = {}
            for label, candidate in (("up", value + step), ("down", max(0.0, value - step))):
                trial = copy.deepcopy(original)
                set_value(trial, key, candidate)
                write_policy(trial)
                measured = errors()
                delta = {name: measured[name] - base[name] for name in FOCUS}
                results[label] = {
                    "value": round(candidate, 5),
                    "delta": {name: round(delta[name], 4) for name in FOCUS},
                }
            write_policy(original)
            print(f"LEVER {key} base {value:.5f} step {step:.5f}", flush=True)
            for label in ("up", "down"):
                entry = results[label]
                changed = {
                    name: delta for name, delta in entry["delta"].items() if abs(delta) > 0.008
                }
                print(
                    f"  {label:4s} value {entry['value']:.5f} "
                    f"delta {json.dumps(changed, ensure_ascii=False)}",
                    flush=True,
                )
        print("DONE", flush=True)
    finally:
        write_policy(original)
    return 0


if __name__ == "__main__":
    sys.exit(main())
