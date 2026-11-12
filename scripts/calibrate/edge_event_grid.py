#!/usr/bin/env python3
"""按最小事件量收尾两条越界判据。

`zone_share_rim` 只差一次近筐出手，`steals_per_game` 只差两次抢断。这里
对两条因果链的直接杠杆做细倍率扫描，随后对最优的近筐杠杆与抢断杠杆做
一次联合网格。评分始终来自权威门禁；最优向量写进独立文件，验证后再写回
主参数表。

用法：

    python3 -u scripts/calibrate/edge_event_grid.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "edge-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-edge-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "edge-grid-report.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

RIM_LEVERS = [
    "drive_base",
    "finish_min_distance_ft",
    "cut_lane_scale",
    "cut_tendency",
    "cut_positional_bonus",
    "shot_aim_sigma_scale",
    "shoot_putback_bonus",
    "finish_priority",
]
STEAL_LEVERS = [
    "steal_gamble_base",
    "pass_catch_steal",
    "steal_skill_base",
    "hand_check_rate",
    "reach_in_foul_rate",
    "bad_pass_rate",
    "bad_pass_skill_base",
    "gather_travel_rate",
    "off_ball_hold_rate",
    "bad_pass_iq_weight",
]
FACTORS = [0.97 + index * 0.0025 for index in range(25)]
PAIR_FACTORS = [0.995, 1.0, 1.005, 1.01, 1.015]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


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
        print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
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
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def describe(score):
    return f"深度 {score[0]:.6f}，失败 {score[1]} 条"


def accept(candidates, labels, best, best_policy, stage):
    results = evaluate(candidates)
    chosen = None
    chosen_stdout = None
    chosen_label = None
    for trial, label, (report, stdout) in zip(candidates, labels, results):
        if report is None:
            continue
        score = score_of(report)
        if score < best:
            best = score
            chosen = trial
            chosen_stdout = stdout
            chosen_label = label
    if chosen is None:
        return best, best_policy
    persist(chosen, chosen_stdout)
    print(f"[{stage}] {chosen_label} {describe(best)}", flush=True)
    return best, chosen


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    if report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score_of(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    ranking = []
    for key in RIM_LEVERS + STEAL_LEVERS:
        if best[0] == 0.0:
            break
        current = get_value(best_policy, key)
        candidates = []
        labels = []
        for factor in FACTORS:
            trial = copy.deepcopy(best_policy)
            set_value(trial, key, current * factor)
            candidates.append(trial)
            labels.append(f"{key}={current * factor:.6f}")
        before = best[0]
        best, candidate = accept(candidates, labels, best, best_policy, "细扫")
        if candidate is not None:
            best_policy = candidate
        ranking.append((key, before - best[0]))
        if best[0] == 0.0:
            break

    if best[0] > 0.0:
        rim_key = max((item for item in ranking if item[0] in RIM_LEVERS), key=lambda x: x[1])[0]
        steal_key = max(
            (item for item in ranking if item[0] in STEAL_LEVERS), key=lambda x: x[1]
        )[0]
        print(f"联合网格：{rim_key} × {steal_key}", flush=True)
        for rim_factor, steal_factor in itertools.product(PAIR_FACTORS, PAIR_FACTORS):
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
                    f"[联合] {rim_factor:.3f} × {steal_factor:.3f} {describe(best)}",
                    flush=True,
                )
            if best[0] == 0.0:
                break

    REPORT.write_text(
        json.dumps(
            {"score": {"depth": best[0], "failed": best[1]}, "ranking": ranking},
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
