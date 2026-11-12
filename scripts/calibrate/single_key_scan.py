#!/usr/bin/env python3
"""单键扫描：每次只改一个数值键，逐批落盘，失败条数优先。

从当前主参数表出发（2 条失败判据）。单键扰动只重洗与之相关的一段事件流，
比多键联合扰动更可能落到 1 条失败的盆地。每批 4 个键立即判定并把结果写进
JSONL；任何改善立即写回独立最优向量与主参数表。

用法：

    python3 -u scripts/calibrate/single_key_scan.py
"""

from __future__ import annotations

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
WORK = ROOT / ".work" / "evidence" / "single-key-scan"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-single-key-best.json"
RESULTS = ROOT / ".work" / "evidence" / "single-key-results.jsonl"
STATE = ROOT / ".work" / "evidence" / "single-key-state.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

BENCHMARK_FIXED = {
    "shot_aim_sigma_scale",
    "shot_phase_sigma_catch",
    "shot_phase_sigma_dribble",
    "shot_phase_sigma_drive",
    "shot_phase_sigma_post",
    "shot_phase_sigma_cut",
    "shot_phase_sigma_gather",
    "shot_phase_sigma_putback",
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
FACTORS = (0.90, 0.96, 0.99, 1.01, 1.04, 1.10)


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
    processes = []
    for index, policy in enumerate(candidates):
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
    results = []
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
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([copy.deepcopy(base)])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    RESULTS.write_text("", encoding="utf-8")
    STATE.write_text(
        json.dumps({"failed": best[0], "depth": best[1], "keys_done": 0}, indent=2) + "\n",
        encoding="utf-8",
    )
    print(f"基线：{describe(best)}", flush=True)
    print(
        "基线失败 " + ",".join(
            layer["criterion"] for layer in baseline["layers"] if not layer["passed"]
        ),
        flush=True,
    )

    keys = []
    for section in ("weights", "geometry"):
        for key, value in base[section].items():
            if key in BENCHMARK_FIXED:
                continue
            if not isinstance(value, (int, float)) or isinstance(value, bool):
                continue
            keys.append((key, value))
    print(f"数值键 {len(keys)} 个，每键 {len(FACTORS)} 个倍率", flush=True)

    deadline = time.monotonic() + 40 * 60.0
    keys_done = 0
    for key, value in keys:
        if time.monotonic() > deadline or best[0] == 0:
            break
        candidates = []
        labels = []
        for factor in FACTORS:
            trial = copy.deepcopy(base)
            set_value(trial, key, value * factor)
            candidates.append(trial)
            labels.append(factor)
        # 分批判定，每批立刻落盘。
        for start in range(0, len(candidates), WORKERS):
            batch = candidates[start : start + WORKERS]
            batch_labels = labels[start : start + WORKERS]
            for trial, factor, (report, stdout) in zip(
                batch, batch_labels, evaluate(batch)
            ):
                if report is None:
                    continue
                value_now = score(report)
                row = {
                    "key": key,
                    "factor": factor,
                    "failed": value_now[0],
                    "depth": value_now[1],
                    "failures": [
                        layer["criterion"] for layer in report["layers"] if not layer["passed"]
                    ],
                }
                with RESULTS.open("a", encoding="utf-8") as handle:
                    handle.write(json.dumps(row, ensure_ascii=False) + "\n")
                if value_now[0] <= 1:
                    print(
                        f"{key:38s} x{factor:.3f} {describe(value_now)} {row['failures']}",
                        flush=True,
                    )
                if value_now < best:
                    best = value_now
                    best_policy = trial
                    POLICY.write_text(
                        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n",
                        encoding="utf-8",
                    )
                    BEST_VECTOR.write_text(
                        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n",
                        encoding="utf-8",
                    )
                    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
                    print(f"改善 {key:38s} x{factor:.3f} {describe(best)}", flush=True)
        keys_done += 1
        if keys_done % 20 == 0:
            STATE.write_text(
                json.dumps(
                    {"failed": best[0], "depth": best[1], "keys_done": keys_done}, indent=2
                ) + "\n",
                encoding="utf-8",
            )
            print(f"已完成 {keys_done}/{len(keys)} 键，当前 {describe(best)}", flush=True)

    STATE.write_text(
        json.dumps(
            {"failed": best[0], "depth": best[1], "keys_done": keys_done}, indent=2
        ) + "\n",
        encoding="utf-8",
    )
    print(f"最终：{describe(best)}，扫描 {keys_done} 个键", flush=True)


if __name__ == "__main__":
    main()
