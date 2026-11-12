#!/usr/bin/env python3
"""传球拦截半径网格：直接测抢断生产链上唯一没扫过的几何杠杆。

`control::pass_intercept_radius` 用
    ball_radius + pass_intercept_base_ft + steal * pass_intercept_steal_ft
算出防守人够到传球线路的手球半径。半径变大 → 更多传球被碰到 →
接住就记抢断，碰不住就打成活球。此前所有搜索都没把这两个几何键放进
杠杆表；`pass_catch_base` 决定碰到的球有多少被接住，三者同属一条因果链。

用法：

    python3 -u scripts/calibrate/intercept_radius_grid.py [base-policy.json]
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import resource
import shutil
import subprocess
import sys
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "intercept-radius-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-intercept-radius-best.json"
REPORT = ROOT / ".work" / "evidence" / "intercept-radius-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

GRID = {
    "pass_intercept_base_ft": (0.92, 1.10, 1.30, 1.50, 1.80),
    "pass_intercept_steal_ft": (1.65, 2.00, 2.40, 2.80),
    "pass_catch_base": (0.34, 0.38),
}
CRITERIA = (
    "creation_share_cut",
    "creation_share_pull_up",
    "fta_per_game",
    "steals_per_game",
)


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
    source = Path(sys.argv[1]) if len(sys.argv) > 1 else POLICY
    base = json.loads(source.read_text(encoding="utf-8"))
    keys = list(GRID)
    combos = list(itertools.product(*(GRID[key] for key in keys)))
    print(f"基线 {source}，候选 {len(combos)} 个", flush=True)
    policies = [build(base, dict(zip(keys, combo))) for combo in combos]
    results = evaluate(policies)

    ranked = []
    for combo, policy, (report, stdout) in zip(combos, policies, results):
        if report is None:
            continue
        by_name = {layer["criterion"]: layer for layer in report["layers"]}
        ranked.append(
            (
                score(report),
                combo,
                policy,
                stdout,
                report,
                {name: by_name[name]["relative_error"] for name in CRITERIA},
            )
        )
    ranked.sort(key=lambda row: row[0])
    for value, combo, _, _, _, errors in ranked[:15]:
        print(
            f"{describe(value)} " + " ".join(f"{v:.2f}" for v in combo)
            + " | " + " ".join(f"{errors[name]:+.3f}" for name in CRITERIA),
            flush=True,
        )

    REPORT.write_text(
        json.dumps(
            [
                {
                    "failed": value[0],
                    "depth": value[1],
                    "values": dict(zip(keys, combo)),
                    "errors": errors,
                    "failures": [
                        layer["criterion"] for layer in report["layers"] if not layer["passed"]
                    ],
                }
                for value, combo, _, _, report, errors in ranked
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    best, combo, policy, stdout, report, _ = ranked[0]
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    print(f"最优 {describe(best)} 组合 {combo}", flush=True)


if __name__ == "__main__":
    main()
