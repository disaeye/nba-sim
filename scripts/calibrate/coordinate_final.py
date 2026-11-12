#!/usr/bin/env python3
"""从主参数表做确定性坐标下降，收尾 39 条门禁判据。

每个杠杆按固定倍率逐项扫描；评分来自当前 `sim2 --gate-only` 的
「越界深度、失败条数」。只有真实改善才写回主参数表。硬截止保证有界退出。

用法：

    python3 -u scripts/calibrate/coordinate_final.py --minutes 20
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import resource
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "coordinate-final"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 与 sustained_search.py 的完整因果杠杆集同源；出手模型夹具里恒等的键也保留。
# 当前四条越界判据的直接因果杠杆，以及在出手基准夹具里恒等为 1、
# 但真实比赛中生效的出手模型杠杆。
LEVERS = [
    "shoot_pull_up_bonus",
    "cut_tendency",
    "cut_lane_scale",
    "cut_positional_bonus",
    "drive_base",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "steal_gamble_base",
    "pass_catch_steal",
    "steal_skill_base",
    "trap_protection_bonus",
    "steal_skill_scale",
    "pass_intercept_steal_ft",
    "poke_reach_steal_ft",
    "bad_pass_rate",
    "gather_travel_rate",
    "shot_quality_contest_sensitivity",
    "shot_aim_movement_sensitivity",
    "shot_movement_reference_rim",
    "shot_fatigue_penalty",
    "shot_gather_sigma_scale",
    "shot_quality_contest_center",
]
FACTORS = [
    0.70, 0.80, 0.86, 0.92, 0.96, 0.985, 0.995,
    1.005, 1.015, 1.04, 1.08, 1.16, 1.30, 1.50,
]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        observed = layer["observed"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


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


def evaluate(candidates, deadline):
    results = []
    for start in range(0, len(candidates), WORKERS):
        if time.monotonic() > deadline:
            results.extend([(None, "")] * (len(candidates) - len(results)))
            break
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
    parser = argparse.ArgumentParser(description="最终确定性坐标下降")
    parser.add_argument("--minutes", type=int, default=20)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base], deadline)[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    for sweep in range(1, 4):
        if best[0] == 0.0 or time.monotonic() > deadline:
            break
        improved_any = False
        for key in LEVERS:
            if best[0] == 0.0 or time.monotonic() > deadline:
                break
            current = get_value(best_policy, key)
            candidates = []
            labels = []
            for factor in FACTORS:
                value = current * factor
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value)
                candidates.append(trial)
                labels.append(f"{key}={value:.6f}")
            for trial, label, (candidate_report, candidate_stdout) in zip(
                candidates, labels, evaluate(candidates, deadline)
            ):
                if candidate_report is None:
                    continue
                score = score_of(candidate_report)
                if score < best:
                    best = score
                    best_policy = trial
                    persist(best_policy, candidate_stdout)
                    improved_any = True
                    print(f"[轮 {sweep}] {label} {describe(best)}", flush=True)
        print(f"[轮 {sweep}] 结束 {describe(best)}", flush=True)
        if not improved_any:
            break

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
