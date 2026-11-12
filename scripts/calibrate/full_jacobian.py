#!/usr/bin/env python3
"""对当前主参数表的全部数值键测一次局部响应，解两条失败判据。

用法：

    python3 -u scripts/calibrate/full_jacobian.py
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
WORK = ROOT / ".work" / "evidence" / "full-jacobian"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-full-jacobian-best.json"
REPORT = ROOT / ".work" / "evidence" / "full-jacobian-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 外部出手基准夹具里会改变预测命中率的键；扰动它们会破坏基准测试。
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
PERTURB = 0.04
MIN_PERTURB = 0.002


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
        if start and (start // WORKERS) % 15 == 0:
            print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def errors(report, criteria):
    return {
        layer["criterion"]: layer["relative_error"]
        for layer in report["layers"]
        if layer["criterion"] in criteria
    }


def solve2(a, b, c, d, x, y):
    det = a * d - b * c
    if abs(det) < 1e-12:
        return None
    return ((x * d - b * y) / det, (a * y - x * c) / det)


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline_report, baseline_stdout = evaluate([base])[0]
    if baseline_report is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline_report)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)
    failures = [
        layer["criterion"] for layer in baseline_report["layers"] if not layer["passed"]
    ]
    print(f"失败判据 {failures}", flush=True)
    if len(failures) != 2:
        print("本脚本按两条失败判据设计，当前条数不符", flush=True)
    criteria = sorted(failures)
    base_errors = errors(baseline_report, set(criteria))
    targets = {name: -base_errors[name] for name in criteria}

    keys = []
    deltas = {}
    for section in ("weights", "geometry"):
        for key, value in base[section].items():
            if key in BENCHMARK_FIXED:
                continue
            if not isinstance(value, (int, float)) or isinstance(value, bool):
                continue
            keys.append(key)
            deltas[key] = max(abs(value) * PERTURB, MIN_PERTURB)
    print(f"数值键 {len(keys)} 个", flush=True)

    candidates = []
    for key in keys:
        trial = copy.deepcopy(base)
        set_value(trial, key, base["weights"].get(key, base["geometry"].get(key)) + deltas[key])
        candidates.append(trial)
    results = evaluate(candidates)

    responses = []
    for key, (report, _) in zip(keys, results):
        if report is None:
            continue
        candidate_errors = errors(report, criteria)
        row = {
            "key": key,
            "responses": {
                name: (candidate_errors.get(name, base_errors[name]) - base_errors[name])
                / deltas[key]
                for name in criteria
            },
            "score": list(score(report)),
            "delta": deltas[key],
        }
        responses.append(row)
        REPORT.write_text(
            json.dumps(
                {"base_errors": base_errors, "responses": responses},
                indent=2,
                ensure_ascii=False,
            ) + "\n",
            encoding="utf-8",
        )
        if row["score"][0] < best[0] or (
            row["score"][0] == best[0] and row["score"][1] < best[1]
        ):
            print(
                f"单键改善 {key:38s} {describe(tuple(row['score']))}",
                flush=True,
            )
    if not responses:
        raise SystemExit("没有可用的响应测量")

    # 每条判据取响应最强的键，按目标方向排序。
    targets = {name: -base_errors[name] for name in criteria}
    for name in criteria:
        ranked = sorted(
            responses,
            key=lambda row: -abs(row["responses"][name]),
        )[:8]
        print(f"--- {name} 目标 {targets[name]:+.4f}", flush=True)
        for row in ranked:
            print(
                f"  {row['key']:38s} 响应 {row['responses'][name]:+10.4f} "
                f"其它 " + " ".join(
                    f"{k}={v:+.3f}" for k, v in row["responses"].items() if k != name
                ),
                flush=True,
            )

    # 每对强响应键解 2×2 线性系统，按多个阻尼系数验证。
    top_keys = sorted(
        {
            row["key"]
            for name in criteria
            for row in sorted(responses, key=lambda r: -abs(r["responses"][name]))[:6]
        }
    )
    by_key = {row["key"]: row for row in responses}
    pairs = list(itertools.combinations(top_keys, 2))
    trials = []
    labels = []
    for left, right in pairs:
        matrix = [
            [by_key[left]["responses"][name], by_key[right]["responses"][name]]
            for name in criteria
        ]
        solution = solve2(
            matrix[0][0], matrix[0][1], matrix[1][0], matrix[1][1],
            targets[criteria[0]],
            targets[criteria[1]],
        )
        if solution is None:
            continue
        for damping in (0.4, 0.7, 1.0):
            trial = copy.deepcopy(base)
            for key, step in zip((left, right), solution):
                current = base["weights"].get(key, base["geometry"].get(key))
                set_value(trial, key, current + damping * step * deltas[key])
            trials.append(trial)
            labels.append(f"{left} x {right} @{damping:.2f}")
    print(f"线性系统候选 {len(trials)} 个", flush=True)
    for trial, label, (report, stdout) in zip(trials, labels, evaluate(trials)):
        if report is None:
            continue
        value = score(report)
        if value < best:
            best = value
            best_policy = trial
            print(f"{describe(value)} {label}", flush=True)
            BEST_VECTOR.write_text(
                json.dumps(trial, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
