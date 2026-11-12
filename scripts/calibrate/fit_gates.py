#!/usr/bin/env python3
"""按权威门禁判据做坐标下降的 policy.json 参数搜索。

判据直接读 `sim2 --gate` 输出的 GateReport：目标、带宽、几何与语境违规
都来自引擎自己，搜索与最终验收用同一份 16 场比赛和同一个判定入口。
这里不重复声明目标数字，也不维护另一套带宽。

用法：

    fit_gates.py --steps 12            # 每个参数走多少轮
    fit_gates.py --dry-run              # 只打印将要动的参数
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")

# 一个参数负责哪些判据。这里只登记真实影响到的那几条，便于审计；
# 评分始终用完整的 GateReport，因此未登记的副作用也会计入。
LEVERS = {
    "shooting_foul_rate_two_point": ["shooting_fouls_per_game", "fta_per_game"],
    "shooting_foul_rate_three_point": ["shooting_fouls_per_game", "fta_per_game"],
    "and_one_rate": ["and_one_per_game"],
    "cut_lane_scale": ["creation_share_cut", "zone_share_rim"],
    "transition_window_seconds": ["creation_share_transition"],
    "shoot_putback_bonus": ["creation_share_putback"],
    "shot_aim_sigma_scale": ["creation_share_cut", "zone_share_rim"],
    "cut_tendency": ["creation_share_cut"],
    "cut_positional_bonus": ["creation_share_cut"],
    "play_call_rate": ["play_activation_rate"],
    "gather_travel_rate": ["turnover_per_possession"],
    "pass_catch_steal": ["steals_per_game", "turnover_per_possession"],
}


def run_gate():
    """跑权威门禁，返回 (GateReport, 进程退出码)。"""
    output = subprocess.run(
        [BINARY, "42", "--gate"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
    )
    for line in output.stdout.splitlines():
        if line.startswith("GATE "):
            return json.loads(line[len("GATE ") :]), 0
    raise RuntimeError("sim2 produced no GATE line\n" + output.stdout[-2000:])


def violations(report):
    """越界量：每条未通过的判据按其相对带宽的越界深度平方计。

    过程层的区间与上下界写在 `detail` 里，几何与语境违规各按一条完整
    判据计入。通过的判据不产生任何奖励，避免搜索为了拉大安全余量把
    参数推离校准基线。
    """
    total = float(report["geometric_violations"] + report["context_violations"])
    details = []
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            band = detail.split("band ")[1].rstrip(")")
            low, high = (float(value) for value in band.split(".."))
            error = layer["relative_error"]
            gap = max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            bounds = detail.removeprefix("区间 ").split("..")
            low, high = (float(value) for value in bounds)
            observed = layer["observed"]
            gap = (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            # 上限或下限：越界深度按目标值折算。
            gap = abs(layer["observed"] - layer["target"]) / max(
                abs(layer["target"]), 1e-6
            )
        total += gap**2
        details.append((layer["criterion"], gap))
    return total, details


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


def step_sizes(policy, key):
    """步长按量级给：比率类用小数点后两位的量纲，时长类用一秒的量纲。"""
    value = current_value(policy, key)
    if key.endswith("_seconds") or key.endswith("_ft"):
        return [0.5, 0.25, 0.1]
    if abs(value) < 0.2:
        return [0.02, 0.01, 0.005]
    return [0.1, 0.05, 0.02]


def score(policy):
    write_policy(policy)
    report, _ = run_gate()
    return violations(report)


def main():
    parser = argparse.ArgumentParser(description="按权威门禁判据做坐标下降的参数搜索")
    parser.add_argument("--steps", type=int, default=12, help="每个参数走多少轮")
    parser.add_argument("--dry-run", action="store_true", help="只打印参数清单")
    args = parser.parse_args()

    policy = read_policy()
    if args.dry_run:
        for key in LEVERS:
            try:
                print(f"{key}: 当前 {current_value(policy, key)}")
            except KeyError as error:
                print(f"{key}: {error}")
        return 0

    best, details = score(policy)
    print(f"起始越界量 {best:.6f}", flush=True)
    for key, error in details:
        print(f"  {key}: 越界 {error:+.4f}", flush=True)

    for step in range(1, args.steps + 1):
        improved = False
        for key in LEVERS:
            base = current_value(policy, key)
            local_best = best
            local_value = base
            for size in step_sizes(policy, key):
                for candidate in (base + size, base - size):
                    if candidate < 0.0:
                        continue
                    trial = copy.deepcopy(policy)
                    set_value(trial, key, candidate)
                    trial_best, _ = score(trial)
                    if trial_best < local_best - 1e-9:
                        local_best = trial_best
                        local_value = candidate
            set_value(policy, key, local_value)
            write_policy(policy)
            if local_best < best - 1e-9:
                best = local_best
                improved = True
                print(
                    f"[{step}] {key} {base:.4f} → {local_value:.4f}  越界量 {best:.6f}",
                    flush=True,
                )
        if not improved:
            print(f"[{step}] 没有进一步改善，停止", flush=True)
            break

    report, _ = run_gate()
    best, details = violations(report)
    write_policy(policy)
    print(f"最终越界量 {best:.6f}  门禁通过 {report['passed']}", flush=True)
    for key, error in details:
        print(f"  {key}: 越界 {error:+.4f}", flush=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
