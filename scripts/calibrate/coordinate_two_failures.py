#!/usr/bin/env python3
"""从 2 条越界向量出发的确定性坐标下降。

每次只动一个杠杆，候选里包含基线本身（factor 1.0），因此结果不会比
起点差。评分按「失败条数、越界深度」排序：从 2 条失败出发，任何 1 条
失败的向量都优先，即使总深度更大。

用法：

    python3 -u scripts/calibrate/coordinate_two_failures.py
"""

from __future__ import annotations

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
WORK = ROOT / ".work" / "evidence" / "coordinate-two"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-coordinate-two-best.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 两条越界判据各自的直接因果杠杆，以及会牵动它们的相邻分布杠杆。
LEVERS = [
    "and_one_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "cut_lane_scale",
    "cut_tendency",
    "cut_positional_bonus",
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "shoot_putback_bonus",
    "pass_catch_base",
    "pass_catch_steal",
    "steal_skill_base",
    "steal_gamble_base",
    "drive_base",
    "finish_priority",
    "post_base",
    "gather_travel_rate",
    "bad_pass_rate",
]
FACTORS = (0.75, 0.85, 0.92, 0.96, 0.98, 1.0, 1.02, 1.04, 1.08, 1.16, 1.30)


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


def main():
    prepare()
    base = json.loads(BASE.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(report)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)

    for sweep in range(1, 4):
        if best[0] == 0:
            break
        improved_any = False
        for key in LEVERS:
            if best[0] == 0:
                break
            current = get_value(best_policy, key)
            candidates = []
            labels = []
            for factor in FACTORS:
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, current * factor)
                candidates.append(trial)
                labels.append(f"{key}={current * factor:.6f}")
            for trial, label, (candidate_report, _) in zip(
                candidates, labels, evaluate(candidates)
            ):
                if candidate_report is None:
                    continue
                value = score(candidate_report)
                if value < best:
                    best = value
                    best_policy = trial
                    improved_any = True
                    print(f"[轮 {sweep}] {label} {describe(best)}", flush=True)
        print(f"[轮 {sweep}] 结束 {describe(best)}", flush=True)
        if not improved_any:
            break

    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final_report, final_stdout = evaluate([best_policy])[0]
    (WORK / "gate-best.log").write_text(final_stdout, encoding="utf-8")
    if final_report is not None:
        for layer in final_report["layers"]:
            if not layer["passed"]:
                print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)
    print(f"最终：{describe(best)}，向量 {BEST_VECTOR}", flush=True)


if __name__ == "__main__":
    main()
