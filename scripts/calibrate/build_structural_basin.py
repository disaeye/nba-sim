#!/usr/bin/env python3
"""构造结构网格里「两条顽固判据同时通过且失败最少」的向量。

用法：

    python3 -u scripts/calibrate/build_structural_basin.py
"""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / ".work" / "evidence" / "policy-before-structural-grid.json"
REPORT = ROOT / ".work" / "evidence" / "structural-grid-report.json"
OUTPUT = ROOT / "data" / "calibration" / "policy.json"
SNAPSHOT = ROOT / ".work" / "evidence" / "policy-structural-basin.json"


def main():
    rows = json.loads(REPORT.read_text(encoding="utf-8"))
    candidates = [
        row
        for row in rows
        if "creation_share_cut" not in row["failures"]
        and "and_one_per_game" not in row["failures"]
    ]
    best = min(candidates, key=lambda row: (row["failed"], row["depth"]))
    policy = json.loads(SOURCE.read_text(encoding="utf-8"))
    policy["geometry"]["cut_open_defender_ft"] = best["cut_open_defender_ft"]
    policy["weights"]["and_one_rate"] = best["and_one_rate"]
    policy["weights"]["shooting_foul_rate_two_point"] = best[
        "shooting_foul_rate_two_point"
    ]
    text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    OUTPUT.write_text(text, encoding="utf-8")
    SNAPSHOT.write_text(text, encoding="utf-8")
    print(
        f"写入盆地向量：失败 {best['failed']} 条，深度 {best['depth']:.6f}，"
        f"cut_open={best['cut_open_defender_ft']}，"
        f"and_one={best['and_one_rate']}，foul2={best['shooting_foul_rate_two_point']}",
        flush=True,
    )
    print(f"剩余失败 {best['failures']}", flush=True)


if __name__ == "__main__":
    main()
