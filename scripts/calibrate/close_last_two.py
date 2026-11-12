#!/usr/bin/env python3
"""收尾两条越界判据的聚焦坐标下降。

从当前主参数表出发（zone_share_rim 与 steals_per_game 越界）。每条候选
扫描都含基线倍率 1.0，结果不会比起点差；评分按「失败条数、越界深度」。
任何改善立即写回主参数表，硬截止保证有界退出。

用法：

    python3 -u scripts/calibrate/close_last_two.py --minutes 20
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import resource
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "close-last-two"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
BEST_VECTOR = ROOT / ".work" / "evidence" / "policy-close-last-two-best.json"
RESULTS = ROOT / ".work" / "evidence" / "close-last-two-results.jsonl"
STATE = ROOT / ".work" / "evidence" / "close-last-two-state.json"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

# 两条越界判据的直接因果链。
LEVERS = [
    "steal_skill_base",
    "steal_gamble_base",
    "pass_catch_steal",
    "pass_catch_base",
    "pass_intercept_steal_ft",
    "pass_intercept_base_ft",
    "bad_pass_rate",
    "gather_travel_rate",
    "finish_priority",
    "finish_min_distance_ft",
    "drive_base",
    "cut_open_defender_ft",
    "shoot_pull_up_bonus",
    "off_ball_hold_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "trap_protection_bonus",
]
FACTORS = (0.90, 0.94, 0.97, 0.99, 1.0, 1.01, 1.03, 1.06, 1.10, 1.16)


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


def get_value(policy, key):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            return policy[section][key]
    raise KeyError(key)


def set_value(policy, key, value):
    for section in ("weights", "geometry"):
        if key in policy[section]:
            policy[section][key] = value
            return
    raise KeyError(key)


def persist(policy, stdout, value, sweep, label):
    text = json.dumps(policy, indent=2, ensure_ascii=False) + "\n"
    POLICY.write_text(text, encoding="utf-8")
    BEST_VECTOR.write_text(text, encoding="utf-8")
    (WORK / "gate-best.log").write_text(stdout, encoding="utf-8")
    STATE.write_text(
        json.dumps(
            {"failed": value[0], "depth": value[1], "sweep": sweep, "label": label},
            indent=2,
        ) + "\n",
        encoding="utf-8",
    )


def main():
    parser = argparse.ArgumentParser(description="收尾两条越界判据")
    parser.add_argument("--minutes", type=int, default=20)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    baseline, baseline_stdout = evaluate([base])[0]
    if baseline is None:
        raise SystemExit("基线门禁没有输出报告")
    best = score(baseline)
    best_policy = copy.deepcopy(base)
    RESULTS.write_text("", encoding="utf-8")
    persist(best_policy, baseline_stdout, best, 0, "baseline")
    print(f"基线：{describe(best)}", flush=True)
    print(
        "失败 " + ",".join(
            layer["criterion"] for layer in baseline["layers"] if not layer["passed"]
        ),
        flush=True,
    )

    for sweep in range(1, 4):
        if best[0] == 0 or time.monotonic() > deadline:
            break
        improved_any = False
        for key in LEVERS:
            if best[0] == 0 or time.monotonic() > deadline:
                break
            current = get_value(best_policy, key)
            candidates = []
            labels = []
            for factor in FACTORS:
                trial = copy.deepcopy(best_policy)
                set_value(trial, key, current * factor)
                candidates.append(trial)
                labels.append(f"{key}={current * factor:.6f}")
            for trial, label, (report, stdout) in zip(
                candidates, labels, evaluate(candidates)
            ):
                if report is None:
                    continue
                value = score(report)
                with RESULTS.open("a", encoding="utf-8") as handle:
                    handle.write(
                        json.dumps(
                            {
                                "sweep": sweep,
                                "label": label,
                                "failed": value[0],
                                "depth": value[1],
                            },
                            ensure_ascii=False,
                        ) + "\n"
                    )
                if value[0] == 0:
                    print(f"全通过 {label}", flush=True)
                if value < best:
                    best = value
                    best_policy = trial
                    persist(best_policy, stdout, best, sweep, label)
                    improved_any = True
                    print(f"[轮 {sweep}] {label} {describe(best)}", flush=True)
        print(f"[轮 {sweep}] 结束 {describe(best)}", flush=True)
        if not improved_any:
            break

    final, final_stdout = evaluate([best_policy])[0]
    if final is not None:
        for layer in final["layers"]:
            if not layer["passed"]:
                print(f"  {layer['criterion']:36s} {layer['relative_error']:+.6f}", flush=True)
    print(f"最终：{describe(best)}", flush=True)


if __name__ == "__main__":
    main()
