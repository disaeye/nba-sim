#!/usr/bin/env python3
"""细网格：节奏 × 中线混沌带，收敛最后一条判据。

单变量扫描显示：`seconds_per_possession` 上调能把 turnover 拉进带，
`backcourt_margin_ft` 放宽能消掉虚假八秒失误。两者都轻微扰动节奏，因此
用二维细网格寻找它们共同满足 39 条判据的窗口，评分来自权威门禁。

用法：

    python3 -u scripts/calibrate/fine_pair_grid.py
"""

from __future__ import annotations

import copy
import json
import os
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "fine-pair-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

PACE = [16.8, 17.2, 17.6, 18.0, 18.4, 18.8, 19.2, 19.6, 20.0, 20.4, 20.8]
MARGIN = [1.8, 1.9, 2.0, 2.1, 2.2, 2.3, 2.4]


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
                results.append((1e6, None, output))
            else:
                results.append((depth_of(report), report, output))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    initial = evaluate([base])[0]
    best = (sum(1 for l in initial[1]["layers"] if not l["passed"]), initial[0])
    best_policy = copy.deepcopy(base)
    best_stdout = initial[2]
    print(f"基线：失败 {best[0]} 条，深度 {best[1]:.6f}", flush=True)

    combos = [(p, m) for p in PACE for m in MARGIN]
    candidates = []
    for pace, margin in combos:
        trial = copy.deepcopy(base)
        trial["weights"]["seconds_per_possession"] = pace
        trial["geometry"]["backcourt_margin_ft"] = margin
        candidates.append(trial)
    results = evaluate(candidates)
    for (pace, margin), trial, (depth, report, stdout) in zip(combos, candidates, results):
        if report is None:
            continue
        failed = sum(1 for l in report["layers"] if not l["passed"])
        score = (failed, depth)
        if score < best:
            best = score
            best_policy = trial
            best_stdout = stdout
            turnover = next(
                (l["relative_error"] for l in report["layers"]
                 if l["criterion"] == "turnover_per_possession"), 0.0
            )
            print(
                f"pace {pace:.2f} margin {margin:.2f} 失败 {failed} "
                f"深度 {depth:.5f} turnover {turnover:+.4f}",
                flush=True,
            )
        if best[0] == 0:
            break

    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best_policy])[0]
    FINAL_LOG.write_text(best_stdout if best[0] == 0 else final[2], encoding="utf-8")
    print(
        f"最终 pace {best_policy['weights']['seconds_per_possession']} "
        f"margin {best_policy['geometry']['backcourt_margin_ft']}："
        f"失败 {sum(1 for l in final[1]['layers'] if not l['passed'])} 条，"
        f"通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)


if __name__ == "__main__":
    main()
