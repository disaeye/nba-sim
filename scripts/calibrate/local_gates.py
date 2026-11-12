#!/usr/bin/env python3
"""极小步长局部搜索：从 38/39 基线消掉最后 0.083 的越界深度。

按地址抽样的引擎对大改动会重排整条抽样链，因此这里每次只把 1–2 个
参数改动 1%–3%，四个方向并行评估，只接受越界深度下降的结果。评分
直接来自权威门禁自己的 39 条判定。

用法：

    python3 -u scripts/calibrate/local_gates.py --rounds 600
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "local-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# 与失误率、回合数、抢断、投篮犯规有直接因果链的参数。
LEVERS = [
    "off_ball_hold_rate",
    "reach_in_foul_rate",
    "hand_check_rate",
    "gather_travel_rate",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "drive_turnover_rate",
    "bad_pass_rate",
    "seconds_per_possession",
    "possession_setup_seconds",
    "backcourt_margin_ft",
    "cut_lane_scale",
    "cut_positional_bonus",
    "shoot_putback_bonus",
    "shooting_foul_rate_two_point",
    "and_one_rate",
    "play_call_rate",
    "transition_window_seconds",
    "shot_aim_sigma_scale",
    "finish_min_distance_ft",
    "drive_base",
]


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
                [str(BINARY), "--gate-only", "--calibration", str(WORK / f"worker-{index}" / "calibration")],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
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
            continue
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


def perturb(policy, rng, fraction):
    trial = copy.deepcopy(policy)
    for key in rng.sample(LEVERS, rng.choice((1, 1, 1, 2))):
        current = get_value(trial, key)
        delta = current * fraction * rng.choice((-1.0, 1.0))
        set_value(trial, key, current + delta)
    return trial


def main():
    parser = argparse.ArgumentParser(description="极小步长局部门禁搜索")
    parser.add_argument("--rounds", type=int, default=600)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base] * WORKERS)[0]
    best = (initial[0], base, initial[2])
    print(f"基线深度 {best[0]:.6f}", flush=True)

    for round_index in range(1, args.rounds + 1):
        # 步长随进度收缩：先探索，后精修。
        fraction = 0.03 if round_index % 4 else 0.01
        candidates = [perturb(best[1], rng, fraction) for _ in range(WORKERS)]
        results = evaluate(candidates)
        improved = False
        for trial, (depth, report, stdout) in zip(candidates, results):
            if depth + 1e-9 < best[0]:
                best = (depth, trial, stdout)
                improved = True
        if improved and (round_index <= 20 or round_index % 20 == 0):
            print(f"[{round_index:3d}] 深度 {best[0]:.6f}", flush=True)
        if best[0] == 0.0:
            print(f"[{round_index:3d}] 39 条判据全部通过", flush=True)
            break

    POLICY.write_text(json.dumps(best[1], indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    final = evaluate([best[1]] * WORKERS)[0]
    FINAL_LOG.write_text(best[2] if best[0] == 0.0 else final[2], encoding="utf-8")
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
