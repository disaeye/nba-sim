#!/usr/bin/env python3
"""从 pass_catch_base 候选出发，收尾两条越界判据。

`zero_key_grid.py` 找到 `pass_catch_base=0.45` 与
`shoot_pull_up_bonus=0.18` 时只剩 `creation_share_cut` 与
`and_one_per_game` 越界。这里在 `and_one_rate`、`cut_lane_scale`、
`cut_tendency` 上做细网格，评分来自权威门禁。

用法：

    python3 -u scripts/calibrate/close_final_two.py
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
WORK = ROOT / ".work" / "evidence" / "close-final-two"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-close-final-two-best.json"
REPORT = ROOT / ".work" / "evidence" / "close-final-two-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 绝对值：cut_tendency 是零附近的小权重，乘法扰动推不动。
GRID = {
    "and_one_rate": (0.19, 0.21, 0.23, 0.25, 0.27, 0.30),
    "cut_lane_scale": (0.32, 0.38, 0.44, 0.50, 0.56),
    "cut_tendency": (0.02, 0.03, 0.045, 0.06),
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
        if start and (start // WORKERS) % 10 == 0:
            print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def build(base, values):
    policy = copy.deepcopy(base)
    for key, value in values.items():
        found = False
        for section in ("weights", "geometry"):
            if key in policy[section]:
                policy[section][key] = value
                found = True
                break
        if not found:
            raise KeyError(key)
    return policy


def main():
    prepare()
    base = json.loads(BASE.read_text(encoding="utf-8"))
    keys = list(GRID)
    combos = list(itertools.product(*(GRID[key] for key in keys)))
    print(f"候选 {len(combos)} 个", flush=True)
    policies = [build(base, dict(zip(keys, combo))) for combo in combos]
    results = evaluate(policies)

    ranked = []
    for combo, policy, (report, stdout) in zip(combos, policies, results):
        if report is None:
            continue
        ranked.append((score(report), combo, policy, stdout, report))
    ranked.sort(key=lambda row: row[0])
    for value, combo, _, _, _ in ranked[:15]:
        print(f"{describe(value)} " + " ".join(f"{v:.3f}" for v in combo), flush=True)

    REPORT.write_text(
        json.dumps(
            [
                {
                    "failed": value[0],
                    "depth": value[1],
                    "values": dict(zip(keys, combo)),
                    "failures": [
                        layer["criterion"] for layer in report["layers"] if not layer["passed"]
                    ],
                }
                for value, combo, _, _, report in ranked
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    best, combo, policy, stdout, report = ranked[0]
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    print(f"最优 {describe(best)} 组合 {combo}", flush=True)
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)


if __name__ == "__main__":
    main()
