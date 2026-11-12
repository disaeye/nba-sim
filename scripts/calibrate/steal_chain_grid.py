#!/usr/bin/env python3
"""收尾 steals_per_game 的联合网格。

从当前主参数表出发，对抢断生产链的三个直接杠杆做绝对档位网格：
切球尝试速率、切球成功率、拦截后接住概率。评分按「失败条数、越界深度」，
任何改善立即写回主参数表与独立最优向量。

用法：

    python3 -u scripts/calibrate/steal_chain_grid.py
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
WORK = ROOT / ".work" / "evidence" / "steal-chain-grid"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-steal-chain-grid-best.json"
REPORT = ROOT / ".work" / "evidence" / "steal-chain-grid-report.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

BASE_GAMBLE = 1.0147469999999998
BASE_SKILL = 1.0352999999999999
BASE_CATCH = 0.45
GAMBLE = [BASE_GAMBLE * f for f in (0.90, 1.0, 1.08, 1.18, 1.30, 1.45, 1.65)]
SKILL = [BASE_SKILL * f for f in (0.90, 1.0, 1.08, 1.18, 1.30, 1.45, 1.65)]
CATCH = [BASE_CATCH, 0.48, 0.52]


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
        if start and (start // WORKERS) % 12 == 0:
            print(f"已判定 {len(results)}/{len(candidates)}", flush=True)
    return results


def build(base, gamble, skill, catch):
    policy = copy.deepcopy(base)
    policy["weights"]["steal_gamble_base"] = gamble
    policy["weights"]["steal_skill_base"] = skill
    policy["weights"]["pass_catch_base"] = catch
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
    for layer in baseline["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)

    combos = list(itertools.product(GAMBLE, SKILL, CATCH))
    print(f"候选 {len(combos)} 个", flush=True)
    policies = [build(base, *combo) for combo in combos]
    results = evaluate(policies)

    rows = []
    for combo, policy, (report, stdout) in zip(combos, policies, results):
        if report is None:
            continue
        value = score(report)
        by_name = {layer["criterion"]: layer for layer in report["layers"]}
        row = {
            "gamble": combo[0],
            "skill": combo[1],
            "catch": combo[2],
            "failed": value[0],
            "depth": value[1],
            "steals": by_name["steals_per_game"]["observed"],
            "steals_error": by_name["steals_per_game"]["relative_error"],
            "failures": [
                layer["criterion"] for layer in report["layers"] if not layer["passed"]
            ],
        }
        rows.append(row)
        if value[0] == 0:
            print(
                f"全通过 gamble={combo[0]:.4f} skill={combo[1]:.4f} catch={combo[2]:.3f}",
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
                f"改善 {describe(best)} gamble={combo[0]:.4f} "
                f"skill={combo[1]:.4f} catch={combo[2]:.3f}",
                flush=True,
            )

    rows.sort(key=lambda row: (row["failed"], row["depth"]))
    REPORT.write_text(json.dumps(rows, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")
    for row in rows[:12]:
        print(
            f"{row['failed']} 条 深度 {row['depth']:.4f} "
            f"gamble={row['gamble']:.4f} skill={row['skill']:.4f} "
            f"catch={row['catch']:.3f} steals={row['steals']:.3f} "
            f"err={row['steals_error']:+.4f} {row['failures'][:4]}",
            flush=True,
        )
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
