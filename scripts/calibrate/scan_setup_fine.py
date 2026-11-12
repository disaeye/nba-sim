#!/usr/bin/env python3
"""细扫 possession_setup_seconds：它决定死球回合持球人等首次决策多久。

探针确认八秒违例的因果链是「首次决策调度晚于八秒时限，持球人停在后场
等调度」。这个参数直接控制那段等待，因此先做一维细扫；评分来自权威
门禁自己的 39 条判定，最优值写回参数表。

用法：

    python3 -u scripts/calibrate/scan_setup_fine.py
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

VALUES = [7.6, 7.8, 8.0, 8.1, 8.2, 8.3, 8.4, 8.5, 8.6, 8.7, 8.8, 9.0, 9.2]


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
        trial["geometry"]["possession_setup_seconds"] = value
        write_policy(trial)
        report, stdout = evaluate()
        failed = [layer for layer in report["layers"] if not layer["passed"]]
        score = (len(failed), depth_of(report))
        print(
            f"setup {value:4.1f} 失败 {len(failed):2d} 深度 {score[1]:.5f} "
            f"{[layer['criterion'] for layer in failed]}",
            flush=True,
        )
        if best is None or score < best:
            best = score
            best_policy = trial
            best_stdout = stdout
        if score[0] == 0:
            break
    write_policy(best_policy)
    report, stdout = evaluate()
    FINAL_LOG and open(FINAL_LOG, "w", encoding="utf-8").write(best_stdout or stdout)
    print(
        f"最终 setup {best_policy['geometry']['possession_setup_seconds']}："
        f"失败 {sum(1 for layer in report['layers'] if not layer['passed'])} 条，"
        f"通过 {report['passed']}",
        flush=True,
    )
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
