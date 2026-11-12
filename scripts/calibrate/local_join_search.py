#!/usr/bin/env python3
"""从当前参数点做有界随机多杠杆局部搜索。

单杠杆坐标下降在离散响应面上会卡在局部极小：抢断、失误、罚球、空切与
加罚各自都不越界太多，但必须同时移动几组相关杠杆才能一起进带。这里每轮
随机挑 3–5 个直接因果杠杆施加小幅扰动，用权威门禁评分，只接受改善并立即
写回主参数表。搜索范围固定，进程受 2 GiB 地址空间上限约束。

用法：

    python3 -u scripts/calibrate/local_join_search.py --trials 240
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "join-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

LEVERS = [
    "cut_tendency",
    "cut_lane_scale",
    "drive_base",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "bad_pass_rate",
    "bad_pass_skill_base",
    "gather_travel_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "transition_take_window_seconds",
    "seconds_per_possession",
    "play_call_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "shoot_early_slack",
    "shoot_putback_bonus",
    "post_base",
    "finish_priority",
]
FACTORS = [0.90, 0.94, 0.97, 0.985, 1.0, 1.015, 1.03, 1.06, 1.10]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
    error = layer["relative_error"]
    if detail.startswith("relative error"):
        low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
        return (low - error) if error < low else (error - high if error > high else 0.0)
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        observed = layer["observed"]
        if observed < low:
            return (low - observed) / max(abs(high - low), 1e-6)
        if observed > high:
            return (observed - high) / max(abs(high - low), 1e-6)
        return 0.0
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return max(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return min(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6))
    return 0.0


def depth(report):
    return sum(abs(excess(layer)) for layer in report["layers"])


def score(report):
    vector = [excess(layer) for layer in report["layers"]]
    return (
        sum(abs(value) for value in vector),
        sum(1 for value in vector if abs(value) > 1e-9),
    )


def describe(value):
    return f"深度 {value[0]:.6f}，失败 {value[1]} 条"


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
                    preexec_fn=limit_address_space,
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
            results.append((report, output))
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
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def main():
    parser = argparse.ArgumentParser(description="有界随机多杠杆局部搜索")
    parser.add_argument("--trials", type=int, default=240)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    assert report is not None, "baseline gate produced no report"
    best = score(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    for batch_start in range(0, args.trials, WORKERS):
        if best[0] == 0.0:
            break
        candidates = []
        for _ in range(WORKERS):
            trial = copy.deepcopy(best_policy)
            for key in rng.sample(LEVERS, rng.randint(3, 5)):
                factor = rng.choice(FACTORS)
                set_value(trial, key, get_value(best_policy, key) * factor)
            candidates.append(trial)
        results = evaluate(candidates)
        for trial, (candidate_report, candidate_stdout) in zip(candidates, results):
            if candidate_report is None:
                continue
            candidate_score = score(candidate_report)
            if candidate_score < best:
                best = candidate_score
                best_policy = trial
                persist(best_policy, candidate_stdout)
                print(
                    f"[{batch_start + len(candidates):4d}/{args.trials}] {describe(best)}",
                    flush=True,
                )
        if (batch_start // WORKERS) % 10 == 0:
            print(f"[{batch_start + WORKERS:4d}/{args.trials}] 当前 {describe(best)}", flush=True)

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
