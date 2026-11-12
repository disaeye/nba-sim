#!/usr/bin/env python3
"""收尾抢断盆地的两条浅越界判据。

用法：

    python3 -u scripts/calibrate/finish_steal_basin.py
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
WORK = ROOT / ".work" / "evidence" / "finish-steal-basin"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-finish-steal-basin-best.json"
REPORT = ROOT / ".work" / "evidence" / "finish-steal-basin-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

FOUL = [0.185, 0.190, 0.195, 0.200, 0.205]
PLAY = [0.74, 0.76, 0.78, 0.80, 0.82, 0.84, 0.86]


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
    return results


def build(base, foul, play):
    policy = copy.deepcopy(base)
    policy["weights"]["shooting_foul_rate_two_point"] = foul
    policy["weights"]["play_call_rate"] = play
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
    print(
        "失败 " + ",".join(
            layer["criterion"] for layer in baseline["layers"] if not layer["passed"]
        ),
        flush=True,
    )

    combos = list(itertools.product(FOUL, PLAY))
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
                "foul": combo[0],
                "play": combo[1],
                "failed": value[0],
                "depth": value[1],
                "failures": [
                    layer["criterion"] for layer in report["layers"] if not layer["passed"]
                ],
            }
        )
        if value[0] == 0:
            print(
                f"全通过 foul={combo[0]:.4f} play={combo[1]:.3f}",
                flush=True,
            )
        if value < best:
            best = value
            best_policy = policy
            text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
            POLICY.write_text(text, encoding="utf-8")
            BEST_VECTOR.write_text(text, encoding="utf-8")
            (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
            print(
                f"改善 {describe(best)} foul={combo[0]:.4f} play={combo[1]:.3f}",
                flush=True,
            )

    rows.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for row in rows[:12]:
        print(
            f"{row['failed']} 条 深度 {row['depth']:.4f} "
            f"foul={row['foul']:.4f} play={row['play']:.3f} {row['failures'][:4]}",
            flush=True,
        )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
