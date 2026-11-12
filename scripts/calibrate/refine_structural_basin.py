#!/usr/bin/env python3
"""从结构盆地向量出发的失败条数优先坐标下降。

每条候选扫描都包含基线倍率 1.0，结果不会比起点差；评分按
「失败条数、越界深度」，任何 3 条失败的候选都优先于 4 条失败的起点。

用法：

    python3 -u scripts/calibrate/refine_structural_basin.py
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
WORK = ROOT / ".work" / "evidence" / "refine-basin"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-refine-basin-best.json"
REPORT = ROOT / ".work" / "evidence" / "refine-basin-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 四条失败判据各自的直接因果杠杆。
LEVERS = [
    "cut_open_defender_ft",
    "cut_lane_scale",
    "cut_tendency",
    "cut_positional_bonus",
    "cut_open_distance_ft",
    "cut_payoff_ratio",
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "shoot_putback_bonus",
    "shoot_expected_points",
    "finish_priority",
    "drive_base",
    "transition_window_seconds",
    "transition_take_window_seconds",
    "steal_gamble_base",
    "steal_skill_base",
    "pass_catch_steal",
    "pass_catch_base",
    "pass_intercept_base_ft",
    "bad_pass_rate",
    "gather_travel_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "off_ball_hold_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "seconds_per_possession",
    "play_call_rate",
    "trap_protection_bonus",
]
FACTORS = (
    0.85, 0.90, 0.94, 0.97, 0.985, 0.995, 1.0,
    1.005, 1.015, 1.03, 1.06, 1.10, 1.20,
)


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


def persist(policy, stdout, value, sweep, key):
    text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    POLICY.write_text(text, encoding="utf-8")
    BEST_VECTOR.write_text(text, encoding="utf-8")
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    REPORT.write_text(
        json.dumps(
            {"failed": value[0], "depth": value[1], "sweep": sweep, "key": key},
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )


def main():
    prepare()
    deadline = time.monotonic() + 40 * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    persist(best_policy, baseline_stdout, best, 0, "baseline")
    print(f"基线：{describe(best)}", flush=True)
    print(
        "失败 " + ",".join(
            layer["criterion"] for layer in baseline["layers"] if not layer["passed"]
        ),
        flush=True,
    )

    for sweep in range(1, 7):
        if best[0] == 0 or time.monotonic() > deadline:
            break
        improved_any = False
        for key in LEVERS:
            if best[0] == 0 or time.monotonic() > deadline:
                break
            current = get_value(best_policy, key)
            candidates = []
            labels = []
            for factor in FACTORS:
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, current * factor)
                candidates.append(trial)
                labels.append(f"{key}={current * factor:.6f}")
            for trial, label, (report, stdout) in zip(
                candidates, labels, evaluate(candidates)
            ):
                if report is None:
                    continue
                value = score(report)
                if value < best:
                    best = value
                    best_policy = trial
                    persist(best_policy, stdout, best, sweep, label)
                    improved_any = True
                    print(f"[轮 {sweep}] {label} {describe(best)}", flush=True)
        print(f"[轮 {sweep}] 结束 {describe(best)}", flush=True)
        if not improved_any:
            break

    final, final_stdout = evaluate([best_policy])[0]
    if final is not None:
        for layer in final["layers"]:
            if not layer["passed"]:
                print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
