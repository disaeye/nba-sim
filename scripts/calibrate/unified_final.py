#!/usr/bin/env python3
"""统一最终搜索：把零值键与实测活杠杆放进同一个有界随机搜索。

`zero_key_grid.py` 证明 `pass_catch_base` 与零值出手权重是活杠杆，但之前的
乘法搜索永远动不了零值键，也没把 `pass_catch_base` 放进杠杆表。这里对零值
键取绝对档位，对其余键取倍率，每轮随机挑 2–5 个杠杆联合扰动，按权威门禁
的「越界深度、失败条数」评分，改善立即写回主参数表。

用法：

    python3 -u scripts/calibrate/unified_final.py --minutes 25
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
WORK = ROOT / ".work" / "evidence" / "unified-final"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-unified-final-best.json"
STATE = ROOT / ".work" / "evidence" / "unified-final-state.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 零值或接近零值的键取绝对档位：乘法扰动推不动它们。
ABSOLUTE = {
    "shoot_catch_bonus": (0.0, 0.05, 0.10, 0.18, 0.25, 0.35),
    "shoot_off_dribble_penalty": (0.0, 0.05, 0.10, 0.18, 0.25),
    "shoot_separation_bonus": (0.0, 0.10, 0.25),
    "pass_catch_base": (0.34, 0.38, 0.42, 0.45, 0.48, 0.52, 0.56),
    "cut_tendency": (0.015, 0.025, 0.034, 0.045, 0.06),
    "cut_positional_bonus": (0.0, 0.001, 0.004, 0.008),
}
# 其余活杠杆取倍率。
MULTIPLICATIVE = (
    "shoot_pull_up_bonus",
    "shoot_putback_bonus",
    "shoot_expected_points",
    "shoot_tendency",
    "shoot_pressure",
    "cut_lane_scale",
    "drive_base",
    "finish_priority",
    "post_base",
    "and_one_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "steal_gamble_base",
    "steal_skill_base",
    "pass_catch_steal",
    "bad_pass_rate",
    "gather_travel_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "trap_protection_bonus",
    "transition_take_window_seconds",
    "transition_window_seconds",
    "seconds_per_possession",
    "play_call_rate",
    "shot_quality_contest_sensitivity",
    "shot_fatigue_penalty",
    "shot_gather_sigma_scale",
)
FACTORS = (0.80, 0.88, 0.94, 0.97, 0.99, 0.995, 1.005, 1.01, 1.03, 1.06, 1.12, 1.25)


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
        sum(excess(layer) for layer in report["layers"]),
        sum(1 for layer in report["layers"] if not layer["passed"]),
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


def persist(policy, stdout, value, trial):
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")
    STATE.write_text(
        json.dumps({"depth": value[0], "failed": value[1], "trial": trial}, indent=2) + "\n",
        encoding="utf-8",
    )


def perturb(base, rng):
    policy = copy.deepcopy(base)
    keys = rng.sample(list(ABSOLUTE) + list(MULTIPLICATIVE), rng.randint(2, 5))
    for key in keys:
        if key in ABSOLUTE:
            set_value(policy, key, rng.choice(ABSOLUTE[key]))
        else:
            set_value(policy, key, get_value(base, key) * rng.choice(FACTORS))
    return policy


def main():
    parser = argparse.ArgumentParser(description="统一最终门禁搜索")
    parser.add_argument("--minutes", type=int, default=25)
    parser.add_argument("--seed", type=int, default=20261117)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))

    report, stdout = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout, best, 0)
    print(f"基线：{describe(best)}", flush=True)

    trial = 0
    last_report = time.monotonic()
    # 前 1/4 时间从当前最优向量出发，之后从外部基准合法的次优向量重启一次。
    alternate = json.loads(
        (ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json").read_text(
            encoding="utf-8"
        )
    )
    restarted = False
    while time.monotonic() < deadline and best[0] > 0.0:
        if not restarted and time.monotonic() > deadline - args.minutes * 60.0 * 0.72:
            restarted = True
            candidate_results = evaluate([alternate])
            if candidate_results[0][0] is not None:
                alt_score = score(candidate_results[0][0])
                if alt_score[1] < best[1] or alt_score[0] < best[0] * 1.5:
                    best_policy = copy.deepcopy(alternate)
                    print(f"重启自 2 条越界向量：{describe(alt_score)}", flush=True)
        candidates = [perturb(best_policy, rng) for _ in range(WORKERS)]
        results = evaluate(candidates)
        for candidate, (candidate_report, candidate_stdout) in zip(candidates, results):
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
