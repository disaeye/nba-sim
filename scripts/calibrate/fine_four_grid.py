#!/usr/bin/env python3
"""四条越界判据的细网格：四条已测活杠杆的小幅组合。

`measure_four_responses.py` 给出了单调区间：`steal_gamble_base` 在
1.0..1.1 改善深度，`steal_skill_base` 在 1.0..1.1 清掉抢断判据，
`shooting_foul_rate_two_point` 与 `shoot_pull_up_bonus` 只需极小幅度
补偿 FTA 与创造方式份额。这里对四个倍率做 3^4 网格，评分来自权威门禁，
最优向量写进独立文件。

用法：

    python3 -u scripts/calibrate/fine_four_grid.py
"""

from __future__ import annotations

import copy
import itertools
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
WORK = ROOT / ".work" / "evidence" / "fine-four-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-fine-four-grid.json"
REPORT = ROOT / ".work" / "evidence" / "fine-four-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

KEYS = [
    "steal_gamble_base",
    "steal_skill_base",
    "shooting_foul_rate_two_point",
    "shoot_pull_up_bonus",
]
FACTORS = [
    (1.0, 1.03, 1.06, 1.09),
    (1.0, 1.04, 1.08),
    (0.97, 0.99, 1.0),
    (0.97, 0.99, 1.0),
]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


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
        return max(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return abs(min(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6)))
    return abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)


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


def build(base, factors):
    policy = copy.deepcopy(base)
    for key, factor in zip(KEYS, factors):
        found = False
        for section in ("weights", "geometry"):
            if key in policy[section]:
                policy[section][key] *= factor
                found = True
                break
        if not found:
            raise KeyError(key)
    return policy


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    combinations = list(itertools.product(*FACTORS))
    print(f"候选组合 {len(combinations)} 个", flush=True)
    policies = [build(base, factors) for factors in combinations]
    results = evaluate(policies)

    ranked = []
    for factors, policy, (report, stdout) in zip(combinations, policies, results):
        if report is None:
            continue
        ranked.append((score(report), factors, policy, stdout, report))
    ranked.sort(key=lambda row: row[0])
    if not ranked:
        raise SystemExit("没有可用结果")
    for value, factors, _, _, _ in ranked[:10]:
        print(f"{describe(value)} " + " ".join(f"{f:.3f}" for f in factors), flush=True)

    best, factors, policy, stdout, report = ranked[0]
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    REPORT.write_text(
        json.dumps(
            {
                "best": {"depth": best[0], "failed": best[1], "factors": factors},
                "top": [
                    {"depth": value[0], "failed": value[1], "factors": row_factors}
                    for value, row_factors, _, _, _ in ranked[:20]
                ],
            },
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    print(f"最优：{describe(best)} 倍率 {factors}", flush=True)
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)


if __name__ == "__main__":
    main()
