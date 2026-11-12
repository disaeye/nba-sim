#!/usr/bin/env python3
"""以回合节奏为核心的组合网格，收敛最后一条越界判据。

`scan_turnovers.py` 显示 `seconds_per_possession` 下调能显著降低失误率，
但会把抢断数推出带宽；`pass_catch_steal` 与 `off_ball_hold_rate` 用来补回
抢断与犯规侧的影响。所有评分都来自权威门禁自己的 39 条判定。

用法：

    python3 -u scripts/calibrate/grid_pace.py
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

PACE = [16.8, 17.2, 17.6, 18.0]
STEAL = [0.4547, 0.54, 0.62]
HOLD = [0.00543, 0.0075]


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
    base_report, _ = evaluate()
    best = (
        sum(1 for layer in base_report["layers"] if not layer["passed"]),
        depth_of(base_report),
    )
    best_policy = copy.deepcopy(original)
    best_stdout = None
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.5f}", flush=True)

    try:
        for pace in PACE:
            for steal in STEAL:
                for hold in HOLD:
                    trial = copy.deepcopy(original)
                    trial["weights"]["seconds_per_possession"] = pace
                    trial["weights"]["pass_catch_steal"] = steal
                    trial["weights"]["off_ball_hold_rate"] = hold
                    write_policy(trial)
                    report, stdout = evaluate()
                    failed = [layer for layer in report["layers"] if not layer["passed"]]
                    depth = depth_of(report)
                    score = (len(failed), depth)
                    if score < best:
                        best = score
                        best_policy = trial
                        best_stdout = stdout
                        print(
                            f"pace {pace:4.1f} steal {steal:.3f} hold {hold:.5f} "
                            f"失败 {len(failed)} 深度 {depth:.5f} "
                            f"{[layer['criterion'] for layer in failed]}",
                            flush=True,
                        )
                    if len(failed) == 0:
                        print("39 条判据全部通过", flush=True)
                        raise StopIteration
    except StopIteration:
        pass
    finally:
        write_policy(best_policy)

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
