#!/usr/bin/env python3
"""四条越界判据的局部响应求解（独立校准目录，不碰主参数表）。

对每条越界判据的直接因果杠杆测一次 +10% 扰动，记录四条判据的越界量变化，
得到 4×4 雅可比；解线性方程求把越界量消掉的倍率，按权威门禁验证后把
最优向量写到独立文件。

用法：

    python3 -u scripts/calibrate/solve_four_gates.py
"""

from __future__ import annotations

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
WORK = ROOT / ".work" / "evidence" / "four-gate-solve"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-four-gate-solve.json"
REPORT = ROOT / ".work" / "evidence" / "four-gate-solve-report.json"
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 局部响应实测后保留的三条活杠杆：cut_tendency 的 10% 扰动对四条
# 判据零响应，不能进雅可比；其余三条对四条判据有可测响应。
PAIRS = [
    ("creation_share_pull_up", "shoot_pull_up_bonus"),
    ("fta_per_game", "shooting_foul_rate_two_point"),
    ("steals_per_game", "steal_gamble_base"),
]
CRITERIA = [
    "creation_share_cut",
    "creation_share_pull_up",
    "fta_per_game",
    "steals_per_game",
]
STEP = 0.10


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    detail = layer["detail"]
    error = layer["relative_error"]
    if layer["passed"]:
        return 0.0
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


def signed_error(layer):
    """带符号的相对误差，便于求目标位移。"""
    return layer["relative_error"]


def vector(report):
    return {
        layer["criterion"]: signed_error(layer)
        for layer in report["layers"]
        if layer["criterion"] in CRITERIA
    }


def depth(report):
    return sum(abs(excess(layer)) for layer in report["layers"])


def score(report):
    return (
        depth(report),
        sum(1 for layer in report["layers"] if not layer["passed"]),
    )


def describe(value):
    return f"深度 {value[0]:.6f}，失败 {value[1]} 条"


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    calibration = WORK / "calibration"
    calibration.mkdir(parents=True)
    for path in CALIBRATION.glob("*.json"):
        shutil.copy2(path, calibration / path.name)
    (WORK / "tactics").symlink_to(TACTICS, target_is_directory=True)


def write_policy(policy):
    (WORK / "calibration" / "policy.json").write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


def gate(policy):
    write_policy(policy)
    output = subprocess.run(
        [str(BINARY), "--gate-only", "--calibration", str(WORK / "calibration")],
        cwd=ROOT, capture_output=True, text=True,
        preexec_fn=limit_address_space,
    ).stdout
    for line in output.splitlines():
        if line.startswith("GATE "):
            return json.loads(line[5:])
    return None


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


def solve_least_squares(jacobian, residual):
    """最小二乘正规方程：杠杆数少于判据数时也能给出最优位移。

    jacobian 每行是一个杠杆对全部判据的响应，regularization 压住病态解。
    """
    n = len(jacobian)
    normal = [[0.0] * n for _ in range(n)]
    right = [0.0] * n
    for column in range(len(residual)):
        column_values = [jacobian[row][column] for row in range(n)]
        for a in range(n):
            right[a] += column_values[a] * residual[column]
            for b in range(n):
                normal[a][b] += column_values[a] * column_values[b]
    for a in range(n):
        normal[a][a] += 0.01
    return solve_linear(normal, right)


def solve_linear(matrix, right):
    """高斯消元解 n×n 线性方程组。"""
    n = len(right)
    a = [list(matrix[i]) + [right[i]] for i in range(n)]
    for col in range(n):
        pivot = max(range(col, n), key=lambda r: abs(a[r][col]))
        if abs(a[pivot][col]) < 1e-12:
            return [0.0] * n
        a[col], a[pivot] = a[pivot], a[col]
        d = a[col][col]
        for j in range(col, n + 1):
            a[col][j] /= d
        for r in range(n):
            if r == col:
                continue
            f = a[r][col]
            for j in range(col, n + 1):
                a[r][j] -= f * a[col][j]
    return [a[i][n] for i in range(n)]


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    base_report = gate(base)
    assert base_report is not None
    base_vec = vector(base_report)
    best = score(base_report)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)
    print("基线误差 " + " ".join(f"{k}={base_vec[k]:+.4f}" for k in CRITERIA), flush=True)

    # 每个杠杆 +10% 扰动，测四条判据的响应。
    jacobian = []
    meta = []
    for criterion, key in PAIRS:
        current = get_value(base, key)
        trial = copy.deepcopy(base)
        set_value(trial, key, current * (1.0 + STEP))
        report = gate(trial)
        assert report is not None
        response = vector(report)
        jacobian.append([
            (response[c] - base_vec[c]) / STEP for c in CRITERIA
        ])
        meta.append((criterion, key, current))
        print(
            f"响应 {key:34s} " + " ".join(
                f"{c}={jacobian[-1][i]:+.4f}" for i, c in enumerate(CRITERIA)
            ),
            flush=True,
        )

    # 目标：把每条判据的带符号相对误差拉回带宽中心（0）。
    target = [-base_vec[c] for c in CRITERIA]
    deltas = solve_least_squares(jacobian, target)
    print("求解倍率 " + " ".join(f"{m[1]}={d:+.4f}" for m, d in zip(meta, deltas)), flush=True)

    for clip in (0.8, 0.6, 0.5, 0.4, 0.3, 0.2, 0.15, 0.10, 0.05):
        trial = copy.deepcopy(base)
        for (criterion, key, current), delta in zip(meta, deltas):
            set_value(trial, key, current * (1.0 + max(-clip, min(clip, delta))))
        report = gate(trial)
        if report is None:
            continue
        candidate = score(report)
        print(
            f"截断 {clip:.2f}：{describe(candidate)} " + " ".join(
                f"{c}={vector(report)[c]:+.4f}" for c in CRITERIA
            ),
            flush=True,
        )
        if candidate < best:
            best, best_policy = candidate, trial

    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    REPORT.write_text(
        json.dumps(
            {
                "base": {c: base_vec[c] for c in CRITERIA},
                "jacobian": jacobian,
                "meta": [{"criterion": m[0], "key": m[1], "value": m[2]} for m in meta],
                "deltas": deltas,
                "best": {"depth": best[0], "failed": best[1]},
            },
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    print(f"最优：{describe(best)}，候选向量 {BEST_VECTOR}", flush=True)


if __name__ == "__main__":
    main()
