#!/usr/bin/env python3
"""从 2 条越界向量出发的失败条数优先搜索。

搜索评分按「失败条数、越界深度」排序，因此任何 1 条失败的向量都优先于
2 条失败的起点。零值键用绝对档位，其余键用倍率；每轮随机联合扰动 2–5 个
杠杆，改善立即写回主参数表与独立最优向量。

用法：

    python3 -u scripts/calibrate/search_two_failures.py --minutes 25
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
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "search-two"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-search-two-best.json"
STATE = ROOT / ".work" / "evidence" / "search-two-state.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

ABSOLUTE = {
    "and_one_rate": (0.19, 0.21, 0.23, 0.25, 0.28, 0.32, 0.36, 0.42),
    "cut_lane_scale": (0.28, 0.32, 0.36, 0.38, 0.42, 0.46, 0.52, 0.60),
    "cut_tendency": (0.01, 0.02, 0.034, 0.05, 0.07),
    "cut_positional_bonus": (0.0, 0.001, 0.004, 0.010),
    "shoot_catch_bonus": (0.0, 0.05, 0.12, 0.22),
    "shoot_pull_up_bonus": (0.14, 0.16, 0.18, 0.20, 0.24),
    "pass_catch_base": (0.34, 0.38, 0.42, 0.45, 0.48),
}
MULTIPLICATIVE = (
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "shoot_putback_bonus",
    "shoot_expected_points",
    "drive_base",
    "finish_priority",
    "post_base",
    "pass_catch_steal",
    "steal_skill_base",
    "steal_gamble_base",
    "bad_pass_rate",
    "gather_travel_rate",
    "trap_protection_bonus",
    "seconds_per_possession",
    "play_call_rate",
)
FACTORS = (0.90, 0.95, 0.98, 0.995, 1.005, 1.02, 1.05, 1.10)


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
    observed = layer["observed"]
    if detail.startswith("relative error"):
        low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
        error = layer["relative_error"]
        return (low - error) if error < low else (error - high if error > high else 0.0)
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        if observed < low:
            return (low - observed) / max(abs(high - low), 1e-6)
        if observed > high:
            return (observed - high) / max(abs(high - low), 1e-6)
        return 0.0
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return max(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return abs(min(0.0, (observed - bound) / max(abs(bound), 1e-6)))
    return abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)


def score(report):
    return (
        sum(1 for layer in report["layers"] if not layer["passed"]),
        sum(excess(layer) for layer in report["layers"]),
    )


def describe(value):
    return f"失败 {value[0]} 条，深度 {value[1]:.6f}"


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


def persist(policy, stdout, value, trial):
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")
    STATE.write_text(
        json.dumps({"failed": value[0], "depth": value[1], "trial": trial}, indent=2) + "\n",
        encoding="utf-8",
    )


def perturb(base, rng):
    policy = copy.deepcopy(base)
    for key in rng.sample(list(ABSOLUTE) + list(MULTIPLICATIVE), rng.randint(2, 5)):
        if key in ABSOLUTE:
            set_value(policy, key, rng.choice(ABSOLUTE[key]))
        else:
            set_value(policy, key, get_value(base, key) * rng.choice(FACTORS))
    return policy


def main():
    parser = argparse.ArgumentParser(description="2 条越界向量上的失败条数优先搜索")
    parser.add_argument("--minutes", type=int, default=25)
    parser.add_argument("--seed", type=int, default=20261119)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    rng = random.Random(args.seed)
    base = json.loads(BASE.read_text(encoding="utf-8"))

    report, stdout = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout, best, 0)
    print(f"基线：{describe(best)}", flush=True)

    trial = 0
    last_report = time.monotonic()
    while time.monotonic() < deadline and best[0] > 0:
        candidates = [perturb(best_policy, rng) for _ in range(WORKERS)]
        for candidate, (candidate_report, candidate_stdout) in zip(candidates, evaluate(candidates)):
            if candidate_report is None:
                continue
            value = score(candidate_report)
            if value < best:
                best = value
                best_policy = candidate
                persist(best_policy, candidate_stdout, best, trial)
                print(f"[{trial:5d}] {describe(best)}", flush=True)
        trial += WORKERS
        if time.monotonic() - last_report >= 120.0:
            last_report = time.monotonic()
            print(f"[{trial:5d}] 当前 {describe(best)}", flush=True)

    print(f"最终：{describe(best)}，共判定 {trial} 个候选", flush=True)


if __name__ == "__main__":
    main()
