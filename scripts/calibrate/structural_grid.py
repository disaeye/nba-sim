#!/usr/bin/env python3
"""三条结构性杠杆的有界网格。

`creation_share_cut` 偏高 31%：切入候选由硬几何阈值
`cut_open_defender_ft` 决定，阈值提高会直接减少切入出手。
`and_one_per_game` 偏低 35%：`and_one_rate` 与 `shooting_foul_rate_two_point`
是加罚事件的两个直接概率来源。其余参数逐位不变。

评分按「失败条数、越界深度」；任何 0 条失败的向量立即写回主参数表。

用法：

    python3 -u scripts/calibrate/structural_grid.py
"""

from __future__ import annotations

import copy
import itertools
import json
import os
import resource
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "structural-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-structural-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "structural-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

CUT_OPEN = (5.3, 5.6, 5.9, 6.2, 6.5, 6.8, 7.2)
AND_ONE = (0.193665, 0.205, 0.215, 0.225, 0.24)
FOUL_TWO = (0.185948, 0.20, 0.215, 0.23)


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
    observed = layer["observed"]
    if detail.startswith("relative error"):
        low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
        error = layer["relative_error"]
        return (low - error) if error < low else (error - high if error > high else 0.0)
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        if observed < low:
            return (low - observed) / max(abs(high - low), 1e-6)
        if observed > high:
            return (observed - high) / max(abs(high - low), 1e-6)
        return 0.0
    if detail.startswith("上限"):
        bound = float(detail.removeprefix("上限 "))
        return max(0.0, (observed - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return abs(min(0.0, (observed - bound) / max(abs(bound), 1e-6)))
    return abs(observed - layer["target"]) / max(abs(layer["target"]), 1e-6)


def score(report):
    return (
        sum(1 for layer in report["layers"] if not layer["passed"]),
        sum(excess(layer) for layer in report["layers"]),
    )


def describe(value):
    return f"失败 {value[0]} 条，深度 {value[1]:.6f}"


def prepare():
    if WORK.exists():
        shutil.rmtree(WORK)
    for index in range(WORKERS):
        calibration = WORK / f"worker-{index}" / "calibration"
        calibration.mkdir(parents=True)
        for path in CALIBRATION.glob("*.json"):
            shutil.copy2(path, calibration / path.name)
        (WORK / f"worker-{index}" / "tactics").symlink_to(
            TACTICS, target_is_directory=True
        )


def evaluate(candidates):
    results = []
    for start in range(0, len(candidates), WORKERS):
        batch = candidates[start : start + WORKERS]
        processes = []
        for index, policy in enumerate(batch):
            path = WORK / f"worker-{index}" / "calibration" / "policy.json"
            path.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            processes.append(
                subprocess.Popen(
                    [str(BINARY), "--gate-only", "--calibration",
                     str(WORK / f"worker-{index}" / "calibration")],
                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                    preexec_fn=limit_address_space,
                )
            )
        for process in processes:
            output = process.stdout.read().decode("utf-8", errors="replace")
            process.wait()
            report = None
            for line in output.splitlines():
                if line.startswith("GATE "):
                    report = json.loads(line[5:])
                    break
            results.append((report, output))
        print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def build(base, cut_open, and_one, foul_two):
    policy = copy.deepcopy(base)
    policy["geometry"]["cut_open_defender_ft"] = cut_open
    policy["weights"]["and_one_rate"] = and_one
    policy["weights"]["shooting_foul_rate_two_point"] = foul_two
    return policy


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)

    combos = list(itertools.product(CUT_OPEN, AND_ONE, FOUL_TWO))
    print(f"候选 {len(combos)} 个", flush=True)
    policies = [build(base, *combo) for combo in combos]
    results = evaluate(policies)

    rows = []
    for combo, policy, (report, stdout) in zip(combos, policies, results):
        if report is None:
            continue
        value = score(report)
        rows.append(
            {
                "cut_open_defender_ft": combo[0],
                "and_one_rate": combo[1],
                "shooting_foul_rate_two_point": combo[2],
                "failed": value[0],
                "depth": value[1],
                "failures": [
                    layer["criterion"] for layer in report["layers"] if not layer["passed"]
                ],
            }
        )
        if value[0] <= 1:
            print(f"{describe(value)} {combo} {rows[-1]['failures']}", flush=True)
        if value < best:
            best = value
            best_policy = policy
            BEST_VECTOR.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            POLICY.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
            print(f"改善 {describe(best)} {combo}", flush=True)

    rows.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for row in rows[:12]:
        print(
            f"{row['failed']} 条 深度 {row['depth']:.4f} "
            f"cut_open={row['cut_open_defender_ft']:.2f} "
            f"and_one={row['and_one_rate']:.4f} foul2={row['shooting_foul_rate_two_point']:.4f} "
            f"{row['failures'][:4]}",
            flush=True,
        )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
