#!/usr/bin/env python3
"""按局部雅可比矩阵求解门禁参数。

在当前参数点附近测量每个杠杆对 39 条判据的响应方向（雅可比），再解一个
带正则的最小二乘：把所有越界判据拉回带宽内，同时尽量不扰动已经通过的
判据。每轮用权威门禁验证，并迭代。

用法：

    python3 -u scripts/calibrate/solve_gates.py --rounds 8
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
WORK = ROOT / ".work" / "evidence" / "jacobian"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

# 与三条越界判据有直接因果链，且响应方向各异的杠杆。
LEVERS = [
    "transition_window_seconds",
    "transition_take_window_seconds",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "pass_catch_steal",
    "steal_gamble_base",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "cut_lane_scale",
    "shot_aim_sigma_scale",
    "seconds_per_possession",
]
STEP = 0.08  # 每个杠杆的局部扰动比例


def layer_excess(layer):
    """判据的带符号越界量；通过时为零。"""
    detail = layer["detail"]
    observed = layer["observed"]
    if detail.startswith("relative error"):
        text = detail.split("band ")[1].rstrip(")")
        low, high = (float(v) for v in text.split(".."))
        error = layer["relative_error"]
        return max(low - error, 0.0) * -1.0 + max(error - high, 0.0)
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return max(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return min(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("区间"):
        text = detail.removeprefix("区间 ")
        low, high = (float(v) for v in text.split(".."))
        return (
            max(low - observed, 0.0) - max(observed - high, 0.0)
        ) / max(abs(high - low), 1e-6)
    return 0.0


def errors(report):
    return {l["criterion"]: layer_excess(l) for l in report["layers"]}


def depth(report):
    return sum(abs(v) for v in errors(report).values())


def failing(report):
    return [l["criterion"] for l in report["layers"] if not l["passed"]]


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
            if report is None:
                results.append(({}, 1e6, output))
            else:
                results.append((errors(report), depth(report), output))
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


def solve_linear(matrix, right):
    """高斯消元解 n×n 线性方程组。"""
    n = len(right)
    augmented = [list(matrix[i]) + [right[i]] for i in range(n)]
    for column in range(n):
        pivot = max(range(column, n), key=lambda r: abs(augmented[r][column]))
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
    parser = argparse.ArgumentParser(description="按局部雅可比求解门禁参数")
    parser.add_argument("--rounds", type=int, default=8)
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    best_result = evaluate([base])[0]
    best = (len(failing_from_errors(best_result[0])), best_result[1])
    best_policy = copy.deepcopy(base)
    best_stdout = best_result[2]
    print(f"基线：越界 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    for round_index in range(1, args.rounds + 1):
        if best[0] == 0:
            break
        # 测量雅可比：每个杠杆上下各扰动一次。
        candidates = []
        meta = []
        for key in LEVERS:
            current = get_value(best_policy, key)
            for sign in (1.0, -1.0):
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, current * (1.0 + sign * STEP))
                candidates.append(trial)
                meta.append((key, sign))
        measured = evaluate(candidates)
        criteria = sorted(best_result[0])
        baseline_vector = best_result[0]
        jacobian = {name: {} for name in criteria}
        for (key, sign), (vector, _, _) in zip(meta, measured):
            for name in criteria:
                # 响应是「扰动后的越界量减去基线越界量」，不是绝对越界量。
                response = vector.get(name, 0.0) - baseline_vector[name]
                jacobian[name][key] = (
                    jacobian[name].get(key, 0.0)
                    + sign * response / (2.0 * STEP)
                )
        # 目标：把当前越界量消掉；已通过的判据目标为零变化。
        target = {name: -best_result[0][name] for name in criteria}
        weights = {name: 1.0 if abs(best_result[0][name]) > 1e-9 else 0.25 for name in criteria}

        n = len(LEVERS)
        normal = [[0.0] * n for _ in range(n)]
        right = [0.0] * n
        for name in criteria:
            row = [jacobian[name].get(key, 0.0) for key in LEVERS]
            weight = weights[name]
            for a in range(n):
                right[a] += weight * row[a] * target[name]
                for b in range(n):
                    normal[a][b] += weight * row[a] * row[b]
        for a in range(n):
            normal[a][a] += 0.05
        solution = solve_linear(normal, right)

        trial = copy.deepcopy(best_policy)
        for key, delta in zip(LEVERS, solution):
            clipped = max(-0.25, min(0.25, delta))
            current = get_value(best_policy, key)
            set_value(trial, key, current * (1.0 + clipped))
        result = evaluate([trial])[0]
        score = (len(failing_from_errors(result[0])), result[1])
        if score < best:
            best = score
            best_policy = trial
            best_result = result
            best_stdout = result[2]
            POLICY.write_text(
                json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n",
                encoding="utf-8",
            )
            print(
                f"[轮 {round_index}] 越界 {best[0]} 条，深度 {best[1]:.6f}",
                flush=True,
            )
        else:
            print(f"[轮 {round_index}] 无改进，保持 {best[0]} 条 / {best[1]:.6f}", flush=True)
            break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(best_stdout, encoding="utf-8")
    print(f"最终：越界 {best[0]} 条，深度 {best[1]:.6f}", flush=True)


def failing_from_errors(vector):
    return [name for name, value in vector.items() if abs(value) > 1e-9]


if __name__ == "__main__":
    main()
