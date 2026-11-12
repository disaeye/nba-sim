#!/usr/bin/env python3
"""收尾两条越界判据：按直接因果杠杆做有界细扫。

评分来自权威门禁：先比越界深度，再比失败条数。每个候选只改一个或两个
杠杆，全部评估后取全局最优并立即持久化。

用法：

    python3 -u scripts/calibrate/finish_band_edges.py --rounds 2
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "band-edges"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# 两条越界判据的直接因果杠杆：近筐出手与抢断。
LEVERS = [
    "drive_base",
    "finish_min_distance_ft",
    "cut_lane_scale",
    "cut_tendency",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "gather_travel_rate",
    "bad_pass_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "shooting_foul_rate_two_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "transition_take_window_seconds",
    "seconds_per_possession",
]
# 两条判据都只差极小的越界量，因此只扫当前值附近的细步长。
FACTORS = [0.94, 0.96, 0.98, 0.99, 0.995, 1.0, 1.005, 1.01, 1.02, 1.04, 1.06]


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


def main():
    parser = argparse.ArgumentParser(description="收尾两条越界判据")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    assert report is not None, "baseline gate produced no report"
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    for round_index in range(1, args.rounds + 1):
        if best[0] == 0.0:
            break
        improved = False
        for key in LEVERS:
            current = get_value(best_policy, key)
            candidates = []
            values = []
            for factor in FACTORS:
                value = current * factor
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, value)
                candidates.append(trial)
                values.append(value)
            results = evaluate(candidates)
            local_best = best
            chosen = None
            chosen_stdout = None
            chosen_value = None
            for trial, value, (candidate_report, candidate_stdout) in zip(
                candidates, values, results
            ):
                if candidate_report is None:
                    continue
                score = score_of(candidate_report)
                if score < local_best:
                    local_best = score
                    chosen = trial
                    chosen_stdout = candidate_stdout
                    chosen_value = value
            if chosen is not None:
                best = local_best
                best_policy = chosen
                persist(best_policy, chosen_stdout)
                improved = True
                print(
                    f"[轮 {round_index}] {key} -> {chosen_value:.6f} {describe(best)}",
                    flush=True,
                )
        print(f"[轮 {round_index}] 结束：{describe(best)}", flush=True)
        if not improved:
            break

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
