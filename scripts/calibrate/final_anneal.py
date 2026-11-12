#!/usr/bin/env python3
"""最终退火搜索：在当前行为上收敛 39 条门禁判据。

只使用实测有响应的生产杠杆；每轮随机挑 1–3 个杠杆施加细步长扰动。
评分来自权威门禁的「越界深度、失败条数」。改善立即写入独立最优向量；
按衰减温度接受少量退步，避免离散事件响应面的局部极小。

全程不写主参数表，搜索结束后由调用方显式采纳。

用法：

    python3 -u scripts/calibrate/final_anneal.py --minutes 30
"""

from __future__ import annotations

import argparse
import copy
import json
import math
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
WORK = ROOT / ".work" / "evidence" / "final-anneal"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-final-anneal-best.json"
STATE = ROOT / ".work" / "evidence" / "final-anneal-state.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 实测有响应的生产杠杆；外部出手基准夹具里恒等为 1 的键也合法参与。
LEVERS = [
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "shoot_off_dribble_penalty",
    "shoot_putback_bonus",
    "cut_lane_scale",
    "cut_positional_bonus",
    "drive_base",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "steal_gamble_base",
    "steal_skill_base",
    "pass_catch_steal",
    "pass_intercept_steal_ft",
    "bad_pass_rate",
    "gather_travel_rate",
    "shot_quality_contest_sensitivity",
    "shot_fatigue_penalty",
    "shot_gather_sigma_scale",
    "trap_protection_bonus",
]
FACTORS = [
    0.955, 0.975, 0.988, 0.996,
    1.004, 1.012, 1.025, 1.045,
]


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
    values = [excess(layer) for layer in report["layers"]]
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    # 失败条数优先，其次总越界深度。
    return (failed, sum(values))


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


def persist(policy, value, trial):
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    STATE.write_text(
        json.dumps(
            {"failed": value[0], "depth": value[1], "trial": trial},
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )


def main():
    parser = argparse.ArgumentParser(description="最终退火搜索")
    parser.add_argument("--minutes", type=int, default=30)
    parser.add_argument("--seed", type=int, default=20261114)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))

    report, _ = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(report)
    best_policy = copy.deepcopy(base)
    current = copy.deepcopy(base)
    current_score = best
    persist(best_policy, best, 0)
    print(f"基线：{describe(best)}", flush=True)

    trial = 0
    last_report = time.monotonic()
    while time.monotonic() < deadline and best[0] > 0:
        candidates = []
        for _ in range(WORKERS):
            candidate = copy.deepcopy(current)
            for key in rng.sample(LEVERS, rng.randint(1, 3)):
                set_value(candidate, key, get_value(current, key) * rng.choice(FACTORS))
            candidates.append(candidate)
        results = evaluate(candidates)
        elapsed = time.monotonic() - (deadline - args.minutes * 60.0)
        fraction = elapsed / (args.minutes * 60.0)
        temperature = 0.02 * (1.0 - fraction) + 0.0005
        for candidate, (candidate_report, _) in zip(candidates, results):
            if candidate_report is None:
                continue
            candidate_score = score(candidate_report)
            delta = (candidate_score[0] - current_score[0]) * 0.1 + (
                candidate_score[1] - current_score[1]
            )
            accept = delta < 0.0 or rng.random() < math.exp(-delta / max(temperature, 1e-9))
            if accept:
                current, current_score = candidate, candidate_score
            if candidate_score < best:
                best, best_policy = candidate_score, candidate
                persist(best_policy, best, trial)
                print(f"[{trial:5d}] {describe(best)}", flush=True)
        trial += WORKERS
        if time.monotonic() - last_report >= 120.0:
            last_report = time.monotonic()
            print(f"[{trial:5d}] 当前 {describe(best)} 退火 {describe(current_score)}", flush=True)

    print(f"最终：{describe(best)}，共判定 {trial} 个候选", flush=True)


if __name__ == "__main__":
    main()
