#!/usr/bin/env python3
"""通用一维细扫：在当前冻结基线上扫描单个参数的细邻域。

按地址抽样的引擎对改动敏感，因此这里用独立校准目录并行评估，每次只改
一个参数，评分来自权威门禁自己的 39 条判定。最优值写回参数表。

用法：

    python3 -u scripts/calibrate/scan_lever.py --key seconds_per_possession \
        --lo 16.5 --hi 19.5 --step 0.1
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "lever-scan"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4


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
                results.append((1e6, None, output))
            else:
                results.append((depth_of(report), report, output))
    return results


def get_value(policy, key):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            return policy[section][key]
    raise KeyError(key)


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def main():
    parser = argparse.ArgumentParser(description="通用一维参数细扫")
    parser.add_argument("--key", required=True)
    parser.add_argument("--lo", type=float, required=True)
    parser.add_argument("--hi", type=float, required=True)
    parser.add_argument("--step", type=float, required=True)
    args = parser.parse_args()

    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    current = get_value(base, args.key)
    count = int(round((args.hi - args.lo) / args.step)) + 1
    values = [round(args.lo + i * args.step, 6) for i in range(count)]
    print(f"{args.key} 当前 {current}，扫描 {len(values)} 个值", flush=True)

    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    candidates = []
    for value in values:
        trial = copy.deepcopy(base)
        set_value(trial, args.key, value)
        candidates.append(trial)
    results = evaluate(candidates)
    for value, trial, (depth, report, stdout) in zip(values, candidates, results):
        if report is None:
            print(f"{value:.4f} 引擎失败", flush=True)
            continue
        failed = [l for l in report["layers"] if not l["passed"]]
        turnover = next(
            (l["relative_error"] for l in report["layers"]
             if l["criterion"] == "turnover_per_possession"), 0.0
        )
        score = (len(failed), depth)
        print(
            f"{value:9.4f} 失败 {len(failed):2d} 深度 {depth:.5f} "
            f"turnover {turnover:+.4f} {[l['criterion'] for l in failed]}",
            flush=True,
        )
        if score < best:
            best = score
            best_policy = trial
            best_stdout = stdout

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best_policy])[0]
    FINAL_LOG.write_text(best_stdout if best[0] == 0 else final[2], encoding="utf-8")
    print(
        f"最终 {args.key}={get_value(best_policy, args.key)}："
        f"失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"深度 {final[0]:.6f}，通过 {final[1]['passed']}",
        flush=True,
    )


if __name__ == "__main__":
    main()
