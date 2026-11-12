#!/usr/bin/env python3
"""从 2 条越界向量出发的纯一维扫描（每次只动一个键）。

用法：

    python3 -u scripts/calibrate/one_dimensional_two.py
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
WORK = ROOT / ".work" / "evidence" / "one-dimensional-two"
BINARY = ROOT / "target" / "release" / "sim2"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-one-dimensional-two-best.json"
REPORT = ROOT / ".work" / "evidence" / "one-dimensional-two-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 每个键给绝对值候选：2 条越界向量的键都在这里保持精确基线值。
SCANS = {
    "and_one_rate": [0.193665, 0.22, 0.25, 0.28, 0.32, 0.36, 0.42, 0.50, 0.60],
    "shooting_foul_rate_two_point": [0.16, 0.18, 0.185948, 0.20, 0.22, 0.25, 0.28],
    "cut_lane_scale": [0.30, 0.34, 0.36, 0.383707, 0.40, 0.42, 0.46, 0.52],
    "cut_tendency": [0.01, 0.02, 0.03, 0.034476, 0.04, 0.05, 0.07],
}


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


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def main():
    prepare()
    base = json.loads(BASE.read_text(encoding="utf-8"))
    baseline_report, _ = evaluate([base])[0]
    best = score(baseline_report)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)

    rows = []
    for key, values in SCANS.items():
        candidates = []
        for value in values:
            trial = copy.deepcopy(base)
            set_value(trial, key, value)
            candidates.append(trial)
        for value, (report, stdout) in zip(values, evaluate(candidates)):
            if report is None:
                continue
            candidate = score(report)
            rows.append({"key": key, "value": value, "failed": candidate[0], "depth": candidate[1]})
            print(f"{key:34s} {value:.6f} {describe(candidate)}", flush=True)
            if candidate < best:
                best, best_policy = candidate, copy.deepcopy(candidates[values.index(value)])
                BEST_VECTOR.write_text(
                    json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n",
                    encoding="utf-8",
                )
                (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")

    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    print(f"最优：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
