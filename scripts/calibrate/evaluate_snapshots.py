#!/usr/bin/env python3
"""评估 .work/evidence 下保存的全部 policy 快照。

每个候选复制进独立的校准目录，再交给 `sim2 --gate-only`，评分来自权威
门禁自己的 39 条判定。脚本不写主参数表，只把最优候选另存为工件。

用法：

    python3 -u scripts/calibrate/evaluate_snapshots.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
EVIDENCE = ROOT / ".work" / "evidence"
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = EVIDENCE / "snapshot-eval"
BINARY = ROOT / "target" / "release" / "sim2"
BEST_OUT = EVIDENCE / "policy-snapshot-best.json"
WORKERS = 4


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


def candidates():
    paths = sorted(EVIDENCE.glob("policy-*.json"))
    paths += sorted(EVIDENCE.glob("*/worker-*/calibration/policy.json"))
    paths += sorted(EVIDENCE.glob("*/*/worker-*/calibration/policy.json"))
    seen = set()
    out = []
    for path in paths:
        try:
            raw = path.read_bytes()
        except OSError:
            continue
        if not raw or raw in seen:
            continue
        seen.add(raw)
        try:
            policy = json.loads(raw)
        except json.JSONDecodeError:
            continue
        if "weights" not in policy or "geometry" not in policy:
            continue
        out.append((path, policy))
    return out


def evaluate(items):
    results = []
    for start in range(0, len(items), WORKERS):
        batch = items[start : start + WORKERS]
        processes = []
        for index, (_, policy) in enumerate(batch):
            path = WORK / f"worker-{index}" / "calibration" / "policy.json"
            path.write_text(json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
            processes.append(
                subprocess.Popen(
                    [str(BINARY), "--gate-only", "--calibration",
                     str(WORK / f"worker-{index}" / "calibration")],
                    cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
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
            if report is None:
                results.append((1e6, None))
            else:
                results.append((depth_of(report), report))
    return results


def main():
    prepare()
    items = candidates()
    print(f"候选快照 {len(items)} 份", flush=True)
    scores = evaluate(items)
    ranked = []
    for (path, policy), (depth, report) in zip(items, scores):
        failed = None if report is None else sum(1 for l in report["layers"] if not l["passed"])
        ranked.append(((failed if failed is not None else 99, depth), path, policy, report))
    ranked.sort(key=lambda row: row[0])
    for (failed, depth), path, _, report in ranked[:20]:
        bad = [] if report is None else [l["criterion"] for l in report["layers"] if not l["passed"]]
        print(f"{failed:2d} 深度 {depth:.6f} {path.relative_to(ROOT)} {bad}", flush=True)
    best = ranked[0]
    BEST_OUT.write_text(
        json.dumps(best[2], indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    print(f"最优另存 {BEST_OUT.relative_to(ROOT)}", flush=True)


if __name__ == "__main__":
    main()
