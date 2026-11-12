#!/usr/bin/env python3
"""全参数细步长单键扫描，收尾 zone_share_rim。

从当前 1 条越界向量出发，逐键施加 ±0.5% 到 ±5% 的扰动。任何 0 条失败的
候选立即写回主参数表并结束；否则保留深度最小的改善。

用法：

    python3 -u scripts/calibrate/close_rim_full.py --minutes 25
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
WORK = ROOT / ".work" / "evidence" / "close-rim-full"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-close-rim-full-best.json"
STATE = ROOT / ".work" / "evidence" / "close-rim-full-state.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

BENCHMARK_FIXED = {
    "shot_aim_sigma_scale",
    "shot_phase_sigma_catch",
    "shot_phase_sigma_dribble",
    "shot_quality_skill_rim",
    "shot_quality_skill_near",
    "shot_quality_skill_mid",
    "shot_quality_skill_three",
    "shot_quality_skill_free_throw",
    "shot_skill_reference_rim",
    "shot_skill_reference_near",
    "shot_skill_reference_mid",
    "shot_skill_reference_three",
    "shot_rim_in_probability",
    "shot_rim_contact_band",
    "shot_distance_decay_three_scale_ft",
    "shot_distance_decay_three_power",
    "shot_fatigue_penalty",
}
FACTORS = (0.95, 0.98, 0.99, 0.995, 1.0, 1.005, 1.01, 1.02, 1.05)


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


def persist(policy, stdout, value, label):
    text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    POLICY.write_text(text, encoding="utf-8")
    BEST_VECTOR.write_text(text, encoding="utf-8")
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    STATE.write_text(
        json.dumps(
            {"failed": value[0], "depth": value[1], "label": label}, indent=2
        ) + "\n",
        encoding="utf-8",
    )


def main():
    parser = argparse.ArgumentParser(description="收尾 zone_share_rim 的全键扫描")
    parser.add_argument("--minutes", type=int, default=25)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    persist(best_policy, baseline_stdout, best, "baseline")
    print(f"基线：{describe(best)}", flush=True)

    keys = []
    for section in ("weights", "geometry"):
        for key, value in base[section].items():
            if key in BENCHMARK_FIXED:
                continue
            if not isinstance(value, (int, float)) or isinstance(value, bool):
                continue
            keys.append((key, value))
    print(f"数值键 {len(keys)} 个", flush=True)

    done = 0
    for key, current in keys:
        if time.monotonic() > deadline or best[0] == 0:
            break
        candidates = []
        labels = []
        for factor in FACTORS:
            trial = copy.deepcopy(base)
            set_value(trial, key, current * factor)
            candidates.append(trial)
            labels.append(f"{key}={current * factor:.6f}")
        for trial, label, (report, stdout) in zip(
            candidates, labels, evaluate(candidates)
        ):
            if report is None:
                continue
            value = score(report)
            if value[0] == 0:
                persist(trial, stdout, value, label)
                print(f"全通过 {label}", flush=True)
                print(f"最终 {describe(value)}", flush=True)
                return
            if value < best:
                best = value
                best_policy = trial
                persist(best_policy, stdout, best, label)
                print(f"改善 {label} {describe(best)}", flush=True)
        done += 1
        if done % 30 == 0:
            print(f"已完成 {done}/{len(keys)} 键，当前 {describe(best)}", flush=True)

    print(f"最终 {describe(best)}，扫描 {done} 个键", flush=True)


if __name__ == "__main__":
    main()
