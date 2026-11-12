#!/usr/bin/env python3
"""扫描后场/中线几何阈值对门禁判据的影响。

八秒违例的根因是「过中线」与「建立前场控制」用了两个不同的距离阈值：
球越过中线后若被拉回后场，前场控制从未成立，于是同一段球路按八秒
违例收场。这里扫描 `backcourt_advance_ft` 与 `backcourt_margin_ft`，
用权威门禁判断哪一组阈值让判据回到带宽内。不改代码，参数可逆。

用法：

    python3 -u scripts/calibrate/scan_backcourt.py
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

FOCUS = [
    "turnover_per_possession",
    "zone_share_rim",
    "possessions_per_game",
    "points_per_game",
    "steals_per_game",
    "fta_per_game",
    "creation_share_cut",
    "creation_share_putback",
]

VALUES = [0.0, 1.0, 2.0, 4.0, 6.0, 8.0, 10.0]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (LIMIT, LIMIT))


def read_policy():
    with open(POLICY, encoding="utf-8") as handle:
        return json.load(handle)


def write_policy(policy):
    with open(POLICY, "w", encoding="utf-8") as handle:
        json.dump(policy, handle, indent=2, ensure_ascii=False)
        handle.write("\n")


def errors():
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
            report = json.loads(line[5:])
            failed = [layer for layer in report["layers"] if not layer["passed"]]
            values = {layer["criterion"]: layer["relative_error"] for layer in report["layers"]}
            return failed, values
    raise RuntimeError("sim2 produced no GATE line")


def main():
    original = read_policy()
    try:
        for advance in VALUES:
            trial = copy.deepcopy(original)
            trial["geometry"]["backcourt_advance_ft"] = advance
            write_policy(trial)
            failed, values = errors()
            bad = [layer["criterion"] for layer in failed]
            print(
                f"advance {advance:4.1f} 失败 {len(failed)} 条 {bad}  "
                f"turnover {values['turnover_per_possession']:+.4f} "
                f"rim {values['zone_share_rim']:+.4f} "
                f"poss {values['possessions_per_game']:+.4f}",
                flush=True,
            )
    finally:
        write_policy(original)


if __name__ == "__main__":
    main()
