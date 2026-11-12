#!/usr/bin/env python3
"""并行门禁参数搜索。

每个 worker 持有独立的校准目录（含全部标定表与 tactics 符号链接），
`sim2 --gate-only --calibration <dir>` 读自己的 policy.json，因此 4 个
候选可以同时评估，不再共享一份参数文件。评分来自权威门禁的 39 条判定。

用法：

    python3 -u scripts/calibrate/parallel_search.py --rounds 100
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import shutil
import subprocess
import sys
from pathlib import Path

# 每个 worker 一个 rayon 线程：4 个 worker 共 4 线程，避免 16 线程争抢
# 4 核导致每个候选都被拖慢。
os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "parallel-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

BOUNDS = {
    "cut_lane_scale": (0.30, 0.65, 0.03),
    "cut_tendency": (0.0, 0.08, 0.006),
    "cut_positional_bonus": (0.0, 0.02, 0.002),
    "shoot_putback_bonus": (0.55, 1.05, 0.04),
    "finish_min_distance_ft": (2.3, 3.8, 0.12),
    "finish_priority": (0.25, 0.60, 0.03),
    "drive_base": (1.35, 2.0, 0.06),
    "post_base": (0.40, 0.75, 0.03),
    "gather_travel_rate": (0.005, 0.055, 0.004),
    "shot_aim_sigma_scale": (0.95, 1.40, 0.03),
    "seconds_per_possession": (16.0, 20.0, 0.3),
    "pass_catch_steal": (0.35, 0.65, 0.02),
    "off_ball_hold_rate": (0.002, 0.009, 0.0006),
    "hand_check_rate": (0.001, 0.008, 0.001),
    "reach_in_foul_rate": (0.008, 0.030, 0.003),
    "play_call_rate": (0.70, 0.86, 0.012),
    "shooting_foul_rate_two_point": (0.12, 0.26, 0.012),
    "and_one_rate": (0.12, 0.26, 0.012),
    "transition_window_seconds": (8.0, 11.5, 0.3),
    "shot_distance_decay_three_scale_ft": (8.5, 12.0, 0.3),
    "possession_setup_seconds": (8.2, 8.9, 0.05),
    "backcourt_margin_ft": (1.5, 3.0, 0.25),
}
KEYS = sorted(BOUNDS)


def read_policy():
    return json.loads(POLICY.read_text(encoding="utf-8"))


def write_policy(policy):
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


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


def prepare_workers():
    if WORK.exists():
        shutil.rmtree(WORK)
    for index in range(WORKERS):
        worker = WORK / f"worker-{index}"
        calibration = worker / "calibration"
        calibration.mkdir(parents=True)
        for path in CALIBRATION.glob("*.json"):
            shutil.copy2(path, calibration / path.name)
        (worker / "tactics").symlink_to(TACTICS, target_is_directory=True)


def evaluate_candidates(candidates):
    processes = []
    for index, policy in enumerate(candidates):
        policy_path = WORK / f"worker-{index}" / "calibration" / "policy.json"
        policy_path.write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        calibration_dir = WORK / f"worker-{index}" / "calibration"
        process = subprocess.Popen(
            [
                str(BINARY),
                "--gate-only",
                "--calibration",
                str(calibration_dir),
            ],
            cwd=ROOT,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        processes.append(process)
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
            raise RuntimeError(
                f"sim2 gate-only failed (exit {process.returncode}): {output[-1500:]}"
            )
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        results.append((failed, depth_of(report), report, output))
    return results


def perturb(policy, rng, width):
    trial = copy.deepcopy(policy)
    for key in rng.sample(KEYS, rng.choice((1, 1, 2, 2, 3, 3))):
        low, high, base_step = BOUNDS[key]
        step = base_step * width
        current = get_value(trial, key)
        candidate = current + rng.choice((-step, step))
        set_value(trial, key, min(max(candidate, low), high))
    return trial


def candidate_policies(base, best_policy, rng, round_index):
    """每 20 轮用一半候选从冻结基线大步重开，避免陷在局部极值。"""
    wide = round_index % 20 == 0
    policies = []
    for index in range(WORKERS):
        if wide and index < 2:
            policies.append(perturb(base, rng, 3.0))
        else:
            policies.append(perturb(best_policy, rng, 1.0))
    return policies


def main():
    parser = argparse.ArgumentParser(description="并行门禁参数搜索")
    parser.add_argument("--rounds", type=int, default=100)
    parser.add_argument("--seed", type=int, default=20261110)
    parser.add_argument("--width", type=float, default=1.0)
    args = parser.parse_args()

    prepare_workers()
    rng = random.Random(args.seed)
    base = read_policy()
    initial = evaluate_candidates([base] * WORKERS)[0]
    best = (initial[0], initial[1])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[3]
    print(f"起始：失败 {best[0]} 条，深度 {best[1]:.5f}", flush=True)

    for round_index in range(1, args.rounds + 1):
        candidates = candidate_policies(base, best_policy, rng, round_index)
        results = evaluate_candidates(candidates)
        improved = False
        for trial, (failed, depth, report, stdout) in zip(candidates, results):
            score = (failed, depth)
            if score < best:
                best = score
                best_policy = trial
                best_stdout = stdout
                improved = True
                bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
                print(
                    f"[{round_index:3d}] 失败 {failed} 深度 {depth:.5f} {bad}",
                    flush=True,
                )
            if failed == 0:
                break
        if best[0] == 0:
            print(f"[{round_index:3d}] 39 条判据全部通过", flush=True)
            break
        if round_index % 10 == 0:
            print(f"[{round_index:3d}] 当前最优 {best[0]} 条 / {best[1]:.5f}", flush=True)

    write_policy(best_policy)
    final = evaluate_candidates([best_policy] * WORKERS)[0]
    FINAL_LOG.write_text(best_stdout or final[3], encoding="utf-8")
    print(
        f"最终：失败 {final[0]} 条，几何 {final[2]['geometric_violations']}，"
        f"语境 {final[2]['context_violations']}，通过 {final[2]['passed']}",
        flush=True,
    )
    for layer in final[2]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    sys.exit(main())
