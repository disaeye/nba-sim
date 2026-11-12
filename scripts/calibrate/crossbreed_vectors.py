#!/usr/bin/env python3
"""已知参数向量之间的插值与交叉，跨过离散响应面。

局部扰动在地址抽样的离散事件流上会卡在局部极小；两个已知低分向量之间的
插值与按键交叉能一次跳到新的盆地。每个候选都由权威门禁判定，评分按
「失败条数、越界深度」。出手模型基准敏感键固定为主参数表的值，保证
`shot_geometry_benchmark` 始终通过。

用法：

    python3 -u scripts/calibrate/crossbreed_vectors.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import random
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "crossbreed"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-crossbreed-best.json"
REPORT = ROOT / ".work" / "evidence" / "crossbreed-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

SOURCES = [
    ".work/evidence/policy-converged.json",
    ".work/evidence/policy-zero-key-grid-best.json",
    ".work/evidence/policy-gauss-newton-best.json",
    ".work/evidence/policy-coordinate-two-best.json",
    ".work/evidence/policy-final-refit-best.json",
    ".work/evidence/policy-refit-best.json",
    ".work/evidence/policy-final-solution.json",
    ".work/evidence/policy-before-band-edges.json",
    ".work/evidence/policy-harvest-best.json",
]
# 外部出手基准夹具里会改变预测命中率的键，统一取主参数表的值。
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
    "shoot_off_dribble_penalty",
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
        print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def normalize(policy, current):
    result = copy.deepcopy(policy)
    for section in ("weights", "geometry"):
        result.setdefault(section, {})
        for key, value in current[section].items():
            result[section].setdefault(key, value)
        for key in BENCHMARK_FIXED:
            if key in current[section]:
                result[section][key] = current[section][key]
    result["provenance"] = current["provenance"]
    result["weights"]["trap_protection_bonus"] = current["weights"]["trap_protection_bonus"]
    return result


def load_vectors(current):
    vectors = []
    for relative in SOURCES:
        path = ROOT / relative
        if not path.is_file():
            continue
        policy = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(policy, dict) or "weights" not in policy:
            continue
        vectors.append((relative, normalize(policy, current)))
    return vectors


def interpolate(left, right, alpha):
    result = copy.deepcopy(left)
    for section in ("weights", "geometry"):
        for key, value in left[section].items():
            other = right[section].get(key)
            if isinstance(value, (int, float)) and isinstance(other, (int, float)):
                result[section][key] = value * (1.0 - alpha) + other * alpha
    return result


def crossover(left, right, rng):
    result = copy.deepcopy(left)
    for section in ("weights", "geometry"):
        for key, value in right[section].items():
            if key in BENCHMARK_FIXED or key not in left[section]:
                continue
            if rng.random() < 0.5:
                result[section][key] = value
    return result


def main():
    prepare()
    current = json.loads(POLICY.read_text(encoding="utf-8"))
    vectors = load_vectors(current)
    print(f"已知向量 {len(vectors)} 份", flush=True)

    # 先逐个重判，找出当前二进制下的真实最优起点。
    baseline_results = evaluate([policy for _, policy in vectors])
    ranked = []
    for (name, policy), (report, _) in zip(vectors, baseline_results):
        if report is None:
            continue
        ranked.append((score(report), name, policy))
    ranked.sort(key=lambda row: row[0])
    for value, name, _ in ranked:
        print(f"{describe(value)} {name}", flush=True)

    top = ranked[:5]
    rng = random.Random(20261121)
    candidates = []
    labels = []
    for (_, left_name, left), (_, right_name, right) in itertools.combinations(top, 2):
        for alpha in (0.25, 0.5, 0.75):
            candidates.append(interpolate(left, right, alpha))
            labels.append(f"插值 {left_name} x {right_name} @{alpha:.2f}")
        for _ in range(3):
            candidates.append(crossover(left, right, rng))
            labels.append(f"交叉 {left_name} x {right_name}")
    print(f"插值与交叉候选 {len(candidates)} 个", flush=True)
    results = evaluate(candidates)

    combined = ranked[:]
    for label, policy, (report, stdout) in zip(labels, candidates, results):
        if report is None:
            continue
        combined.append((score(report), label, policy))
        if score(report)[0] <= 1:
            print(f"{describe(score(report))} {label}", flush=True)
    combined.sort(key=lambda row: row[0])
    best, label, best_policy = combined[0]
    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    REPORT.write_text(
        json.dumps(
            [
                {"failed": value[0], "depth": value[1], "label": name}
                for value, name, _ in combined[:30]
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    print(f"最优 {describe(best)} {label}", flush=True)


if __name__ == "__main__":
    main()
