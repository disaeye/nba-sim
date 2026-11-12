#!/usr/bin/env python3
"""两参数网格：把 setup=8.0 引出的回合节奏偏移拉回带宽内。

用法：

    python3 -u scripts/calibrate/grid_setup.py
"""

from __future__ import annotations

import copy
import json
import os
import resource
import subprocess

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
POLICY = os.path.join(ROOT, "data", "calibration", "policy.json")
BINARY = os.path.join(ROOT, "target", "release", "sim2")
FINAL_LOG = os.path.join(ROOT, ".work", "evidence", "gate-final.log")
LIMIT = 2 * 1024 * 1024 * 1024

PACE = [18.0, 18.6, 19.2, 19.8, 20.4]
CUT = [0.405, 0.445, 0.485, 0.525]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (LIMIT, LIMIT))


def read_policy():
    with open(POLICY, encoding="utf-8") as handle:
        return json.load(handle)


def write_policy(policy):
    with open(POLICY, "w", encoding="utf-8") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def evaluate():
    output = subprocess.run(
        [BINARY, "--gate-only"],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=True,
        preexec_fn=limit_address_space,
    )
    for line in output.stdout.splitlines():
        if line.startswith("GATE "):
            return json.loads(line[5:]), output.stdout
    raise RuntimeError("sim2 produced no GATE line")


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            band = detail.split("band ")[1].rstrip(")")
            low, high = (float(value) for value in band.split(".."))
            error = layer["relative_error"]
            total += max(low - error, 0.0) + max(error - high, 0.0)
        elif detail.startswith("区间"):
            bounds = detail.removeprefix("区间 ").split("..")
            low, high = (float(value) for value in bounds)
            observed = layer["observed"]
            total += (max(low - observed, 0.0) + max(observed - high, 0.0)) / max(
                abs(high - low), 1e-6
            )
        else:
            total += abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)
    return total


def main():
    original = read_policy()
    best = None
    best_policy = None
    best_stdout = None
    try:
        for pace in PACE:
            for cut in CUT:
                trial = copy.deepcopy(original)
                trial["geometry"]["possession_setup_seconds"] = 8.0
                trial["weights"]["seconds_per_possession"] = pace
                trial["weights"]["cut_lane_scale"] = cut
                write_policy(trial)
                report, stdout = evaluate()
                failed = [layer for layer in report["layers"] if not layer["passed"]]
                depth = depth_of(report)
                print(
                    f"pace {pace:4.1f} cut {cut:.3f} 失败 {len(failed)} 深度 {depth:.5f} "
                    f"{[layer['criterion'] for layer in failed]}",
                    flush=True,
                )
                score = (len(failed), depth)
                if best is None or score < best:
                    best = score
                    best_policy = trial
                    best_stdout = stdout
    finally:
        if best_policy is not None:
            write_policy(best_policy)
        else:
            write_policy(original)

    report, stdout = evaluate()
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
    failed = [layer for layer in report["layers"] if not layer["passed"]]
    print(
        f"最终：失败 {len(failed)} 条，几何 {report['geometric_violations']}，"
        f"语境 {report['context_violations']}，通过 {report['passed']}",
        flush=True,
    )
    for layer in failed:
        print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
