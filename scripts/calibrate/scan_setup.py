#!/usr/bin/env python3
"""扫描 `possession_setup_seconds` 对权威门禁的影响。

死球回合的首次决策时刻是 `advance_seconds + setup_seconds`。这个值只
决定持球人等第一次决策多久，不改变推进目标、候选动作或任何物理结果。

用法：

    python3 -u scripts/calibrate/scan_setup.py
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
LIMIT = 2 * 1024 * 1024 * 1024

VALUES = [0.5, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 8.5]


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
            return json.loads(line[5:])
    raise RuntimeError("sim2 produced no GATE line")


def main():
    original = read_policy()
    try:
        for value in VALUES:
            trial = copy.deepcopy(original)
            trial["geometry"]["possession_setup_seconds"] = value
            write_policy(trial)
            report = evaluate()
            failed = [layer for layer in report["layers"] if not layer["passed"]]
            errors = {layer["criterion"]: layer["relative_error"] for layer in report["layers"]}
            print(
                f"setup {value:4.1f} 失败 {len(failed)} 条 "
                f"turnover {errors['turnover_per_possession']:+.4f} "
                f"poss {errors['possessions_per_game']:+.4f} "
                f"rim {errors['zone_share_rim']:+.4f} "
                f"{[layer['criterion'] for layer in failed]}",
                flush=True,
            )
    finally:
        write_policy(original)


if __name__ == "__main__":
    main()
