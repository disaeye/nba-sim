#!/usr/bin/env python3
"""按最小事件量收尾两条越界判据。

`zone_share_rim` 只差一次近筐出手，`steals_per_game` 只差两次抢断。这里
只对这两条因果链的直接杠杆做细倍率扫描，再对两个杠杆做一次联合网格；
评分始终来自权威门禁，最优向量立即写回主参数表。

用法：

    python3 -u scripts/calibrate/close_band_edges.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "close-edges"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# 近筐出手与抢断的直接杠杆。
RIM_LEVERS = ["drive_base", "finish_min_distance_ft", "cut_lane_scale"]
STEAL_LEVERS = ["steal_gamble_base", "pass_catch_steal", "hand_check_rate"]
FACTORS = [0.97, 0.985, 0.995, 1.0, 1.005, 1.015, 1.03]
PAIR_FACTORS = [0.99, 0.995, 1.0, 1.005, 1.01]


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


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth_of(report), failed)


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


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


def accept_trials(base, trials, labels, best, best_policy):
    results = evaluate(trials)
    chosen = None
    chosen_stdout = None
    chosen_label = None
    for trial, label, (report, stdout) in zip(trials, labels, results):
        if report is None:
            continue
        score = score_of(report)
        if score < best:
            best = score
            chosen = trial
            chosen_stdout = stdout
            chosen_label = label
    return best, chosen, chosen_stdout, chosen_label


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    assert report is not None, "baseline gate produced no report"
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    # 单杠杆细扫：每个候选只改一个直接杠杆。
    for key in RIM_LEVERS + STEAL_LEVERS:
        if best[0] == 0.0:
            break
        current = get_value(best_policy, key)
        trials = []
        labels = []
        for factor in FACTORS:
            trial = copy.deepcopy(best_policy)
            set_value(trial, key, current * factor)
            trials.append(trial)
            labels.append(f"{key}={current * factor:.6f}")
        best, chosen, chosen_stdout, label = accept_trials(
            best_policy, trials, labels, best, best_policy
        )
        if chosen is not None:
            best_policy = chosen
            persist(best_policy, chosen_stdout)
            print(f"[单杠杆] {label} {describe(best)}", flush=True)

    # 两个杠杆联合网格：每条因果链各取一个杠杆，覆盖离散事件的跳变点。
    if best[0] > 0.0:
        rim_key = RIM_LEVERS[0]
        steal_key = STEAL_LEVERS[0]
        for rim_factor, steal_factor in itertools.product(PAIR_FACTORS, PAIR_FACTORS):
            if rim_factor == 1.0 and steal_factor == 1.0:
                continue
            trial = copy.deepcopy(best_policy)
            set_value(trial, rim_key, get_value(best_policy, rim_key) * rim_factor)
            set_value(trial, steal_key, get_value(best_policy, steal_key) * steal_factor)
            candidate_report, candidate_stdout = evaluate([trial])[0]
            if candidate_report is None:
                continue
            score = score_of(candidate_report)
            if score < best:
                best = score
                best_policy = trial
                persist(best_policy, candidate_stdout)
                print(
                    f"[联合] {rim_key}={rim_factor} {steal_key}={steal_factor} {describe(best)}",
                    flush=True,
                )
            if best[0] == 0.0:
                break

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
