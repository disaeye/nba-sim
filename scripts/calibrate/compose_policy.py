#!/usr/bin/env python3
"""把 policy.json 恢复到 HEAD 的两阶段校准基线。

后续工作包新增的行为键必须保留：生产代码用 `PolicyTable::w/g` 读它们，
缺键会就地失败。因此这里以 HEAD 的参数为主体，只把工作树里新增的键
并入，不改动 HEAD 已有键的取值。

用法：

    python3 scripts/calibrate/compose_policy.py --head <HEAD 快照> --out <输出>
"""

from __future__ import annotations

import argparse
import json
import os

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))


def load(path):
    with open(path, encoding="utf-8") as handle:
        return json.load(handle)


def main():
    parser = argparse.ArgumentParser(description="合成恢复后的参数表")
    parser.add_argument(
        "--head", default=os.path.join(ROOT, ".work", "evidence", "policy-head.json")
    )
    parser.add_argument(
        "--out", default=os.path.join(ROOT, "data", "calibration", "policy.json")
    )
    args = parser.parse_args()

    head = load(args.head)
    current = load(args.out)
    added = []
    for section in ("weights", "geometry"):
        for key, value in current[section].items():
            if key not in head[section]:
                head[section][key] = value
                added.append(f"{section}.{key}")
    with open(args.out, "w", encoding="utf-8") as handle:
        json.dump(head, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    print(f"恢复 {len(head['weights'])} 个权重与 {len(head['geometry'])} 个几何量")
    print(f"保留新增键 {len(added)} 个：{', '.join(added)}")


if __name__ == "__main__":
    main()
