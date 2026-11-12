#!/usr/bin/env python3
"""扫描后场停滞救援的触发时点。

`backcourt_decision_rescue_fraction` 是八秒时限内的触发比例：1.0 时
不触发（回到原始调度），0.5 时在计时过半后触发。评分来自权威门禁
自己的 39 条判定，最优值写回参数表。

用法：

    python3 -u scripts/calibrate/scan_rescue_fraction.py
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

VALUES = [0.50, 0.55, 0.60, 0.65, 0.70, 0.75, 0.80, 0.85, 0.90, 0.95, 1.0]


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


def main():
    original = read_policy()
    best = None
    best_policy = None
    best_stdout = None
    for value in VALUES:
        trial = copy.deepcopy(original)
        trial["geometry"]["backcourt_decision_rescue_fraction"] = value
        write_policy(trial)
        report, stdout = evaluate()
        failed = [l for l in report["layers"] if not l["passed"]]
        depth = depth_of(report)
        turnover = next(
            (l["relative_error"] for l in report["layers"]
             if l["criterion"] == "turnover_per_possession"), 0.0
        )
        score = (len(failed), depth)
        print(
            f"fraction {value:.2f} 失败 {len(failed):2d} 深度 {depth:.5f} "
            f"turnover {turnover:+.4f} {[l['criterion'] for l in failed]}",
            flush=True,
        )
        if best is None or score < best:
            best = score
            best_policy = trial
            best_stdout = stdout
        if not failed:
            break
    write_policy(best_policy)
    report, stdout = evaluate()
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
    print(
        f"最终 fraction {best_policy['geometry']['backcourt_decision_rescue_fraction']}："
        f"失败 {sum(1 for l in report['layers'] if not l['passed'])} 条，"
        f"通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
