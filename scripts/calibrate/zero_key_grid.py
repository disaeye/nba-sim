#!/usr/bin/env python3
"""四条越界判据在「零值键」上的有界网格。

乘法搜索永远动不了 `shoot_catch_bonus`、`shoot_off_dribble_penalty` 这类
零值权重；这里给它们绝对数值，同时扫描能直接改变抢断归属的接球阈值与
出手创造方式权重。评分来自权威门禁。

用法：

    python3 -u scripts/calibrate/zero_key_grid.py
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
WORK = ROOT / ".work" / "evidence" / "zero-key-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "zero-key-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 键 → 绝对值候选。零值键用绝对值，其余键给能改变离散事件的档位。
GRID = {
    "shoot_catch_bonus": (0.0, 0.15, 0.30),
    "shoot_off_dribble_penalty": (0.0, 0.15, 0.30),
    "pass_catch_base": (0.34, 0.45, 0.55),
    "shoot_pull_up_bonus": (0.18, 0.20, 0.22),
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
    base = json.loads(POLICY.read_text(encoding="utf-8"))
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


if __name__ == "__main__":
    main()
