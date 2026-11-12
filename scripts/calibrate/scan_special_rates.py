#!/usr/bin/env python3
"""扫描新增特殊犯规抽样率，找一组让权威门禁 39 条判据全部通过的参数。

`double_foul_rate` 与 `player_technical_rate` 是本轮新增的可标定参数。
按地址抽样的引擎里，它们会重排后续抽样地址，因此用 4 个独立校准目录
并行评估一组网格，评分直接来自 `sim2 --gate-only` 的 39 条判定。

率的下界取自在 16 场完整比赛里能稳定检出两类事实的量级（探针实测：
double=1e-04 检出 4 次，technical=8e-05 检出 7 次）。

用法：

    python3 -u scripts/calibrate/scan_special_rates.py
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
WORK = ROOT / ".work" / "evidence" / "rate-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

DOUBLE_RATES = [1.0e-4, 1.2e-4, 1.5e-4, 2.0e-4, 2.5e-4, 3.0e-4, 4.0e-4, 5.0e-4]
TECHNICAL_RATES = [8.0e-5, 1.0e-4, 1.5e-4, 2.0e-4, 3.0e-4, 4.0e-4]


def depth_of(report):
    total = 0.0
    for layer in report["layers"]:
        if layer["passed"]:
            continue
        detail = layer["detail"]
        if detail.startswith("relative error"):
            band = detail.split("band ")[1].rstrip(")")
            low, high = (float(v) for v in band.split(".."))
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
    processes = []
    for index, policy in enumerate(candidates):
        path = WORK / f"worker-{index}" / "calibration" / "policy.json"
        path.write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        processes.append(
            subprocess.Popen(
                [
                    str(BINARY),
                    "--gate-only",
                    "--calibration",
                    str(WORK / f"worker-{index}" / "calibration"),
                ],
                cwd=ROOT,
                stdout=subprocess.PIPE,
                stderr=subprocess.STDOUT,
            )
        )
    results = []
    for process in processes:
        output = process.stdout.read().decode("utf-8", errors="replace")
        process.wait()
        report = None
        stdout = output
        for line in output.splitlines():
            if line.startswith("GATE "):
                report = json.loads(line[5:])
                break
        if report is None:
            raise RuntimeError(f"gate-only failed (exit {process.returncode}): {output[-1200:]}")
        failed = sum(1 for layer in report["layers"] if not layer["passed"])
        results.append((failed, depth_of(report), report, stdout))
    return results


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    combos = [(d, t) for d in DOUBLE_RATES for t in TECHNICAL_RATES]
    best = None
    best_policy = None
    best_stdout = None
    print(f"网格：{len(combos)} 组，{WORKERS} 路并行", flush=True)
    for start in range(0, len(combos), WORKERS):
        batch = combos[start : start + WORKERS]
        candidates = []
        for double, technical in batch:
            trial = copy.deepcopy(base)
            trial["weights"]["double_foul_rate"] = double
            trial["weights"]["player_technical_rate"] = technical
            candidates.append(trial)
        results = evaluate(candidates)
        for (double, technical), trial, (failed, depth, report, stdout) in zip(
            batch, candidates, results
        ):
            score = (failed, depth)
            if best is None or score < best:
                best = score
                best_policy = trial
                best_stdout = stdout
                bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
                print(
                    f"double {double:.5f} technical {technical:.5f} "
                    f"失败 {failed} 深度 {depth:.5f} {bad}",
                    flush=True,
                )
        if best[0] == 0:
            break
    POLICY.write_text(
        json.dumps(best_policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(best_stdout, encoding="utf-8")
    final = evaluate([best_policy] * WORKERS)[0]
    print(
        f"最终：失败 {final[0]} 条，几何 {final[2]['geometric_violations']}，"
        f"语境 {final[2]['context_violations']}，通过 {final[2]['passed']}",
        flush=True,
    )
    for layer in final[2]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)
    print(
        f"选定 double {best_policy['weights']['double_foul_rate']} "
        f"technical {best_policy['weights']['player_technical_rate']}",
        flush=True,
    )


if __name__ == "__main__":
    main()
