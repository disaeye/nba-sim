#!/usr/bin/env python3
"""39 条门禁的阻尼高斯-牛顿求解。

响应面由地址抽样的离散事件构成：一步线性化在外推时会失效，单杠杆扫描
又会卡在局部极小。每轮重新测量全部直接因果杠杆的小扰动雅可比，解带
正则的最小二乘，把越界判据拉回带宽中心、同时抑制已通过判据的移动；
再并行评估多个阻尼系数，只接受权威门禁分数的真实改善。

搜索只写独立最优向量文件，结束时由调用方决定是否写回主参数表。

用法：

    python3 -u scripts/calibrate/gauss_newton_final.py --rounds 8
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
WORK = ROOT / ".work" / "evidence" / "gauss-newton"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-gauss-newton-best.json"
REPORT = ROOT / ".work" / "evidence" / "gauss-newton-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 响应表实测活跃、且外部出手基准夹具里恒等为 1 或完全不进入的杠杆。
LEVERS = [
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "shoot_off_dribble_penalty",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "steal_gamble_base",
    "steal_skill_base",
    "pass_catch_steal",
    "cut_lane_scale",
    "cut_tendency",
    "drive_base",
    "shot_quality_contest_sensitivity",
    "shot_fatigue_penalty",
    "shot_gather_sigma_scale",
    "trap_protection_bonus",
]
# 每个杠杆的小扰动：相对 5% 或至少 0.02 绝对值（零值键也能动）。
PERTURB_FRACTION = 0.05
MIN_PERTURB = 0.02
MAX_STEP_UNITS = 3.0
DAMPINGS = (0.3, 0.6, 1.0, 1.5)
REGULARIZATION = 0.25


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def signed_error(layer):
    """把每条判据归一到「0 在带宽中心、越界时带符号」的量。"""
    detail = layer["detail"]
    observed = layer["observed"]
    target = layer["target"]
    if detail.startswith("relative error"):
        return layer["relative_error"]
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        center = (low + high) * 0.5
        return (observed - center) / max(abs(center), 1e-6)
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return (observed - bound) / max(abs(bound), 1e-6)
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return (observed - bound) / max(abs(bound), 1e-6)
    return (observed - target) / max(abs(target), 1e-6)


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
        return max(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return abs(min(0.0, (observed - bound) / max(abs(bound), 1e-6)))
    return abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)


def errors(report):
    return [signed_error(layer) for layer in report["layers"]]


def score(report):
    depth = sum(excess(layer) for layer in report["layers"])
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth, failed)


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


def evaluate(candidates, deadline=None):
    results = []
    for start in range(0, len(candidates), WORKERS):
        if deadline is not None and time.monotonic() > deadline:
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


def perturb(base, key, delta):
    trial = copy.deepcopy(base)
    set_value(trial, key, get_value(base, key) + delta)
    return trial


def solve_linear(matrix, right):
    n = len(right)
    a = [list(matrix[i]) + [right[i]] for i in range(n)]
    for col in range(n):
        pivot = max(range(col, n), key=lambda row: abs(a[row][col]))
        if abs(a[pivot][col]) < 1e-12:
            return [0.0] * n
        a[col], a[pivot] = a[pivot], a[col]
        divisor = a[col][col]
        for index in range(col, n + 1):
            a[col][index] /= divisor
        for row in range(n):
            if row == col:
                continue
            factor = a[row][col]
            for index in range(col, n + 1):
                a[row][index] -= factor * a[col][index]
    return [a[i][n] for i in range(n)]


def solve_damped(jacobian, residual):
    """带正则的最小二乘：step 单位是各杠杆的小扰动 delta。"""
    n = len(jacobian)
    normal = [[0.0] * n for _ in range(n)]
    right = [0.0] * n
    for row, target in enumerate(residual):
        column = [jacobian[lever][row] for lever in range(n)]
        for a in range(n):
            right[a] += column[a] * target
            for b in range(n):
                normal[a][b] += column[a] * column[b]
    for a in range(n):
        normal[a][a] += REGULARIZATION
    return solve_linear(normal, right)


def main():
    parser = argparse.ArgumentParser(description="39 条门禁的阻尼高斯-牛顿求解")
    parser.add_argument("--rounds", type=int, default=8)
    parser.add_argument("--minutes", type=int, default=25)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))

    base_report, base_stdout = evaluate([base])[0]
    if base_report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(base_report)
    best_policy = copy.deepcopy(base)
    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"基线：{describe(best)}", flush=True)

    rounds = []
    for round_index in range(1, args.rounds + 1):
        if best[0] == 0.0 or time.monotonic() > deadline:
            break
        current = best_policy
        base_errors = errors(evaluate([current])[0][0])
        deltas = []
        for key in LEVERS:
            value = get_value(current, key)
            deltas.append(max(abs(value) * PERTURB_FRACTION, MIN_PERTURB))

        measurements = evaluate(
            [perturb(current, key, delta) for key, delta in zip(LEVERS, deltas)],
            deadline,
        )
        jacobian = []
        for (report, _), delta in zip(measurements, deltas):
            if report is None:
                jacobian.append([0.0] * len(base_errors))
                continue
            response = errors(report)
            jacobian.append(
                [(response[i] - base_errors[i]) / delta for i in range(len(base_errors))]
            )

        # 越界判据目标是回到中心；已通过判据目标是不动。
        base_report = evaluate([current])[0][0]
        residual = [
            -signed_error(layer) if not layer["passed"] else 0.0
            for layer in base_report["layers"]
        ]
        step = solve_damped(jacobian, residual)
        step = [max(-MAX_STEP_UNITS, min(MAX_STEP_UNITS, s)) for s in step]

        candidates = []
        labels = []
        for damping in DAMPINGS:
            trial = copy.deepcopy(current)
            for key, delta, amount in zip(LEVERS, deltas, step):
                set_value(trial, key, get_value(current, key) + damping * amount * delta)
            candidates.append(trial)
            labels.append(f"阻尼 {damping:.2f}")

        round_best = best
        for trial, label, (report, stdout) in zip(
            candidates, labels, evaluate(candidates, deadline)
        ):
            if report is None:
                continue
            candidate = score(report)
            if candidate < round_best:
                round_best = candidate
                best_policy = trial
                BEST_VECTOR.write_text(
                    json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n",
                    encoding="utf-8",
                )
                print(f"[轮 {round_index}] {label} {describe(candidate)}", flush=True)
        improved = round_best < best
        best = round_best
        rounds.append({"round": round_index, "depth": best[0], "failed": best[1]})
        print(f"[轮 {round_index}] 结束 {describe(best)}", flush=True)
        if not improved:
            break

    REPORT.write_text(
        json.dumps(rounds, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"最终：{describe(best)}，最优向量 {BEST_VECTOR}", flush=True)


if __name__ == "__main__":
    main()
