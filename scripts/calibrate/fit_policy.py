#!/usr/bin/env python3
"""按职责分组的 policy 参数拟合。

每个参数组有独立的键集合、独立的取值范围和独立的拟合入口。校准器不再
用一个统一区间覆盖全部权重，因此：

- 瞄准误差、距离衰减、防守干扰、动作阶段、决策温度、行为偏好各自调整，
  任何一组的调整不会串写到其它组；
- 时间阈值（秒）、速度尺度（英尺每秒）与距离尺度（英尺）各有自己的物理
  量纲与范围，不存在同一个数值被同时写进三种量纲的可能；
- 接近零的物理概率用组内的下界表达，不再用全局下界顶住。

用法：

    fit_policy.py --group shot_quality      # 只拟合瞄准误差组
    fit_policy.py --group all                # 按顺序拟合全部组
    fit_policy.py --group all --dry-run      # 只打印参数分组，不跑模拟
"""

from __future__ import annotations

import argparse
import json
import math
import os
import subprocess
import sys

import freeze

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
PROFILE = os.path.join(ROOT, "data", "calibration", "shot_profile.json")
POSSESSION = os.path.join(ROOT, "data", "calibration", "possession.json")
BINARY = (
    os.path.join(ROOT, "target", "release", "sim2")
    if os.path.exists(os.path.join(ROOT, "target", "release", "sim2"))
    else os.path.join(ROOT, "target", "debug", "sim2")
)

SEEDS = "1..9"


class Group:
    """一个参数组：键集合、取值范围与拟合入口。

    每个键有自己的范围，因此组内也不会出现「同一个数值同时表示英尺与
    无量纲」的情况。`unit` 只是给人看的说明，边界才是实际约束。
    """

    def __init__(self, name, bounds, unit, description):
        self.name = name
        self.bounds = bounds
        self.unit = unit
        self.description = description

    @property
    def keys(self):
        return tuple(self.bounds)

    def clamp(self, key, value):
        low, high = self.bounds[key]
        return min(max(value, low), high)

    def owns(self, key):
        return key in self.bounds


# 参数组定义。每组的键与取值范围在这里是唯一的声明处。
GROUPS = [
    Group(
        name="aim_error",
        bounds={
            "shot_aim_sigma_scale": (0.2, 3.0),
            "shot_aim_movement_sensitivity": (0.0, 2.0),
        },
        unit="dimensionless",
        description="瞄准误差：出手误差总尺度与移动带来的额外误差",
    ),
    Group(
        name="distance_decay",
        bounds={
            "shot_distance_decay_three_scale_ft": (2.0, 60.0),
            "shot_distance_decay_three_power": (1.0, 4.0),
            "shot_aim_distance_scale": (10.0, 200.0),
        },
        unit="feet",
        description="距离衰减：三分线外衰减的特征长度与幂次、两分区域距离尺度",
    ),
    Group(
        name="contest",
        bounds={"shot_quality_contest_sensitivity": (0.0, 4.0)},
        unit="dimensionless",
        description="防守干扰：干扰强度对瞄准误差的放大",
    ),
    Group(
        name="shot_phase",
        bounds={
            "shot_phase_sigma_catch": (0.5, 2.0),
            "shot_phase_sigma_dribble": (0.5, 2.0),
            "shot_phase_sigma_drive": (0.5, 2.0),
            "shot_phase_sigma_post": (0.5, 2.0),
            "shot_phase_sigma_cut": (0.5, 2.0),
            "shot_phase_sigma_gather": (0.5, 2.0),
            "shot_phase_sigma_putback": (0.5, 2.0),
            "shot_gather_sigma_scale": (0.5, 1.5),
        },
        unit="dimensionless",
        description="动作阶段：各持球阶段对瞄准误差的倍率",
    ),
    Group(
        name="decision_temperature",
        bounds={
            "temperature_min": (1.0, 20.0),
            "temperature_max": (1.0, 20.0),
            "temperature_fatigue_discount": (0.0, 1.0),
        },
        unit="dimensionless",
        description="决策温度：逆温度区间与体能折扣",
    ),
    Group(
        name="pace",
        bounds={
            "seconds_per_possession": (1.0, 60.0),
            "dribble_setup": (1.0, 60.0),
            "shoot_early_slack": (1.0, 60.0),
        },
        unit="seconds",
        description="节奏：回合时长与出手宽限（全部是秒）",
    ),
    Group(
        name="behavior",
        bounds={
            "shoot_catch_bonus": (0.0, 12.0),
            "shoot_off_dribble_penalty": (0.0, 12.0),
            "drive_base": (0.0, 12.0),
            "post_base": (0.0, 12.0),
            "cut_tendency": (0.0, 12.0),
            "pass_base": (0.0, 12.0),
            "pass_open": (0.0, 12.0),
            "epv_bonus_scale": (0.01, 1.0),
        },
        unit="dimensionless",
        description="行为偏好：各候选动作的偏好权重",
    ),
]

GROUP_BY_NAME = {group.name: group for group in GROUPS}


def owned_keys():
    """返回所有被某个组声明的键，并检查没有键被两个组同时声明。"""
    seen = {}
    for group in GROUPS:
        for key in group.keys:
            if key in seen:
                raise SystemExit(
                    f"参数 {key} 同时属于 {seen[key]} 与 {group.name}：一个键只能有一个组"
                )
            seen[key] = group.name
    return seen


def targets():
    profile = json.load(open(PROFILE))
    possession = json.load(open(POSSESSION))
    league = possession["league"]
    return {
        "creation": profile["creation_share"],
        "zone": profile["zone_share"],
        "possessions_per_game": league["possessions_per_game"],
        "fga_per_possession": league["fga_per_possession"],
        "turnover_per_possession": league["turnover_per_possession"],
        "fta_per_game": league["fta_per_game"],
        "points_per_game": league["points_per_game"],
    }


def run_sim():
    output = subprocess.run(
        [BINARY, "--seeds", SEEDS, "--json"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    for line in output.stdout.splitlines():
        if line.startswith("METRICS "):
            return json.loads(line[len("METRICS ") :])
    raise RuntimeError("sim2 produced no METRICS line\n" + output.stdout[-2000:])


def ratio(target, actual):
    return (target / actual) if actual > 1e-9 else 1.0


def damp(value, factor, power=0.5):
    return value * (factor**power)


def loss(metrics, goal):
    terms = []
    for key, value in goal["creation"].items():
        terms.append((metrics["creation_share"][key] - value) ** 2)
    for key, value in goal["zone"].items():
        terms.append((metrics["zone_share"][key] - value) ** 2)
    terms.append(
        (metrics["possessions_per_game"] / goal["possessions_per_game"] - 1.0) ** 2
    )
    terms.append(
        (metrics["fga_per_possession"] / goal["fga_per_possession"] - 1.0) ** 2
    )
    terms.append(
        (
            metrics["turnover_per_possession"] / goal["turnover_per_possession"]
        )
        ** 2
    )
    terms.append((metrics["fta_per_game"] / goal["fta_per_game"] - 1.0) ** 2)
    terms.append((metrics["points_per_game"] / goal["points_per_game"] - 1.0) ** 2)
    return math.sqrt(sum(terms) / len(terms))


def adjust_group(group, weights, metrics, goal):
    """按组调整参数。只触碰该组声明的键，并用该组自己的范围收敛。"""
    w = dict(weights)
    if group.name == "aim_error":
        # 命中率偏高说明瞄准误差太小，反之偏大。
        zone = metrics["zone_share"]
        expected = goal["zone"]
        skew = 0.0
        for key, value in zone.items():
            skew += (value - expected.get(key, value)) * expected.get(key, 0.0)
        factor = 1.0 - 0.5 * max(-1.0, min(1.0, skew * 4.0))
        w["shot_aim_sigma_scale"] = group.clamp(
            "shot_aim_sigma_scale", damp(w["shot_aim_sigma_scale"], factor)
        )
    elif group.name == "distance_decay":
        three = metrics["zone_share"].get("three", 0.0)
        expected_three = goal["zone"].get("three", three)
        # 三分占比偏高说明三分线外的出手太多，衰减要更陡。
        factor = ratio(expected_three, three) if three > 1e-9 else 1.0
        w["shot_distance_decay_three_scale_ft"] = group.clamp(
            "shot_distance_decay_three_scale_ft",
            damp(w["shot_distance_decay_three_scale_ft"], factor),
        )
    elif group.name == "contest":
        contest = metrics.get("mean_contest", 0.0)
        factor = 1.0 - min(0.5, contest)
        w["shot_quality_contest_sensitivity"] = group.clamp(
            "shot_quality_contest_sensitivity",
            damp(w["shot_quality_contest_sensitivity"], factor),
        )
    elif group.name == "shot_phase":
        creation = metrics["creation_share"]
        factor = ratio(goal["creation"]["set"], creation["set"])
        w["shot_phase_sigma_catch"] = group.clamp(
            "shot_phase_sigma_catch", damp(w["shot_phase_sigma_catch"], factor)
        )
        factor = ratio(goal["creation"]["pull_up"], creation["pull_up"])
        w["shot_phase_sigma_dribble"] = group.clamp(
            "shot_phase_sigma_dribble", damp(w["shot_phase_sigma_dribble"], factor)
        )
        factor = ratio(goal["creation"]["drive"], creation["drive"])
        w["shot_phase_sigma_drive"] = group.clamp(
            "shot_phase_sigma_drive", damp(w["shot_phase_sigma_drive"], factor)
        )
    elif group.name == "decision_temperature":
        # 温度区间只影响选择的集中度：动作分布越接近目标就收窄区间。
        spread = max(
            abs(metrics["creation_share"][key] - goal["creation"][key])
            for key in goal["creation"]
        )
        factor = 1.0 - min(0.3, spread)
        w["temperature_max"] = group.clamp(
            "temperature_max", damp(w["temperature_max"], factor)
        )
    elif group.name == "pace":
        factor = ratio(metrics["possessions_per_game"], goal["possessions_per_game"])
        w["seconds_per_possession"] = group.clamp(
            "seconds_per_possession", damp(w["seconds_per_possession"], factor)
        )
        w["dribble_setup"] = group.clamp(
            "dribble_setup", damp(w["dribble_setup"], factor)
        )
        w["shoot_early_slack"] = group.clamp(
            "shoot_early_slack", damp(w["shoot_early_slack"], factor)
        )
    elif group.name == "behavior":
        creation = metrics["creation_share"]
        w["shoot_catch_bonus"] = group.clamp(
            "shoot_catch_bonus",
            damp(w["shoot_catch_bonus"], ratio(goal["creation"]["set"], creation["set"])),
        )
        w["shoot_off_dribble_penalty"] = group.clamp(
            "shoot_off_dribble_penalty",
            damp(
                w["shoot_off_dribble_penalty"],
                ratio(creation["pull_up"], goal["creation"]["pull_up"]),
            ),
        )
        w["drive_base"] = group.clamp(
            "drive_base", damp(w["drive_base"], ratio(goal["creation"]["drive"], creation["drive"]))
        )
        w["post_base"] = group.clamp(
            "post_base", damp(w["post_base"], ratio(goal["creation"]["post"], creation["post"]))
        )
        w["cut_tendency"] = group.clamp(
            "cut_tendency",
            damp(
                w["cut_tendency"],
                ratio(goal["creation"]["cut"], creation["cut"]),
            ),
        )
    for key in group.keys:
        w[key] = group.clamp(key, w[key])
    return w


def write_policy(policy, weights):
    policy["weights"] = weights
    with open(POLICY, "w") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def fit_group(group, policy, goal, iterations):
    weights = policy["weights"]
    best = None
    for iteration in range(1, iterations + 1):
        metrics = run_sim()
        current = loss(metrics, goal)
        print(
            f"[{group.name}] iter {iteration:2d}  loss {current:.4f}  "
            f"poss {metrics['possessions_per_game']:.1f}  "
            f"pts {metrics['points_per_game']:.1f}  "
            f"three {metrics['zone_share']['three']:.3f}"
        )
        if best is None or current < best[0]:
            best = (current, dict(weights))
        if current < 0.02:
            break
        weights = adjust_group(group, weights, metrics, goal)
        policy["weights"] = weights
        with open(POLICY, "w") as handle:
            json.dump(policy, handle, indent=2, ensure_ascii=False)
            handle.write("\n")
    if best is None:
        raise RuntimeError(f"[{group.name}] 没有任何一次评估，无法给出参数")
    policy["weights"] = best[1]
    write_policy(policy, best[1])
    print(f"[{group.name}] best loss {best[0]:.4f}")
    return best


def main():
    parser = argparse.ArgumentParser(description="按职责分组的 policy 参数拟合")
    parser.add_argument(
        "--group",
        default="all",
        help="要拟合的参数组名称，或 all；可用 --list 查看全部组",
    )
    parser.add_argument(
        "--iterations", type=int, default=12, help="每组的迭代次数"
    )
    parser.add_argument(
        "--dry-run", action="store_true", help="只打印参数分组，不跑模拟"
    )
    parser.add_argument("--list", action="store_true", help="列出全部参数组")
    args = parser.parse_args()

    owned = owned_keys()

    if args.list or args.dry_run:
        listing = {
            group.name: {
                "description": group.description,
                "unit": group.unit,
                "bounds": {key: list(bounds) for key, bounds in group.bounds.items()},
            }
            for group in GROUPS
        }
        print(json.dumps(listing, indent=2, ensure_ascii=False))
        return 0

    if args.group != "all" and args.group not in GROUP_BY_NAME:
        raise SystemExit(
            f"未知参数组 {args.group}；可用：{', '.join(GROUP_BY_NAME)} 或 all"
        )

    # 两阶段校准的冻结锁：命中率类表在这一阶段不得改动。
    if freeze.check(verbose=False) != 0:
        raise SystemExit("冻结表校验失败：policy 优化阶段不允许改动命中率类表")

    goal = targets()
    policy = json.load(open(POLICY))
    groups = GROUPS if args.group == "all" else [GROUP_BY_NAME[args.group]]
    for group in groups:
        fit_group(group, policy, goal, args.iterations)
    if freeze.check(verbose=False) != 0:
        raise SystemExit("优化过程改动了冻结表")
    metrics = run_sim()
    print(json.dumps(metrics, indent=2))
    # 报告每个键归属的组，便于检查没有跨组串写。
    print(json.dumps({"group_ownership": owned}, indent=2, ensure_ascii=False))
    return 0


if __name__ == "__main__":
    sys.exit(main())
