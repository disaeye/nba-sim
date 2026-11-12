#!/usr/bin/env python3
"""扫描扑防造犯规的两个直接杠杆（只读冻结快照，不写主参数表）。

这两个键不在敏感性矩阵里：`resolve/shot.rs` 的 `closeout_response` 用
`closeout_draw_contact_rate` 决定是否造接触，用
`closeout_draw_contact_foul_chance` 决定是否吹犯规。两者直接产生罚球，
是当前 `fta_per_game` 越界的主要可调来源。

用法：

    python3 -u scripts/calibrate/scan_closeout.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "scan-closeout"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
RESULT = ROOT / ".work" / "evidence" / "closeout-best.json"
WORKERS = 4

RATE = [0.20, 0.25, 0.30, 0.36, 0.42, 0.50]
CHANCE = [0.25, 0.35, 0.47, 0.60, 0.72]


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
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    for index in range(WORKERS):
        calibration = WORK / f"worker-{index}" / "calibration"
        calibration.mkdir(parents=True)
        for path in CALIBRATION.glob("*.json"):
            shutil.copy2(path, calibration / path.name)
        (WORK / f"worker-{index}" / "tactics").symlink_to(TACTICS, target_is_directory=True)


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
            if report is None:
                results.append((1e6, None))
            else:
                results.append((depth_of(report), report))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)
    combos = list(itertools.product(RATE, CHANCE))
    candidates = []
    for rate, chance in combos:
        trial = copy.deepcopy(base)
        trial["weights"]["closeout_draw_contact_rate"] = rate
        trial["weights"]["closeout_draw_contact_foul_chance"] = chance
        candidates.append(trial)
    results = evaluate(candidates)
    best_policy = None
    for (rate, chance), trial, (depth, report) in zip(combos, candidates, results):
        if report is None:
            continue
        failed = [l for l in report["layers"] if not l["passed"]]
        score = (len(failed), depth)
        if score < best:
            best = score
            best_policy = trial
            fta = next(l["relative_error"] for l in report["layers"]
                       if l["criterion"] == "fta_per_game")
            print(
                f"rate {rate:.2f} chance {chance:.2f} 失败 {len(failed)} "
                f"深度 {depth:.5f} fta {fta:+.4f} {[l['criterion'] for l in failed]}",
                flush=True,
            )
    if best_policy is not None:
        RESULT.write_text(
            json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        print(f"最优另存 {RESULT.relative_to(ROOT)}（未写主参数表）", flush=True)
    else:
        print("没有找到改进", flush=True)


if __name__ == "__main__":
    main()
