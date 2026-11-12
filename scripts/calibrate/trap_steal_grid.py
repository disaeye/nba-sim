#!/usr/bin/env python3
"""三条因果链的小网格：护球先验 × 抢断技能 × 干拔加成。

`trap_protection_bonus` 让被包夹护球进入生产流，护球窗口按规则降低切球
成功率；它因此与 `steals_per_game` 直接耦合。`steal_skill_base` 是切球
成功率倍率，`shoot_pull_up_bonus` 调干拔出手份额。这里对三者做有界网格，
评分用权威门禁的越界深度与失败条数，结果写进独立报告。

用法：

    python3 -u scripts/calibrate/trap_steal_grid.py
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
WORK = ROOT / ".work" / "evidence" / "trap-steal-grid"
BINARY = ROOT / "target" / "release" / "sim2"
PROBE = ROOT / ".work" / "probe" / "target" / "release" / "probe-clock"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-trap-steal-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "trap-steal-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

BONUSES = (0.15, 0.2, 0.3, 0.4, 0.6)
SKILLS = (0.95, 1.0, 1.05, 1.10, 1.15)
PULLUPS = (0.90, 0.95, 1.0)


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
    observed = layer["observed"]
    error = layer["relative_error"]
    if detail.startswith("relative error"):
        low, high = (float(v) for v in detail.split("band ")[1].rstrip(")").split(".."))
        return (low - error) if error < low else (error - high if error > high else 0.0)
    if detail.startswith("区间"):
        low, high = (float(v) for v in detail.removeprefix("区间 ").split(".."))
        observed = layer["observed"]
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
    return abs(layer["observed"] - layer["target"]) / max(abs(layer["target"]), 1e-6)


def score(report):
    depth = sum(excess(layer) for layer in report["layers"])
    failed = sum(1 for layer in report["layers"] if not layer["passed"])
    return (depth, failed)


def describe(value):
    return f"深度 {value[0]:.6f}，失败 {value[1]} 条"


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


def build(base, bonus, skill, pullup_factor):
    policy = copy.deepcopy(base)
    policy["weights"]["trap_protection_bonus"] = bonus
    policy["weights"]["steal_skill_base"] = skill
    policy["weights"]["shoot_pull_up_bonus"] *= pullup_factor
    return policy


def main():
    prepare()
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    combos = list(itertools.product(BONUSES, SKILLS, PULLUPS))
    print(f"候选组合 {len(combos)} 个", flush=True)
    policies = [build(base, *combo) for combo in combos]
    results = evaluate(policies)

    ranked = []
    for combo, policy, (report, stdout) in zip(combos, policies, results):
        if report is None:
            continue
        ranked.append((score(report), combo, policy, stdout, report))
    ranked.sort(key=lambda row: row[0])
    if not ranked:
        raise SystemExit("没有可用结果")
    for value, combo, _, _, _ in ranked[:15]:
        print(
            f"{describe(value)} bonus={combo[0]:.2f} skill={combo[1]:.2f} "
            f"pullup={combo[2]:.2f}",
            flush=True,
        )

    REPORT.write_text(
        json.dumps(
            [
                {
                    "depth": value[0],
                    "failed": value[1],
                    "bonus": combo[0],
                    "skill": combo[1],
                    "pullup": combo[2],
                    "failures": [
                        layer["criterion"]
                        for layer in report["layers"]
                        if not layer["passed"]
                    ],
                }
                for value, combo, _, _, report in ranked
            ],
            indent=2,
            ensure_ascii=False,
        ) + "\n",
        encoding="utf-8",
    )
    best, combo, policy, stdout, report = ranked[0]
    BEST_VECTOR.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    print(f"最优 {describe(best)} 组合 {combo}", flush=True)
    for layer in report["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)


if __name__ == "__main__":
    main()
