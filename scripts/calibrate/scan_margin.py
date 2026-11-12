#!/usr/bin/env python3
"""细扫 backcourt_margin_ft：中线混沌带宽度决定八秒计时是否推进。

`check_backcourt` 只有在球处于后场（`in_backcourt`）时才累计八秒计时。
持球人停在靠近中线的位置被判八秒违例，是探针确认的虚假失误来源。
本扫描直接改这个几何阈值，评分来自权威门禁自己的 39 条判定。

用法：

    python3 -u scripts/calibrate/scan_margin.py
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

VALUES = [2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.0, 7.0, 8.0]


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
    try:
        for value in VALUES:
            trial = copy.deepcopy(original)
            trial["geometry"]["backcourt_margin_ft"] = value
            write_policy(trial)
            report, stdout = evaluate()
            failed = [layer for layer in report["layers"] if not layer["passed"]]
            score = (len(failed), depth_of(report))
            print(
                f"margin {value:4.1f} 失败 {len(failed):2d} 深度 {score[1]:.5f} "
                f"turnover {next((round(l['relative_error'], 4) for l in report['layers'] if l['criterion'] == 'turnover_per_possession'), 0):+.4f} "
                f"{[layer['criterion'] for layer in failed]}",
                flush=True,
            )
            if best is None or score < best:
                best = score
                best_policy = trial
                best_stdout = stdout
            if score[0] == 0:
                break
    finally:
        write_policy(best_policy)

    report, stdout = evaluate()
    with open(FINAL_LOG, "w", encoding="utf-8") as handle:
        handle.write(best_stdout or stdout)
    print(
        f"最终 margin {best_policy['geometry']['backcourt_margin_ft']}："
        f"失败 {sum(1 for layer in report['layers'] if not layer['passed'])} 条，通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
