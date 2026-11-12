#!/usr/bin/env python3
"""两条失败判据直接杠杆的细步长二维网格。

`one_dimensional_two.py` 的 and_one_rate 从基线 0.193665 直接跳到 0.22，
cut_lane_scale 从 0.383707 跳到 0.40；两条判据都只差几个离散事件，
细步长窗口可能存在。

用法：

    python3 -u scripts/calibrate/fine_two_grid.py
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
WORK = ROOT / ".work" / "evidence" / "fine-two-grid"
BINARY = ROOT / "target" / "release" / "sim2"
BASE = ROOT / ".work" / "evidence" / "policy-zero-key-grid-best.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-fine-two-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "fine-two-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

AND_ONE = [0.193665 + i * 0.002 for i in range(12)]
CUT_LANE = [0.370 + i * 0.005 for i in range(7)]


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


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def main():
    prepare()
    base = json.loads(BASE.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    print(f"基线：{describe(best)}", flush=True)

    combos = list(itertools.product(AND_ONE, CUT_LANE))
    candidates = []
    for and_one, cut_lane in combos:
        trial = copy.deepcopy(base)
        set_value(trial, "and_one_rate", and_one)
        set_value(trial, "cut_lane_scale", cut_lane)
        candidates.append(trial)
    print(f"候选 {len(candidates)} 个", flush=True)
    results = evaluate(candidates)

    rows = []
    for (and_one, cut_lane), policy, (report, stdout) in zip(combos, candidates, results):
        if report is None:
            continue
        value = score(report)
        rows.append(
            {
                "and_one_rate": and_one,
                "cut_lane_scale": cut_lane,
                "failed": value[0],
                "depth": value[1],
                "failures": [
                    layer["criterion"] for layer in report["layers"] if not layer["passed"]
                ],
            }
        )
        if value < best:
            best = value
            best_policy = policy
            BEST_VECTOR.write_text(
                json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
            )
            (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
            print(
                f"改善 and_one={and_one:.6f} cut_lane={cut_lane:.4f} {describe(value)}",
                flush=True,
            )

    rows.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for row in rows[:12]:
        print(
            f"{row['failed']} 条 深度 {row['depth']:.4f} "
            f"and_one={row['and_one_rate']:.6f} cut_lane={row['cut_lane_scale']:.4f} "
            f"{row['failures']}",
            flush=True,
        )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
