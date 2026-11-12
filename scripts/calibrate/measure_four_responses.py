#!/usr/bin/env python3
"""四条越界判据的直接杠杆响应表。

在独立校准目录里逐候选运行权威门禁，打印四条判据的相对误差、
越界深度与失败条数。用于选出能同时清掉四条判据的倍率组合。

用法：

    python3 -u scripts/calibrate/measure_four_responses.py
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
WORK = ROOT / ".work" / "evidence" / "four-response"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
REPORT = ROOT / ".work" / "evidence" / "four-response-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

CRITERIA = [
    "creation_share_cut",
    "creation_share_pull_up",
    "fta_per_game",
    "steals_per_game",
]

# 单杠杆候选；(标签, {键: 倍率})。
CANDIDATES = [
    ("baseline", {}),
    ("gamble1.1", {"steal_gamble_base": 1.1}),
    ("gamble1.2", {"steal_gamble_base": 1.2}),
    ("gamble1.3", {"steal_gamble_base": 1.3}),
    ("gamble1.4", {"steal_gamble_base": 1.4}),
    ("gamble1.5", {"steal_gamble_base": 1.5}),
    ("skill1.1", {"steal_skill_base": 1.1}),
    ("skill1.2", {"steal_skill_base": 1.2}),
    ("skill1.3", {"steal_skill_base": 1.3}),
    ("skill1.5", {"steal_skill_base": 1.5}),
    ("pullup0.9", {"shoot_pull_up_bonus": 0.9}),
    ("pullup0.95", {"shoot_pull_up_bonus": 0.95}),
    ("pullup0.98", {"shoot_pull_up_bonus": 0.98}),
    ("foul0.9", {"shooting_foul_rate_two_point": 0.9}),
    ("foul0.95", {"shooting_foul_rate_two_point": 0.95}),
    ("foul0.98", {"shooting_foul_rate_two_point": 0.98}),
    ("cutlane0.9", {"cut_lane_scale": 0.9}),
    ("cutlane0.95", {"cut_lane_scale": 0.95}),
    ("cutlane1.05", {"cut_lane_scale": 1.05}),
    ("gamble1.3+foul0.95", {"steal_gamble_base": 1.3, "shooting_foul_rate_two_point": 0.95}),
    ("gamble1.3+pullup0.95", {"steal_gamble_base": 1.3, "shoot_pull_up_bonus": 0.95}),
    ("gamble1.4+foul0.95+pullup0.95", {
        "steal_gamble_base": 1.4,
        "shooting_foul_rate_two_point": 0.95,
        "shoot_pull_up_bonus": 0.95,
    }),
    ("gamble1.5+foul0.9+pullup0.9", {
        "steal_gamble_base": 1.5,
        "shooting_foul_rate_two_point": 0.9,
        "shoot_pull_up_bonus": 0.9,
    }),
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


def build(base, factors):
    policy = copy.deepcopy(base)
    for key, factor in factors.items():
        found = False
        for section in ("weights", "geometry"):
            if key in policy[section]:
                policy[section][key] *= factor
                found = True
                break
        if not found:
            raise KeyError(key)
    return policy


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
            results.append(report)
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    policies = [build(base, factors) for _, factors in CANDIDATES]
    reports = evaluate(policies)

    rows = []
    for (label, factors), report in zip(CANDIDATES, reports):
        if report is None:
            print(f"{label:42s} 门禁无输出", flush=True)
            continue
        by_name = {layer["criterion"]: layer for layer in report["layers"]}
        errors = [by_name[c]["relative_error"] for c in CRITERIA]
        depth = sum(excess(layer) for layer in report["layers"])
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        row = {
            "label": label,
            "factors": factors,
            "depth": depth,
            "failed": failed,
            "errors": {c: by_name[c]["relative_error"] for c in CRITERIA},
            "all_failures": [
                layer["criterion"] for layer in report["layers"] if not layer["passed"]
            ],
        }
        rows.append(row)
        print(
            f"{label:42s} 深度 {depth:.4f} 失败 {failed:2d} "
            + " ".join(f"{e:+.3f}" for e in errors),
            flush=True,
        )
    REPORT.write_text(
        json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )


if __name__ == "__main__":
    main()
