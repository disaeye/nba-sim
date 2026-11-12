#!/usr/bin/env python3
"""用当前内核重新汇总全部已保存参数向量，保留真实最优解。

外部出手基准夹具只覆盖固定能力、无防守、静止、满体能、Catch/Dribble 两种
阶段；所有向量里会影响这个夹具的出手模型键统一取当前主参数表的值。其余
已调好的分布杠杆来自历史向量，新增策略键从当前主参数表补齐。评分由当前
`sim2 --gate-only` 给出，最优向量写回主参数表。

用法：

    python3 -u scripts/calibrate/harvest_current_binary.py
"""

from __future__ import annotations

import copy
import hashlib
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
WORK = ROOT / ".work" / "evidence" / "current-binary-harvest"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-current-binary-best.json"
REPORT = ROOT / ".work" / "evidence" / "current-binary-harvest-report.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 这些键在外部出手基准夹具里会改变预测命中率，统一取当前基准验证过的值。
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


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
            observed = layer["observed"]
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(layer["observed"] - layer["target"]) / max(
                abs(layer["target"]), 1e-6
            )
    return total


def score_of(report):
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    # 失败条数优先：2 条失败的向量比 4 条失败的向量更接近最终验收。
    return (failed, depth_of(report))


def describe(score):
    return f"失败 {score[0]} 条，深度 {score[1]:.6f}"


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


def normalize(candidate, current):
    result = copy.deepcopy(current)
    for section in ("weights", "geometry"):
        source = candidate.get(section, {})
        for key, value in source.items():
            if key in BENCHMARK_FIXED or key not in result[section]:
                continue
            if not isinstance(value, (int, float)) or isinstance(value, bool):
                continue
            result[section][key] = value
    result["provenance"] = current["provenance"]
    # 被包夹护球先验固定为已经验证过生产证据的值，避免搜索把生产链关掉。
    result["weights"]["trap_protection_bonus"] = current["weights"][
        "trap_protection_bonus"
    ]
    return result


def collect(current):
    paths = set([str(POLICY)])
    paths.update(str(path) for path in (ROOT / ".work").rglob("policy*.json"))
    paths.update(
        str(path)
        for path in (ROOT / ".work").rglob("*/calibration/policy.json")
    )
    vectors = {}
    for text_path in sorted(paths):
        path = Path(text_path)
        if str(WORK) in str(path):
            continue
        candidate = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(candidate, dict):
            continue
        if "weights" not in candidate or "geometry" not in candidate:
            continue
        normalized = normalize(candidate, current)
        digest = hashlib.sha256(
            json.dumps(normalized, sort_keys=True).encode("utf-8")
        ).hexdigest()[:12]
        vectors.setdefault(digest, (str(path), normalized))
    return list(vectors.items())


def main():
    prepare()
    current = json.loads(POLICY.read_text(encoding="utf-8"))
    vectors = collect(current)
    print(f"唯一参数向量 {len(vectors)} 个", flush=True)
    results = evaluate([policy for _, (_, policy) in vectors])

    ranked = []
    for (digest, (path, policy)), (report, stdout) in zip(vectors, results):
        if report is None:
            print(f"跳过 {digest} {path}：门禁没有输出报告", flush=True)
            continue
        ranked.append((score_of(report), digest, path, policy, stdout, report))
    ranked.sort(key=lambda row: row[0])
    if not ranked:
        raise SystemExit("没有可用参数向量")

    REPORT.write_text(
        json.dumps(
            [
                {
                    "digest": digest,
                    "path": path,
                    "depth": score[1],
                    "failed": score[0],
                    "failures": [
                        layer["criterion"]
                        for layer in report["layers"]
                        if not layer["passed"]
                    ],
                }
                for score, digest, path, _, _, report in ranked
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    for score, digest, path, _, _, _ in ranked[:10]:
        print(f"{describe(score)} {digest} {path}", flush=True)

    best, digest, path, best_policy, best_stdout, report = ranked[0]
    BEST_VECTOR.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(best_stdout, encoding="utf-8")
    print(f"已保留 {digest} {path}：{describe(best)}", flush=True)
    for layer in report["layers"]:
        if not layer["passed"]:
            print(
                f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}",
                flush=True,
            )


if __name__ == "__main__":
    main()
