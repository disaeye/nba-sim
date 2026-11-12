#!/usr/bin/env python3
"""带种群的定向门禁收敛。

一维扫描显示 `backcourt_margin_ft` 在 4.0–4.5 能把虚假八秒失误消掉，
但会引出若干轻微越界。单一起点的贪心搜索会一直退回 margin=2.0 的
「1 条越界」局部极值，因此这里保留多个 margin 起点并行演化，只按
越界深度（各判据超出自身带宽的部分之和）排序，深度为 0 即 39 条全通过。

每个 worker 持有独立校准目录，`sim2 --gate-only` 读自己的 policy.json。

用法：

    python3 -u scripts/calibrate/beam_gates.py --rounds 150
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import random
import shutil
import subprocess
from pathlib import Path

os.environ.setdefault("RAYON_NUM_THREADS", "1")

ROOT = Path(__file__).resolve().parents[2]
CALIBRATION = ROOT / "data" / "calibration"
TACTICS = ROOT / "data" / "tactics"
WORK = ROOT / ".work" / "evidence" / "beam-search"
BINARY = ROOT / "target" / "release" / "sim2"
POLICY = CALIBRATION / "policy.json"
FINAL_LOG = ROOT / ".work" / "evidence" / "gate-final.log"
WORKERS = 4

MARGIN_STARTS = [3.75, 4.0, 4.25, 4.5]

# (下界, 上界, 步长)。步长刻意小：按地址抽样的引擎对大改动会重排
# 整条抽样链，只有小步才保留局部可比较性。
LEVERS = {
    "cut_lane_scale": (0.30, 0.65, 0.02),
    "cut_tendency": (0.0, 0.08, 0.005),
    "cut_positional_bonus": (0.0, 0.02, 0.0015),
    "shoot_putback_bonus": (0.55, 1.05, 0.03),
    "finish_min_distance_ft": (2.3, 3.8, 0.1),
    "finish_priority": (0.25, 0.60, 0.02),
    "drive_base": (1.35, 2.0, 0.05),
    "post_base": (0.40, 0.75, 0.02),
    "gather_travel_rate": (0.005, 0.055, 0.003),
    "shot_aim_sigma_scale": (0.95, 1.40, 0.02),
    "seconds_per_possession": (16.0, 20.0, 0.25),
    "pass_catch_steal": (0.35, 0.65, 0.015),
    "steal_gamble_base": (0.50, 1.00, 0.04),
    "off_ball_hold_rate": (0.002, 0.009, 0.0005),
    "hand_check_rate": (0.001, 0.008, 0.0008),
    "reach_in_foul_rate": (0.008, 0.030, 0.002),
    "play_call_rate": (0.70, 0.86, 0.008),
    "shooting_foul_rate_two_point": (0.12, 0.26, 0.008),
    "shooting_foul_rate_three_point": (0.02, 0.08, 0.005),
    "and_one_rate": (0.12, 0.26, 0.008),
    "transition_window_seconds": (8.0, 11.5, 0.2),
    "shot_distance_decay_three_scale_ft": (8.5, 12.0, 0.2),
    "possession_setup_seconds": (7.8, 9.2, 0.15),
    "backcourt_margin_ft": (3.5, 5.5, 0.15),
    "drive_finish_distance_ft": (4.5, 7.5, 0.2),
}
KEYS = sorted(LEVERS)


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
    processes = []
    for index, policy in enumerate(candidates):
        path = WORK / f"worker-{index}" / "calibration" / "policy.json"
        path.write_text(
            json.dumps(policy, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
        )
        processes.append(
            subprocess.Popen(
                [str(BINARY), "--gate-only", "--calibration", str(WORK / f"worker-{index}" / "calibration")],
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
        for line in output.splitlines():
            if line.startswith("GATE "):
                report = json.loads(line[5:])
                break
        if report is None:
            raise RuntimeError(f"gate-only failed (exit {process.returncode}): {output[-1200:]}")
        results.append((depth_of(report), report, output))
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


def perturb(policy, rng, width):
    trial = copy.deepcopy(policy)
    for key in rng.sample(KEYS, rng.choice((1, 1, 2, 2, 3))):
        low, high, step = LEVERS[key]
        step *= width
        current = get_value(trial, key)
        candidate = current + rng.choice((-step, step))
        set_value(trial, key, min(max(candidate, low), high))
    return trial


def main():
    parser = argparse.ArgumentParser(description="带种群的门禁参数收敛")
    parser.add_argument("--rounds", type=int, default=150)
    parser.add_argument("--seed", type=int, default=20261110)
    args = parser.parse_args()

    prepare()
    rng = random.Random(args.seed)
    base = json.loads(POLICY.read_text(encoding="utf-8"))

    # 种群初始化：每个成员固定在一个 margin 起点，避免全部退回局部极值。
    population = []
    for margin in MARGIN_STARTS:
        policy = copy.deepcopy(base)
        policy["geometry"]["backcourt_margin_ft"] = margin
        population.append(policy)
    initial = evaluate(population)
    beam = [(score, policy, stdout) for policy, (score, _, stdout) in zip(population, initial)]
    beam.sort(key=lambda entry: entry[0])
    print(
        "起始深度：" + " ".join(f"{entry[0]:.5f}" for entry in beam),
        flush=True,
    )

    best = beam[0]
    for round_index in range(1, args.rounds + 1):
        parents = [entry[1] for entry in beam]
        children = []
        for index, parent in enumerate(parents):
            # 前两个成员用小步精修，后两个用较大步跨出局部极值。
            children.append(perturb(parent, rng, 1.0 if index < 2 else 2.5))
        measured = evaluate(children)
        candidates = list(beam) + [
            (score, child, stdout)
            for child, (score, _, stdout) in zip(children, measured)
        ]
        # 按 margin 分桶保留多样性，避免四个成员全部收敛到同一个起点。
        buckets: dict[str, list] = {}
        for entry in candidates:
            margin = round(entry[1]["geometry"]["backcourt_margin_ft"] * 4) / 4
            buckets.setdefault(str(margin), []).append(entry)
        chosen = []
        for key in sorted(buckets, key=lambda value: min(entry[0] for entry in buckets[value])):
            local = sorted(buckets[key], key=lambda entry: entry[0])
            chosen.extend(local[:2])
        chosen.sort(key=lambda entry: entry[0])
        beam = chosen[:WORKERS]
        if beam[0][0] < best[0]:
            best = beam[0]
        if round_index % 10 == 0 or beam[0][0] == 0.0:
            report = json.loads(
                next(line[5:] for line in beam[0][2].splitlines() if line.startswith("GATE "))
            )
            bad = [layer["criterion"] for layer in report["layers"] if not layer["passed"]]
            print(
                f"[{round_index:3d}] 最优深度 {beam[0][0]:.5f} 失败 {len(bad)} 条 {bad}",
                flush=True,
            )
        if beam[0][0] == 0.0:
            break

    POLICY.write_text(
        json.dumps(best[1], indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    final = evaluate([best[1]] * WORKERS)[0]
    FINAL_LOG.write_text(best[2] if best[0] == 0.0 else final[2], encoding="utf-8")
    print(
        f"最终：失败 {sum(1 for layer in final[1]['layers'] if not layer['passed'])} 条，"
        f"深度 {final[0]:.5f}，几何 {final[1]['geometric_violations']}，"
        f"语境 {final[1]['context_violations']}，通过 {final[1]['passed']}",
        flush=True,
    )
    for layer in final[1]["layers"]:
        if not layer["passed"]:
            print(f"  {layer['criterion']}: {layer['relative_error']:+.4f}", flush=True)
    print(
        f"margin {best[1]['geometry']['backcourt_margin_ft']} "
        f"setup {best[1]['geometry']['possession_setup_seconds']}",
        flush=True,
    )


if __name__ == "__main__":
    main()
