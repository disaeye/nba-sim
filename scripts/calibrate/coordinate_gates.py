#!/usr/bin/env python3
"""按判据的因果链做坐标下降，收敛剩余 7 条越界判据。

行为已锁定：`policy::tick` 在后场停滞且首次决策未消费时触发一次决策，
虚假八秒失误消失，`turnover_per_possession` 进带。其余 7 条越界各自有
直接因果杠杆，因此对这 7 条逐项做小步长扫描，评分始终用权威门禁的
39 条判定；每一轮保留全局最优，避免单参数改动把别的判据推出带宽。

用法：

    python3 -u scripts/calibrate/coordinate_gates.py --sweeps 6
"""

from __future__ import annotations

import argparse
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
WORK = ROOT / ".work" / "evidence" / "coordinate-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# 每条越界判据对应的直接杠杆，以及相对当前值的候选倍率。
LEVERS = [
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "block_base",
    "and_one_rate",
    "cut_lane_scale",
    "cut_tendency",
    "cut_positional_bonus",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "off_ball_hold_rate",
    "reach_in_foul_rate",
    "hand_check_rate",
    "transition_window_seconds",
    "transition_take_window_seconds",
    "finish_min_distance_ft",
    "drive_base",
    "shot_aim_sigma_scale",
    "play_call_rate",
    "shoot_putback_bonus",
    "seconds_per_possession",
    "gather_travel_rate",
]
FACTORS = [0.88, 0.92, 0.96, 1.0, 1.04, 1.08, 1.12]


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
    parser = argparse.ArgumentParser(description="按因果链做坐标下降")
    parser.add_argument("--sweeps", type=int, default=6)
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base])[0]
    best = initial[0]
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线深度 {best:.6f}", flush=True)

    for sweep in range(1, args.sweeps + 1):
        improved_any = False
        for key in LEVERS:
            current = get_value(best_policy, key)
            candidates = []
            values = []
            for factor in FACTORS:
                value = current * factor
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value)
                candidates.append(trial)
                values.append(value)
            results = evaluate(candidates)
            local_best = best
            local_value = current
            for trial, value, (depth, _, stdout) in zip(candidates, values, results):
                if depth + 1e-9 < local_best:
                    local_best = depth
                    local_value = value
                    chosen = trial
            if local_best + 1e-9 < best:
                best = local_best
                best_policy = chosen
                best_stdout = results[values.index(local_value)][2]
                improved_any = True
                print(
                    f"[轮 {sweep}] {key}: {current:.6f} → {local_value:.6f} 深度 {best:.6f}",
                    flush=True,
                )
            if best == 0.0:
                break
        print(f"[轮 {sweep}] 结束，深度 {best:.6f}", flush=True)
        if best == 0.0 or not improved_any:
            break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best_policy])[0]
    FINAL_LOG.write_text(best_stdout if best == 0.0 else final[2], encoding="utf-8")
    print(
        f"最终：深度 {final[0]:.6f}，失败 "
        f"{sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"几何 {final[1]['geometric_violations']}，"
        f"语境 {final[1]['context_violations']}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
