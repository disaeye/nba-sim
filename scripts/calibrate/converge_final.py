#!/usr/bin/env python3
"""最终门禁收敛：三阶段有界搜索。

阶段一：直接因果杠杆的粗步长坐标下降。
阶段二：多杠杆联合扰动，跨过离散事件的跳变点。
阶段三：极细步长收尾边界判据。

每个候选都由权威门禁判定，任何改善立即写回主参数表。硬截止保证搜索在
有界时间内结束，结束后主参数表保留搜索到的最优向量。

用法：

    python3 -u scripts/calibrate/converge_final.py --minutes 35
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import resource
import shutil
import subprocess
import time
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")
ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "converge"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4
ADDRESS_SPACE_LIMIT = 2 * 1024 * 1024 * 1024

LEVERS = [
    "cut_tendency",
    "cut_lane_scale",
    "cut_positional_bonus",
    "drive_base",
    "pass_catch_steal",
    "steal_gamble_base",
    "steal_skill_base",
    "bad_pass_rate",
    "bad_pass_skill_base",
    "gather_travel_rate",
    "shooting_foul_rate_two_point",
    "shooting_foul_rate_three_point",
    "closeout_draw_contact_rate",
    "closeout_draw_contact_foul_chance",
    "and_one_rate",
    "transition_take_window_seconds",
    "transition_window_seconds",
    "seconds_per_possession",
    "play_call_rate",
    "hand_check_rate",
    "reach_in_foul_rate",
    "shoot_early_slack",
    "shoot_putback_bonus",
    "post_base",
    "finish_priority",
]
COARSE = [0.80, 0.88, 0.94, 0.98, 1.0, 1.02, 1.06, 1.12, 1.25]
JOINT = [0.94, 0.97, 0.985, 1.0, 1.015, 1.03, 1.06]
FINE = [0.992, 0.996, 0.998, 1.0, 1.002, 1.004, 1.008]


def limit_address_space():
    resource.setrlimit(resource.RLIMIT_AS, (ADDRESS_SPACE_LIMIT, ADDRESS_SPACE_LIMIT))


def excess(layer):
    if layer["passed"]:
        return 0.0
    detail = layer["detail"]
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
        return max(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6))
    if detail.startswith("下限"):
        bound = float(detail.removeprefix("下限 "))
        return min(0.0, (layer["observed"] - bound) / max(abs(bound), 1e-6))
    return 0.0


def vector(report):
    return {layer["criterion"]: excess(layer) for layer in report["layers"]}


def score(report):
    values = list(vector(report).values())
    return (
        sum(abs(value) for value in values),
        sum(1 for value in values if abs(value) > 1e-9),
    )


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


def evaluate(candidates, deadline):
    results = []
    for start in range(0, len(candidates), WORKERS):
        if time.monotonic() > deadline:
            results.extend([(None, "")] * (len(candidates) - len(results)))
            break
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


def persist(policy, stdout):
    POLICY.write_text(
        json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    FINAL_LOG.write_text(stdout, encoding="utf-8")


def accept(candidates, labels, state, deadline, stage):
    results = evaluate(candidates, deadline)
    for trial, label, (report, stdout) in zip(candidates, labels, results):
        if report is None:
            continue
        candidate = score(report)
        if candidate < state["score"]:
            state["score"] = candidate
            state["policy"] = trial
            persist(trial, stdout)
            print(f"[{stage}] {label} {describe(candidate)}", flush=True)
    return state


def coordinate(state, factors, deadline, stage, sweeps):
    for sweep in range(1, sweeps + 1):
        if state["score"][0] == 0.0 or time.monotonic() > deadline:
            break
        for key in LEVERS:
            if state["score"][0] == 0.0 or time.monotonic() > deadline:
                break
            current = get_value(state["policy"], key)
            candidates = []
            labels = []
            for factor in factors:
                trial = copy.deepcopy(state["policy"])
                set_value(trial, key, current * factor)
                candidates.append(trial)
                labels.append(f"{key}={current * factor:.6f}")
            state = accept(candidates, labels, state, deadline, f"{stage} {sweep}")
        print(f"[{stage} {sweep}] {describe(state['score'])}", flush=True)
    return state


def joint(state, trials, deadline, seed):
    rng = random.Random(seed)
    for batch_start in range(0, trials, WORKERS):
        if state["score"][0] == 0.0 or time.monotonic() > deadline:
            break
        candidates = []
        labels = []
        for _ in range(WORKERS):
            trial = copy.deepcopy(state["policy"])
            chosen = rng.sample(LEVERS, rng.randint(3, 5))
            for key in chosen:
                factor = rng.choice(JOINT)
                set_value(trial, key, get_value(state["policy"], key) * factor)
            candidates.append(trial)
            labels.append("+".join(chosen))
        state = accept(candidates, labels, state, deadline, "联合")
        if (batch_start // WORKERS) % 10 == 0:
            print(
                f"[联合 {batch_start + WORKERS}/{trials}] {describe(state['score'])}",
                flush=True,
            )
    return state


def main():
    parser = argparse.ArgumentParser(description="最终门禁三阶段有界搜索")
    parser.add_argument("--minutes", type=int, default=35)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    deadline = time.monotonic() + args.minutes * 60.0
    base = json.loads(POLICY.read_text(encoding="utf-8"))
    report, stdout = evaluate([base], deadline)[0]
    assert report is not None, "baseline gate produced no report"
    state = {"score": score(report), "policy": copy.deepcopy(base)}
    persist(base, stdout)
    print(f"基线：{describe(state['score'])}", flush=True)

    state = coordinate(state, COARSE, deadline, "粗扫", 2)
    state = joint(state, 240, deadline, args.seed)
    state = coordinate(state, FINE, deadline, "细扫", 2)
    state = joint(state, 160, deadline, args.seed + 1)
    state = coordinate(state, FINE, deadline, "收尾", 1)

    print(f"最终：{describe(state['score'])}", flush=True)


if __name__ == "__main__":
    main()
