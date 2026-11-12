#!/usr/bin/env python3
"""在当前参数点做一次局部响应求解，收尾剩余越界判据。

每个直接因果杠杆只扰动一次，记录 39 条判据相对基线的变化；随后解一个带
正则的最小二乘，把越界判据拉回带宽内，同时尽量不扰动已经通过的判据。
求出的倍率经权威门禁验证，改善时立即写回主参数表。

用法：

    python3 -u scripts/calibrate/solve_remaining_gates.py --rounds 3
"""

from __future__ import annotations

import argparse
import copy
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
WORK = ROOT / ".work" / "evidence" / "gate-solve"
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
    "gather_travel_rate",
    "shooting_foul_rate_two_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "transition_take_window_seconds",
    "seconds_per_possession",
]
STEP = 0.06
CRITERIA_WEIGHTS = {
    # 越界判据权重相同；其余判据保留较小权重以抑制把它们推出带宽。
}


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


def errors(report):
    return {layer["criterion"]: excess(layer) for layer in report["layers"]}


def depth(vector):
    return sum(abs(value) for value in vector.values())


def score(report):
    vector = errors(report)
    return (
        depth(vector),
        sum(1 for value in vector.values() if abs(value) > 1e-9),
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


def solve_linear(matrix, right):
    n = len(right)
    augmented = [list(matrix[i]) + [right[i]] for i in range(n)]
    for column in range(n):
        pivot = max(range(column, n), key=lambda row: abs(augmented[row][column]))
        if abs(augmented[pivot][column]) < 1e-12:
            return [0.0] * n
        augmented[column], augmented[pivot] = augmented[pivot], augmented[column]
        divisor = augmented[column][column]
        for index in range(column, n + 1):
            augmented[column][index] /= divisor
        for row in range(n):
            if row == column:
                continue
            factor = augmented[row][column]
            for index in range(column, n + 1):
                augmented[row][index] -= factor * augmented[column][index]
    return [augmented[i][n] for i in range(n)]


def main():
    parser = argparse.ArgumentParser(description="局部响应求解剩余越界判据")
    parser.add_argument("--rounds", type=int, default=3)
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base])[0]
    assert report is not None, "baseline gate produced no report"
    best = score(report)
    best_vector = errors(report)
    best_policy = copy.deepcopy(base)
    persist(best_policy, stdout)
    print(f"基线：{describe(best)}", flush=True)

    for round_index in range(1, args.rounds + 1):
        if best[0] == 0.0:
            break
        candidates = []
        meta = []
        for key in LEVERS:
            current = get_value(best_policy, key)
            trial = copy.deepcopy(best_policy)
            set_value(trial, key, current * (1.0 + STEP))
            candidates.append(trial)
            meta.append((key, current))
        results = evaluate(candidates)
        criteria = sorted(best_vector)
        jacobian = {name: [0.0] * len(LEVERS) for name in criteria}
        for column, ((report, _), _) in enumerate(zip(results, meta)):
            if report is None:
                continue
            response = errors(report)
            for name in criteria:
                jacobian[name][column] = (
                    response.get(name, 0.0) - best_vector[name]
                ) / STEP

        # 目标是把每个越界量消掉；越界判据权重 1，已通过判据权重 0.2。
        target = {name: -best_vector[name] for name in criteria}
        weights = {
            name: CRITERIA_WEIGHTS.get(name, 1.0 if abs(best_vector[name]) > 1e-9 else 0.2)
            for name in criteria
        }
        n = len(LEVERS)
        normal = [[0.0] * n for _ in range(n)]
        right = [0.0] * n
        for name in criteria:
            row = jacobian[name]
            weight = weights[name]
            for a in range(n):
                right[a] += weight * row[a] * target[name]
                for b in range(n):
                    normal[a][b] += weight * row[a] * row[b]
        for a in range(n):
            normal[a][a] += 0.10
        solution = solve_linear(normal, right)

        trial = copy.deepcopy(best_policy)
        for (key, current), delta in zip(meta, solution):
            clipped = max(-0.20, min(0.20, delta))
            set_value(trial, key, current * (1.0 + clipped))
        result, result_stdout = evaluate([trial])[0]
        if result is None:
            print(f"[轮 {round_index}] 门禁没有输出报告", flush=True)
            break
        candidate_score = score(result)
        improved = candidate_score < best
        if improved:
            best = candidate_score
            best_vector = errors(result)
            best_policy = trial
            persist(best_policy, result_stdout)
        print(f"[轮 {round_index}] {describe(best)}", flush=True)
        if not improved:
            break

    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
