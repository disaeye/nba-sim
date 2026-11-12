#!/usr/bin/env python3
"""最终门禁参数搜索：每步立即持久化最优解。

从当前 `data/calibration/policy.json` 出发，混合三种扰动宽度并行评估，
评分来自权威门禁自己的 39 条判定。任何一次改进都会立刻写回主参数表、
另存快照并更新 gate-final.log，因此进程被中断也不会丢失最优解。

用法：

    python3 -u scripts/calibrate/final_search.py --minutes 40 --seed 20261110
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "final-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_SNAPSHOT = ROOT / ".work" / "evidence" / "policy-final-best.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# (下界, 上界)。步长按当前值的百分比给，因此每个参数都在自己的量纲里动。
BOUNDS = {
    "cut_lane_scale": (0.30, 0.70),
    "cut_tendency": (0.0, 0.09),
    "cut_positional_bonus": (0.0, 0.02),
    "shoot_putback_bonus": (0.50, 1.10),
    "shoot_catch_bonus": (0.0, 0.5),
    "finish_min_distance_ft": (2.2, 3.9),
    "finish_priority": (0.22, 0.62),
    "drive_base": (1.30, 2.10),
    "post_base": (0.38, 0.80),
    "gather_travel_rate": (0.004, 0.060),
    "shot_aim_sigma_scale": (0.90, 1.45),
    "shot_distance_decay_three_scale_ft": (8.0, 13.0),
    "seconds_per_possession": (15.0, 21.0),
    "possession_setup_seconds": (7.5, 9.2),
    "backcourt_margin_ft": (1.6, 3.0),
    "pass_catch_steal": (0.32, 0.70),
    "steal_gamble_base": (0.45, 1.05),
    "steal_skill_base": (0.60, 1.15),
    "off_ball_hold_rate": (0.0015, 0.010),
    "hand_check_rate": (0.0008, 0.009),
    "reach_in_foul_rate": (0.006, 0.035),
    "play_call_rate": (0.68, 0.88),
    "shooting_foul_rate_two_point": (0.10, 0.30),
    "shooting_foul_rate_three_point": (0.015, 0.09),
    "closeout_draw_contact_rate": (0.20, 0.60),
    "closeout_draw_contact_foul_chance": (0.25, 0.85),
    "and_one_rate": (0.10, 0.30),
    "transition_window_seconds": (7.5, 12.0),
    "transition_take_window_seconds": (6.0, 10.0),
    "drive_finish_distance_ft": (4.0, 8.0),
    "block_base": (0.20, 0.42),
    "rebound_boxout_weight": (0.60, 1.40),
}
KEYS = sorted(BOUNDS)


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
    processes = []
    for index, policy in enumerate(candidates):
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
    results = []
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


def persist(policy, stdout):
    body = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    POLICY.write_text(body, encoding="utf-8")
    BEST_SNAPSHOT.write_text(body, encoding="utf-8")
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def perturb(policy, rng):
    trial = copy.deepcopy(policy)
    mode = rng.random()
    if mode < 0.60:
        count, width = 1, rng.uniform(0.005, 0.030)
    elif mode < 0.90:
        count, width = 2, rng.uniform(0.020, 0.080)
    else:
        count, width = 3, rng.uniform(0.060, 0.180)
    for key in rng.sample(KEYS, count):
        low, high = BOUNDS[key]
        current = get_value(trial, key)
        candidate = current * (1.0 + rng.choice((-width, width)))
        set_value(trial, key, min(max(candidate, low), high))
    return trial


def main():
    parser = argparse.ArgumentParser(description="立即持久化的最终门禁搜索")
    parser.add_argument("--minutes", type=float, default=40.0)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base] * WORKERS)[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)
    if best[0] == 0:
        persist(best_policy, best_stdout)
        print("基线已经 39 条全通过", flush=True)
        return 0

    deadline = time.monotonic() + args.minutes * 60.0
    round_index = 0
    while time.monotonic() < deadline:
        round_index += 1
        candidates = [perturb(best_policy, rng) for _ in range(WORKERS)]
        results = evaluate(candidates)
        for trial, (depth, report, stdout) in zip(candidates, results):
            if report is None:
                continue
            failed = sum(1 for l in report["layers"] if not l["passed"])
            score = (failed, depth)
            if score < best:
                best = score
                best_policy = trial
                best_stdout = stdout
                persist(best_policy, best_stdout)
                bad = [l["criterion"] for l in report["layers"] if not l["passed"]]
                print(
                    f"[{round_index:4d}] 失败 {failed} 深度 {depth:.6f} {bad}",
                    flush=True,
                )
            if best[0] == 0:
                break
        if best[0] == 0:
            print(f"[{round_index:4d}] 39 条判据全部通过", flush=True)
            break
        if round_index % 50 == 0:
            print(f"[{round_index:4d}] 当前最优 {best[0]} 条 / {best[1]:.6f}", flush=True)

    persist(best_policy, best_stdout)
    print(f"搜索结束：共 {round_index} 轮，最优 {best[0]} 条 / {best[1]:.6f}", flush=True)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
