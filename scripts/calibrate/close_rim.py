#!/usr/bin/env python3
"""收尾 zone_share_rim 的细步长扫描。

当前批次需要近筐出手占比从 0.259 升到至少 0.268（带宽下限）。
仅扫描会改变近筐/中距离/三分出手选择的策略杠杆；出手模型保真参数
保持不动，其余参数逐位不变。评分按「失败条数、越界深度」。

用法：

    python3 -u scripts/calibrate/close_rim.py
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
WORK = ROOT / ".work" / "evidence" / "close-rim"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-close-rim-best.json"
REPORT = ROOT / ".work" / "evidence" / "close-rim-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

LEVERS = [
    "finish_priority",
    "finish_min_distance_ft",
    "drive_base",
    "drive_kick_open_defender_ft",
    "cut_open_defender_ft",
    "cut_lane_scale",
    "cut_tendency",
    "cut_open_distance_ft",
    "shoot_pull_up_bonus",
    "shoot_catch_bonus",
    "shoot_putback_bonus",
    "shoot_near_bonus",
    "shoot_mid_bonus",
    "shoot_expected_points",
    "shoot_pressure",
    "post_base",
    "seconds_per_possession",
    "possession_setup_seconds",
    "transition_window_seconds",
    "transition_take_window_seconds",
]
FACTORS = (0.94, 0.97, 0.985, 0.995, 1.0, 1.005, 1.015, 1.03, 1.06)


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
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (WORK / "gate-best.log").write_text(baseline_stdout, encoding="utf-8")
    print(f"基线：{describe(best)}", flush=True)
    for layer in baseline["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)

    rows = []
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
        for trial, label, (report, stdout) in zip(
            candidates, labels, evaluate(candidates)
        ):
            if report is None:
                continue
            value = score(report)
            by_name = {layer["criterion"]: layer for layer in report["layers"]}
            rows.append(
                {
                    "label": label,
                    "failed": value[0],
                    "depth": value[1],
                    "rim": by_name["zone_share_rim"]["observed"],
                    "rim_error": by_name["zone_share_rim"]["relative_error"],
                    "failures": [
                        layer["criterion"] for layer in report["layers"] if not layer["passed"]
                    ],
                }
            )
            if value[0] == 0:
                print(f"全通过 {label}", flush=True)
            if value < best:
                best = value
                best_policy = trial
                text = json.dumps(trial, indent=2, ensure_ascii=False) + "\n"
                POLICY.write_text(text, encoding="utf-8")
                BEST_VECTOR.write_text(text, encoding="utf-8")
                (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
                print(f"改善 {label} {describe(best)}", flush=True)
        print(f"完成 {key:38s} 当前 {describe(best)}", flush=True)

    rows.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for row in rows[:12]:
        print(
            f"{row['failed']} 条 深度 {row['depth']:.4f} rim={row['rim']:.4f} "
            f"err={row['rim_error']:+.4f} {row['label']} {row['failures'][:3]}",
            flush=True,
        )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
