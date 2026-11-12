#!/usr/bin/env python3
"""构造抢断链网格里抢断达标、剩余两条浅越界的向量。

用法：

    python3 -u scripts/calibrate/build_steal_basin.py
"""

from __future__ import annotations

import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SOURCE = ROOT / ".work" / "evidence" / "policy-before-steal-grid.json"
REPORT = ROOT / ".work" / "evidence" / "steal-chain-grid-report.json"
OUTPUT = ROOT / "data" / "calibration" / "policy.json"
SNAPSHOT = ROOT / ".work" / "evidence" / "policy-steal-basin.json"


def main():
    rows = json.loads(REPORT.read_text(encoding="utf-8"))
    candidates = [
        row
        for row in rows
        if "steals_per_game" not in row["failures"]
    ]
    best = min(candidates, key=lambda row: (row["failed"], row["depth"]))
    policy = json.loads(SOURCE.read_text(encoding="utf-8"))
    policy["weights"]["steal_gamble_base"] = best["gamble"]
    policy["weights"]["steal_skill_base"] = best["skill"]
    policy["weights"]["pass_catch_base"] = best["catch"]
    text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    OUTPUT.write_text(text, encoding="utf-8")
    SNAPSHOT.write_text(text, encoding="utf-8")
    print(
        f"写入抢断盆地：失败 {best['failed']} 条，深度 {best['depth']:.6f}，"
        f"gamble={best['gamble']:.6f} skill={best['skill']:.6f} catch={best['catch']:.3f}，"
        f"抢断 {best['steals']:.4f}",
        flush=True,
    )
    print(f"剩余失败 {best['failures']}", flush=True)


if __name__ == "__main__":
    main()
