#!/usr/bin/env python3
"""两阶段校准的冻结锁（charter：先锁命中率类表，再优化 policy.json）。

第一阶段拟合出来的三张表——`shot_make`、`rebound`、`freethrow`——决定
一次出手/一次篮板/一次罚球本身的概率。第二阶段只允许调 `policy.json`
里的行为权重。如果第二阶段偷偷改了这三张表，球队总体三分命中率可以靠
抬高基础命中率凑出来，两阶段就退化成一次没有约束的联合拟合。

`frozen.lock.json` 记录冻结时刻三张表的 SHA-256；`--check` 在任何时候
都能验证它们没被动过，`fit_policy.py` 在开跑前和写回 policy.json 之后
各校验一次。
"""

import argparse
import hashlib
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
CALIBRATION = os.path.join(ROOT, "data", "calibration")
LOCK = os.path.join(CALIBRATION, "frozen.lock.json")
FROZEN = ["shot_make", "rebound", "freethrow"]


def digest(name):
    path = os.path.join(CALIBRATION, f"{name}.json")
    with open(path, "rb") as handle:
        return hashlib.sha256(handle.read()).hexdigest()


def current():
    return {name: digest(name) for name in FROZEN}


def check(verbose=True):
    if not os.path.exists(LOCK):
        print(f"❌ 缺少冻结锁 {os.path.relpath(LOCK, ROOT)}", file=sys.stderr)
        return 1
    with open(LOCK) as handle:
        locked = json.load(handle)
    tables = locked.get("tables", {})
    now = current()
    changed = [name for name in FROZEN if tables.get(name) != now[name]]
    if changed:
        print("❌ 冻结表被改动：" + "、".join(changed), file=sys.stderr)
        print(
            "   第一阶段的三张表必须在优化 policy.json 之前锁死；"
            "重新生成它们要显式执行 freeze.py --write，并重跑条件分布校验。",
            file=sys.stderr,
        )
        return 1
    if verbose:
        print("冻结表未改动：" + "、".join(FROZEN))
    return 0


def write():
    payload = {
        "tables": current(),
        "note": "第一阶段冻结锁：命中率、篮板、罚球三张表。policy.json 优化阶段不得改动。",
    }
    with open(LOCK, "w") as handle:
        json.dump(payload, handle, indent=2, ensure_ascii=False)
        handle.write("\n")
    print(f"已写入冻结锁 {os.path.relpath(LOCK, ROOT)}")
    return 0


def main():
    parser = argparse.ArgumentParser(description="两阶段校准的冻结锁")
    parser.add_argument("--write", action="store_true", help="记录当前三张表的哈希")
    parser.add_argument("--check", action="store_true", help="校验三张表未改动")
    args = parser.parse_args()
    if args.write:
        return write()
    return check()


if __name__ == "__main__":
    sys.exit(main())
